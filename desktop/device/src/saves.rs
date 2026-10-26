//! Saves and states on a device, synced with the library (docs/PLAN.md, "Save sync and the
//! Devices page", D180–D186): each save profile's folders found on the device (RetroArch's
//! from its `retroarch.cfg`), the saves there of the games the device holds, and a sync that
//! sends what changed here, brings what changed elsewhere and never overwrites a save that
//! changed both here and elsewhere (D184), nor writes one while its emulator runs (D185).
//! What the device last had of each save is kept in `saves.json` in Go's data folder.

use crate::client::{Refused, Server};
use anyhow::{Context, Result};
use gamerepo_wire::saves::{same_files, Profile, SaveFile, SavesAsk, SavesList, Unit, UnitState, Upload, Uploaded};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// A save is read once it's been still this long, so one being written isn't taken (D185).
pub const STILL: i64 = 30;

/// A game the device holds: its key and its file in the ROMs folder (`snes/Game.zip`).
#[derive(Clone, Debug)]
pub struct Held {
    pub key: String,
    pub rom: String,
}

/// Where RetroArch keeps saves and states on this device, as its settings say.
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct RetroArch {
    /// Its settings file (none found: its usual folders)
    pub config: Option<PathBuf>,
    /// None: beside the game
    pub saves: Option<PathBuf>,
    pub states: Option<PathBuf>,
    pub saves_by_content: bool,
    pub saves_by_core: bool,
    pub states_by_content: bool,
    pub states_by_core: bool,
}

/// A save found on the device: its files (by `{name}`), where they are and the newest's time.
#[derive(Clone, Debug, PartialEq)]
pub struct Local {
    pub unit: Unit,
    pub dir: PathBuf,
    /// The game's own name on the device, for `{name}`
    pub stem: String,
    pub files: Vec<SaveFile>,
}

impl Local {
    pub fn played(&self) -> i64 {
        self.files.iter().map(|f| f.modified).max().unwrap_or(0)
    }
}

/// What the device last had of a save.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Had {
    pub version: String,
    pub files: Vec<(String, String)>,
    /// When it was last synced (seconds since 1970)
    pub at: i64,
}

/// The last sync, as Settings and a game's page show it.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Summary {
    pub at: i64,
    pub sent: u32,
    pub brought: u32,
    /// Saves that differ here and in the library (D184), by their units' ids
    pub conflicts: Vec<String>,
    /// Saves that waited because their emulator was running (D185)
    pub waiting: u32,
    pub error: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Kept {
    pub had: BTreeMap<String, Had>,
    pub last: Summary,
}

pub fn load(data: &Path) -> Kept {
    std::fs::read(data.join("saves.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn store(data: &Path, k: &Kept) -> Result<()> {
    crate::write_private(&data.join("saves.json"), &serde_json::to_vec_pretty(k)?)
}

fn os() -> &'static str {
    if cfg!(target_os = "android") {
        "android"
    } else if cfg!(windows) {
        "windows"
    } else {
        "linux"
    }
}

/// A profile's path with its `{home}`, `{appdata}` and `{config}` filled in, once for each home.
fn expand(p: &str, homes: &[PathBuf]) -> Vec<PathBuf> {
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    let mut out = vec![];
    for h in homes {
        let config = env("XDG_CONFIG_HOME").unwrap_or_else(|| h.join(".config"));
        let appdata = env("APPDATA").unwrap_or_else(|| h.join("AppData").join("Roaming"));
        let s = p.replace("{home}", &h.to_string_lossy()).replace("{config}", &config.to_string_lossy()).replace("{appdata}", &appdata.to_string_lossy());
        let pb = PathBuf::from(s);
        if !out.contains(&pb) {
            out.push(pb);
        }
        if !p.contains("{home}") && !p.contains("{config}") && !p.contains("{appdata}") {
            break;
        }
    }
    out
}

/// `retroarch.cfg`'s settings, `key = "value"` a line.
pub fn read_cfg(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().trim_matches('"').to_string()))
        .filter(|(k, _)| !k.starts_with('#'))
        .collect()
}

