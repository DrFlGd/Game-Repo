//! Saves and states on a device, synced with the library (docs/PLAN.md, "Save sync and the
//! Devices page", D180–D186): each save profile's folders found on the device (RetroArch's
//! from its `retroarch.cfg`), the saves there of the games the device holds, and a sync that
//! sends what changed here, brings what changed elsewhere and never overwrites a save that
//! changed both here and elsewhere (D184), nor writes one while its emulator runs (D185).
//! Emulators of their own (D192) keep saves in their own folder (PPSSPP, Dolphin, the PCSX2
//! family, DuckStation, …), by the game's disc ID, its name or as a memory card every game
//! shares: a profile's [`Spot`]s say where, and each emulator's folder can be set in
//! Settings (D193). What the device last had of each save, and the IDs the library read
//! for its games, are kept in `saves.json` in Go's data folder. The library is reached
//! through a [`Library`]: Game Repo Server for Go, or the library's own folder when Game
//! Repo syncs a card's saves itself (D189).

use crate::client::{Refused, Server};
use anyhow::{Context, Result};
use gamerepo_wire::saves::{cards_key, same_files, Ids, IdsAsk, Loose, LooseAsk, Profile, SaveFile, SavesAsk, SavesList, Spot, Unit, UnitState, Upload, Uploaded, MAX_FILE};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
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
    /// Saves found that no game here is named like, sent to the library's *Saves not
    /// matched* (D195)
    pub loose: u32,
    pub error: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Kept {
    /// By [`had_key`]
    pub had: BTreeMap<String, Had>,
    pub last: Summary,
    /// The games' disc IDs as the library read them ("" none), by key
    pub ids: BTreeMap<String, String>,
    /// The saves not matched sent already, by their file's SHA-1 (D195)
    pub loose: BTreeSet<String>,
    /// A library too old for them isn't asked again before this (seconds since 1970)
    pub loose_after: i64,
}

/// What a save's last sync is kept by: its unit's id, after its emulator's when that keeps
/// it in a folder of its own (`dolphin>gc/…|dolphin|`), so two emulators here with the
/// same save don't mix theirs up.
pub fn had_key(p: &Profile, u: &Unit) -> String {
    if p.kind == "folders" {
        format!("{}>{}", p.id, u.id())
    } else {
        u.id()
    }
}

/// The unit a [`had_key`] is of.
pub fn unit_of(had_key: &str) -> Option<Unit> {
    let id = match had_key.split_once('>') {
        Some((p, rest)) if !p.is_empty() && p.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') => rest,
        _ => had_key,
    };
    Unit::from_id(id)
}

