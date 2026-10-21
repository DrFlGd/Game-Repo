//! A device's side of Game Repo Server (docs/PLAN.md, "Phase 9a design"): finding the
//! server and pairing with it, asking for a plan for some games, fetching its files into
//! ES-DE's folders (three at once, each resumed where it stopped and checked by its SHA-1
//! before it's put in place), merging the gamelist entries and custom collection lines
//! into the device's own, and keeping a record of what it holds
//! (`ES-DE/gamerepo/device.json`), so games can be removed with their files and entries
//! while sets other games need stay. Nothing is written outside ES-DE's folders.
//!
//! `gamerepo-cli device …` is built on it, standing in for the device client with a folder
//! that holds `ES-DE/` and `ROMs/`; 9b's client is built on it too.

pub mod app;
pub mod catalogue;
pub mod client;
pub mod installs;
pub mod pcgames;
pub mod queue;
pub mod removing;
pub mod scan;
pub mod steam;

pub use client::{Server, Stopped, Watch};
/// What it and the server say to each other, and where releases are published
pub use gamerepo_wire as wire;

use anyhow::{anyhow, bail, Context, Result};
use gamerepo_wire::api::{self, Held, HeldFile, Holding, Info, PlanAsk, Record};
use gamerepo_wire::{gamelist, merge_lines, Left, Plan};
use sha1::Digest;
use std::collections::{BTreeMap, HashSet};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The record's format.
pub const RECORD: u32 = 1;
/// A file written whole is first kept under its name and this.
pub const PART: &str = ".gamerepo-part";
/// The files of games still coming are kept in this folder of their place until every one
/// is in, so ES-DE never lists half a game (an `.m3u` folder with one disc of two, say).
pub const COMING: &str = ".gamerepo-coming";

/// A device: its ES-DE folders (its places, the ES-DE layout's own: `roms`, `esde`, `media`),
/// and on Windows and Linux its PC games folders (`pc`, `pcmedia`, and `startmenu` on
/// Windows, D111).
pub struct Device {
    pub home: PathBuf,
    pub places: BTreeMap<String, PathBuf>,
    /// The app's data folder, where the Steam shortcuts it wants are kept (D109)
    pub data: Option<PathBuf>,
}

fn xml_attr_unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// A setting of ES-DE's `es_settings.xml` (`<string name="ROMDirectory" value="…" />`).
fn es_setting(xml: &str, name: &str) -> Option<String> {
    let at = xml.find(&format!("name=\"{name}\""))?;
    let tag_start = xml[..at].rfind('<')?;
    let tag_end = at + xml[at..].find('>')?;
    let tag = &xml[tag_start..tag_end];
    let v = tag.split("value=\"").nth(1)?.split('"').next()?;
    Some(xml_attr_unescape(v))
}

impl Device {
    /// The device whose home folder is `home`: ES-DE's folder is `home/ES-DE`, its ROMs
    /// folder and media folder as its settings say (else `home/ROMs` and
    /// `ES-DE/downloaded_media`), as ES-DE finds them (C3).
    pub fn open(home: &Path) -> Result<Device> {
        Device::open_esde(&home.join("ES-DE"), home)
    }

    /// The device whose ES-DE folder is `esde` (one found elsewhere, or set in the client's
    /// Settings, C3), `~` in its settings being `home`.
    pub fn open_esde(esde: &Path, home: &Path) -> Result<Device> {
        let esde = esde.to_path_buf();
        // ES-DE 3 keeps them in settings/, ES-DE 2 beside its gamelists
        let settings = [esde.join("settings").join("es_settings.xml"), esde.join("es_settings.xml")].into_iter().find(|p| p.is_file()).unwrap_or_else(|| esde.join("settings").join("es_settings.xml"));
        if !settings.is_file() {
            bail!("{} has no ES-DE settings ({}): start ES-DE on it once first.", esde.display(), settings.display());
        }
        let xml = std::fs::read_to_string(&settings)?;
        let expand = |v: String| -> PathBuf {
            let v = v.replace("%ESPATH%", &esde.to_string_lossy());
            match v.strip_prefix('~') {
                Some(rest) => home.join(rest.trim_start_matches(['/', '\\'])),
                None => PathBuf::from(v),
            }
        };
        let roms = es_setting(&xml, "ROMDirectory").filter(|v| !v.trim().is_empty()).map(expand).unwrap_or_else(|| home.join("ROMs"));
        let media = es_setting(&xml, "MediaDirectory").filter(|v| !v.trim().is_empty()).map(expand).unwrap_or_else(|| esde.join("downloaded_media"));
        let places = BTreeMap::from([("roms".to_string(), roms), ("esde".to_string(), esde), ("media".to_string(), media)]);
        Ok(Device { home: home.to_path_buf(), places, data: None })
    }