/// A folder as RetroArch's settings give it: `:` for RetroArch's own folder, `~` home, and
/// `default` or none for beside the game.
fn cfg_dir(v: Option<&String>, cfg_dir: &Path, home: &Path) -> Option<PathBuf> {
    let v = v.map(|s| s.trim()).filter(|s| !s.is_empty() && *s != "default")?;
    let rest = |s: &str| s.trim_start_matches(['/', '\\']).to_string();
    Some(if let Some(r) = v.strip_prefix(':') {
        cfg_dir.join(rest(r))
    } else if let Some(r) = v.strip_prefix('~') {
        home.join(rest(r))
    } else {
        PathBuf::from(v)
    })
}

/// RetroArch on this device, from the first of the profile's settings files found (or the
/// one set in Settings), else its usual folders; None: neither is there.
pub fn retroarch(p: &Profile, homes: &[PathBuf], set: Option<&Path>) -> Option<RetroArch> {
    let os = os();
    let candidates: Vec<PathBuf> = set.map(|s| vec![s.to_path_buf()]).unwrap_or_else(|| p.configs.get(os).into_iter().flatten().flat_map(|c| expand(c, homes)).collect());
    let home = homes.first().cloned().unwrap_or_default();
    for c in candidates {
        let Ok(text) = std::fs::read_to_string(&c) else { continue };
        let cfg = read_cfg(&text);
        let dir = c.parent().map(Path::to_path_buf).unwrap_or_default();
        let on = |k: &str| cfg.get(k).is_some_and(|v| v == "true");
        let beside_saves = on("savefiles_in_content_dir");
        let beside_states = on("savestates_in_content_dir");
        return Some(RetroArch {
            saves: if beside_saves { None } else { cfg_dir(cfg.get("savefile_directory"), &dir, &home) },
            states: if beside_states { None } else { cfg_dir(cfg.get("savestate_directory"), &dir, &home) },
            saves_by_content: on("sort_savefiles_by_content_enable"),
            saves_by_core: on("sort_savefiles_enable"),
            states_by_content: on("sort_savestates_by_content_enable"),
            states_by_core: on("sort_savestates_enable"),
            config: Some(c),
        });
    }
    let first = |m: &BTreeMap<String, Vec<String>>| m.get(os).into_iter().flatten().flat_map(|c| expand(c, homes)).find(|p| p.is_dir());
    let (saves, states) = (first(&p.saves_folders), first(&p.states_folders));
    (saves.is_some() || states.is_some()).then_some(RetroArch { saves, states, ..Default::default() })
}

/// A file's time in seconds since 1970.
fn modified(m: &std::fs::Metadata) -> i64 {
    m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// What a file after the game's name is: a save (`.srm`), a state's slot and whether it's
/// the state's picture, or neither.
fn what(p: &Profile, suffix: &str) -> Option<(String, bool)> {
    if p.saves.iter().any(|s| s.eq_ignore_ascii_case(suffix)) {
        return Some((String::new(), false));
    }
    let (head, tail) = p.state.split_once("{slot}").unwrap_or((&p.state, ""));
    let mut rest = suffix;
    let mut extra = false;
    for e in &p.state_extras {
        if let Some(r) = rest.strip_suffix(e.as_str()) {
            rest = r;
            extra = true;
            break;
        }
    }
    let mid = rest.strip_prefix(head)?.strip_suffix(tail)?;
    let slot = match mid {
        "" => "0".to_string(),
        ".auto" => "auto".to_string(),
        n if n.len() <= 3 && n.bytes().all(|b| b.is_ascii_digit()) => n.to_string(),
        _ => return None,
    };
    Some((slot, extra))
}

/// The suffix a state's slot has (`.state`, `.state3`, `.state.auto`).
#[cfg(test)]
fn state_suffix(p: &Profile, slot: &str) -> String {
    let n = match slot {
        "0" => String::new(),
        "auto" => ".auto".into(),
        s => s.into(),
    };
    p.state.replace("{slot}", &n)
}

/// The game's own name on the device (its file's name without the extension), and its
/// folder in the ROMs folder (ES-DE's system folder).
fn stem_of(rom: &str) -> (String, String) {
    let parts: Vec<&str> = rom.split('/').collect();
    let file = parts.last().copied().unwrap_or("");
    let stem = file.rsplit_once('.').map(|(s, _)| s).unwrap_or(file).to_string();
    (stem, parts.first().copied().unwrap_or("").to_string())
}

/// Every file under `dir` (and its folders, two deep), by its name.
fn files_under(dir: &Path, depth: usize, out: &mut Vec<(PathBuf, String, std::fs::Metadata)>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let Ok(m) = e.metadata() else { continue };
        let name = e.file_name().to_string_lossy().to_string();
        if m.is_dir() {
            if depth > 0 && !name.starts_with('.') {
                files_under(&e.path(), depth - 1, out);
            }
        } else {
            out.push((dir.to_path_buf(), name, m));
        }
    }
}

