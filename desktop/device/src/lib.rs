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

pub mod client;

pub use client::Server;

use anyhow::{anyhow, bail, Context, Result};
use gamerepo_wire::api::{self, Held, HeldFile, Holding, Info, PlanAsk, Record};
use gamerepo_wire::{gamelist, merge_lines, remove_lines, Left, Plan};
use sha1::Digest;
use std::collections::{BTreeMap, HashSet};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The record's format.
pub const RECORD: u32 = 1;
/// A file still coming is kept under its name and this.
pub const PART: &str = ".gamerepo-part";

/// A device: its ES-DE folders (its places, the ES-DE layout's own: `roms`, `esde`, `media`).
pub struct Device {
    pub home: PathBuf,
    pub places: BTreeMap<String, PathBuf>,
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
        let esde = home.join("ES-DE");
        let settings = esde.join("settings").join("es_settings.xml");
        if !settings.is_file() {
            bail!("{} has no ES-DE settings ({}): start ES-DE on it once first.", home.display(), settings.display());
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
        Ok(Device { home: home.to_path_buf(), places })
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
}

impl Default for AddOptions {
    fn default() -> Self {
        Self { media: vec!["cover".into()], manuals: false, size: 0, collection: String::new(), bios: false, at_once: 3 }
    }
}

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

/// Add games to the device: a plan for them, its files fetched and put in place, the
/// entries merged into its gamelists and collection, and the record kept.
pub fn add(s: &Server, d: &Device, keys: &[String], o: &AddOptions, progress: &mut dyn FnMut(&str)) -> Result<Added> {
    let ask = PlanAsk { games: keys.to_vec(), media: o.media.clone(), manuals: o.manuals, size: o.size, collection: o.collection.clone(), bios: o.bios, ..Default::default() };
    let plan: Plan = s.call("POST", "/v1/plans", Some(&ask))?;
    let mut rec = d.record();
    let had: HashSet<(String, String, String)> = rec.games.iter().flat_map(|g| &g.files).map(|f| (f.place.clone(), f.path.clone(), f.sha1.clone())).collect();
    // what's to be fetched: those not there as the record has them
    let mut todo = vec![];
    let mut out = Added { left: plan.left.clone(), notes: plan.notes.clone(), ..Default::default() };
    for (i, f) in plan.files.iter().enumerate() {
        let dest = d.at(&f.place, &f.path)?;
        let there = f.sha1.as_ref().is_some_and(|h| had.contains(&(f.place.clone(), f.path.clone(), h.clone()))) && std::fs::metadata(&dest).is_ok_and(|m| m.len() == f.size);
        if there {
            out.there += 1;
        } else {
            todo.push(i);
        }
    }
    progress(&format!("{} files to fetch, {} there already", todo.len(), out.there));
    let sha1s: Arc<Mutex<BTreeMap<usize, String>>> = Arc::default();
    let queue = Arc::new(Mutex::new(todo.clone()));
    let fetched = Arc::new(Mutex::new(0u64));
    let failed: Arc<Mutex<Vec<String>>> = Arc::default();
    std::thread::scope(|sc| {
        for _ in 0..o.at_once.clamp(1, api::DOWNLOADS) {
            let (queue, sha1s, fetched, failed) = (queue.clone(), sha1s.clone(), fetched.clone(), failed.clone());
            let plan = &plan;
            sc.spawn(move || loop {
                let Some(i) = queue.lock().unwrap().pop() else { break };
                let f = &plan.files[i];
                match fetch_one(s, d, f) {
                    Ok((n, h)) => {
                        *fetched.lock().unwrap() += n;
                        sha1s.lock().unwrap().insert(i, h);
                    }
                    Err(e) => failed.lock().unwrap().push(format!("{}: {e:#}", f.path)),
                }
            });
        }
    });
    let failed = failed.lock().unwrap().clone();
    if !failed.is_empty() {
        bail!("{} of the files didn't come: {}", failed.len(), failed.join("; "));
    }
    out.files = todo.len();
    out.bytes = *fetched.lock().unwrap();
    let sha1s = sha1s.lock().unwrap().clone();
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
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    rec.server = s.server.clone();
    for key in keys {
        let g: serde_json::Value = s.get(&format!("/v1/games/{}", encode(key))).unwrap_or_default();
        let system = g["game"]["system"].as_str().map(String::from).unwrap_or_else(|| key.split('/').next().unwrap_or("").to_string());
        let mut files: Vec<HeldFile> = vec![];
        for f in files_of(&plan, key, &system) {
            let i = plan.files.iter().position(|x| std::ptr::eq(x, f)).unwrap();
            let sha1 = sha1s.get(&i).cloned().or_else(|| f.sha1.clone()).unwrap_or_default();
            files.push(HeldFile { place: f.place.clone(), path: f.path.clone(), size: f.size, sha1, role: f.role.clone(), needed_by: f.needed_by.clone() });
        }
        let entries = plan.lists.iter().flat_map(|l| l.entries.iter().filter(|e| e.key == *key).map(|e| (l.place.clone(), l.path.clone(), e.path.clone()))).collect();
        rec.games.retain(|h| h.key != *key);
        rec.games.push(Held { key: key.clone(), title: g["game"]["title"].as_str().unwrap_or(key).to_string(), system, added: now, files, entries });
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

/// One file fetched, checked against its SHA-1 and put in place: the bytes fetched and its SHA-1.
fn fetch_one(s: &Server, d: &Device, f: &gamerepo_wire::File) -> Result<(u64, String)> {
    let dest = d.at(&f.place, &f.path)?;
    let part = dest.with_file_name(format!("{}{PART}", dest.file_name().unwrap_or_default().to_string_lossy()));
    let mut n = s.fetch(&f.url, &part, None)?;
    let want = s.sha1_of(&f.url, Duration::from_secs(600))?;
    let mut got = sha1_file(&part)?;
    if got != want {
        // once more from the start (a resume of a file that changed in between)
        let _ = std::fs::remove_file(&part);
        n += s.fetch(&f.url, &part, None)?;
        got = sha1_file(&part)?;
        if got != s.sha1_of(&f.url, Duration::from_secs(600))? {
            let _ = std::fs::remove_file(&part);
            bail!("it came through damaged twice");
        }
    }
    if dest.exists() {
        std::fs::remove_file(&dest)?;
    }
    std::fs::rename(&part, &dest)?;
    let tag = part.with_extension(format!("{}.etag", part.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default()));
    let _ = std::fs::remove_file(tag);
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
/// their gamelist entries and collection lines. Only what the record lists is touched.
pub fn remove(d: &Device, keys: &[String]) -> Result<Removed> {
    let mut rec = d.record();
    let (gone, stay): (Vec<Held>, Vec<Held>) = rec.games.drain(..).partition(|g| keys.contains(&g.key));
    rec.games = stay;
    if gone.len() < keys.iter().collect::<HashSet<_>>().len() {
        let unknown: Vec<&String> = keys.iter().filter(|k| !gone.iter().any(|g| &g.key == *k)).collect();
        bail!("This device doesn't hold {} from the server.", unknown.iter().map(|k| k.as_str()).collect::<Vec<_>>().join(", "));
    }
    let needed: HashSet<(String, String)> = rec.games.iter().flat_map(|g| &g.files).map(|f| (f.place.clone(), f.path.clone())).collect();
    let mut out = Removed { games: gone.len(), ..Default::default() };
    let mut dirs: HashSet<(String, PathBuf)> = HashSet::new();
    let mut done: HashSet<(String, String)> = HashSet::new();
    for g in &gone {
        for f in &g.files {
            if !done.insert((f.place.clone(), f.path.clone())) {
                continue;
            }
            if needed.contains(&(f.place.clone(), f.path.clone())) {
                out.kept += 1;
                continue;
            }
            let p = d.at(&f.place, &f.path)?;
            if p.is_file() {
                std::fs::remove_file(&p)?;
                out.files += 1;
            }
            if let Some(parent) = p.parent() {
                dirs.insert((f.place.clone(), parent.to_path_buf()));
            }
        }
    }
    // the folders left empty, up to their place
    for (place, dir) in dirs {
        let top = &d.places[&place];
        let mut at = dir;
        while at.starts_with(top) && at != *top {
            if std::fs::remove_dir(&at).is_err() {
                break;
            }
            at = match at.parent() {
                Some(p) => p.to_path_buf(),
                None => break,
            };
        }
    }
    // their entries and collection lines
    let mut by_list: BTreeMap<(String, String), (Vec<String>, Vec<String>)> = BTreeMap::new();
    for g in &gone {
        for (place, path, entry) in &g.entries {
            let e = by_list.entry((place.clone(), path.clone())).or_default();
            match entry.strip_prefix("line:") {
                Some(line) => e.1.push(line.to_string()),
                None => e.0.push(entry.clone()),
            }
        }
    }
    for ((place, path), (entries, lines)) in by_list {
        let p = d.at(&place, &path)?;
        let Ok(old) = std::fs::read_to_string(&p) else { continue };
        let mut text = old.clone();
        if let Some(t) = gamelist::remove(&text, &entries) {
            text = t;
        }
        if let Some(t) = remove_lines(&text, &lines) {
            text = t;
        }
        if text != old {
            write_whole(&p, text.as_bytes())?;
        }
    }
    d.save_record(&rec)?;
    Ok(out)
}

/// The servers announcing themselves on the network (D91), each with its address.
pub fn find(wait: Duration) -> Result<Vec<(Info, String)>> {
    let daemon = mdns_sd::ServiceDaemon::new()?;
    let rx = daemon.browse(api::SERVICE)?;
    let until = std::time::Instant::now() + wait;
    let mut out: Vec<(Info, String)> = vec![];
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
                let Some(ip) = ips.first() else { continue };
                let address = match ip {
                    std::net::IpAddr::V6(v6) => format!("[{v6}]:{}", r.port),
                    v4 => format!("{v4}:{}", r.port),
                };
                if !out.iter().any(|(i, _)| i.server == info.server) {
                    out.push((info, address));
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    let _ = daemon.stop_browse(api::SERVICE);
    let _ = daemon.shutdown();
    Ok(out)
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
