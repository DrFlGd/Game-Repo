//! PC games (docs/PLAN.md 3.5, "Phase 4 design", D25, D26): one folder of games,
//! `<library>/PC/` unless Settings › PC games chooses another, each game in
//! `<Title> (<Year>)/` with `install/`, `patches/`, `extras/`, `dlc/`, an optional
//! `extracted/` and its `game.json`, which holds everything the app knows about it. The
//! folder isn't scanned: the games are read from their `game.json` files.
//!
//! Import reads an inbox (or the files chosen) into items, each hashed and identified
//! (a GOG `goggame-<id>.info`, innoextract, Steam's `steam_appid.txt`, the name, then a
//! search), and plans an Organise change that copies (or moves) them into the folder and
//! writes their `game.json`, with the preview, the journal and Undo.
//!
//! Since D55 the `game.json` here keeps what only a PC game has (its id, title and year,
//! source, version, ids, files, program, where it's installed); its details, pictures and
//! documents are in `_library/games/pc/<id>/` as every other game's are (pcmeta.rs).

use crate::archive;
use crate::config::write_json;
use crate::hash;
use crate::library::{now, Library, APP_DIR};
use crate::organise::{self, key, safe_name, Op, Plan, Src};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

pub const MANIFEST: &str = "game.json";
/// The folders of a game: what each kind of file goes in.
pub const PARTS: &[(&str, &str)] = &[("install", "install"), ("patch", "patches"), ("extra", "extras"), ("dlc", "dlc"), ("extracted", "extracted")];

// ------------------------------------------------------------ the folder

/// The PC games folder (D25).
pub fn folder(lib: &Library) -> PathBuf {
    match lib.meta()["pc"]["folder"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
        Some(f) if Path::new(f).is_absolute() => PathBuf::from(f),
        Some(f) => lib.root().join(f),
        None => lib.root().join("PC"),
    }
}

/// Choose the PC games folder (none: the library's `PC/`). Kept relative when it's in the library.
pub fn set_folder(lib: &Library, dir: Option<&Path>) -> Result<PathBuf> {
    lib.writable()?;
    let mut pc = lib.meta()["pc"].as_object().cloned().unwrap_or_default();
    match dir {
        Some(d) => {
            if d.starts_with(lib.root().join(APP_DIR)) {
                bail!("The PC games folder can't be in the library's own _library folder.");
            }
            let v = lib.relative(d).filter(|r| !r.is_empty()).unwrap_or_else(|| d.display().to_string());
            pc.insert("folder".into(), json!(v));
        }
        None => {
            pc.remove("folder");
        }
    }
    lib.update_meta(json!({ "pc": if pc.is_empty() { Value::Null } else { Value::Object(pc) } }))?;
    Ok(folder(lib))
}

// ------------------------------------------------------------ game.json

/// A file of a PC game, as `game.json` lists it.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PcFile {
    /// Its path in the game's folder, with `/`
    pub path: String,
    /// install | patch | extra | dlc
    pub kind: String,
    pub size: u64,
    pub crc32: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha1: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default)]
    pub added: String,
}

/// The program that starts a game: in its folder (`extracted/…`) or anywhere (installed).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Launch {
    pub program: String,
    #[serde(default)]
    pub args: String,
    #[serde(default)]
    pub dir: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PcGame {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub year: Option<i32>,
    /// gog | steam | retail | other
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub version: String,
    /// The sources' ids for it: gog, steam, launchbox…
    #[serde(default)]
    pub ids: BTreeMap<String, String>,
    /// What isn't a detail (a GOG slug); before D55 also its details (developer,
    /// publisher, released, genres, description) and the source they came from
    /// (`identified_by`), which pcmeta.rs moves to `_library/games/pc/<id>/`
    #[serde(default)]
    pub details: Map<String, Value>,
    #[serde(default)]
    pub files: Vec<PcFile>,
    #[serde(default)]
    pub launch: Option<Launch>,
    /// Where it's installed, when not in its folder
    #[serde(default)]
    pub installed: Option<String>,
    #[serde(default)]
    pub imported: String,
    #[serde(default)]
    pub modified: String,
    /// Keys a newer app wrote, kept as they are
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl PcGame {
    /// Its folder's name: `Title (Year)`.
    pub fn dir_name(&self) -> String {
        dir_name(&self.title, self.year)
    }
}

pub fn dir_name(title: &str, year: Option<i32>) -> String {
    let t = if title.trim().is_empty() { "Untitled" } else { title.trim() };
    safe_name(&match year {
        Some(y) => format!("{t} ({y})"),
        None => t.to_string(),
    })
}

/// A PC game and its folder.
#[derive(Clone, Debug)]
pub struct Entry {
    pub dir: PathBuf,
    pub game: PcGame,
}

impl Entry {
    /// What the page shows.
    pub fn to_json(&self, lib: &Library) -> Value {
        let g = &self.game;
        let size: u64 = g.files.iter().map(|f| f.size).sum();
        let launch = g.launch.as_ref().map(|l| program_path(&self.dir, l).display().to_string());
        json!({
            "id": g.id, "title": g.title, "year": g.year, "source": g.source, "version": g.version, "ids": g.ids,
            "details": crate::pcmeta::view(lib, self)["details"], "files": g.files, "launch": g.launch, "program": launch, "installed": g.installed,
            "imported": g.imported, "modified": g.modified, "size": size,
            "dir": self.dir.display().to_string(), "folder": self.dir.file_name().map(|n| n.to_string_lossy().into_owned()),
            "extracted": self.dir.join("extracted").is_dir(), "unpacked": g.extra.get("unpacked"),
            "media": media(lib, &g.id),
        })
    }
}

/// The full path of a game's program.
pub fn program_path(dir: &Path, l: &Launch) -> PathBuf {
    let p = Path::new(&l.program);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        l.program.split('/').fold(dir.to_path_buf(), |p, part| p.join(part))
    }
}

fn read_manifest(p: &Path) -> Option<PcGame> {
    let bytes = std::fs::read(p).ok()?;
    let g = match serde_json::from_slice::<PcGame>(&bytes) {
        Ok(g) => g,
        // edited by hand ("year": "1999", a number for an id): read what can be, rather than
        // the game going from the list and its Steam shortcut with it (M31)
        Err(_) => serde_json::from_value(lenient(serde_json::from_slice(&bytes).ok()?)).ok()?,
    };
    Some(g).filter(|g| !g.id.is_empty())
}

/// A `game.json` with each known key made the type it should be, or left out.
fn lenient(mut v: Value) -> Value {
    let text = |x: &Value| match x {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    };
    let Some(o) = v.as_object_mut() else { return v };
    for k in ["id", "title", "source", "version", "imported", "modified", "installed"] {
        if let Some(x) = o.get(k) {
            match text(x) {
                Some(t) => o.insert(k.into(), json!(t)),
                None => o.remove(k),
            };
        }
    }
    if let Some(y) = o.get("year") {
        let y = y.as_i64().or_else(|| y.as_f64().map(|f| f as i64)).or_else(|| y.as_str().and_then(|s| s.trim().parse().ok()));
        o.insert("year".into(), json!(y));
    }
    if let Some(ids) = o.get("ids").cloned() {
        let ids: Map<String, Value> = ids.as_object().into_iter().flatten().filter_map(|(k, x)| Some((k.clone(), json!(text(x)?)))).collect();
        o.insert("ids".into(), json!(ids));
    }
    if o.get("details").is_some_and(|d| !d.is_object()) {
        o.remove("details");
    }
    if let Some(files) = o.get("files").cloned() {
        let files: Vec<Value> = files.as_array().into_iter().flatten().filter(|f| serde_json::from_value::<PcFile>((*f).clone()).is_ok()).cloned().collect();
        o.insert("files".into(), json!(files));
    }
    if o.get("launch").is_some_and(|l| serde_json::from_value::<Option<Launch>>(l.clone()).is_err()) {
        o.remove("launch");
    }
    if !o.contains_key("title") {
        o.insert("title".into(), json!(""));
    }
    v
}