pub fn load(data: &Path) -> Kept {
    std::fs::read(data.join("saves.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn store(data: &Path, k: &Kept) -> Result<()> {
    crate::write_private(&data.join("saves.json"), &serde_json::to_vec_pretty(k)?)
}

pub fn os() -> &'static str {
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
        let local = env("LOCALAPPDATA").unwrap_or_else(|| h.join("AppData").join("Local"));
        let s = p
            .replace("{home}", &h.to_string_lossy())
            .replace("{config}", &config.to_string_lossy())
            .replace("{appdata}", &appdata.to_string_lossy())
            .replace("{localappdata}", &local.to_string_lossy())
            .replace("{documents}", &h.join("Documents").to_string_lossy());
        let pb = PathBuf::from(s);
        if !out.contains(&pb) {
            out.push(pb);
        }
        if !["{home}", "{config}", "{appdata}", "{localappdata}", "{documents}"].iter().any(|t| p.contains(t)) {
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
        // (saves and states in one folder, as Batocera's and Knulli's are, read once)
        let mut roots: Vec<PathBuf> = vec![];
        for dir in [&ra.saves, &ra.states] {
            let root = dir.clone().unwrap_or_else(|| content.clone());
            if !roots.contains(&root) {
                roots.push(root);
            }
        }
        for root in &roots {
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

/// RetroArch's saves in its saves folder that none of the games held is named like (D195):
/// a game from elsewhere, or one the library has under another name. None looked for when
/// saves are beside the games.
pub fn loose_retroarch(p: &Profile, ra: &RetroArch, roms: &Path, held: &[Held]) -> Vec<(Loose, PathBuf)> {
    let Some(root) = ra.saves.as_ref().filter(|r| !r.starts_with(roms)) else { return vec![] };
    let stems: HashSet<String> = held.iter().map(|h| stem_of(&h.rom).0).collect();
    let mut files = vec![];
    files_under(root, 2, &mut files);
    let mut out = vec![];
    for (dir, name, m) in files {
        let lower = name.to_ascii_lowercase();
        let Some(suffix) = p.saves.iter().find(|s| lower.ends_with(&s.to_ascii_lowercase()) && name.len() > s.len()) else { continue };
        let game = &name[..name.len() - suffix.len()];
        if stems.contains(game) || m.len() == 0 || m.len() > MAX_FILE || name.starts_with('.') {
            continue;
        }
        let file = SaveFile { name: name.clone(), sha1: String::new(), size: m.len(), modified: modified(&m) };
        out.push((Loose { format: p.save_format.clone(), emulator: p.name.clone(), file, game: game.to_string(), suffix: name[game.len()..].to_string() }, dir.join(&name)));
    }
    out
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

/// A `folders` emulator's folder here: the one set in Settings (D193), else the first of
/// its usual ones that holds one of the folders it makes. None: it isn't here.
pub fn folder_of(p: &Profile, homes: &[PathBuf], set: Option<&Path>) -> Option<PathBuf> {
    if let Some(s) = set {
        return s.is_dir().then(|| s.to_path_buf());
    }
    p.roots.get(os()).into_iter().flatten().flat_map(|c| expand(c, homes)).find(|d| d.is_dir() && (p.look_for.is_empty() || p.look_for.iter().any(|l| d.join(l).exists())))
}

/// The systems a spot is for.
fn spot_systems<'a>(p: &'a Profile, s: &'a Spot) -> &'a [String] {
    if s.systems.is_empty() {
        &p.systems
    } else {
        &s.systems
    }
}

/// A game's names for a spot's paths ([`Spot`]): from its disc ID, its file and its key.
pub fn tokens(key: &str, stem: &str, id: &str) -> Vec<(&'static str, String)> {
    let mut t = vec![("{name}", stem.to_string()), ("{title}", key.split_once('/').map(|(_, n)| n).unwrap_or(key).to_string())];
    if id.is_empty() {
        return t;
    }
    t.push(("{id}", id.to_string()));
    t.push(("{id_compact}", id.replace('-', "")));
    if id.len() >= 4 && id.is_ascii() {
        let id4 = &id[..4];
        t.push(("{id4}", id4.to_string()));
        t.push(("{id4_hex}", hex::encode(id4.as_bytes())));
        if let Some(r) = id.as_bytes().get(3).map(|b| match b {
            b'E' | b'N' => "USA",
            b'J' => "JAP",
            _ => "EUR",
        }) {
            t.push(("{region}", r.to_string()));
        }
    }
    if id.len() == 16 && id.bytes().all(|b| b.is_ascii_hexdigit()) {
        t.push(("{tid_low}", id[8..].to_ascii_lowercase()));
    }
    t
}

/// A path pattern with a game's names filled in; None: it needs one the game hasn't got.
fn fill(pattern: &str, tokens: &[(&str, String)]) -> Option<String> {
    let mut s = pattern.to_string();
    for (t, v) in tokens {
        s = s.replace(t, v);
    }
    let rest = s.replace("{slot}", "");
    (!rest.contains('{')).then_some(s)
}

/// Whether a name matches a pattern's part (`*`, `?`, `{slot}` the digits of a state's
/// slot, or `auto`/`resume`), without case: Some(the slot, "" for none).
fn match_part(pat: &[char], name: &[char]) -> Option<String> {
    const SLOT: &[char] = &['{', 's', 'l', 'o', 't', '}'];
    if pat.is_empty() {
        return name.is_empty().then(String::new);
    }
    if pat.starts_with(SLOT) {
        let rest = &pat[SLOT.len()..];
        for word in ["auto", "resume"] {
            let w: Vec<char> = word.chars().collect();
            if name.len() >= w.len() && name[..w.len()].iter().zip(&w).all(|(a, b)| a.eq_ignore_ascii_case(b)) && match_part(rest, &name[w.len()..]).is_some() {
                return Some("auto".into());
            }
        }
        let digits = name.iter().take_while(|c| c.is_ascii_digit()).count().min(3);
        for n in (1..=digits).rev() {
            if match_part(rest, &name[n..]).is_some() {
                return Some(name[..n].iter().collect());
            }
        }
        return None;
    }
    match pat[0] {
        '*' => (0..=name.len()).find_map(|i| match_part(&pat[1..], &name[i..])),
        '?' => (!name.is_empty()).then(|| match_part(&pat[1..], &name[1..])).flatten(),
        c => (!name.is_empty() && name[0].eq_ignore_ascii_case(&c)).then(|| match_part(&pat[1..], &name[1..])).flatten(),
    }
}

/// Whether a path (its parts) matches a pattern's: `**` any number of folders.
fn match_path(pat: &[&str], path: &[&str]) -> Option<String> {
    let Some((first, rest)) = pat.split_first() else { return path.is_empty().then(String::new) };
    if *first == "**" {
        return (0..=path.len()).rev().find_map(|i| match_path(rest, &path[i..]));
    }
    let (name, more) = path.split_first()?;
    let a: Vec<char> = first.chars().collect();
    let b: Vec<char> = name.chars().collect();
    let slot = match_part(&a, &b)?;
    let after = match_path(rest, more)?;
    Some(if slot.is_empty() { after } else { slot })
}

/// The folder a pattern starts in, before its first part that's matched (`PSP/SAVEDATA`).
fn fixed_part(pattern: &str) -> String {
    let parts: Vec<&str> = pattern.split('/').collect();
    // the last part is a file's name
    parts[..parts.len() - 1].iter().take_while(|p| !p.contains(['*', '?', '{'])).copied().collect::<Vec<_>>().join("/")
}

/// Every file under `dir`, by its path from `base` (`/` between folders).
fn walk(base: &Path, dir: &Path, depth: usize, out: &mut Vec<(String, std::fs::Metadata)>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let Ok(m) = e.metadata() else { continue };
        let path = e.path();
        if m.is_dir() {
            if depth > 0 {
                walk(base, &path, depth - 1, out);
            }
        } else if let Ok(rel) = path.strip_prefix(base) {
            let rel = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/");
            if !rel.split('/').any(|p| p.starts_with(".gamerepo") || p.ends_with(".gamerepo-part")) {
                out.push((rel, m));
            }
        }
    }
}

/// The saves and states of the games held, in a `folders` emulator's folder `root` and
/// beside the games (D192), and its memory cards every game shares. `ids` the games' disc
/// IDs by key.
pub fn find_folders(p: &Profile, root: &Path, roms: &Path, held: &[Held], ids: &BTreeMap<String, String>) -> Vec<Local> {
    let mut lists: HashMap<PathBuf, Vec<(String, std::fs::Metadata)>> = HashMap::new();
    let mut list = |base: &Path, fixed: &str, depth: usize| -> Vec<(String, std::fs::Metadata)> {
        lists
            .entry(base.join(fixed))
            .or_insert_with(|| {
                let mut v = vec![];
                walk(base, &if fixed.is_empty() { base.to_path_buf() } else { base.join(fixed) }, depth, &mut v);
                v
            })
            .clone()
    };
    let mut out: BTreeMap<String, Local> = BTreeMap::new();
    let mut add = |unit: Unit, dir: &Path, stem: &str, name: String, m: &std::fs::Metadata| {
        let l = out.entry(unit.id()).or_insert_with(|| Local { unit, dir: dir.to_path_buf(), stem: stem.to_string(), files: vec![] });
        if !l.files.iter().any(|f| f.name == name) {
            l.files.push(SaveFile { name, sha1: String::new(), size: m.len(), modified: modified(m) });
        }
    };
    let unit_for = |s: &Spot, key: &str, slot: String| {
        if s.kind == "state" {
            Unit::state(key, &p.state_format, if slot.is_empty() { "0" } else { &slot })
        } else {
            Unit::save(key, &p.save_format)
        }
    };
    let mut systems: Vec<String> = vec![];
    for h in held {
        let (stem, _) = stem_of(&h.rom);
        let system = h.key.split('/').next().unwrap_or("").to_string();
        if stem.is_empty() || !p.systems.contains(&system) {
            continue;
        }
        if !systems.contains(&system) {
            systems.push(system.clone());
        }
        let content = roms.join(Path::new(&h.rom)).parent().map(Path::to_path_buf).unwrap_or_else(|| roms.to_path_buf());
        let toks = tokens(&h.key, &stem, ids.get(&h.key).map(String::as_str).unwrap_or(""));
        for s in p.spots.iter().filter(|s| !s.whole && spot_systems(p, s).contains(&system)) {
            let base = if s.beside { content.clone() } else { root.to_path_buf() };
            for pat in &s.files {
                let Some(filled) = fill(pat, &toks) else { continue };
                let parts: Vec<&str> = filled.split('/').collect();
                let fixed = fixed_part(&filled);
                for (rel, m) in list(&base, &fixed, if s.beside { 0 } else { 8 }).iter() {
                    let Some(slot) = match_path(&parts, &rel.split('/').collect::<Vec<_>>()) else { continue };
                    // a name that's the game's own file's is kept as `{name}`, for each device's own
                    let name = if pat.contains("{name}") { rel.replace(stem.as_str(), "{name}") } else { rel.clone() };
                    add(unit_for(s, &h.key, slot), &base, &stem, name, m);
                }
            }
        }
    }
    // the memory cards every game shares, for each system with a game here
    for s in p.spots.iter().filter(|s| s.whole) {
        for system in systems.iter().filter(|x| spot_systems(p, s).contains(x)) {
            for pat in &s.files {
                let parts: Vec<&str> = pat.split('/').collect();
                for (rel, m) in list(root, &fixed_part(pat), 8).iter() {
                    if match_path(&parts, &rel.split('/').collect::<Vec<_>>()).is_some() {
                        add(Unit::save(&cards_key(system), &p.save_format), root, "", rel.clone(), m);
                    }
                }
            }
        }
    }
    out.into_values().collect()
}

/// Whether a save in this format is one this emulator reads.
pub fn takes(p: &Profile, format: &str) -> bool {
    if p.kind == "folders" {
        format == p.save_format || format == p.state_format
    } else {
        format == p.save_format || format.strip_prefix(&p.save_format).is_some_and(|r| r.starts_with(':'))
    }
}

/// Where a save the device hasn't got goes in a `folders` emulator (its files' names say
/// the rest), and the game's name there for `{name}`; None: not one it reads, or not for a
/// game here.
pub fn place_folders(p: &Profile, root: &Path, roms: &Path, held: &[Held], unit: &Unit) -> Option<(PathBuf, String)> {
    if !takes(p, &unit.format) {
        return None;
    }
    let system = unit.key.split('/').next().unwrap_or("");
    if !p.systems.iter().any(|s| s == system) {
        return None;
    }
    if unit.key == cards_key(system) {
        return held.iter().any(|h| h.key.starts_with(&format!("{system}/"))).then(|| (root.to_path_buf(), String::new()));
    }
    let h = held.iter().find(|h| h.key == unit.key)?;
    let (stem, _) = stem_of(&h.rom);
    let kind = if unit.is_state() { "state" } else { "save" };
    let beside = p.spots.iter().any(|s| s.kind == kind && s.beside && spot_systems(p, s).iter().any(|x| x == system));
    let dir = if beside { roms.join(Path::new(&h.rom)).parent()?.to_path_buf() } else { root.to_path_buf() };
    Some((dir, stem))
}

/// Whether any of the profile's emulators is running here (D185). Android doesn't say: Go
/// syncs while it's in front, when an emulator isn't, and its job in the background writes
/// nothing (`Here::busy`).
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

/// Where an emulator keeps its saves here.
#[derive(Clone, Debug)]
pub enum At {
    RetroArch(RetroArch),
    /// A `folders` emulator's own folder
    Folder(PathBuf),
}

/// An emulator found on this device for the sync: its profile and where its saves are.
pub struct Here<'a> {
    pub profile: &'a Profile,
    pub at: At,
    pub roms: PathBuf,
    pub emulator: String,
    /// Whether its emulator's running: None asks this computer ([`running`], D185); a card's
    /// isn't, and on Android in the background it might be, so nothing's written then
    pub busy: Option<bool>,
    /// Only the games in this folder of the ROMs folder (a card's system, whose saves'
    /// folders are its own)
    pub only: Option<String>,
}

/// Where saves are synced with: the library, through Game Repo Server for Go, or its own
/// folder in Game Repo for a card (D189).
pub trait Library {
    /// The disc IDs of these games (D192); None: the library can't say.
    fn ids(&self, keys: &[String]) -> Result<Option<BTreeMap<String, String>>>;
    /// The library's saves of these games, and whether states are synced.
    fn list(&self, keys: Vec<String>) -> Result<SavesList>;
    /// A save here kept as a version, its files read from where they are.
    fn send(&self, l: &Local, base: &str, emulator: &str) -> Result<Uploaded>;
    /// A file of one of the library's versions.
    fn bytes(&self, unit: &Unit, version: &str, name: &str) -> Result<Vec<u8>>;
    /// Saves not matched to a game kept apart (D195), each with where its file is; false:
    /// the library can't keep them.
    fn loose(&self, saves: &[(Loose, PathBuf)]) -> Result<bool>;
}

impl Library for Server {
    fn ids(&self, keys: &[String]) -> Result<Option<BTreeMap<String, String>>> {
        match self.call::<Ids>("POST", "/v1/ids", Some(&IdsAsk { keys: keys.to_vec() })) {
            Ok(got) => Ok(Some(got.ids)),
            // a library too old to read them says nothing
            Err(e) if e.downcast_ref::<Refused>().is_some() => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn list(&self, keys: Vec<String>) -> Result<SavesList> {
        self.call("POST", "/v1/saves", Some(&SavesAsk { keys }))
    }

    /// Its files sent (those the server hasn't got), then the save as a version.
    fn send(&self, l: &Local, base: &str, emulator: &str) -> Result<Uploaded> {
        for f in &l.files {
            let at = format!("/v1/saves/blob/{}", f.sha1);
            if !self.has(&at)? {
                let bytes = std::fs::read(l.dir.join(f.name.replace("{name}", &l.stem)))?;
                self.put_bytes(&at, &bytes)?;
            }
        }
        let up = Upload { unit: l.unit.clone(), base: base.into(), files: l.files.clone(), emulator: emulator.into() };
        self.call("POST", "/v1/saves/versions", Some(&up)).map_err(|e| match e.downcast::<Refused>() {
            Ok(r) => anyhow::anyhow!("{}", r.message),
            Err(e) => e,
        })
    }

    fn bytes(&self, unit: &Unit, version: &str, name: &str) -> Result<Vec<u8>> {
        Server::bytes(self, &format!("/v1/saves/file?unit={}&version={}&name={}", encode(&unit.id()), encode(version), encode(name)))
    }

    fn loose(&self, saves: &[(Loose, PathBuf)]) -> Result<bool> {
        for (l, path) in saves {
            let at = format!("/v1/saves/blob/{}", l.file.sha1);
            if !self.has(&at)? {
                self.put_bytes(&at, &std::fs::read(path)?)?;
            }
        }
        match self.call::<serde_json::Value>("POST", "/v1/saves/loose", Some(&LooseAsk { saves: saves.iter().map(|(l, _)| l.clone()).collect() })) {
            Ok(_) => Ok(true),
            Err(e) if e.downcast_ref::<Refused>().is_some() => Ok(false),
            Err(e) => Err(e),
        }
    }
}

impl Here<'_> {
    fn find(&self, held: &[Held], ids: &BTreeMap<String, String>) -> Vec<Local> {
        match &self.at {
            At::RetroArch(ra) => find_retroarch(self.profile, ra, &self.roms, held),
            At::Folder(root) => find_folders(self.profile, root, &self.roms, held, ids),
        }
    }

    fn place(&self, held: &[Held], unit: &Unit, found: &[Local]) -> Option<(PathBuf, String)> {
        match &self.at {
            At::RetroArch(ra) => {
                let h = held.iter().find(|h| h.key == unit.key)?;
                Some((place_retroarch(self.profile, ra, &self.roms, h, unit, found)?, stem_of(&h.rom).0))
            }
            At::Folder(root) => place_folders(self.profile, root, &self.roms, held, unit),
        }
    }
}

/// One sync (D184): every save of the games held sent when only this device changed it,
/// brought when only the library did, and a conflict when both did (the device's kept, and
/// sent as a version of its own).
pub fn sync(s: &dyn Library, data: &Path, heres: &[Here], held: &[Held]) -> Summary {
    // one sync at a time, whichever program it's in: Go's window, or the one in the background (D186)
    let lock = std::fs::create_dir_all(data).and_then(|_| std::fs::File::options().create(true).truncate(false).write(true).open(data.join("saves.lock")));
    let _held = match lock {
        Ok(f) => match f.try_lock() {
            Ok(()) => Some(f),
            Err(std::fs::TryLockError::WouldBlock) => return load(data).last,
            Err(_) => None,
        },
        Err(_) => None,
    };
    let mut kept = load(data);
    let mut sum = Summary { at: now(), ..Default::default() };
    let mut errors = vec![];
    if let Err(e) = ask_ids(s, &mut kept, heres, held) {
        errors.push(format!("{e:#}"));
    }
    for here in heres {
        if let Err(e) = sync_with(s, &mut kept, &mut sum, here, held) {
            errors.push(if heres.len() > 1 { format!("{}: {e:#}", here.profile.name) } else { format!("{e:#}") });
        }
    }
    sum.error = errors.join(" ");
    kept.last = sum.clone();
    let _ = store(data, &kept);
    sum
}

/// The disc IDs of the games here the emulators found need, those not asked for before
/// (D192). A library too old to read them says nothing.
fn ask_ids(s: &dyn Library, kept: &mut Kept, heres: &[Here], held: &[Held]) -> Result<()> {
    let wants = |key: &str| {
        let system = key.split('/').next().unwrap_or("");
        heres.iter().any(|h| h.profile.spots.iter().any(|sp| sp.files.iter().any(|f| f.contains("{id") || f.contains("{region}") || f.contains("{tid_low}")) && spot_systems(h.profile, sp).iter().any(|x| x == system)))
    };
    let mut keys: Vec<String> = held.iter().map(|h| h.key.clone()).filter(|k| !kept.ids.contains_key(k) && wants(k)).collect();
    keys.sort();
    keys.dedup();
    for chunk in keys.chunks(200) {
        match s.ids(chunk)? {
            Some(got) => kept.ids.extend(got),
            None => return Ok(()),
        }
    }
    Ok(())
}

fn sync_with(s: &dyn Library, kept: &mut Kept, sum: &mut Summary, here: &Here, held: &[Held]) -> Result<()> {
    let p = here.profile;
    let held: Vec<Held> = held.iter().filter(|h| here.only.as_ref().is_none_or(|f| stem_of(&h.rom).1 == *f)).cloned().collect();
    let held = held.as_slice();
    let mut found = here.find(held, &kept.ids);
    for l in found.iter_mut() {
        for f in l.files.iter_mut() {
            f.sha1 = crate::sha1_file(&l.dir.join(f.name.replace("{name}", &l.stem)))?;
        }
    }
    let keys: Vec<String> = {
        let mut k: Vec<String> = held.iter().map(|h| h.key.clone()).collect();
        // the memory cards every game of a system shares
        for s in p.spots.iter().filter(|s| s.whole) {
            for system in spot_systems(p, s) {
                if held.iter().any(|h| h.key.starts_with(&format!("{system}/"))) {
                    k.push(cards_key(system));
                }
            }
        }
        k.sort();
        k.dedup();
        k
    };
    let list = s.list(keys)?;
    let library: HashMap<String, UnitState> = list.units.into_iter().map(|u| (u.unit.id(), u)).collect();
    let local: HashMap<String, &Local> = found.iter().map(|l| (l.unit.id(), l)).collect();
    let busy = here.busy.unwrap_or_else(|| running(p));
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
        if lib.is_some_and(|u| !takes(p, &u.unit.format)) {
            continue;
        }
        let hk = had_key(p, lib.map(|u| &u.unit).or(here_now.map(|l| &l.unit)).unwrap());
        let had = kept.had.get(&hk).cloned();
        let same_as_had = |l: &Local| had.as_ref().is_some_and(|h| same_as(&l.files, &h.files));
        match (here_now, lib) {
            (Some(l), lib) => {
                if lib.is_some_and(|u| same_files(&u.current.files, &l.files)) {
                    let u = lib.unwrap();
                    kept.had.insert(hk.clone(), had_of(&u.current.id, &l.files));
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
                    bring(s, kept, &hk, &l.dir, &l.stem, u)?;
                    sum.brought += 1;
                    continue;
                }
                // changed here (or never synced): sent, once it's been still a while
                if now() - l.played() < STILL {
                    sum.waiting += 1;
                    continue;
                }
                let base = had.map(|h| h.version).unwrap_or_default();
                let r = s.send(l, &base, &here.emulator)?;
                kept.had.insert(hk.clone(), had_of(&r.version.id, &l.files));
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
                let Some((dir, stem)) = here.place(held, &u.unit, &found) else { continue };
                if busy {
                    sum.waiting += 1;
                    continue;
                }
                bring(s, kept, &hk, &dir, &stem, u)?;
                sum.brought += 1;
            }
            (None, None) => {}
        }
    }
    // saves no game here is named like, to the library's *Saves not matched* once each (D195)
    if let (At::RetroArch(ra), true) = (&here.at, kept.loose_after <= now()) {
        let mut loose = vec![];
        for (mut l, path) in loose_retroarch(p, ra, &here.roms, held) {
            if now() - l.file.modified < STILL {
                continue;
            }
            l.file.sha1 = crate::sha1_file(&path)?;
            if !kept.loose.contains(&l.file.sha1) && !loose.iter().any(|(x, _): &(Loose, PathBuf)| x.file.sha1 == l.file.sha1) {
                loose.push((l, path));
            }
        }
        for chunk in loose.chunks(50) {
            if !s.loose(chunk)? {
                // (a library too old for them: asked again tomorrow)
                kept.loose_after = now() + 24 * 3600;
                break;
            }
            kept.loose.extend(chunk.iter().map(|(l, _)| l.file.sha1.clone()));
            sum.loose += chunk.len() as u32;
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

/// The library's newest version written into `dir` under the device's name for the game,
/// each file checked, written beside and then put in place.
fn bring(s: &dyn Library, kept: &mut Kept, hk: &str, dir: &Path, stem: &str, u: &UnitState) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("couldn't make {}", dir.display()))?;
    let mut done = vec![];
    for f in &u.current.files {
        let bytes = s.bytes(&u.unit, &u.current.id, &f.name)?;
        let sha1 = hex::encode(<sha1::Sha1 as sha1::Digest>::digest(&bytes));
        if sha1 != f.sha1 {
            anyhow::bail!("{} came with the wrong SHA-1.", f.name);
        }
        let to = dir.join(f.name.replace("{name}", stem));
        if let Some(up) = to.parent() {
            std::fs::create_dir_all(up).with_context(|| format!("couldn't make {}", up.display()))?;
        }
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
    kept.had.insert(hk.to_string(), had_of(&u.current.id, &u.current.files));
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

    fn built(id: &str) -> Profile {
        gamerepo_wire::saves::built_in().into_iter().find(|p| p.id == id).unwrap()
    }

    #[test]
    fn a_patterns_parts_match_names_and_say_the_slot() {
        let m = |p: &str, n: &str| match_path(&p.split('/').collect::<Vec<_>>(), &n.split('/').collect::<Vec<_>>());
        assert_eq!(m("PSP/SAVEDATA/ULUS10041*/*", "PSP/SAVEDATA/ULUS10041DATA00/DATA.BIN"), Some(String::new()));
        assert_eq!(m("PSP/SAVEDATA/ULUS10041*/*", "psp/savedata/ULUS10042DATA00/DATA.BIN"), None);
        assert_eq!(m("StateSaves/GALE01.s{slot}", "StateSaves/GALE01.s07"), Some("07".into()));
        assert_eq!(m("savestates/SLUS-00594_{slot}.sav", "savestates/SLUS-00594_resume.sav"), Some("auto".into()));
        assert_eq!(m("Wii/title/00010000/52534245/data/**", "Wii/title/00010000/52534245/data/sub/banner.bin"), Some(String::new()));
        assert_eq!(m("sdmc/Nintendo 3DS/*/*/title/00040000/00030700/data/**", "sdmc/Nintendo 3DS/0000/0000/title/00040000/00030700/data/00000001/main"), Some(String::new()));
        assert_eq!(m("{x}.sav", "a.sav"), None, "a pattern isn't matched as it is");
        let t = tokens("wii/Wii Sports (USA)", "Wii Sports", "RSPE01");
        assert_eq!(fill("Wii/title/00010000/{id4_hex}/data/**", &t).as_deref(), Some("Wii/title/00010000/52535045/data/**"));
        assert_eq!(fill("GC/{region}/Card A/*-{id4}-*.gci", &t).as_deref(), Some("GC/USA/Card A/*-RSPE-*.gci"));
        assert_eq!(fill("{tid_low}", &t), None, "a Wii disc has no 3DS title ID");
        assert_eq!(fill("x/{tid_low}", &tokens("n3ds/G", "G", "0004000000030700")).as_deref(), Some("x/00030700"));
    }

    #[test]
    fn emulators_of_their_own_keep_saves_by_id_by_name_and_as_shared_cards() {
        let dir = std::env::temp_dir().join(format!("gamerepo-test-emus-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let roms = dir.join("ROMs");
        let w = |p: &Path, t: &str| {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, t).unwrap();
        };
        // PPSSPP: saves by the disc's ID, states with their pictures
        let psp = dir.join("PPSSPP");
        w(&psp.join("PSP/SAVEDATA/ULUS10041DATA00/DATA.BIN"), "save");
        w(&psp.join("PSP/SAVEDATA/ULUS10041DATA00/ICON0.PNG"), "icon");
        w(&psp.join("PSP/SAVEDATA/ULUS99999DATA00/DATA.BIN"), "another game");
        w(&psp.join("PSP/PPSSPP_STATE/ULUS10041_1.00_0.ppst"), "state");
        w(&psp.join("PSP/PPSSPP_STATE/ULUS10041_1.00_0.jpg"), "jpg");
        let p = built("ppsspp");
        assert_eq!(folder_of(&p, &[dir.clone()], Some(&psp)), Some(psp.clone()));
        let held = vec![Held { key: "psp/Game (USA)".into(), rom: "psp/Game (USA).cso".into() }];
        let ids: BTreeMap<String, String> = [("psp/Game (USA)".to_string(), "ULUS-10041".to_string())].into();
        let found = find_folders(&p, &psp, &roms, &held, &ids);
        let got: Vec<(String, Vec<String>)> = found.iter().map(|l| (l.unit.id(), l.files.iter().map(|f| f.name.clone()).collect())).collect();
        assert_eq!(got.len(), 2, "{got:?}");
        assert!(got.contains(&("psp/Game (USA)|ppsspp|".into(), vec!["PSP/SAVEDATA/ULUS10041DATA00/DATA.BIN".into(), "PSP/SAVEDATA/ULUS10041DATA00/ICON0.PNG".into()])) || got.contains(&("psp/Game (USA)|ppsspp|".into(), vec!["PSP/SAVEDATA/ULUS10041DATA00/ICON0.PNG".into(), "PSP/SAVEDATA/ULUS10041DATA00/DATA.BIN".into()])), "{got:?}");
        assert!(got.iter().any(|(id, f)| id == "psp/Game (USA)|ppsspp:PPSSPP|0" && f.len() == 2), "{got:?}");
        assert!(find_folders(&p, &psp, &roms, &held, &BTreeMap::new()).is_empty(), "without its ID nothing's taken");
        // a save from elsewhere goes in the emulator's folder; another emulator's nowhere
        assert_eq!(place_folders(&p, &psp, &roms, &held, &Unit::save("psp/Game (USA)", "ppsspp")), Some((psp.clone(), "Game (USA)".into())));
        assert_eq!(place_folders(&p, &psp, &roms, &held, &Unit::save("psp/Game (USA)", "retroarch")), None);
        assert_eq!(place_folders(&p, &psp, &roms, &held, &Unit::save("psp/Other", "ppsspp")), None, "not a game here");

        // PCSX2: memory cards every game shares are one save, under the system's cards
        let ps2 = dir.join("ARMSX2");
        w(&ps2.join("memcards/Mcd001.ps2"), "card");
        w(&ps2.join("sstates/SLUS-20062 (5BBF6C6D).01.p2s"), "state");
        let p = built("armsx2");
        let held = vec![Held { key: "ps2/GTA (USA)".into(), rom: "ps2/GTA (USA).chd".into() }];
        let ids: BTreeMap<String, String> = [("ps2/GTA (USA)".to_string(), "SLUS-20062".to_string())].into();
        let ids_of: Vec<String> = find_folders(&p, &ps2, &roms, &held, &ids).iter().map(|l| l.unit.id()).collect();
        assert_eq!(ids_of, ["ps2/GTA (USA)|pcsx2:ARMSX2|01", "ps2/Memory cards|pcsx2|"]);
        assert_eq!(place_folders(&p, &ps2, &roms, &held, &Unit::save("ps2/Memory cards", "pcsx2")), Some((ps2.clone(), String::new())));
        assert_eq!(place_folders(&p, &ps2, &roms, &held, &Unit::state("ps2/GTA (USA)", "pcsx2:PCSX2", "01")), None, "another PCSX2's state isn't loaded");

        // mGBA: beside the game, by its file's name
        w(&roms.join("gba/Zelda (USA).gba"), "rom");
        w(&roms.join("gba/Zelda (USA).sav"), "save");
        w(&roms.join("gba/Zelda (USA).ss2"), "state");
        let p = built("mgba");
        let held = vec![Held { key: "gba/The Legend of Zelda (USA)".into(), rom: "gba/Zelda (USA).gba".into() }];
        let f = find_folders(&p, &dir, &roms, &held, &BTreeMap::new());
        let names: Vec<(String, String)> = f.iter().map(|l| (l.unit.id(), l.files[0].name.clone())).collect();
        assert_eq!(names, [("gba/The Legend of Zelda (USA)|mgba:mGBA|2".to_string(), "{name}.ss2".to_string()), ("gba/The Legend of Zelda (USA)|mgba|".into(), "{name}.sav".into())]);
        assert_eq!(f[0].dir, roms.join("gba"));
        assert_eq!(place_folders(&p, &dir, &roms, &held, &Unit::save("gba/The Legend of Zelda (USA)", "mgba")), Some((roms.join("gba"), "Zelda (USA)".into())));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_saves_last_sync_is_kept_by_its_emulator_too() {
        let u = Unit::save("gc/Game (USA)", "dolphin");
        let k = had_key(&built("dolphin"), &u);
        assert_eq!(k, "dolphin>gc/Game (USA)|dolphin|");
        assert_eq!(unit_of(&k), Some(u.clone()));
        assert_eq!(had_key(&profile(), &Unit::save("snes/A>B", "retroarch")), "snes/A>B|retroarch|");
        assert_eq!(unit_of("snes/A>B|retroarch|"), Some(Unit::save("snes/A>B", "retroarch")));
    }
}