/// The saves and states of the games held, as RetroArch keeps them (D182, D183).
pub fn find_retroarch(p: &Profile, ra: &RetroArch, roms: &Path, held: &[Held]) -> Vec<Local> {
    // the folders each is looked in, read once each
    let mut cache: HashMap<PathBuf, Vec<(PathBuf, String, std::fs::Metadata)>> = HashMap::new();
    let mut out: BTreeMap<String, Local> = BTreeMap::new();
    for h in held {
        let (stem, system) = stem_of(&h.rom);
        if stem.is_empty() || h.key.starts_with("pc/") {
            continue;
        }
        let content = roms.join(Path::new(&h.rom)).parent().map(Path::to_path_buf).unwrap_or_else(|| roms.to_path_buf());
        let mut roots: Vec<(PathBuf, bool)> = vec![];
        for (dir, states) in [(&ra.saves, false), (&ra.states, true)] {
            let root = dir.clone().unwrap_or_else(|| content.clone());
            if !roots.iter().any(|(r, s)| *r == root && *s == states) {
                roots.push((root, states));
            }
        }
        for (root, _) in &roots {
            let files = cache.entry(root.clone()).or_insert_with(|| {
                let mut v = vec![];
                files_under(root, if root.starts_with(roms) { 0 } else { 2 }, &mut v);
                v
            });
            for (dir, name, m) in files.iter() {
                let Some(suffix) = name.strip_prefix(stem.as_str()).filter(|s| s.starts_with('.')) else { continue };
                let Some((slot, _)) = what(p, suffix) else { continue };
                // a state's core is the folder it's sorted into, else the system's usual core
                let folder = dir.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
                let core = if dir != root && !folder.eq_ignore_ascii_case(&system) { folder } else { p.cores.get(&system).cloned().unwrap_or_default() };
                let unit = if slot.is_empty() { Unit::save(&h.key, &p.save_format) } else { Unit::state(&h.key, &format!("{}:{core}", p.save_format), &slot) };
                let l = out.entry(format!("{}\0{}", unit.id(), dir.display())).or_insert_with(|| Local { unit, dir: dir.clone(), stem: stem.clone(), files: vec![] });
                l.files.push(SaveFile { name: format!("{{name}}{suffix}"), sha1: String::new(), size: m.len(), modified: modified(m) });
            }
        }
    }
    // a save in two folders (two cores): the one played last
    let mut best: BTreeMap<String, Local> = BTreeMap::new();
    for l in out.into_values() {
        let id = l.unit.id();
        if best.get(&id).is_none_or(|b| b.played() < l.played()) {
            best.insert(id, l);
        }
    }
    best.into_values().collect()
}

/// Where a save the device hasn't got goes, as RetroArch would put it; None: a state whose
/// core isn't the one this device's RetroArch uses for the system.
pub fn place_retroarch(p: &Profile, ra: &RetroArch, roms: &Path, h: &Held, unit: &Unit, found: &[Local]) -> Option<PathBuf> {
    let (_, system) = stem_of(&h.rom);
    let content = roms.join(Path::new(&h.rom)).parent().map(Path::to_path_buf)?;
    // the core this device uses for the system: where its other saves are, else the usual
    let used = found.iter().filter(|l| l.unit.key.split('/').next() == unit.key.split('/').next() || l.dir.ends_with(&system)).find_map(|l| l.unit.format.split_once(':').map(|(_, c)| c.to_string()));
    let core = used.or_else(|| p.cores.get(&system).cloned()).unwrap_or_default();
    let (root, by_content, by_core) = if unit.is_state() {
        let (_, want) = unit.format.split_once(':').unwrap_or(("", ""));
        if !want.eq_ignore_ascii_case(&core) {
            return None;
        }
        (&ra.states, ra.states_by_content, ra.states_by_core)
    } else {
        (&ra.saves, ra.saves_by_content, ra.saves_by_core)
    };
    let Some(root) = root.clone() else { return Some(content) };
    let mut d = root;
    if by_content {
        d = d.join(content.file_name().map(|f| f.to_os_string()).unwrap_or_default());
    }
    if by_core {
        d = d.join(&core);
    }
    Some(d)
}