#[cfg(test)]
thread_local! {
    /// How many times the PC games were listed on this thread (M25's test).
    pub(crate) static LISTED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Every PC game, by title.
pub fn list(lib: &Library) -> Vec<Entry> {
    #[cfg(test)]
    LISTED.with(|n| n.set(n.get() + 1));
    let mut v: Vec<Entry> = std::fs::read_dir(folder(lib))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| Some(Entry { game: read_manifest(&e.path().join(MANIFEST))?, dir: e.path() }))
        .collect();
    v.sort_by(|a, b| a.game.title.to_lowercase().cmp(&b.game.title.to_lowercase()).then(a.game.year.cmp(&b.game.year)));
    v
}

pub fn get(lib: &Library, id: &str) -> Result<Entry> {
    list(lib).into_iter().find(|e| e.game.id == id).ok_or_else(|| anyhow!("There's no PC game {id}."))
}

/// A new game id: "pc-" and the time, as the library's other ids.
fn new_id() -> String {
    format!("pc-{}", organise::new_id().replace('-', "").to_lowercase())
}

/// The text of a game's `game.json` (sorted keys, as the library's JSON is written).
fn manifest_text(g: &PcGame) -> Result<String> {
    Ok(serde_json::to_string_pretty(&crate::config::sorted(&serde_json::to_value(g)?))? + "\n")
}

pub(crate) fn write_manifest(dir: &Path, g: &PcGame) -> Result<()> {
    write_json(&dir.join(MANIFEST), &serde_json::to_value(g)?)
}

/// Change a game's title, year, ids, program or install folder. A new title or year renames
/// its folder (when the new name is free). Its details are changed on its page (pcmeta.rs).
pub fn update(lib: &Library, id: &str, patch: &Value) -> Result<Entry> {
    lib.writable()?;
    let mut e = get(lib, id)?;
    let g = &mut e.game;
    if let Some(t) = patch["title"].as_str() {
        if t.trim().is_empty() {
            bail!("Give the game a title.");
        }
        g.title = t.trim().to_string();
    }
    if patch.get("year").is_some() {
        g.year = patch["year"].as_i64().map(|y| y as i32).filter(|y| (1950..=2100).contains(y));
    }
    for k in ["source", "version"] {
        if let Some(v) = patch[k].as_str() {
            let v = v.trim().to_string();
            if k == "source" {
                g.source = v;
            } else {
                g.version = v;
            }
        }
    }
    if let Some(ids) = patch["ids"].as_object() {
        for (k, v) in ids {
            match v.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                Some(s) => {
                    g.ids.insert(k.clone(), s.to_string());
                }
                None => {
                    g.ids.remove(k);
                }
            }
        }
    }
    if patch.get("launch").is_some() {
        g.launch = match &patch["launch"] {
            Value::Null => None,
            l => {
                let program = l["program"].as_str().unwrap_or("").trim().replace('\\', "/");
                if program.is_empty() {
                    None
                } else {
                    let p = Path::new(&program);
                    let rel = if p.is_absolute() { p.strip_prefix(&e.dir).ok().map(|r| r.to_string_lossy().replace('\\', "/")) } else { None };
                    Some(Launch { program: rel.unwrap_or(program), args: l["args"].as_str().unwrap_or("").to_string(), dir: l["dir"].as_str().unwrap_or("").to_string() })
                }
            }
        };
    }
    if patch.get("installed").is_some() {
        g.installed = patch["installed"].as_str().map(str::trim).filter(|s| !s.is_empty()).map(String::from);
        if let Some(i) = &g.installed {
            if !Path::new(i).is_dir() {
                bail!("There's no folder {i}.");
            }
        }
    }
    g.modified = now();
    let want = e.dir.with_file_name(g.dir_name());
    if key(&want) != key(&e.dir) && !want.exists() && std::fs::rename(&e.dir, &want).is_ok() {
        e.dir = want;
    } else if key(&want) != key(&e.dir) && cfg!(windows) && want.to_string_lossy().to_lowercase() == e.dir.to_string_lossy().to_lowercase() {
        // only the case changed: through a temporary name
        let tmp = e.dir.with_file_name(format!("{}.gamerepo-tmp", g.id));
        if std::fs::rename(&e.dir, &tmp).is_ok() {
            e.dir = if std::fs::rename(&tmp, &want).is_ok() { want } else { tmp };
        }
    }
    write_manifest(&e.dir, &e.game)?;
    Ok(e)
}

// ------------------------------------------------------------ media

/// Where a game's artwork is kept (D13): `_library/games/pc/<id>/media/`.
pub fn media_dir(lib: &Library, id: &str) -> PathBuf {
    lib.root().join(APP_DIR).join("games").join("pc").join(id).join("media")
}

/// A game's artwork by kind (cover, banner, hero, logo, icon): its library path.
pub fn media(lib: &Library, id: &str) -> Map<String, Value> {
    crate::identify::media_of(lib, &media_dir(lib, id))
}

/// Keep a piece of artwork (replacing the one of that kind).
pub fn save_media(lib: &Library, id: &str, kind: &str, ext: &str, bytes: &[u8]) -> Result<PathBuf> {
    crate::identify::save_media(&media_dir(lib, id), kind, ext, bytes)
}

// ------------------------------------------------------------ names

/// What a GOG installer's (or patch's) file name says: its slug and version.
/// `setup_hollow_knight_1.5.78.11833_(64bit)_(51439).exe` → ("hollow_knight", "1.5.78.11833").
pub fn gog_name(name: &str) -> Option<(String, String, bool)> {
    let lower = name.to_lowercase();
    let (patch, rest) = if let Some(r) = lower.strip_prefix("setup_") {
        (false, r)
    } else if let Some(r) = lower.strip_prefix("patch_") {
        (true, r)
    } else {
        return None;
    };
    let rest = rest.strip_suffix(".exe").or_else(|| rest.strip_suffix(".bin").map(|r| r.rsplit_once('-').map_or(r, |(a, _)| a)))?;
    let rest = if patch { rest.split("_to_").next().unwrap_or(rest) } else { rest };
    // tokens without the bracketed builds
    let tokens: Vec<&str> = rest.split('_').filter(|t| !t.is_empty() && !t.starts_with('(')).collect();
    let is_version = |t: &str| {
        let t = t.strip_prefix('v').unwrap_or(t);
        t.chars().next().is_some_and(|c| c.is_ascii_digit()) && (t.contains('.') || t.chars().all(|c| c.is_ascii_digit()) && t.len() >= 4)
    };
    let at = tokens.iter().position(|t| is_version(t)).unwrap_or(tokens.len());
    if at == 0 {
        return None;
    }
    let slug = tokens[..at].join("_");
    let version = tokens.get(at..).map(|v| v.join("_")).unwrap_or_default();
    Some((slug, version, patch))
}