    /// Where a plan's path in a place is on the device: never outside the place.
    pub fn at(&self, place: &str, path: &str) -> Result<PathBuf> {
        let dir = self.places.get(place).ok_or_else(|| anyhow!("This device has no place {place}."))?;
        let rel = Path::new(path);
        if path.is_empty() || rel.components().any(|c| !matches!(c, Component::Normal(_))) {
            bail!("{path} isn't a path inside {place}.");
        }
        Ok(dir.join(rel))
    }

    fn gamerepo(&self) -> PathBuf {
        self.places["esde"].join("gamerepo")
    }

    pub fn record(&self) -> Record {
        std::fs::read(self.gamerepo().join("device.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Record { format: RECORD, ..Default::default() })
    }

    fn save_record(&self, r: &Record) -> Result<()> {
        write_whole(&self.gamerepo().join("device.json"), &serde_json::to_vec_pretty(r)?)
    }

    /// The server this stand-in is paired with, kept in `ES-DE/gamerepo/server.json` (a real
    /// device keeps its token in the system's secure storage).
    pub fn server(&self) -> Result<Server> {
        let p = self.gamerepo().join("server.json");
        serde_json::from_slice(&std::fs::read(&p).map_err(|_| anyhow!("This device isn't paired with a server yet."))?).with_context(|| format!("{} can't be read", p.display()))
    }

    pub fn keep_server(&self, s: &Server) -> Result<()> {
        write_whole(&self.gamerepo().join("server.json"), &serde_json::to_vec_pretty(s)?)
    }

    pub fn forget_server(&self) -> Result<()> {
        let _ = std::fs::remove_file(self.gamerepo().join("server.json"));
        Ok(())
    }

    /// What the device holds, and the free space where its games go.
    pub fn holding(&self) -> Holding {
        let r = self.record();
        let mut seen = HashSet::new();
        let size = r.games.iter().flat_map(|g| &g.files).filter(|f| seen.insert((f.place.clone(), f.path.clone()))).map(|f| f.size).sum();
        let roms = &self.places["roms"];
        let free = fs4::available_space(existing(roms)).unwrap_or(0);
        Holding { games: r.games.len() as u64, size, free }
    }
}

/// The nearest folder of `p` that's there.
fn existing(p: &Path) -> &Path {
    let mut p = p;
    while !p.exists() {
        match p.parent() {
            Some(x) => p = x,
            None => break,
        }
    }
    p
}

/// Whether two folders are on the same drive, so their free space is the same space.
fn same_drive(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        matches!((std::fs::metadata(existing(a)), std::fs::metadata(existing(b))), (Ok(x), Ok(y)) if x.dev() == y.dev())
    }
    #[cfg(not(unix))]
    {
        let root = |p: &Path| std::fs::canonicalize(existing(p)).ok().and_then(|c| c.components().next().map(|c| c.as_os_str().to_ascii_lowercase()));
        root(a).is_some() && root(a) == root(b)
    }
}

/// A file written whole, through a temporary name.
fn write_whole(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_file_name(format!("{}{PART}", path.file_name().unwrap_or_default().to_string_lossy()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// A file written whole that only its user can read (mode 600 on Linux).
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_file_name(format!("{}{PART}", path.file_name().unwrap_or_default().to_string_lossy()));
    let _ = std::fs::remove_file(&tmp);
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    std::io::Write::write_all(&mut o.open(&tmp)?, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub fn sha1_file(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = sha1::Sha1::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

/// What's asked for besides the games.
#[derive(Clone, Debug)]
pub struct AddOptions {
    pub media: Vec<String>,
    pub manuals: bool,
    /// The device's longest screen side
    pub size: u32,
    pub collection: String,
    pub bios: bool,
    /// Files fetched at once
    pub at_once: usize,
    /// Only the games' media and manuals, for games the device has (D102)
    pub media_only: bool,
    /// Games the device has under names of its own (from elsewhere, D102): each one's key,
    /// and its files in the ROMs folder, the first ES-DE's, which the record takes in
    pub adopt: Vec<(String, Vec<HeldFile>)>,
    /// PC games' pictures for Steam's grid too (Linux, D112)
    pub steam_art: bool,
}

impl Default for AddOptions {
    fn default() -> Self {
        Self { media: vec!["cover".into()], manuals: false, size: 0, collection: String::new(), bios: false, at_once: 3, media_only: false, adopt: vec![], steam_art: false }
    }
}

impl AddOptions {
    /// What the server is asked for, for these games.
    pub fn ask(&self, keys: &[String]) -> PlanAsk {
        PlanAsk {
            games: keys.to_vec(),
            media: self.media.clone(),
            manuals: self.manuals,
            size: self.size,
            collection: self.collection.clone(),
            bios: self.bios,
            media_only: self.media_only,
            named: self.adopt.iter().filter_map(|(k, files)| Some((k.clone(), files.first()?.path.clone()))).collect(),
            steam_art: self.steam_art,
            ..Default::default()
        }
    }
}

/// A file came through damaged twice (or changed in the library while it came): the plan
/// is asked for again.
#[derive(Debug)]
pub struct Damaged(pub String);

impl std::fmt::Display for Damaged {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} came through damaged twice.", self.0)
    }
}

impl std::error::Error for Damaged {}

/// The record and the lists are changed by one thing at a time (a game put in place, a
/// removal).
pub(crate) static WRITING: Mutex<()> = Mutex::new(());

#[derive(Debug, Default)]
pub struct Added {
    pub games: usize,
    pub files: usize,
    /// Fetched now (a file resumed counts what was left of it)
    pub bytes: u64,
    /// There already, as the plan has them
    pub there: usize,
    pub left: Vec<Left>,
    pub notes: Vec<String>,
}

/// The catalogue game's files (its own, and shared ones it needs) of a plan.
fn files_of<'a>(plan: &'a Plan, key: &str, system: &str) -> Vec<&'a gamerepo_wire::File> {
    plan.files
        .iter()
        .filter(|f| f.key == key || (f.key.is_empty() && (f.needed_by.iter().any(|k| k == key) || (f.needed_by.is_empty() && f.system == system))))
        .collect()
}

/// The plan's files the device doesn't have as the record has them (their places in
/// `plan.files`), and how many it has.
pub fn to_fetch(d: &Device, plan: &Plan) -> Result<(Vec<usize>, usize)> {
    let rec = d.record();
    let had: HashSet<(String, String, String)> = rec.games.iter().flat_map(|g| &g.files).map(|f| (f.place.clone(), f.path.clone(), f.sha1.clone())).collect();
    let (mut todo, mut there) = (vec![], 0);
    for (i, f) in plan.files.iter().enumerate() {
        let dest = d.at(&f.place, &f.path)?;
        if f.sha1.as_ref().is_some_and(|h| had.contains(&(f.place.clone(), f.path.clone(), h.clone()))) && std::fs::metadata(&dest).is_ok_and(|m| m.len() == f.size) {
            there += 1;
        } else {
            todo.push(i);
        }
    }
    Ok((todo, there))
}

/// Where a file of a plan is kept while it comes.
pub fn part_of(d: &Device, f: &gamerepo_wire::File) -> Result<PathBuf> {
    d.at(&f.place, &f.path)?;
    Ok(d.places[&f.place].join(COMING).join(&f.path))
}

/// A part's ETag, kept beside it.
fn etag_of(part: &Path) -> PathBuf {
    part.with_extension(format!("{}.etag", part.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default()))
}

/// What's kept of a plan's files still coming, removed (a game taken off the queue).
pub fn drop_parts(d: &Device, plan: &Plan) {
    for f in &plan.files {
        if let Ok(part) = part_of(d, f) {
            let _ = std::fs::remove_file(&part);
            let _ = std::fs::remove_file(etag_of(&part));
            tidy_coming(d, &f.place, &part);
        }
    }
}

/// The folders of `.gamerepo-coming/` a part was in, removed while they're empty.
fn tidy_coming(d: &Device, place: &str, part: &Path) {
    let top = d.places[place].join(COMING);
    let mut at = part.parent().map(Path::to_path_buf);
    while let Some(dir) = at {
        if !dir.starts_with(&top) || std::fs::remove_dir(&dir).is_err() || dir == top {
            break;
        }
        at = dir.parent().map(Path::to_path_buf);
    }
}

/// Add games to the device: a plan for them, its files fetched and put in place, the
/// entries merged into its gamelists and collection, and the record kept.
pub fn add(s: &Server, d: &Device, keys: &[String], o: &AddOptions, progress: &mut dyn FnMut(&str)) -> Result<Added> {
    let plan: Plan = s.call("POST", "/v1/plans", Some(&o.ask(keys)))?;
    add_planned(s, d, &plan, keys, o, Watch::default(), progress)
}

/// [`add`] with the plan asked for already, watched by the queue (D99): a file that stops
/// is kept to carry on from, and the games are put in the lists and the record only once
/// every file is in.
pub fn add_planned(s: &Server, d: &Device, plan: &Plan, keys: &[String], o: &AddOptions, watch: Watch, progress: &mut dyn FnMut(&str)) -> Result<Added> {
    let (todo, there) = to_fetch(d, plan)?;
    let mut out = Added { left: plan.left.clone(), notes: plan.notes.clone(), there, ..Default::default() };
    progress(&format!("{} files to fetch, {} there already", todo.len(), out.there));
    let sha1s: Arc<Mutex<BTreeMap<usize, String>>> = Arc::default();
    let queue = Arc::new(Mutex::new(todo.clone()));
    let fetched = Arc::new(Mutex::new(0u64));
    let failed: Arc<Mutex<Vec<(String, anyhow::Error)>>> = Arc::default();
    std::thread::scope(|sc| {
        for _ in 0..o.at_once.clamp(1, api::DOWNLOADS) {
            let (queue, sha1s, fetched, failed) = (queue.clone(), sha1s.clone(), fetched.clone(), failed.clone());
            sc.spawn(move || loop {
                let Some(i) = queue.lock().unwrap().pop() else { break };
                let f = &plan.files[i];
                match fetch_one(s, d, f, watch) {
                    Ok((n, h)) => {
                        *fetched.lock().unwrap() += n;
                        sha1s.lock().unwrap().insert(i, h);
                    }
                    Err(e) => failed.lock().unwrap().push((f.path.clone(), e)),
                }
            });
        }
    });
    let mut failed = std::mem::take(&mut *failed.lock().unwrap());
    if !failed.is_empty() {
        // a stop or a refusal says why for them all
        if let Some(at) = failed.iter().position(|(_, e)| e.is::<client::Stopped>()) {
            return Err(failed.remove(at).1);
        }
        if let Some(at) = failed.iter().position(|(_, e)| e.is::<client::Refused>() || e.is::<Damaged>()) {
            return Err(failed.remove(at).1);
        }
        let n = failed.len();
        let (path, e) = failed.remove(0);
        return Err(e.context(format!("{n} of the files didn't come (the first, {path})")));
    }
    out.files = todo.len();
    out.bytes = *fetched.lock().unwrap();
    let sha1s = sha1s.lock().unwrap().clone();
    let _writing = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    // every file in, each put in place
    for &i in &todo {
        let f = &plan.files[i];
        let (dest, part) = (d.at(&f.place, &f.path)?, part_of(d, f)?);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if dest.exists() {
            std::fs::remove_file(&dest)?;
        }
        std::fs::rename(&part, &dest)?;
        let _ = std::fs::remove_file(etag_of(&part));
        tidy_coming(d, &f.place, &part);
    }
    // the gamelists and the collection, merged with the device's own
    for l in &plan.lists {
        let p = d.at(&l.place, &l.path)?;
        let old = std::fs::read_to_string(&p).ok();
        if let Some(new) = gamelist::merge_entries(old.as_deref(), &l.entries) {
            write_whole(&p, new.as_bytes())?;
        }
    }
    for c in &plan.collections {
        let p = d.at(&c.place, &c.path)?;
        let old = std::fs::read_to_string(&p).ok();
        if let Some(new) = merge_lines(old.as_deref(), &c.lines) {
            write_whole(&p, new.as_bytes())?;
        }
    }
    // the record: each game with its files, entries and collection lines
    let mut rec = d.record();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    rec.server = s.server.clone();
    for key in keys {
        let g: serde_json::Value = s.get(&format!("/v1/games/{}", encode(key))).unwrap_or_default();
        let system = g["game"]["system"].as_str().map(String::from).unwrap_or_else(|| key.split('/').next().unwrap_or("").to_string());
        let mut files: Vec<HeldFile> = o.adopt.iter().filter(|(k, _)| k == key).flat_map(|(_, f)| f.clone()).collect();
        for f in files_of(plan, key, &system) {
            let i = plan.files.iter().position(|x| std::ptr::eq(x, f)).unwrap();
            let sha1 = sha1s.get(&i).cloned().or_else(|| f.sha1.clone()).unwrap_or_default();
            files.push(HeldFile { place: f.place.clone(), path: f.path.clone(), size: f.size, sha1, role: f.role.clone(), needed_by: f.needed_by.clone() });
        }
        // a PC game's shortcut (D112): a Start menu one is the game's file too
        if let Some(p) = plan.programs.iter().find(|p| p.key == *key) {
            match pcgames::shortcut(d, d.data.as_deref(), p) {
                Ok(Some(f)) => {
                    files.retain(|x| (&x.place, &x.path) != (&f.place, &f.path));
                    files.push(f);
                }
                Ok(None) => {}
                Err(e) => out.notes.push(format!("{}'s shortcut couldn't be made: {e:#}", p.name)),
            }
        }
        let entries: Vec<(String, String, String)> = plan.lists.iter().flat_map(|l| l.entries.iter().filter(|e| e.key == *key).map(|e| (l.place.clone(), l.path.clone(), e.path.clone()))).collect();
        let title = g["game"]["title"].as_str().unwrap_or(key).to_string();
        match rec.games.iter_mut().find(|h| h.key == *key).filter(|_| o.media_only) {
            // media for a game held already: its files and entries joined to what it has
            Some(h) => {
                for f in files {
                    match h.files.iter_mut().find(|x| (&x.place, &x.path) == (&f.place, &f.path)) {
                        // a file of the game's it has already, as the record has it
                        Some(_) if f.sha1.is_empty() => {}
                        Some(x) => *x = f,
                        None => h.files.push(f),
                    }
                }
                for e in entries {
                    if !h.entries.contains(&e) {
                        h.entries.push(e);
                    }
                }
            }
            None => {
                rec.games.retain(|h| h.key != *key);
                rec.games.push(Held { key: key.clone(), title, system, added: now, files, entries });
            }
        }
        out.games += 1;
    }
    // the collection's lines are the games' too, by the file or folder ES-DE starts each from
    if !plan.collections.is_empty() {
        let lines: Vec<(String, String, String)> = plan.collections.iter().flat_map(|c| c.lines.iter().map(|l| (c.place.clone(), c.path.clone(), l.clone()))).collect();
        let mut extra: BTreeMap<String, Vec<(String, String, String)>> = BTreeMap::new();
        for h in rec.games.iter().filter(|h| keys.contains(&h.key)) {
            let starts: HashSet<String> = h.files.iter().filter(|f| f.place == "roms").flat_map(|f| ancestors(&f.path)).collect();
            for (place, path, line) in &lines {
                let rel = line.trim().replace('\\', "/");
                let rel = rel.split_once("%ROMPATH%/").map(|(_, r)| r.to_string()).unwrap_or(rel);
                if starts.contains(&rel) {
                    extra.entry(h.key.clone()).or_default().push((place.clone(), path.clone(), format!("line:{}", line.trim())));
                }
            }
        }
        for h in rec.games.iter_mut() {
            if let Some(v) = extra.remove(&h.key) {
                h.entries.extend(v);
            }
        }
    }
    rec.format = RECORD;
    d.save_record(&rec)?;
    let _ = s.call::<serde_json::Value>("PUT", "/v1/device", Some(&d.holding()));
    Ok(out)
}

/// Every path a file's path is in, and itself (`psx/Q.m3u/Q.m3u`: `psx`, `psx/Q.m3u`, `psx/Q.m3u/Q.m3u`).
fn ancestors(path: &str) -> Vec<String> {
    let parts: Vec<&str> = path.split('/').collect();
    (1..=parts.len()).map(|n| parts[..n].join("/")).collect()
}

fn encode(s: &str) -> String {
    let mut out = String::new();
    for c in s.bytes() {
        if c.is_ascii_alphanumeric() || b"-._~/!$&'()*+,;=:@".contains(&c) {
            out.push(c as char);
        } else {
            out.push_str(&format!("%{c:02X}"));
        }
    }
    out
}

/// One file fetched and checked against its SHA-1, left in `.gamerepo-coming/` until its
/// game's every file is in: the bytes fetched and its SHA-1.
fn fetch_one(s: &Server, d: &Device, f: &gamerepo_wire::File, watch: Watch) -> Result<(u64, String)> {
    let part = part_of(d, f)?;
    // one that came whole before a pause is only checked
    let whole = f.size > 0 && std::fs::metadata(&part).is_ok_and(|m| m.len() == f.size);
    let mut n = if whole { 0 } else { s.fetch_watched(&f.url, &part, None, watch)? };
    let want = s.sha1_of(&f.url, Duration::from_secs(600))?;
    let mut got = sha1_file(&part)?;
    if got != want {
        // once more from the start (a resume of a file that changed in between)
        watch.take_back(std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0));
        let _ = std::fs::remove_file(&part);
        n += s.fetch_watched(&f.url, &part, None, watch)?;
        got = sha1_file(&part)?;
        if got != s.sha1_of(&f.url, Duration::from_secs(600))? {
            let _ = std::fs::remove_file(&part);
            return Err(Damaged(f.path.clone()).into());
        }
    }
    Ok((n, got))
}

#[derive(Debug, Default)]
pub struct Removed {
    pub games: usize,
    pub files: usize,
    /// Kept for other games that need them
    pub kept: usize,
}

/// Remove games from the device: their files (but not those another game it holds needs),
/// their gamelist entries and collection lines, set aside first and then deleted
/// ([`removing`]). Only what the record lists is touched.
pub fn remove(d: &Device, keys: &[String]) -> Result<Removed> {
    let rec = d.record();
    let unknown: Vec<&str> = keys.iter().filter(|k| !rec.games.iter().any(|g| &g.key == *k)).map(|k| k.as_str()).collect();
    if !unknown.is_empty() {
        bail!("This device doesn't hold {} from the server.", unknown.join(", "));
    }
    let asks: Vec<removing::Ask> = keys.iter().map(|k| removing::Ask { key: k.clone(), ..Default::default() }).collect();
    let r = removing::remove(d, &[], &asks)?;
    Ok(Removed { games: r.keys.len(), files: r.files.len(), kept: r.kept.len() })
}

/// Each server heard on the network (D91) until `wait` is up or `each` says that's
/// enough: what it is, and every address it announces (IPv4 first).
static ON_BROWSE: std::sync::OnceLock<Box<dyn Fn(bool) + Send + Sync>> = std::sync::OnceLock::new();
static BROWSING: std::sync::Mutex<usize> = std::sync::Mutex::new(0);

/// Told `true` as looking for libraries on the network starts and `false` once nothing
/// looks any more (Android drops multicast unless an app holds its lock meanwhile).
pub fn on_browse(f: Box<dyn Fn(bool) + Send + Sync>) {
    let _ = ON_BROWSE.set(f);
}

/// While one lives, something's looking on the network (counted, as two can at once).
struct Browsing;

impl Browsing {
    fn start() -> Browsing {
        let mut n = BROWSING.lock().unwrap_or_else(|e| e.into_inner());
        *n += 1;
        if *n == 1 {
            if let Some(f) = ON_BROWSE.get() {
                f(true);
            }
        }
        Browsing
    }
}

impl Drop for Browsing {
    fn drop(&mut self) {
        let mut n = BROWSING.lock().unwrap_or_else(|e| e.into_inner());
        *n -= 1;
        if *n == 0 {
            if let Some(f) = ON_BROWSE.get() {
                f(false);
            }
        }
    }
}

fn browse(wait: Duration, mut each: impl FnMut(Info, Vec<String>) -> bool) -> Result<()> {
    let _browsing = Browsing::start();
    let daemon = mdns_sd::ServiceDaemon::new()?;
    let rx = daemon.browse(api::SERVICE)?;
    let until = std::time::Instant::now() + wait;
    loop {
        let left = until.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            break;
        }
        match rx.recv_timeout(left) {
            Ok(mdns_sd::ServiceEvent::ServiceResolved(r)) => {
                let txt: Vec<(String, String)> = r.txt_properties.iter().map(|p| (p.key().to_string(), p.val_str().to_string())).collect();
                let Some(info) = Info::from_txt(&txt) else { continue };
                let mut ips: Vec<std::net::IpAddr> = r.addresses.iter().map(|a| a.to_ip_addr()).collect();
                ips.sort_by_key(|ip| (!ip.is_ipv4(), ip.is_loopback()));
                let addresses = ips
                    .iter()
                    .map(|ip| match ip {
                        std::net::IpAddr::V6(v6) => format!("[{v6}]:{}", r.port),
                        v4 => format!("{v4}:{}", r.port),
                    })
                    .collect();
                if each(info, addresses) {
                    break;
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    let _ = daemon.stop_browse(api::SERVICE);
    let _ = daemon.shutdown();
    Ok(())
}

/// The servers announcing themselves on the network (D91), each with its address.
pub fn find(wait: Duration) -> Result<Vec<(Info, String)>> {
    let mut out: Vec<(Info, String)> = vec![];
    browse(wait, |info, addresses| {
        if let Some(address) = addresses.into_iter().next() {
            if !out.iter().any(|(i, _)| i.server == info.server) {
                out.push((info, address));
            }
        }
        false
    })?;
    Ok(out)
}

/// Where the server `id` answers now: each address it announces is tried with `answers`
/// as it's heard, and the first that answers is given back as soon as it does (None once
/// `wait` is up).
pub fn find_server(id: &str, wait: Duration, mut answers: impl FnMut(&str) -> bool) -> Result<Option<String>> {
    let mut found = None;
    let mut tried = std::collections::HashSet::new();
    browse(wait, |info, addresses| {
        if info.server == id {
            found = addresses.into_iter().find(|a| tried.insert(a.clone()) && answers(a));
        }
        found.is_some()
    })?;
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn es_des_folders_are_read_from_its_settings() {
        let home = std::env::temp_dir().join(format!("gamerepo-device-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("ES-DE/settings")).unwrap();
        assert!(Device::open(&home.join("nothing")).is_err());
        std::fs::write(home.join("ES-DE/settings/es_settings.xml"), "<?xml version=\"1.0\"?>\n<string name=\"MediaDirectory\" value=\"\" />\n<string name=\"ROMDirectory\" value=\"~/Games &amp; More\" />\n").unwrap();
        let d = Device::open(&home).unwrap();
        assert_eq!(d.places["roms"], home.join("Games & More"));
        assert_eq!(d.places["media"], home.join("ES-DE").join("downloaded_media"));
        assert!(d.at("roms", "psx/A.cue").is_ok());
        assert!(d.at("roms", "../outside.txt").is_err());
        assert!(d.at("roms", "/etc/passwd").is_err());
        assert!(d.at("saves", "x").is_err());
        let _ = std::fs::remove_dir_all(&home);
    }
}