/// Whether any of the profile's emulators is running here (D185). On Android Go syncs only
/// while it's in front, so an emulator isn't.
pub fn running(p: &Profile) -> bool {
    let names: Vec<String> = p.processes.iter().map(|n| n.to_ascii_lowercase()).collect();
    if cfg!(target_os = "android") || names.is_empty() {
        return false;
    }
    if cfg!(windows) {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let out = std::process::Command::new("tasklist").args(["/FO", "CSV", "/NH"]).creation_flags(0x0800_0000).output();
            if let Ok(o) = out {
                let text = String::from_utf8_lossy(&o.stdout).to_ascii_lowercase();
                return names.iter().any(|n| text.contains(&format!("\"{n}\"")));
            }
        }
        return false;
    }
    std::fs::read_dir("/proc").into_iter().flatten().flatten().any(|e| {
        std::fs::read_to_string(e.path().join("comm")).is_ok_and(|c| {
            let c = c.trim().to_ascii_lowercase();
            names.iter().any(|n| *n == c)
        })
    })
}

fn encode(s: &str) -> String {
    s.bytes().map(|c| if c.is_ascii_alphanumeric() || b"-._~".contains(&c) { (c as char).to_string() } else { format!("%{c:02X}") }).collect()
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Where the sync is on this device: the profiles, and what each found.
pub struct Here<'a> {
    pub profile: &'a Profile,
    pub ra: RetroArch,
    pub roms: PathBuf,
    pub emulator: String,
}

/// One sync (D184): every save of the games held sent when only this device changed it,
/// brought when only the library did, and a conflict when both did (the device's kept, and
/// sent as a version of its own).
pub fn sync(s: &Server, data: &Path, here: &Here, held: &[Held]) -> Summary {
    let mut kept = load(data);
    let mut sum = Summary { at: now(), ..Default::default() };
    if let Err(e) = sync_with(s, &mut kept, &mut sum, here, held) {
        sum.error = format!("{e:#}");
    }
    kept.last = sum.clone();
    let _ = store(data, &kept);
    sum
}