/// A title from a GOG slug: "hollow_knight" → "Hollow Knight".
pub fn title_from_slug(slug: &str) -> String {
    slug.split(['_', '-'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// What a file or folder name says: title, year, version, source.
/// `Some Game (1998) [GOG] v1.2.zip` → ("Some Game", 1998, "1.2", "gog").
pub fn parse_name(name: &str) -> (String, Option<i32>, String, String) {
    let ext = Path::new(name).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let stem = if ["zip", "7z", "rar", "exe", "msi", "iso", "cue", "bin", "img", "mdf", "mds", "nrg"].contains(&ext.as_str()) { &name[..name.len() - ext.len() - 1] } else { name };
    let (mut year, mut source, mut version) = (None, String::new(), String::new());
    let mut words = String::new();
    let mut depth = 0;
    let mut group = String::new();
    for c in stem.chars() {
        match c {
            '(' | '[' | '{' => {
                depth += 1;
                group.clear();
            }
            ')' | ']' | '}' if depth > 0 => {
                depth -= 1;
                let g = group.trim().to_lowercase();
                if g.len() == 4 && g.chars().all(|c| c.is_ascii_digit()) && (g.starts_with("19") || g.starts_with("20")) {
                    year = g.parse().ok();
                } else if ["gog", "gog.com", "steam", "retail", "humble", "itch", "itch.io", "epic"].contains(&g.as_str()) {
                    source = g.trim_end_matches(".com").trim_end_matches(".io").to_string();
                }
                words.push(' ');
            }
            _ if depth > 0 => group.push(c),
            _ => words.push(c),
        }
    }
    // dots and underscores as spaces when the name has none
    let spaced = if words.contains(' ') && words.trim().contains(' ') { words.replace('_', " ") } else { words.replace(['_', '.'], " ") };
    let mut kept = vec![];
    for w in spaced.split_whitespace() {
        let lw = w.to_lowercase();
        let vnum = lw.strip_prefix('v').filter(|r| r.chars().next().is_some_and(|c| c.is_ascii_digit()));
        if let Some(v) = vnum {
            version = v.to_string();
        } else if ["gog", "steam", "setup", "installer", "-"].contains(&lw.as_str()) && !kept.is_empty() {
            if lw == "gog" || lw == "steam" {
                source = lw;
            }
        } else if version.is_empty() {
            kept.push(w);
        }
    }
    let title = kept.join(" ").trim_matches(|c: char| c == '-' || c == ' ').to_string();
    (if title.is_empty() { stem.to_string() } else { title }, year, version, source)
}

/// A title as compared: lower case letters and digits only, "&" as "and", no leading "the".
pub fn norm_title(s: &str) -> String {
    let s = s.to_lowercase().replace('&', " and ");
    let s: String = s.chars().map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect();
    let words: Vec<&str> = s.split_whitespace().collect();
    let words = if words.first() == Some(&"the") { &words[1..] } else { &words[..] };
    words.concat()
}

// ------------------------------------------------------------ what's in a file

/// What a `goggame-<id>.info` says: its id, name and program to start.
pub fn gog_info(text: &str) -> Option<(String, String, Option<Launch>)> {
    let v: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;
    let id = match &v["gameId"] {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    // a DLC's file sits beside the game's own; only the game's (gameId == rootGameId) counts
    if v["rootGameId"].as_str().is_some_and(|r| r != id) {
        return None;
    }
    let name = v["name"].as_str().unwrap_or("").to_string();
    let tasks = v["playTasks"].as_array().cloned().unwrap_or_default();
    let primary = tasks
        .iter()
        .filter(|t| t["category"].as_str().unwrap_or("game") == "game" && t["type"].as_str() != Some("URLTask"))
        .find(|t| t["isPrimary"] == true)
        .or_else(|| tasks.iter().find(|t| t["type"].as_str() != Some("URLTask") && t["path"].is_string()));
    let launch = primary.and_then(|t| {
        Some(Launch {
            program: slashes(t["path"].as_str()?),
            args: t["arguments"].as_str().unwrap_or("").to_string(),
            dir: slashes(t["workingDir"].as_str().unwrap_or("")),
        })
    });
    Some((id, name, launch))
}

/// A path as GOG writes it ("bin\\x64\\a.exe", "bin//x64//a.exe") with single forward slashes.
fn slashes(p: &str) -> String {
    p.replace('\\', "/").split('/').filter(|s| !s.is_empty()).collect::<Vec<_>>().join("/")
}

/// The Steam app id in a `steam_appid.txt`, or an `appmanifest_<id>.acf`'s name.
pub fn steam_id_of(name: &str, text: Option<&str>) -> Option<String> {
    let lower = name.rsplit('/').next().unwrap_or(name).to_lowercase();
    if lower == "steam_appid.txt" {
        let id: String = text?.trim_start_matches('\u{feff}').trim().chars().take_while(|c| c.is_ascii_digit()).collect();
        return (!id.is_empty()).then_some(id);
    }
    let id = lower.strip_prefix("appmanifest_")?.strip_suffix(".acf")?;
    (!id.is_empty() && id.chars().all(|c| c.is_ascii_digit())).then(|| id.to_string())
}

/// The GOG id innoextract finds in a GOG installer (`--gog-game-id`).
pub fn innoextract_gog_id(installer: &Path) -> Option<String> {
    let tool = crate::tools::find("innoextract")?;
    let out = crate::tools::command(&tool).arg("--gog-game-id").arg(installer).output().ok()?;
    parse_gog_game_id(&String::from_utf8_lossy(&out.stdout))
}

/// "GOG.com game ID is 1308320804" (innoextract's words; with `--silent` just the id) → the id.
pub fn parse_gog_game_id(text: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let l = l.trim();
        if l.len() >= 5 && l.chars().all(|c| c.is_ascii_digit()) {
            return Some(l.to_string());
        }
        let at = l.to_lowercase().find("game id")?;
        let id: String = l[at..].chars().skip_while(|c| !c.is_ascii_digit()).take_while(|c| c.is_ascii_digit()).collect();
        (id.len() >= 5).then_some(id)
    })
}

// ------------------------------------------------------------ the inbox

/// One of an item's files.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct InFile {
    pub path: String,
    /// Its path from the item's base (a folder item's own folder; else its name)
    pub rel: String,
    pub size: u64,
    pub crc32: String,
    #[serde(default)]
    pub sha1: Option<String>,
}

/// Something in the inbox: a game's installer (with its parts), archive, disc or folder.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Item {
    pub key: String,
    pub name: String,
    /// gog | installer | msi | archive | disc | folder | patch | other
    pub kind: String,
    /// The folder its files' `rel` paths start from
    pub base: String,
    pub files: Vec<InFile>,
    /// GOG patches for the same game
    pub patches: Vec<InFile>,
    pub title: String,
    pub year: Option<i32>,
    pub version: String,
    pub source: String,
    pub ids: BTreeMap<String, String>,
    pub launch: Option<Launch>,
    pub details: Map<String, Value>,
    /// How it was identified: goggame | innoextract | steam | name | search | you
    pub found: String,
    /// ready | look (needs a look) | have (in the library already) | update (goes into a game there)
    pub status: String,
    /// The PC game it goes into (or is in already), or "item:<key>" for another item's new game
    pub game: Option<String>,
    pub game_title: Option<String>,
    /// install | patch | extra | dlc | extracted
    pub part: String,
    pub include: bool,
    pub note: Option<String>,
    /// Artwork a source offered, fetched once the import has run: kind → URL
    pub art: BTreeMap<String, String>,
}

/// The items in an inbox folder: what's in it, without the app's own folders.
pub fn inbox_paths(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("couldn't read {}", dir.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let n = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            !n.starts_with('.') && !n.ends_with(".part") && !n.ends_with(".crdownload") && !n.ends_with(".tmp") && n != "desktop.ini" && n != "thumbs.db"
        })
        .collect();
    v.sort();
    Ok(v)
}

fn kind_of_file(name: &str) -> &'static str {
    let lower = name.to_lowercase();
    let ext = crate::platforms::ext_of(&lower);
    match ext.as_str() {
        "exe" if gog_name(&lower).is_some_and(|g| g.2) => "patch",
        "exe" if gog_name(&lower).is_some() => "gog",
        "exe" => "installer",
        "msi" => "msi",
        "zip" | "7z" | "rar" => "archive",
        "iso" | "cue" | "bin" | "img" | "mdf" | "mds" | "nrg" | "ccd" | "sub" => "disc",
        _ => "other",
    }
}

/// Group the paths into items: a GOG installer with its `.bin` parts and patches, a
/// disc's sheet with its tracks, everything else on its own.
fn group(paths: &[PathBuf]) -> Vec<Item> {
    let mut items: Vec<Item> = vec![];
    let mut by_stem: HashMap<String, usize> = HashMap::new();
    let mut gog_slugs: HashMap<String, usize> = HashMap::new();
    let mut patches: Vec<(String, PathBuf)> = vec![];
    let mut sorted: Vec<&PathBuf> = paths.iter().collect();
    // installers before their parts and patches
    sorted.sort_by_key(|p| {
        let n = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        (n.ends_with(".bin") || n.starts_with("patch_"), n)
    });
    for p in sorted {
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let base = p.parent().map(|d| d.display().to_string()).unwrap_or_default();
        if p.is_dir() {
            items.push(Item { key: format!("i{}", items.len()), name: name.clone(), kind: "folder".into(), base: p.display().to_string(), part: "extracted".into(), ..Default::default() });
            continue;
        }
        let lower = name.to_lowercase();
        let file = InFile { path: p.display().to_string(), rel: name.clone(), ..Default::default() };
        if let Some((slug, _, is_patch)) = gog_name(&lower) {
            if is_patch {
                patches.push((slug, p.clone()));
                continue;
            }
            if lower.ends_with(".bin") {
                if let Some(&i) = gog_slugs.get(&slug) {
                    items[i].files.push(file);
                    continue;
                }
            } else {
                gog_slugs.insert(slug, items.len());
            }
        }
        let stem = Path::new(&lower).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let kind = kind_of_file(&name);
        if kind == "disc" {
            if let Some(&i) = by_stem.get(&stem) {
                items[i].files.push(file);
                continue;
            }
            by_stem.insert(stem, items.len());
        }
        let part = if kind == "other" { "extra" } else { "install" };
        items.push(Item { key: format!("i{}", items.len()), name, kind: kind.into(), base, files: vec![file], part: part.into(), ..Default::default() });
    }
    for (slug, p) in patches {
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let file = InFile { path: p.display().to_string(), rel: name.clone(), ..Default::default() };
        match gog_slugs.get(&slug) {
            Some(&i) => items[i].patches.push(file),
            None => items.push(Item {
                key: format!("i{}", items.len()), name, kind: "patch".into(), base: p.parent().map(|d| d.display().to_string()).unwrap_or_default(),
                files: vec![file], part: "patch".into(), ..Default::default()
            }),
        }
    }
    items
}

/// The files of a folder item, below it.
fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).collect();
    v.sort();
    for p in v {
        if p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(".gamerepo")) {
            continue;
        }
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

fn rel_of(base: &Path, p: &Path) -> String {
    p.strip_prefix(base).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_else(|_| p.file_name().unwrap_or_default().to_string_lossy().into_owned())
}

/// What one file found inside an item says (`at`: where the item's files go, for the program).
fn found_inside(item: &mut Item, name: &str, text: Option<&str>, at: &str) {
    let base = name.rsplit('/').next().unwrap_or(name).to_lowercase();
    if base.starts_with("goggame-") && base.ends_with(".info") {
        let id_from_name = base.trim_start_matches("goggame-").trim_end_matches(".info").to_string();
        if let Some((id, title, launch)) = text.and_then(gog_info) {
            item.ids.insert("gog".into(), id);
            if !title.is_empty() {
                item.title = title;
            }
            if let Some(mut l) = launch {
                // the info file's paths are from its own folder
                let dir = name.rsplit_once('/').map(|(d, _)| format!("{d}/")).unwrap_or_default();
                l.program = format!("{at}{dir}{}", l.program);
                if !l.dir.is_empty() {
                    l.dir = format!("{at}{dir}{}", l.dir);
                }
                item.launch.get_or_insert(l);
            }
        } else if !id_from_name.is_empty() && id_from_name.chars().all(|c| c.is_ascii_digit()) {
            item.ids.entry("gog".into()).or_insert(id_from_name);
        }
        item.found = "goggame".into();
        item.source = "gog".into();
    } else if let Some(id) = steam_id_of(name, text) {
        item.ids.entry("steam".into()).or_insert(id);
        if item.found != "goggame" {
            item.found = "steam".into();
            item.source = "steam".into();
        }
    }
}

fn names_the_game(n: &str) -> bool {
    let b = n.rsplit('/').next().unwrap_or(n).to_lowercase();
    (b.starts_with("goggame-") && b.ends_with(".info")) || b == "steam_appid.txt"
}

/// Look inside an item's files for what names the game: a `goggame-<id>.info`, a
/// `steam_appid.txt`. A folder's files are read; an archive's names are listed and its
/// small files read (only from ZIPs, or other archives under 256 MB, which have to be
/// unpacked from the start).
fn look_inside(item: &mut Item, cancel: &AtomicBool) {
    let mut seen: Vec<(String, Option<String>)> = vec![];
    if item.kind == "folder" {
        for f in item.files.iter().filter(|f| names_the_game(&f.rel) && f.size < 1 << 20) {
            seen.push((f.rel.clone(), std::fs::read_to_string(&f.path).ok()));
        }
    } else if item.kind == "archive" {
        let path = PathBuf::from(&item.files[0].path);
        let kind = archive::kind_of(&path.to_string_lossy()).unwrap_or("zip");
        let Ok(listing) = archive::read(&path, kind, false, cancel) else { return };
        let names: Vec<String> = listing.members.iter().map(|m| m.name.clone()).filter(|n| names_the_game(n)).collect();
        let mut texts: HashMap<String, String> = HashMap::new();
        if !names.is_empty() && (kind == "zip" || item.files[0].size < 256 << 20) {
            let _ = archive::each_wanted(&path, &names_the_game, cancel, &mut |n, size, r| {
                if size < 1 << 20 {
                    let mut s = String::new();
                    r.read_to_string(&mut s)?;
                    texts.insert(n.to_string(), s);
                }
                Ok(())
            });
        }
        seen.extend(names.into_iter().map(|n| {
            let t = texts.remove(&n);
            (n, t)
        }));
    }
    // a goggame file first: it says the most
    seen.sort_by_key(|(n, _)| !n.to_lowercase().contains("goggame-"));
    for (n, text) in seen {
        found_inside(item, &n, text.as_deref(), "extracted/");
    }
}

/// A source's answer for an item, used when its title (and year, if both have one) agree.
#[derive(Clone, Debug, Default)]
pub struct Match {
    pub source: String,
    pub id: String,
    pub title: String,
    pub year: Option<i32>,
    pub details: Map<String, Value>,
    pub art: BTreeMap<String, String>,
}

/// Read the paths into items: grouped, hashed, looked inside and identified, then
/// checked against the PC games there already. `search` asks the sources about an item
/// nothing inside names (by title and year).
pub fn read_items(
    lib: &Library,
    paths: &[PathBuf],
    search: &dyn Fn(&str, Option<i32>) -> Option<Match>,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Value),
) -> Result<Vec<Item>> {
    let mut items = group(paths);
    // a folder's files
    for it in items.iter_mut().filter(|i| i.kind == "folder") {
        let base = PathBuf::from(&it.base);
        let mut files = vec![];
        walk(&base, &mut files);
        it.files = files.iter().map(|p| InFile { path: p.display().to_string(), rel: rel_of(&base, p), ..Default::default() }).collect();
    }
    let total: u64 = items.iter().flat_map(|i| i.files.iter().chain(i.patches.iter())).map(|f| std::fs::metadata(&f.path).map(|m| m.len()).unwrap_or(0)).sum();
    let mut done: u64 = 0;
    let n = items.len();
    for (i, it) in items.iter_mut().enumerate() {
        for f in it.files.iter_mut().chain(it.patches.iter_mut()) {
            if cancel.load(Ordering::Relaxed) {
                bail!("stopped");
            }
            progress(json!({ "item": i + 1, "items": n, "name": f.rel, "done": done, "total": total }));
            let mut file = std::fs::File::open(&f.path).with_context(|| format!("couldn't read {}", f.path))?;
            let size = file.metadata()?.len();
            let h = hash::hash_reader(&mut file, size, "", cancel).with_context(|| format!("couldn't read {}", f.path))?;
            f.size = size;
            f.crc32 = h.crc32;
            f.sha1 = h.sha1;
            done += size;
        }
        identify_item(it, cancel);
    }
    // what's in the library already
    let games = list(lib);
    let mut by_sha1: HashMap<String, (String, String)> = HashMap::new();
    for e in &games {
        for f in &e.game.files {
            if let Some(s) = &f.sha1 {
                by_sha1.insert(s.clone(), (e.game.id.clone(), e.game.title.clone()));
            }
        }
    }
    for (i, it) in items.iter_mut().enumerate() {
        // each may ask a source on the web: Stop is heard between them (M30)
        if cancel.load(Ordering::Relaxed) {
            bail!("stopped");
        }
        progress(json!({ "item": i + 1, "items": n, "name": it.name, "done": total, "total": total }));
        let held: Vec<&(String, String)> = it.files.iter().filter_map(|f| f.sha1.as_ref().and_then(|s| by_sha1.get(s))).collect();
        if !it.files.is_empty() && held.len() == it.files.len() && it.patches.iter().all(|p| p.sha1.as_ref().is_some_and(|s| by_sha1.contains_key(s))) {
            it.status = "have".into();
            it.game = Some(held[0].0.clone());
            it.game_title = Some(held[0].1.clone());
            it.include = false;
            continue;
        }
        if it.found.is_empty() || it.found == "name" {
            if let Some(m) = search(&it.title, it.year) {
                apply_match(it, &m);
                it.found = "search".into();
            }
        }
        // the same game by a source's id
        let same = games.iter().find(|e| it.ids.iter().any(|(k, v)| e.game.ids.get(k) == Some(v)));
        match same {
            Some(e) => {
                it.status = "update".into();
                it.game = Some(e.game.id.clone());
                it.game_title = Some(e.game.title.clone());
            }
            None => {
                it.status = if it.found == "name" || it.found.is_empty() || it.kind == "other" || it.kind == "patch" { "look" } else { "ready" }.into();
            }
        }
        it.include = it.status != "have";
    }
    Ok(items)
}