fn sync_with(s: &Server, kept: &mut Kept, sum: &mut Summary, here: &Here, held: &[Held]) -> Result<()> {
    let p = here.profile;
    let mut found = find_retroarch(p, &here.ra, &here.roms, held);
    for l in found.iter_mut() {
        for f in l.files.iter_mut() {
            f.sha1 = crate::sha1_file(&l.dir.join(f.name.replace("{name}", &l.stem)))?;
        }
    }
    let keys: Vec<String> = {
        let mut k: Vec<String> = held.iter().map(|h| h.key.clone()).collect();
        k.sort();
        k.dedup();
        k
    };
    let list: SavesList = s.call("POST", "/v1/saves", Some(&SavesAsk { keys }))?;
    let library: HashMap<String, UnitState> = list.units.into_iter().map(|u| (u.unit.id(), u)).collect();
    let local: HashMap<String, &Local> = found.iter().map(|l| (l.unit.id(), l)).collect();
    let busy = running(p);
    let mut ids: Vec<String> = library.keys().chain(local.keys()).cloned().collect();
    ids.sort();
    ids.dedup();
    for id in ids {
        let lib = library.get(&id);
        let here_now = local.get(&id).copied();
        if lib.is_some_and(|u| u.unit.is_state()) && !list.states || here_now.is_some_and(|l| l.unit.is_state()) && !list.states {
            continue;
        }
        // only the library's own format goes where this device can read it
        if here_now.is_none() && lib.is_some_and(|u| !u.unit.format.starts_with(&p.save_format)) {
            continue;
        }
        let had = kept.had.get(&id).cloned();
        let same_as_had = |l: &Local| had.as_ref().is_some_and(|h| same_as(&l.files, &h.files));
        match (here_now, lib) {
            (Some(l), lib) => {
                if lib.is_some_and(|u| same_files(&u.current.files, &l.files)) {
                    let u = lib.unwrap();
                    kept.had.insert(id.clone(), had_of(&u.current.id, &l.files));
                    if u.conflict && u.against.iter().any(|a| had.as_ref().is_some_and(|h| h.version == *a)) {
                        sum.conflicts.push(id);
                    }
                    continue;
                }
                if same_as_had(l) {
                    // unchanged here: the library's newer one, unless this device is a side of its conflict
                    let Some(u) = lib else { continue };
                    let h = had.unwrap();
                    if u.current.id == h.version {
                        continue;
                    }
                    if u.against.contains(&h.version) {
                        sum.conflicts.push(id);
                        continue;
                    }
                    if busy {
                        sum.waiting += 1;
                        continue;
                    }
                    bring(s, kept, &l.dir, &l.stem, u)?;
                    sum.brought += 1;
                    continue;
                }
                // changed here (or never synced): sent, once it's been still a while
                if now() - l.played() < STILL {
                    sum.waiting += 1;
                    continue;
                }
                let base = had.map(|h| h.version).unwrap_or_default();
                let r = send(s, l, &base, &here.emulator)?;
                kept.had.insert(id.clone(), had_of(&r.version.id, &l.files));
                sum.sent += 1;
                if r.state.conflict {
                    sum.conflicts.push(id);
                }
            }
            (None, Some(u)) => {
                // the library's, for a game held here with none of its own
                if u.conflict && had.as_ref().is_some_and(|h| u.against.contains(&h.version)) {
                    sum.conflicts.push(id);
                    continue;
                }
                let Some(h) = held.iter().find(|h| h.key == u.unit.key) else { continue };
                let Some(dir) = place_retroarch(p, &here.ra, &here.roms, h, &u.unit, &found) else { continue };
                if busy {
                    sum.waiting += 1;
                    continue;
                }
                let (stem, _) = stem_of(&h.rom);
                bring(s, kept, &dir, &stem, u)?;
                sum.brought += 1;
            }
            (None, None) => {}
        }
    }
    Ok(())
}

fn same_as(files: &[SaveFile], had: &[(String, String)]) -> bool {
    let mut a: Vec<(String, String)> = files.iter().map(|f| (f.name.clone(), f.sha1.clone())).collect();
    let mut b = had.to_vec();
    a.sort();
    b.sort();
    a == b
}

fn had_of(version: &str, files: &[SaveFile]) -> Had {
    Had { version: version.into(), files: files.iter().map(|f| (f.name.clone(), f.sha1.clone())).collect(), at: now() }
}

/// A save's files sent (those the server hasn't got), then the save as a version.
fn send(s: &Server, l: &Local, base: &str, emulator: &str) -> Result<Uploaded> {
    for f in &l.files {
        let at = format!("/v1/saves/blob/{}", f.sha1);
        if !s.has(&at)? {
            let bytes = std::fs::read(l.dir.join(f.name.replace("{name}", &l.stem)))?;
            s.put_bytes(&at, &bytes)?;
        }
    }
    let up = Upload { unit: l.unit.clone(), base: base.into(), files: l.files.clone(), emulator: emulator.into() };
    s.call("POST", "/v1/saves/versions", Some(&up)).map_err(|e| match e.downcast::<Refused>() {
        Ok(r) => anyhow::anyhow!("{}", r.message),
        Err(e) => e,
    })
}