/// Put what a source said about an item into it.
pub fn apply_match(it: &mut Item, m: &Match) {
    it.ids.insert(m.source.clone(), m.id.clone());
    if !m.title.is_empty() {
        it.title = m.title.clone();
    }
    if m.year.is_some() {
        it.year = m.year;
    }
    for (k, v) in &m.details {
        it.details.insert(k.clone(), v.clone());
    }
    // which source the details are from, for its answer once it's imported (D55)
    it.details.insert("identified_by".into(), json!(m.source));
    for (k, v) in &m.art {
        it.art.insert(k.clone(), v.clone());
    }
    if it.source.is_empty() && (m.source == "gog" || m.source == "steam") {
        it.source = m.source.clone();
    }
}

/// What the item's files and name say about it.
fn identify_item(it: &mut Item, cancel: &AtomicBool) {
    let first = it.files.first().map(|f| f.rel.clone()).unwrap_or_else(|| it.name.clone());
    match it.kind.as_str() {
        "gog" | "patch" => {
            let (slug, version, _) = gog_name(&first.to_lowercase()).unwrap_or_default();
            it.title = title_from_slug(&slug);
            it.version = version;
            it.source = "gog".into();
            it.found = "name".into();
            it.details.insert("gog_slug".into(), json!(slug));
            if it.kind == "gog" {
                if let Some(id) = it.files.first().and_then(|f| innoextract_gog_id(Path::new(&f.path))) {
                    it.ids.insert("gog".into(), id);
                    it.found = "innoextract".into();
                }
            }
        }
        _ => {
            let (title, year, version, source) = parse_name(&it.name);
            it.title = title;
            it.year = year;
            it.version = version;
            it.source = source;
            it.found = "name".into();
            look_inside(it, cancel);
        }
    }
    if it.source.is_empty() && it.kind != "other" {
        it.source = "other".into();
    }
}

// ------------------------------------------------------------ the import

/// Plan the import of these items (as the page left them): an Organise change (D26).
/// `mode`: copy (hardlinked on the same drive unless `always_copy`) or move.
pub fn plan_import(lib: &Library, items: &[Item], mode: &str, always_copy: bool) -> Result<Plan> {
    lib.writable()?;
    let root = folder(lib);
    if !["copy", "move"].contains(&mode) {
        bail!("Copy or move?");
    }
    let mut existing: HashMap<String, Entry> = list(lib).into_iter().map(|e| (e.game.id.clone(), e)).collect();
    // each new game, by the item that makes it
    struct Target {
        dir: PathBuf,
        game: PcGame,
        old: Option<PathBuf>,
        ops: Vec<Op>,
    }
    let mut targets: Vec<Target> = vec![];
    let mut by_item: HashMap<String, usize> = HashMap::new();
    let mut by_game: HashMap<String, usize> = HashMap::new();
    let mut claimed: HashSet<String> = HashSet::new();
    let stamp = now();
    let included: Vec<&Item> = items.iter().filter(|i| i.include).collect();
    // items naming another item's game come after it
    let mut order: Vec<&Item> = included.iter().copied().filter(|i| !i.game.as_deref().is_some_and(|g| g.starts_with("item:"))).collect();
    order.extend(included.iter().copied().filter(|i| i.game.as_deref().is_some_and(|g| g.starts_with("item:"))));
    for it in order {
        if it.files.is_empty() && it.patches.is_empty() {
            continue;
        }
        let t = match it.game.as_deref() {
            Some(g) if g.starts_with("item:") => *by_item.get(&g[5..]).ok_or_else(|| anyhow!("{} goes with an item that isn't being imported.", it.name))?,
            Some(g) if existing.contains_key(g) || by_game.contains_key(g) => match by_game.get(g) {
                Some(&t) => t,
                None => {
                    let e = existing.remove(g).unwrap();
                    targets.push(Target { old: Some(e.dir.join(MANIFEST)), dir: e.dir, game: e.game, ops: vec![] });
                    by_game.insert(g.to_string(), targets.len() - 1);
                    targets.len() - 1
                }
            },
            _ => {
                if it.title.trim().is_empty() {
                    bail!("{} needs a title.", it.name);
                }
                let mut name = dir_name(&it.title, it.year);
                let mut n = 2;
                while root.join(&name).exists() || claimed.contains(&name.to_lowercase()) {
                    name = format!("{} {n}", dir_name(&it.title, it.year));
                    n += 1;
                }
                claimed.insert(name.to_lowercase());
                let game = PcGame {
                    id: new_id(),
                    title: it.title.trim().to_string(),
                    year: it.year,
                    source: it.source.clone(),
                    version: it.version.clone(),
                    ids: it.ids.clone(),
                    details: it.details.clone(),
                    imported: stamp.clone(),
                    ..Default::default()
                };
                targets.push(Target { dir: root.join(name), game, old: None, ops: vec![] });
                targets.len() - 1
            }
        };
        by_item.insert(it.key.clone(), t);
        let target = &mut targets[t];
        let part = PARTS.iter().find(|(k, _)| *k == it.part).map(|(_, d)| *d).unwrap_or("install");
        if target.game.launch.is_none() {
            target.game.launch = it.launch.clone().filter(|_| part == "extracted");
        }
        for (k, v) in &it.ids {
            target.game.ids.entry(k.clone()).or_insert_with(|| v.clone());
        }
        if !it.version.is_empty() && it.part == "install" {
            target.game.version = it.version.clone();
        }
        let base = PathBuf::from(&it.base);
        for (f, kind) in it.files.iter().map(|f| (f, it.part.as_str())).chain(it.patches.iter().map(|f| (f, "patch"))) {
            let dir = PARTS.iter().find(|(k, _)| *k == kind).map(|(_, d)| *d).unwrap_or("install");
            let rel = format!("{dir}/{}", f.rel);
            // there already (the same file) or a name to find
            if let Some(have) = target.game.files.iter().find(|x| x.path.eq_ignore_ascii_case(&rel)) {
                if have.sha1 == f.sha1 {
                    continue;
                }
            }
            let mut rel = rel;
            let mut n = 2;
            while target.game.files.iter().any(|x| x.path.eq_ignore_ascii_case(&rel)) || (target.old.is_some() && rel_path(&target.dir, &rel).exists()) {
                let p = Path::new(&f.rel);
                let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
                let parent = f.rel.rsplit_once('/').map(|(d, _)| format!("{d}/")).unwrap_or_default();
                rel = format!("{dir}/{parent}{stem} ({n}){ext}");
                n += 1;
            }
            let to = rel_path(&target.dir, &rel);
            let src = Src {
                file_id: 0,
                root: format!("dir:{}", base.display()),
                rel: f.rel.clone(),
                path: f.path.clone(),
                member: None,
                size: f.size,
                crc32: f.crc32.clone(),
                sha1: f.sha1.clone(),
            };
            let op = match mode {
                "move" => "move",
                _ if !always_copy && organise::same_drive(Path::new(&f.path), &target.dir) => "link",
                _ => "copy",
            };
            target.ops.push(Op { op: op.into(), game: target.game.title.clone(), to: to.display().to_string(), from: vec![src], names: vec![], aside: vec![], size: f.size, note: None });
            target.game.files.push(PcFile {
                path: rel,
                kind: if kind == "extracted" { "extracted".into() } else { kind.to_string() },
                size: f.size,
                crc32: f.crc32.clone(),
                sha1: f.sha1.clone(),
                version: (!it.version.is_empty()).then(|| it.version.clone()),
                added: stamp.clone(),
            });
        }
        if !it.art.is_empty() {
            let art = target.game.extra.entry("art").or_insert_with(|| json!({}));
            for (k, v) in &it.art {
                art[k] = json!(v);
            }
            // the source it's from, so it's kept as that source's (D55)
            if let Some(s) = it.details.get("identified_by") {
                target.game.extra.insert("art_from".into(), s.clone());
            }
        }
    }
    let mut ops = vec![];
    let mut games = 0;
    for mut t in targets {
        if t.ops.is_empty() {
            continue;
        }
        games += 1;
        t.game.modified = stamp.clone();
        let text = manifest_text(&t.game)?;
        let to = t.dir.join(MANIFEST);
        let from = match &t.old {
            Some(old) => vec![Src {
                file_id: 0, root: format!("dir:{}", t.dir.display()), rel: MANIFEST.into(), path: old.display().to_string(), member: None,
                size: std::fs::metadata(old).map(|m| m.len()).unwrap_or(0), crc32: String::new(), sha1: None,
            }],
            None => vec![],
        };
        let title = t.game.title.clone();
        ops.extend(t.ops);
        ops.push(Op { op: "text".into(), game: title, to: to.display().to_string(), from, names: vec![text.clone()], aside: vec![], size: text.len() as u64, note: Some("game.json".into()) });
    }
    let n = games;
    Ok(Plan {
        id: organise::new_id(),
        created: now(),
        kind: "import".into(),
        label: format!("Imported {n} PC game{}", if n == 1 { "" } else { "s" }),
        title: format!("Import {n} PC game{}", if n == 1 { "" } else { "s" }),
        dat: Value::Null,
        platform: "pc".into(),
        mode: mode.into(),
        format: "keep".into(),
        out: Some(root.display().to_string()),
        space: organise::space_needed(&ops),
        ops,
        games: n,
        in_place: 0,
        conflicts: vec![],
        duplicates: vec![],
        left: vec![],
        new_root: None, new_roots: vec![],
        present: vec![],
        export: None,
    })
}

fn rel_path(dir: &Path, rel: &str) -> PathBuf {
    rel.split('/').filter(|p| !p.is_empty() && *p != "." && *p != "..").fold(dir.to_path_buf(), |p, part| p.join(safe_name(part)))
}

// ------------------------------------------------------------ unpacking, the program

/// Files that start something other than the game.
pub fn not_the_game(name: &str) -> bool {
    let n = name.to_lowercase();
    ["unins", "setup", "install", "vcredist", "vc_redist", "dxsetup", "dxwebsetup", "directx", "crashhandler", "crashreport", "uploader", "redist", "dotnet", "physx", "oalinst", "ue4prereq", "cleanup"]
        .iter()
        .any(|w| n.contains(w))
}

/// The programs in a game's unpacked copy (or where it's installed), the likeliest first.
pub fn programs(lib: &Library, id: &str) -> Result<Vec<String>> {
    let e = get(lib, id)?;
    let mut found = vec![];
    for (base, prefix) in [(e.dir.join("extracted"), Some("extracted/")), (e.game.installed.as_ref().map(PathBuf::from).unwrap_or_default(), None)] {
        if !base.is_dir() {
            continue;
        }
        let mut files = vec![];
        walk(&base, &mut files);
        for p in files {
            let ext = crate::platforms::ext_of(&p.to_string_lossy());
            if !["exe", "bat", "cmd", "lnk"].contains(&ext.as_str()) {
                continue;
            }
            let rel = rel_of(&base, &p);
            let depth = rel.matches('/').count();
            let shown = match prefix {
                Some(pre) => format!("{pre}{rel}"),
                None => p.display().to_string(),
            };
            found.push((not_the_game(&rel), depth, shown));
        }
    }
    found.sort();
    Ok(found.into_iter().map(|(_, _, p)| p).collect())
}

/// Unpack a game's archive (by the app) or GOG installer (by innoextract) into its
/// `extracted/` folder, and take the program to start from `goggame-*.info` (or the only
/// likely program). `file`: which of its files (its first installer or archive if none).
pub fn unpack(lib: &Library, id: &str, file: Option<&str>, cancel: &AtomicBool, progress: &mut dyn FnMut(Value)) -> Result<Value> {
    lib.writable()?;
    let e = get(lib, id)?;
    let f = match file {
        Some(p) => e.game.files.iter().find(|x| x.path == p).ok_or_else(|| anyhow!("{} has no file {p}.", e.game.title))?,
        None => e
            .game
            .files
            .iter()
            .filter(|x| x.kind == "install")
            .find(|x| archive::kind_of(&x.path).is_some() || gog_name(x.path.rsplit('/').next().unwrap_or("")).is_some_and(|g| !g.2) && x.path.to_lowercase().ends_with(".exe"))
            .ok_or_else(|| anyhow!("{} has no archive or GOG installer to unpack. Install it, then use Installed at….", e.game.title))?,
    };
    let src = rel_path(&e.dir, &f.path);
    let out = e.dir.join("extracted");
    if out.exists() && std::fs::read_dir(&out)?.next().is_some() {
        bail!("{} is unpacked already (remove the unpacked copy first).", e.game.title);
    }
    let staging = e.dir.join(".gamerepo-unpacking");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)?;
    let r = (|| -> Result<()> {
        if archive::kind_of(&f.path).is_some() {
            let total = f.size.max(1);
            let mut done: u64 = 0;
            archive::each(&src, cancel, &mut |name, size, r| {
                let rel = crate::library::rel_inside(name).map_err(|e| std::io::Error::other(e.to_string()))?;
                let to = staging.join(rel);
                if let Some(d) = to.parent() {
                    std::fs::create_dir_all(d)?;
                }
                let mut w = std::fs::File::create(&to)?;
                std::io::copy(r, &mut w)?;
                done += size;
                progress(json!({ "name": name, "done": done.min(total), "total": total }));
                Ok(())
            })
            .with_context(|| format!("couldn't unpack {}", src.display()))?;
        } else {
            let tool = crate::tools::find("innoextract").ok_or_else(|| anyhow!("Unpacking a GOG installer needs innoextract (Settings › Tools)."))?;
            progress(json!({ "name": f.path, "done": 0, "total": 1 }));
            let out = crate::tools::command(&tool)
                .args(["--gog", "--exclude-temp", "--silent", "--output-dir"])
                .arg(&staging)
                .arg(&src)
                .output()
                .context("couldn't run innoextract")?;
            if !out.status.success() {
                bail!("innoextract couldn't unpack {}: {}", f.path, String::from_utf8_lossy(&out.stderr).trim());
            }
        }
        if cancel.load(Ordering::Relaxed) {
            bail!("stopped");
        }
        Ok(())
    })();
    if let Err(err) = r {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(err);
    }
    let _ = std::fs::remove_dir(&out);
    std::fs::rename(&staging, &out)?;
    // the program to start
    let mut files = vec![];
    walk(&out, &mut files);
    let mut launch = None;
    for p in &files {
        let name = p.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
        if name.starts_with("goggame-") && name.ends_with(".info") {
            if let Some((gid, _, Some(mut l))) = std::fs::read_to_string(p).ok().as_deref().and_then(gog_info) {
                let dir = rel_of(&out, p.parent().unwrap_or(&out));
                let pre = if dir.is_empty() || p.parent() == Some(out.as_path()) { "extracted/".to_string() } else { format!("extracted/{dir}/") };
                l.program = format!("{pre}{}", l.program);
                if !l.dir.is_empty() {
                    l.dir = format!("{pre}{}", l.dir);
                }
                launch = Some((gid, l));
                break;
            }
        }
    }
    let mut g = get(lib, id)?;
    if let Some((gid, l)) = launch {
        g.game.ids.entry("gog".into()).or_insert(gid);
        g.game.launch.get_or_insert(l);
    }
    if g.game.launch.is_none() {
        let progs: Vec<String> = programs(lib, id)?.into_iter().filter(|p| !not_the_game(p)).collect();
        if progs.len() == 1 {
            g.game.launch = Some(Launch { program: progs[0].clone(), ..Default::default() });
        }
    }
    g.game.extra.insert("unpacked".into(), json!({ "from": f.path, "at": now() }));
    g.game.modified = now();
    write_manifest(&g.dir, &g.game)?;
    Ok(json!({ "id": id, "title": g.game.title, "files": files.len(), "launch": g.game.launch }))
}