/// The library's newest version written into `dir` under the device's name for the game,
/// each file checked, written beside and then put in place.
fn bring(s: &Server, kept: &mut Kept, dir: &Path, stem: &str, u: &UnitState) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("couldn't make {}", dir.display()))?;
    let mut done = vec![];
    for f in &u.current.files {
        let path = format!("/v1/saves/file?unit={}&version={}&name={}", encode(&u.unit.id()), encode(&u.current.id), encode(&f.name));
        let bytes = s.bytes(&path)?;
        let sha1 = hex::encode(<sha1::Sha1 as sha1::Digest>::digest(&bytes));
        if sha1 != f.sha1 {
            anyhow::bail!("{} came with the wrong SHA-1.", f.name);
        }
        let to = dir.join(f.name.replace("{name}", stem));
        let part = to.with_file_name(format!(".{}.gamerepo-part", to.file_name().unwrap_or_default().to_string_lossy()));
        std::fs::write(&part, &bytes)?;
        if f.modified > 0 {
            if let Ok(file) = std::fs::File::options().write(true).open(&part) {
                let _ = file.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(f.modified as u64));
            }
        }
        done.push((part, to));
    }
    for (part, to) in done {
        std::fs::rename(&part, &to)?;
    }
    kept.had.insert(u.unit.id(), had_of(&u.current.id, &u.current.files));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> Profile {
        gamerepo_wire::saves::built_in().into_iter().find(|p| p.id == "retroarch").unwrap()
    }

    #[test]
    fn a_files_suffix_says_what_it_is() {
        let p = profile();
        assert_eq!(what(&p, ".srm"), Some((String::new(), false)));
        assert_eq!(what(&p, ".state"), Some(("0".into(), false)));
        assert_eq!(what(&p, ".state3.png"), Some(("3".into(), true)));
        assert_eq!(what(&p, ".state.auto"), Some(("auto".into(), false)));
        assert_eq!(what(&p, ".zip"), None);
        assert_eq!(state_suffix(&p, "0"), ".state");
        assert_eq!(state_suffix(&p, "auto"), ".state.auto");
        assert_eq!(state_suffix(&p, "2"), ".state2");
    }

    #[test]
    fn retroarchs_settings_say_where_and_how_its_saves_are_sorted() {
        let dir = std::env::temp_dir().join(format!("gamerepo-test-ra-cfg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let cfg = dir.join(".config/retroarch/retroarch.cfg");
        std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
        std::fs::write(&cfg, "savefile_directory = \"~/saves\"\nsavestate_directory = \":/states\"\nsort_savefiles_by_content_enable = \"true\"\nsort_savestates_enable = \"true\"\n").unwrap();
        let p = profile();
        let ra = retroarch(&p, &[dir.clone()], Some(&cfg)).unwrap();
        assert_eq!(ra.saves, Some(dir.join("saves")));
        assert_eq!(ra.states, Some(dir.join(".config/retroarch/states")));
        assert!(ra.saves_by_content && !ra.saves_by_core && ra.states_by_core);

        // the saves and states of a held game found, a state's core from its folder
        let roms = dir.join("ROMs");
        std::fs::create_dir_all(roms.join("snes")).unwrap();
        std::fs::write(roms.join("snes/Game (USA).zip"), "rom").unwrap();
        std::fs::create_dir_all(dir.join("saves/snes")).unwrap();
        std::fs::write(dir.join("saves/snes/Game (USA).srm"), "save").unwrap();
        let states = dir.join(".config/retroarch/states/Snes9x");
        std::fs::create_dir_all(&states).unwrap();
        std::fs::write(states.join("Game (USA).state2"), "state").unwrap();
        std::fs::write(states.join("Game (USA).state2.png"), "png").unwrap();
        std::fs::write(states.join("Other.state"), "not held").unwrap();
        let held = vec![Held { key: "snes/Game (USA)".into(), rom: "snes/Game (USA).zip".into() }];
        let found = find_retroarch(&p, &ra, &roms, &held);
        let ids: Vec<String> = found.iter().map(|l| l.unit.id()).collect();
        assert_eq!(ids, vec!["snes/Game (USA)|retroarch:Snes9x|2", "snes/Game (USA)|retroarch|"]);
        assert_eq!(found[0].files.len(), 2, "a state's picture goes with it");
        // a save from elsewhere goes where RetroArch would put it; a state of another core nowhere
        let other = Unit::state("snes/Game (USA)", "retroarch:bsnes", "1");
        assert_eq!(place_retroarch(&p, &ra, &roms, &held[0], &other, &found), None);
        assert_eq!(place_retroarch(&p, &ra, &roms, &held[0], &Unit::state("snes/Game (USA)", "retroarch:Snes9x", "1"), &found), Some(states.clone()));
        assert_eq!(place_retroarch(&p, &ra, &roms, &held[0], &Unit::save("snes/Game (USA)", "retroarch"), &found), Some(dir.join("saves/snes")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