/// Remove the copy Unpack made (only one Unpack made).
pub fn remove_unpacked(lib: &Library, id: &str) -> Result<()> {
    lib.writable()?;
    let mut e = get(lib, id)?;
    if !e.game.extra.contains_key("unpacked") {
        bail!("{}'s extracted folder came with the game, not from Unpack, so it isn't removed here.", e.game.title);
    }
    let out = e.dir.join("extracted");
    if out.exists() {
        std::fs::remove_dir_all(&out).with_context(|| format!("couldn't remove {}", out.display()))?;
    }
    e.game.extra.remove("unpacked");
    if e.game.launch.as_ref().is_some_and(|l| l.program.starts_with("extracted/")) {
        e.game.launch = None;
    }
    e.game.modified = now();
    write_manifest(&e.dir, &e.game)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::library::tests::temp_dir;

    #[test]
    fn names_say_what_they_can() {
        assert_eq!(gog_name("setup_hollow_knight_1.5.78.11833_(64bit)_(51439).exe"), Some(("hollow_knight".into(), "1.5.78.11833".into(), false)));
        assert_eq!(gog_name("setup_hollow_knight_1.5.78.11833_(64bit)_(51439)-1.bin").map(|g| g.0), Some("hollow_knight".into()));
        assert_eq!(gog_name("setup_doom_3_1.3.1_(20235).exe").map(|g| (g.0, g.1)), Some(("doom_3".into(), "1.3.1".into())));
        assert_eq!(gog_name("patch_hollow_knight_1.4_(1)_to_1.5_(2).exe").map(|g| (g.0, g.2)), Some(("hollow_knight".into(), true)));
        assert_eq!(gog_name("installer.exe"), None);
        assert_eq!(title_from_slug("hollow_knight"), "Hollow Knight");
        assert_eq!(parse_name("Some Game (1998) [GOG] v1.2.zip"), ("Some Game".into(), Some(1998), "1.2".into(), "gog".into()));
        assert_eq!(parse_name("Another_Game_v2.0.7z").0, "Another Game");
        assert_eq!(parse_name("Third.Game.2004.iso").0, "Third Game 2004");
        assert_eq!(norm_title("The Witcher 3: Wild Hunt"), norm_title("Witcher 3 - Wild Hunt"));
        assert_eq!(parse_gog_game_id("Inspecting \"Hollow Knight\" - setup data version 6.0.0 (unicode)\nGOG.com game ID is 1308320804\n"), Some("1308320804".into()));
        let info = r#"{"gameId":"1308320804","name":"Hollow Knight","playTasks":[{"category":"document","path":"manual.pdf","type":"FileTask"},{"category":"game","isPrimary":true,"path":"hollow_knight.exe","workingDir":"","type":"FileTask"}]}"#;
        let (id, name, l) = gog_info(info).unwrap();
        assert_eq!((id.as_str(), name.as_str(), l.unwrap().program.as_str()), ("1308320804", "Hollow Knight", "hollow_knight.exe"));
        assert_eq!(steam_id_of("game/steam_appid.txt", Some("367520\n")), Some("367520".into()));
        assert_eq!(steam_id_of("appmanifest_367520.acf", None), Some("367520".into()));
    }

    /// A zip of these files.
    pub(crate) fn zip_of(p: &Path, files: &[(&str, &[u8])]) {
        let mut w = zip::ZipWriter::new(std::fs::File::create(p).unwrap());
        for (n, b) in files {
            w.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap();
            std::io::Write::write_all(&mut w, b).unwrap();
        }
        w.finish().unwrap();
    }

    pub(crate) fn no_search(_: &str, _: Option<i32>) -> Option<Match> {
        None
    }

    #[test]
    fn an_inbox_is_read_imported_undone_and_a_new_version_joins_its_game() {
        let dir = temp_dir("pc-import");
        let lib = Library::open(dir.join("lib")).unwrap();
        let inbox = dir.join("Downloads");
        std::fs::create_dir_all(&inbox).unwrap();
        std::fs::write(inbox.join("setup_hollow_knight_1.5_(64bit)_(51439).exe"), b"installer one").unwrap();
        std::fs::write(inbox.join("setup_hollow_knight_1.5_(64bit)_(51439)-1.bin"), b"part one").unwrap();
        std::fs::write(inbox.join("patch_hollow_knight_1.4_(1)_to_1.5_(2).exe"), b"a patch").unwrap();
        zip_of(&inbox.join("Zeta Quest (1999).zip"), &[("Zeta/goggame-1207658930.info", br#"{"gameId":"1207658930","name":"Zeta Quest","playTasks":[{"category":"game","isPrimary":true,"path":"zeta.exe","type":"FileTask"}]}"#), ("Zeta/zeta.exe", b"exe")]);
        std::fs::create_dir_all(inbox.join("Loose Game/bin")).unwrap();
        std::fs::write(inbox.join("Loose Game/bin/loose.exe"), b"loose").unwrap();
        std::fs::write(inbox.join("Loose Game/steam_appid.txt"), b"4000").unwrap();
        let cancel = AtomicBool::new(false);
        let paths = inbox_paths(&inbox).unwrap();
        let items = read_items(&lib, &paths, &no_search, &cancel, &mut |_| {}).unwrap();
        assert_eq!(items.len(), 3, "{items:#?}");
        let hk = items.iter().find(|i| i.kind == "gog").unwrap();
        assert_eq!((hk.title.as_str(), hk.files.len(), hk.patches.len(), hk.version.as_str(), hk.status.as_str()), ("Hollow Knight", 2, 1, "1.5", "look"));
        let zeta = items.iter().find(|i| i.kind == "archive").unwrap();
        assert_eq!((zeta.title.as_str(), zeta.ids.get("gog").map(String::as_str), zeta.found.as_str(), zeta.status.as_str(), zeta.year), ("Zeta Quest", Some("1207658930"), "goggame", "ready", Some(1999)));
        let loose = items.iter().find(|i| i.kind == "folder").unwrap();
        assert_eq!((loose.ids.get("steam").map(String::as_str), loose.files.len()), (Some("4000"), 2));
        // identified by hand, then imported
        let mut items = items;
        items.iter_mut().find(|i| i.kind == "gog").unwrap().ids.insert("gog".into(), "1308320804".into());
        let plan = plan_import(&lib, &items, "copy", true).unwrap();
        assert_eq!(plan.games, 3);
        assert_eq!((plan.title.as_str(), plan.label.as_str()), ("Import 3 PC games", "Imported 3 PC games"), "review M40");
        assert_eq!(plan.ops.iter().filter(|o| o.op == "text").count(), 3);
        let ix = crate::index::Index::open(&dir.join("index.sqlite"), &lib.id().unwrap()).unwrap();
        let r = organise::run(&ix, &lib, &plan, &cancel, &mut |_| {}).unwrap();
        assert_eq!(r["failed"].as_array().unwrap().len(), 0, "{r}");
        let games = list(&lib);
        assert_eq!(games.iter().map(|e| e.game.title.as_str()).collect::<Vec<_>>(), ["Hollow Knight", "Loose Game", "Zeta Quest"]);
        let hk = &games[0];
        assert_eq!(hk.dir.file_name().unwrap(), "Hollow Knight");
        assert!(hk.dir.join("install/setup_hollow_knight_1.5_(64bit)_(51439)-1.bin").is_file() && hk.dir.join("patches/patch_hollow_knight_1.4_(1)_to_1.5_(2).exe").is_file());
        assert_eq!(hk.game.files.len(), 3);
        let loose = &games[1];
        assert!(loose.dir.join("extracted/bin/loose.exe").is_file());
        assert!(inbox.join("Zeta Quest (1999).zip").is_file(), "copied, not moved");
        assert_eq!(games[2].dir.file_name().unwrap(), "Zeta Quest (1999)");
        // read again: all in the library already
        let again = read_items(&lib, &paths, &no_search, &cancel, &mut |_| {}).unwrap();
        assert!(again.iter().all(|i| i.status == "have" && !i.include), "{again:#?}");
        // a newer installer of the same GOG game joins it
        let newer = dir.join("newer");
        std::fs::create_dir_all(&newer).unwrap();
        std::fs::write(newer.join("setup_hollow_knight_1.6_(64bit)_(60000).exe"), b"installer two").unwrap();
        let mut items = read_items(&lib, &inbox_paths(&newer).unwrap(), &no_search, &cancel, &mut |_| {}).unwrap();
        items[0].ids.insert("gog".into(), "1308320804".into());
        items[0].game = Some(hk.game.id.clone());
        let plan2 = plan_import(&lib, &items, "move", false).unwrap();
        organise::run(&ix, &lib, &plan2, &cancel, &mut |_| {}).unwrap();
        let hk2 = get(&lib, &hk.game.id).unwrap();
        assert_eq!((hk2.game.files.len(), hk2.game.version.as_str()), (4, "1.6"));
        assert!(!newer.join("setup_hollow_knight_1.6_(64bit)_(60000).exe").exists(), "moved");
        // Undo puts the old game.json back and the installer back in the inbox
        organise::undo(&ix, &lib, &plan2.id, &cancel, &mut |_| {}).unwrap();
        assert_eq!(get(&lib, &hk.game.id).unwrap().game.files.len(), 3);
        assert!(newer.join("setup_hollow_knight_1.6_(64bit)_(60000).exe").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_game_is_unpacked_with_its_program_and_changed_by_hand() {
        let dir = temp_dir("pc-unpack");
        let lib = Library::open(dir.join("lib")).unwrap();
        let inbox = dir.join("in");
        std::fs::create_dir_all(&inbox).unwrap();
        zip_of(&inbox.join("Zeta Quest.zip"), &[("Zeta/goggame-1207658930.info", br#"{"gameId":"1207658930","name":"Zeta Quest","playTasks":[{"category":"game","isPrimary":true,"path":"bin/zeta.exe","workingDir":"bin","type":"FileTask"}]}"#), ("Zeta/bin/zeta.exe", b"exe"), ("Zeta/unins000.exe", b"x")]);
        let cancel = AtomicBool::new(false);
        let items = read_items(&lib, &inbox_paths(&inbox).unwrap(), &no_search, &cancel, &mut |_| {}).unwrap();
        let plan = plan_import(&lib, &items, "copy", true).unwrap();
        let ix = crate::index::Index::open(&dir.join("index.sqlite"), &lib.id().unwrap()).unwrap();
        organise::run(&ix, &lib, &plan, &cancel, &mut |_| {}).unwrap();
        let id = list(&lib)[0].game.id.clone();
        let r = unpack(&lib, &id, None, &cancel, &mut |_| {}).unwrap();
        assert_eq!(r["launch"]["program"], "extracted/Zeta/bin/zeta.exe", "{r}");
        let e = get(&lib, &id).unwrap();
        assert!(program_path(&e.dir, e.game.launch.as_ref().unwrap()).is_file());
        assert_eq!(programs(&lib, &id).unwrap(), ["extracted/Zeta/bin/zeta.exe", "extracted/Zeta/unins000.exe"]);
        // by hand: a year renames its folder
        let e = update(&lib, &id, &json!({ "year": 2001 })).unwrap();
        assert_eq!(e.dir.file_name().unwrap(), "Zeta Quest (2001)");
        remove_unpacked(&lib, &id).unwrap();
        let e = get(&lib, &id).unwrap();
        assert!(!e.dir.join("extracted").exists() && e.game.launch.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_hand_edited_game_stays_listed_titles_are_read_once_and_stop_is_heard() {
        let dir = temp_dir("pc-lenient");
        let lib = Library::open(dir.join("lib")).unwrap();
        // M31: a year as text and a number for an id, as a hand edit may leave them
        let x = folder(&lib).join("Xeno (1999)");
        std::fs::create_dir_all(&x).unwrap();
        std::fs::write(x.join(MANIFEST), r#"{"id":"pc-x","title":"Xeno","year":"1999","ids":{"steam":4000,"gog":null},"files":[{"path":"a"}],"launch":"xeno.exe"}"#).unwrap();
        let g = get(&lib, "pc-x").unwrap().game;
        assert_eq!((g.year, g.ids.get("steam").map(String::as_str), g.files.len(), g.launch.is_none()), (Some(1999), Some("4000"), 0, true));
        // M25: a sync reads the PC games once for every PC game's folder, not once for each
        for i in 0..30 {
            let d = folder(&lib).join(format!("Game {i}"));
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join(MANIFEST), format!(r#"{{"id":"pc-{i}","title":"Game {i}"}}"#)).unwrap();
            let m = crate::meta::games_root(&lib).join("pc").join(format!("pc-{i}"));
            std::fs::create_dir_all(&m).unwrap();
            std::fs::write(m.join("game.json"), r#"{"platform":"pc"}"#).unwrap();
        }
        let ix = crate::index::Index::open(&dir.join("index.sqlite"), &lib.id().unwrap()).unwrap();
        let before = LISTED.with(|n| n.get());
        crate::meta::sync(&ix, &lib).unwrap();
        assert!(LISTED.with(|n| n.get()) - before <= 1);
        let t: String = ix.conn().query_row("SELECT title FROM meta_games WHERE folder LIKE '%/pc-7'", [], |r| r.get(0)).unwrap();
        assert_eq!(t, "Game 7");
        // M30: Stop is heard between the items a source is asked about
        let inbox = dir.join("Downloads");
        std::fs::create_dir_all(&inbox).unwrap();
        std::fs::write(inbox.join("Alpha Thing.exe"), b"one").unwrap();
        std::fs::write(inbox.join("Beta Thing.exe"), b"two").unwrap();
        let (stop, asked) = (AtomicBool::new(false), std::sync::atomic::AtomicUsize::new(0));
        let search = |_: &str, _: Option<i32>| {
            asked.fetch_add(1, Ordering::Relaxed);
            stop.store(true, Ordering::Relaxed);
            None
        };
        assert!(read_items(&lib, &inbox_paths(&inbox).unwrap(), &search, &stop, &mut |_| {}).is_err());
        assert_eq!(asked.load(Ordering::Relaxed), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
