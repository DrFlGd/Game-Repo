//! Export layouts (docs/PLAN.md, "Phase 5b design", D36–D39): where an export puts the
//! games, gamelists, pictures, descriptions, manuals and BIOS files, as a JSON file of
//! paths made of tokens. The built-in ones ship with the app (`desktop/core/layouts/`)
//! and are changed only by saving a copy; the user's own are `_library/layouts/<id>.json`.
//! A path starts from a *place* the Export dialog asks for (`{card}`, `{roms}`…).

use crate::config;
use crate::library::{Library, APP_DIR};
use crate::meta;
use crate::organise::safe_name;
use crate::platforms;
use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

/// The layout file's version this app reads and writes.
pub const FORMAT: u32 = 1;

/// The built-in layouts, in the order the Export dialog offers them.
const BUILT_IN: &[&str] = &[
    include_str!("../layouts/esde.json"),
    include_str!("../layouts/esde-card.json"),
    include_str!("../layouts/knulli.json"),
    include_str!("../layouts/rocknix.json"),
    include_str!("../layouts/muos.json"),
    include_str!("../layouts/retroarch.json"),
    include_str!("../layouts/pegasus.json"),
    include_str!("../layouts/launchbox.json"),
    include_str!("../layouts/playnite.json"),
    include_str!("../layouts/folder.json"),
];

/// A layout. Keys the app doesn't know are kept (`extra`), at every level.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    pub format: u32,
    pub id: String,
    /// A built-in layout's own version (a copy's `copied_from` names the one it came from)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
    pub name: String,
    pub about: String,
    pub places: Vec<Place>,
    /// The layout's folder for a system (by id), with the firmware's name for it; null:
    /// the device doesn't run it. A system not listed uses its id.
    pub systems: BTreeMap<String, Option<SystemOut>>,
    pub games: Games,
    pub gamelist: Option<Gamelist>,
    /// Where each kind of picture (meta::MEDIA), the video and the manual go
    pub media: BTreeMap<String, Out>,
    /// Details written as text files (`description`)
    pub text: BTreeMap<String, Out>,
    /// Every document of a game
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documents: Option<Out>,
    pub bios: Option<Out>,
    pub pictures: Pictures,
    /// `esde-cfg`: the collection as an ES-DE custom collection
    pub collection: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copied_from: Option<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
    #[serde(skip)]
    pub builtin: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Place {
    pub id: String,
    pub label: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub example: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// It can be left empty: what goes under it isn't written
    #[serde(skip_serializing_if = "is_false")]
    pub optional: bool,
    /// Not asked for: worked out from another place (`{esde}/downloaded_media`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// ES-DE's setting that names it instead, when set (`MediaDirectory`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub es_setting: Option<String>,
    /// A path on the device the export is for (`/storage/roms`), not a folder here: what a
    /// list names the games by (RetroArch's playlists, D62); nothing is written under it
    #[serde(skip_serializing_if = "is_false")]
    pub device: bool,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SystemOut {
    Folder(String),
    Named {
        folder: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Games {
    pub path: String,
    /// `frontend`: a game of one file is the file, one of several a folder named as its
    /// sheet (as ES-DE and the firmwares show them); `naming`: as Settings › Naming says
    pub style: String,
    /// Where the games' updates, DLC and patches go, ending in `/{file}` (D66); none:
    /// beside the games
    #[serde(skip_serializing_if = "Option::is_none")]
    pub addons: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Gamelist {
    /// `es-xml`: EmulationStation's gamelist.xml; `pegasus`: Pegasus' metadata.pegasus.txt
    /// (D63); `retroarch`: a RetroArch playlist (D62); `launchbox`: a LaunchBox platform's
    /// XML file (D64)
    pub kind: String,
    pub path: String,
    /// The tag each detail goes in (meta::FIELDS' keys)
    pub fields: BTreeMap<String, String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Out {
    pub path: String,
    /// The gamelist tag it's written in, as a path from the system's games folder
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Pictures {
    /// The longest side, for this device (0: as they are); the Export dialog can change it
    pub max: u32,
    /// `keep`, or `png` for a firmware that only shows PNG
    #[serde(rename = "as")]
    pub as_: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for Pictures {
    fn default() -> Self {
        Pictures { max: 0, as_: "keep".into(), extra: Map::new() }
    }
}

/// The kinds `media` can have: the pictures and video, and the manual.
pub fn media_kinds() -> Vec<&'static str> {
    meta::MEDIA.iter().map(|m| m.0).chain(["manual"]).collect()
}

/// The details `text` can write.
pub const TEXT_KINDS: &[&str] = &["description"];

/// The kinds of list a layout can write (`gamelist.kind`).
pub const GAMELISTS: &[&str] = &["es-xml", "pegasus", "retroarch", "launchbox"];

/// The tokens a path can use, by what it's for. `playlist` is the name RetroArch knows a
/// system by: the layout's name for it when it gives one, else its No-Intro or Redump
/// name; `label` is a game's title as the layout's list finds its pictures by it
/// (`export::label`: RetroArch's thumbnails, LaunchBox's images).
const SYSTEM_TOKENS: &[&str] = &["folder", "system", "system_name", "dat_name", "playlist"];
const GAME_TOKENS: &[&str] = &["file", "stem", "title", "label"];

/// A problem with a layout: where (`media.cover`) and what, in plain words.
#[derive(Clone, Debug, Serialize)]
pub struct Problem {
    pub at: String,
    pub message: String,
}

impl Layout {
    /// The layout's folder for a system; None: the device doesn't run it.
    pub fn folder(&self, system: &str) -> Option<String> {
        match self.systems.get(system) {
            None => Some(system.to_string()),
            Some(None) => None,
            Some(Some(SystemOut::Folder(f))) => Some(f.clone()),
            Some(Some(SystemOut::Named { folder, .. })) => Some(folder.clone()),
        }
    }

    /// The layout's name for a system (muOS's catalogue name), else the app's.
    pub fn system_name(&self, system: &str) -> String {
        self.given_name(system).unwrap_or_else(|| platforms::name_of(system))
    }

    fn given_name(&self, system: &str) -> Option<String> {
        match self.systems.get(system) {
            Some(Some(SystemOut::Named { name: Some(n), .. })) if !n.trim().is_empty() => Some(n.clone()),
            _ => None,
        }
    }

    pub fn place(&self, id: &str) -> Option<&Place> {
        self.places.iter().find(|p| p.id == id)
    }

    /// The places the Export dialog asks for (those not worked out from another).
    pub fn asked(&self) -> impl Iterator<Item = &Place> {
        self.places.iter().filter(|p| p.from.is_none())
    }

    /// Every path the layout has: (where it is, the path).
    fn paths(&self) -> Vec<(String, &str)> {
        let mut v = vec![("games.path".to_string(), self.games.path.as_str())];
        if let Some(g) = &self.gamelist {
            v.push(("gamelist.path".into(), g.path.as_str()));
        }
        for (k, o) in &self.media {
            v.push((format!("media.{k}"), o.path.as_str()));
        }
        for (k, o) in &self.text {
            v.push((format!("text.{k}"), o.path.as_str()));
        }
        if let Some(d) = &self.documents {
            v.push(("documents".into(), d.path.as_str()));
        }
        if let Some(b) = &self.bios {
            v.push(("bios".into(), b.path.as_str()));
        }
        for p in &self.places {
            if let Some(f) = &p.from {
                v.push((format!("places.{}", p.id), f.as_str()));
            }
        }
        v
    }

    /// What can be carried with it: details (into a gamelist or text files), which
    /// pictures, the manual, documents, BIOS files.
    pub fn carries(&self) -> Value {
        let fields: Vec<&str> = match &self.gamelist {
            Some(g) => meta::FIELDS.iter().map(|f| f.0).filter(|k| g.fields.contains_key(*k)).collect(),
            None => self.text.keys().map(String::as_str).filter(|k| TEXT_KINDS.contains(k)).collect(),
        };
        json!({
            "fields": fields,
            "media": self.media.keys().filter(|k| *k != "manual").collect::<Vec<_>>(),
            "manuals": self.media.contains_key("manual"),
            "docs": self.documents.is_some(),
            "bios": self.bios.is_some(),
        })
    }

    /// The layout as the page shows it in a list.
    pub fn summary(&self) -> Value {
        let base = self.copied_from.as_ref().and_then(|c| c["id"].as_str()).and_then(|id| built_in().into_iter().find(|b| b.id == id));
        let newer = match (&base, &self.copied_from) {
            (Some(b), Some(c)) => b.version.unwrap_or(0) as u64 > c["version"].as_u64().unwrap_or(0),
            _ => false,
        };
        json!({
            "id": self.id, "name": self.name, "about": self.about, "builtin": self.builtin,
            "places": self.asked().map(|p| json!({ "id": p.id, "label": p.label, "example": p.example, "note": p.note, "optional": p.optional, "device": p.device })).collect::<Vec<_>>(),
            "carries": self.carries(), "pictures": self.pictures.max, "gamelist": self.gamelist.is_some(), "collection": self.collection.is_some(),
            "copied_from": self.copied_from.as_ref().map(|c| json!({ "id": c["id"], "name": base.as_ref().map(|b| b.name.clone()), "newer": newer })),
        })
    }
}

// ------------------------------------------------------------ paths and tokens

/// A path's place and its parts after it, with the tokens it uses: `{card}/roms/{folder}/{file}`
/// is ("card", ["roms", "{folder}", "{file}"]).
fn split(path: &str) -> Result<(String, Vec<String>), String> {
    let p = path.trim().replace('\\', "/");
    let Some(rest) = p.strip_prefix('{') else { return Err("It must start with a place, such as {card}.".into()) };
    let Some(end) = rest.find('}') else { return Err("A { has no }.".into()) };
    let place = rest[..end].to_string();
    let after = &rest[end + 1..];
    if !after.is_empty() && !after.starts_with('/') {
        return Err(format!("A / must follow {{{place}}}."));
    }
    let parts: Vec<String> = after.split('/').filter(|s| !s.is_empty()).map(String::from).collect();
    if parts.iter().any(|s| s == ".." || s == ".") {
        return Err("It can't go up a folder (..).".into());
    }
    Ok((place, parts))
}

/// The tokens a part uses.
fn tokens(part: &str) -> Result<Vec<String>, String> {
    let mut v = vec![];
    let mut rest = part;
    while let Some(a) = rest.find('{') {
        let Some(b) = rest[a..].find('}') else { return Err("A { has no }.".into()) };
        v.push(rest[a + 1..a + b].to_string());
        rest = &rest[a + b + 1..];
    }
    if rest.contains('}') {
        return Err("A } has no {.".into());
    }
    Ok(v)
}

/// What the tokens stand for, for one file.
pub type Vars = HashMap<&'static str, String>;

/// A path made from a layout's path: the place's folder, then each part with its tokens
/// filled in (made safe for a file name, except `bios_file`, which keeps its folders).
pub fn expand(path: &str, places: &BTreeMap<String, PathBuf>, vars: &Vars) -> Result<Option<PathBuf>> {
    let (place, parts) = split(path).map_err(|e| anyhow!("{path}: {e}"))?;
    let Some(base) = places.get(&place) else { return Ok(None) };
    let mut out = base.clone();
    for part in parts {
        let mut s = String::new();
        let mut rest = part.as_str();
        let mut raw: Option<String> = None;
        while let Some(a) = rest.find('{') {
            let b = rest[a..].find('}').ok_or_else(|| anyhow!("{path}: a {{ has no }}"))? + a;
            s.push_str(&rest[..a]);
            let name = &rest[a + 1..b];
            let value = vars.get(name).ok_or_else(|| anyhow!("{path}: {{{name}}} isn't known here"))?;
            if name == "bios_file" && part == "{bios_file}" {
                raw = Some(value.clone());
            } else {
                s.push_str(&safe_name(value));
            }
            rest = &rest[b + 1..];
        }
        s.push_str(rest);
        match raw {
            // a BIOS file's own folders (dc/dc_boot.bin)
            Some(r) => {
                for seg in r.split(['/', '\\']).filter(|x| !x.is_empty() && *x != ".." && *x != ".") {
                    out.push(safe_name(seg));
                }
            }
            None => out.push(s),
        }
    }
    Ok(Some(out))
}

/// The tokens for a system.
pub fn system_vars(l: &Layout, system: &str) -> Option<Vars> {
    let folder = l.folder(system)?;
    let dat = platforms::by_id(system).and_then(|p| p.dat_names.first().map(|s| s.to_string())).unwrap_or_else(|| system.to_string());
    let mut v = Vars::new();
    v.insert("folder", folder);
    v.insert("system", system.to_string());
    v.insert("system_name", l.system_name(system));
    v.insert("playlist", l.given_name(system).unwrap_or_else(|| dat.clone()));
    v.insert("dat_name", dat);
    Some(v)
}

/// The games' folder for a system (the games' path without its `/{file}`).
pub fn games_dir(l: &Layout, system: &str, places: &BTreeMap<String, PathBuf>) -> Result<Option<PathBuf>> {
    let Some(vars) = system_vars(l, system) else { return Ok(None) };
    let dir = l.games.path.trim().replace('\\', "/");
    let dir = dir.strip_suffix("/{file}").ok_or_else(|| anyhow!("The layout's games path must end in /{{file}}."))?;
    expand(dir, places, &vars)
}

/// Where a system's updates, DLC and patches go: the layout's `games.addons`, else the
/// games' own folder (D66).
pub fn addons_dir(l: &Layout, system: &str, places: &BTreeMap<String, PathBuf>) -> Result<Option<PathBuf>> {
    let Some(path) = l.games.addons.as_deref().filter(|p| !p.trim().is_empty()) else { return games_dir(l, system, places) };
    let Some(vars) = system_vars(l, system) else { return Ok(None) };
    let path = path.trim().replace('\\', "/");
    let dir = path.strip_suffix("/{file}").ok_or_else(|| anyhow!("The layout's add-ons path must end in /{{file}}."))?;
    expand(dir, places, &vars)
}

/// The place the games go under (the games' path's first part).
pub fn games_place(l: &Layout) -> Option<String> {
    place_of(&l.games.path)
}

/// The place a path starts from (`{launchbox}/Data/…` is `launchbox`).
pub fn place_of(path: &str) -> Option<String> {
    split(path).ok().map(|x| x.0)
}

/// The places' folders for an export: those asked for, then those worked out from them
/// (ES-DE's media folder, from its settings when they name one).
pub fn resolve_places(l: &Layout, given: &BTreeMap<String, PathBuf>) -> Result<BTreeMap<String, PathBuf>> {
    let mut out: BTreeMap<String, PathBuf> = BTreeMap::new();
    for p in l.asked() {
        match given.get(&p.id).filter(|d| !d.as_os_str().is_empty()) {
            // a path on the device, as it's written there
            Some(d) if p.device => {
                out.insert(p.id.clone(), d.clone());
            }
            Some(d) if d.is_absolute() => {
                out.insert(p.id.clone(), d.clone());
            }
            Some(d) => bail!("{} ({}) must be a whole path, such as {}.", p.label, d.display(), if p.example.is_empty() { "E:\\" } else { &p.example }),
            None if p.optional => {}
            None => bail!("Choose {}.", lower_first(&p.label)),
        }
    }
    for p in l.places.iter().filter(|p| p.from.is_some()) {
        let (base, _) = split(p.from.as_deref().unwrap_or("")).map_err(|e| anyhow!("{e}"))?;
        let Some(base_dir) = out.get(&base).cloned() else { continue };
        let set = p.es_setting.as_deref().and_then(|s| es_setting(&base_dir, s));
        let dir = match set {
            Some(d) => d,
            None => match expand(p.from.as_deref().unwrap_or(""), &out, &Vars::new())? {
                Some(d) => d,
                None => continue,
            },
        };
        out.insert(p.id.clone(), dir);
    }
    Ok(out)
}

fn lower_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) if !s.starts_with("ES-DE") && !s.starts_with("SD") => f.to_lowercase().chain(c).collect(),
        _ => s.to_string(),
    }
}

/// An ES-DE setting naming a folder (`MediaDirectory`, `ROMDirectory`), as ES-DE reads
/// it from `<ES-DE>/settings/es_settings.xml` (`~` and `%ESPATH%` worked out).
pub fn es_setting(es: &Path, name: &str) -> Option<PathBuf> {
    let text = std::fs::read_to_string(es.join("settings").join("es_settings.xml")).ok()?;
    let want = format!("name=\"{name}\"");
    let set = text.lines().find(|l| l.contains(&want)).and_then(|l| l.split("value=\"").nth(1)).and_then(|v| v.split('"').next())?;
    let set = set.replace("&amp;", "&").replace("&quot;", "\"").replace("&apos;", "'").replace("&lt;", "<").replace("&gt;", ">");
    match set.trim() {
        "" => None,
        s if s.starts_with('~') => Some(config::home_dir().join(s.trim_start_matches('~').trim_start_matches(['/', '\\']))),
        s if s.starts_with("%ESPATH%") => Some(es.join(s.trim_start_matches("%ESPATH%").trim_start_matches(['/', '\\']))),
        s => Some(PathBuf::from(s)),
    }
}

// ------------------------------------------------------------ the check

/// What's wrong with a layout, each problem with where it is. None: it can be used.
pub fn check(l: &Layout) -> Vec<Problem> {
    let mut v: Vec<Problem> = vec![];
    let mut say = |at: &str, m: String| v.push(Problem { at: at.to_string(), message: m });
    if l.name.trim().is_empty() {
        say("name", "Give the layout a name.".into());
    }
    let mut ids = HashSet::new();
    for p in &l.places {
        if p.id.is_empty() || !p.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
            say(&format!("places.{}", p.id), format!("A place's id ({}) is lower-case letters, digits and _.", p.id));
        } else if SYSTEM_TOKENS.contains(&p.id.as_str()) || GAME_TOKENS.contains(&p.id.as_str()) || ["ext", "kind", "doc_file", "bios_file"].contains(&p.id.as_str()) {
            say(&format!("places.{}", p.id), format!("{{{}}} is a token, so it can't be a place's id.", p.id));
        } else if !ids.insert(p.id.clone()) {
            say(&format!("places.{}", p.id), format!("Two places are called {}.", p.id));
        }
        if p.from.is_none() && p.label.trim().is_empty() {
            say(&format!("places.{}", p.id), format!("Say what the place {} is (its label).", p.id));
        }
    }
    if l.asked().next().is_none() {
        say("places", "A layout needs a place to export to, such as {card}.".into());
    }
    let kinds = media_kinds();
    for k in l.media.keys() {
        if !kinds.contains(&k.as_str()) {
            say(&format!("media.{k}"), format!("There's no kind of picture called {k} ({}).", kinds.join(", ")));
        }
    }
    for k in l.text.keys() {
        if !TEXT_KINDS.contains(&k.as_str()) {
            say(&format!("text.{k}"), format!("Only these details can be text files: {}.", TEXT_KINDS.join(", ")));
        }
    }
    // each path: its place, its tokens, and what it must have
    let mut seen: HashMap<String, String> = HashMap::new();
    for (at, path) in l.paths() {
        let (place, parts) = match split(path) {
            Ok(x) => x,
            Err(e) => {
                say(&at, e);
                continue;
            }
        };
        if !l.places.iter().any(|p| p.id == place) {
            say(&at, format!("There's no place called {{{place}}}."));
            continue;
        }
        if l.place(&place).is_some_and(|p| p.device) {
            say(&at, format!("{{{place}}} is a path on the device, so nothing can be written under it."));
            continue;
        }
        if at.starts_with("places.") {
            if l.place(&place).is_some_and(|p| p.from.is_some()) {
                say(&at, "A place worked out from another must start from one that's asked for.".into());
            }
            continue;
        }
        let mut allowed: Vec<&str> = SYSTEM_TOKENS.to_vec();
        let what = at.split('.').next().unwrap_or("");
        match what {
            "games" => allowed.push("file"),
            "gamelist" => {}
            "bios" => allowed.push("bios_file"),
            "documents" => allowed.extend(GAME_TOKENS.iter().chain(&["doc_file"])),
            _ => {
                allowed.extend(GAME_TOKENS);
                allowed.extend(["ext", "kind"]);
                if at == "media.manual" {
                    allowed.push("doc_file");
                }
            }
        }
        let mut used: Vec<String> = vec![];
        let mut bad = false;
        for (i, part) in parts.iter().enumerate() {
            match tokens(part) {
                Err(e) => {
                    say(&at, e);
                    bad = true;
                }
                Ok(t) => {
                    for name in &t {
                        if !allowed.contains(&name.as_str()) {
                            say(&at, format!("{{{name}}} isn't known here (it can use {}).", allowed.iter().map(|a| format!("{{{a}}}")).collect::<Vec<_>>().join(", ")));
                            bad = true;
                        }
                        if name == "bios_file" && part != "{bios_file}" {
                            say(&at, "{bios_file} is a part of its own (it keeps the file's folders).".into());
                            bad = true;
                        }
                        if name == "file" && (what != "games" || i + 1 != parts.len() || part != "{file}") && what == "games" {
                            say(&at, "The games' path ends in /{file}, and has it nowhere else.".into());
                            bad = true;
                        }
                    }
                    used.extend(t);
                }
            }
        }
        if parts.is_empty() {
            say(&at, "It needs a file name after the place.".into());
            continue;
        }
        if bad {
            continue;
        }
        let has = |t: &str| used.iter().any(|u| u == t);
        match what {
            "games" => {
                if parts.last().map(String::as_str) != Some("{file}") {
                    say(&at, "The games' path ends in /{file}.".into());
                }
            }
            "bios" if !has("bios_file") => say(&at, "It needs {bios_file}.".into()),
            "documents" if !has("doc_file") => say(&at, "It needs {doc_file}, the document's own name.".into()),
            "media" | "text" | "documents" if !(has("stem") || has("file") || has("title") || has("label")) => {
                say(&at, "It needs {stem}, {file}, {title} or {label}, so each game's is its own.".into());
            }
            _ => {}
        }
        // two kinds that would write the same file
        let k = at.split('.').nth(1).unwrap_or("");
        let shape = format!("{{{place}}}/{}", parts.join("/")).replace("{kind}", k).to_lowercase();
        if let Some(other) = seen.insert(shape, at.clone()) {
            say(&at, format!("It's the same path as {other}, so one would replace the other."));
        }
    }
    // a gamelist's tagged pictures are under the games' folder (EmulationStation's and
    // Pegasus' paths start there; LaunchBox's start at its own folder)
    if let Some(g) = &l.gamelist {
        if !GAMELISTS.contains(&g.kind.as_str()) {
            say("gamelist.kind", format!("There's no kind of gamelist called {} ({}).", g.kind, GAMELISTS.join(", ")));
        }
        // Pegasus' keys have dots (assets.boxFront)
        let tag_ok = |t: &str| if g.kind == "pegasus" { t.split('.').all(valid_tag) } else { valid_tag(t) };
        for (k, tag) in &g.fields {
            if !meta::FIELDS.iter().any(|f| f.0 == k) {
                say(&format!("gamelist.fields.{k}"), format!("There's no detail called {k}."));
            }
            if !tag_ok(tag) {
                say(&format!("gamelist.fields.{k}"), format!("{tag} isn't a tag (letters, digits, - and _)."));
            }
        }
        let dir = l.games.path.trim().replace('\\', "/");
        let dir = dir.strip_suffix("/{file}").unwrap_or(&dir).to_lowercase();
        for (k, o) in l.media.iter().filter(|(_, o)| o.tag.is_some()) {
            let tag = o.tag.as_deref().unwrap_or("");
            if g.kind == "retroarch" {
                say(&format!("media.{k}"), "A RetroArch playlist names no pictures (RetroArch finds them by the game's label).".into());
                continue;
            }
            if !tag_ok(tag) {
                say(&format!("media.{k}"), format!("{tag} isn't a tag (letters, digits, - and _)."));
            }
            if g.kind != "launchbox" && !o.path.trim().replace('\\', "/").to_lowercase().starts_with(&format!("{dir}/")) {
                say(&format!("media.{k}"), "A picture written into the gamelist must be in the games' folder or under it.".into());
            }
        }
    } else if l.media.values().any(|o| o.tag.is_some()) {
        let k = l.media.iter().find(|(_, o)| o.tag.is_some()).map(|(k, _)| k.clone()).unwrap_or_default();
        say(&format!("media.{k}"), "It has a gamelist tag, but the layout has no gamelist.".into());
    }
    if !["", "frontend", "naming"].contains(&l.games.style.as_str()) {
        say("games.style", format!("The games' style is frontend or naming, not {}.", l.games.style));
    }
    if !["keep", "png", "jpg"].contains(&l.pictures.as_.as_str()) {
        say("pictures.as", format!("Pictures are kept as they are (keep), or made png or jpg, not {}.", l.pictures.as_));
    }
    match l.collection.as_deref() {
        None | Some("esde-cfg") => {}
        Some(c) => say("collection", format!("There's no kind of collection file called {c} (esde-cfg).")),
    }
    for (s, out) in &l.systems {
        let f = match out {
            Some(SystemOut::Folder(f)) => Some(f),
            Some(SystemOut::Named { folder, .. }) => Some(folder),
            None => None,
        };
        if let Some(f) = f {
            if f.trim().is_empty() || f.contains(['/', '\\']) || safe_name(f) != *f {
                say(&format!("systems.{s}"), format!("{f:?} can't be a folder's name."));
            }
        }
    }
    v
}

fn valid_tag(t: &str) -> bool {
    !t.is_empty() && t.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

// ------------------------------------------------------------ the files

fn dir(lib: &Library) -> PathBuf {
    lib.root().join(APP_DIR).join("layouts")
}

/// A layout from its JSON (refused if a newer app made it).
pub fn parse(v: &Value) -> Result<Layout> {
    if !v.is_object() {
        bail!("That isn't a layout (a JSON object).");
    }
    let format = v["format"].as_u64().unwrap_or(FORMAT as u64);
    if format > FORMAT as u64 {
        bail!("This layout was made by a newer Game Repo (its format is {format}); update the app to use it.");
    }
    let mut l: Layout = serde_json::from_value(v.clone()).map_err(|e| anyhow!("That isn't a layout Game Repo can read: {e}"))?;
    l.format = FORMAT;
    if l.games.style.is_empty() {
        l.games.style = "frontend".into();
    }
    Ok(l)
}

/// The built-in layouts.
pub fn built_in() -> Vec<Layout> {
    BUILT_IN
        .iter()
        .map(|t| {
            let mut l = parse(&serde_json::from_str(t).expect("a built-in layout is JSON")).expect("a built-in layout reads");
            l.builtin = true;
            l
        })
        .collect()
}

/// Every layout: the built-in ones, then the user's own by name.
pub fn all(lib: Option<&Library>) -> Vec<Layout> {
    let mut v = built_in();
    let mut mine: Vec<Layout> = lib
        .map(|lib| {
            std::fs::read_dir(dir(lib))
                .into_iter()
                .flatten()
                .flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
                .filter_map(|e| parse(&config::read_json_object(&e.path())).ok())
                .filter(|l| !l.id.is_empty() && !v.iter().any(|b| b.id == l.id))
                .collect()
        })
        .unwrap_or_default();
    mine.sort_by_key(|l| l.name.to_lowercase());
    v.extend(mine);
    v
}

/// The layouts as the page lists them.
pub fn list(lib: Option<&Library>) -> Vec<Value> {
    all(lib).iter().map(Layout::summary).collect()
}

/// A layout by its id.
pub fn get(lib: Option<&Library>, id: &str) -> Result<Layout> {
    all(lib).into_iter().find(|l| l.id == id).ok_or_else(|| anyhow!("There's no layout {id}."))
}

/// A layout as JSON, as its file has it.
pub fn to_json(l: &Layout) -> Value {
    serde_json::to_value(l).unwrap_or(Value::Null)
}

/// An id for a new layout from its name, not used by another.
fn new_id(lib: &Library, name: &str) -> String {
    let mut base = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            base.push(c);
        } else if !base.is_empty() && !base.ends_with('-') {
            base.push('-');
        }
    }
    let base: String = base.trim_end_matches('-').chars().take(48).collect();
    let base = if base.is_empty() { "layout".to_string() } else { base };
    let taken: HashSet<String> = all(Some(lib)).into_iter().map(|l| l.id).collect();
    if !taken.contains(&base) {
        return base;
    }
    (2..).map(|n| format!("{base}-{n}")).find(|id| !taken.contains(id)).unwrap()
}

/// Save a layout. A built-in one (or `as_new`) is saved as a new layout with its own id,
/// remembering the built-in it came from; the user's own is replaced. Refused, with
/// its problems, when the check finds any. Returns the layout as saved.
pub fn save(lib: &Library, v: &Value, as_new: bool) -> Result<Layout> {
    lib.writable()?;
    let mut l = parse(v)?;
    let problems = check(&l);
    if let Some(p) = problems.first() {
        bail!("{}{}", p.message, if problems.len() > 1 { format!(" (and {} more)", problems.len() - 1) } else { String::new() });
    }
    let from_builtin = built_in().into_iter().find(|b| b.id == l.id);
    let exists = all(Some(lib)).iter().any(|x| x.id == l.id && !x.builtin);
    if as_new || from_builtin.is_some() || l.id.is_empty() || !exists {
        if let Some(b) = &from_builtin {
            l.copied_from = Some(json!({ "id": b.id, "version": b.version.unwrap_or(0) }));
            if l.name == b.name {
                l.name = format!("{} (mine)", b.name);
            }
        }
        if as_new || from_builtin.is_some() || l.id.is_empty() || crate::library::valid_id(&l.id).is_err() {
            l.id = new_id(lib, &l.name);
        }
    }
    crate::library::valid_id(&l.id).map_err(|_| anyhow!("A layout's id ({}) is letters, digits and -.", l.id))?;
    l.version = None;
    l.builtin = false;
    std::fs::create_dir_all(dir(lib))?;
    config::write_json(&dir(lib).join(format!("{}.json", l.id)), &to_json(&l))?;
    Ok(l)
}

/// Remove one of the user's layouts (exports made with it are refused until one is chosen).
pub fn remove(lib: &Library, id: &str) -> Result<()> {
    lib.writable()?;
    if built_in().iter().any(|b| b.id == id) {
        bail!("A built-in layout can't be removed.");
    }
    crate::library::valid_id(id).map_err(|_| anyhow!("There's no layout {id}."))?;
    std::fs::remove_file(dir(lib).join(format!("{id}.json"))).map_err(|_| anyhow!("There's no layout {id}."))
}

/// Bring in a layout from a file someone shared: saved as a new one when its id is taken.
pub fn import(lib: &Library, path: &Path) -> Result<Layout> {
    let text = std::fs::read_to_string(path).map_err(|e| anyhow!("couldn't read {}: {e}", path.display()))?;
    let v: Value = serde_json::from_str(&text).map_err(|e| anyhow!("{} isn't JSON: {e}", path.display()))?;
    let id = v["id"].as_str().unwrap_or("");
    let taken = all(Some(lib)).iter().any(|l| l.id == id);
    let mut v = v.clone();
    if taken && !built_in().iter().any(|b| b.id == id) {
        v["id"] = json!("");
    }
    save(lib, &v, taken)
}

/// Write a layout to a file, to give to someone.
pub fn write_file(lib: Option<&Library>, id: &str, to: &Path) -> Result<()> {
    let l = get(lib, id)?;
    let mut v = to_json(&l);
    if let Some(o) = v.as_object_mut() {
        o.insert("format".into(), json!(FORMAT));
    }
    let text = serde_json::to_string_pretty(&v)? + "\n";
    std::fs::write(to, text).map_err(|e| anyhow!("couldn't write {}: {e}", to.display()))
}

/// Where the user's layouts are kept (for *Open the folder*).
pub fn folder(lib: &Library) -> PathBuf {
    dir(lib)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_layouts_read_and_pass_their_check() {
        let b = built_in();
        assert_eq!(b.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(), ["esde", "esde-card", "knulli", "rocknix", "muos", "retroarch", "pegasus", "launchbox", "playnite", "folder"]);
        for l in &b {
            assert_eq!(check(l).iter().map(|p| format!("{}: {}", p.at, p.message)).collect::<Vec<_>>(), Vec::<String>::new(), "{}", l.id);
        }
        let k = &b[2];
        assert_eq!(k.folder("genesis").as_deref(), Some("megadrive"));
        assert_eq!(k.folder("gb").as_deref(), Some("gb"));
        let m = &b[4];
        assert_eq!(m.system_name("snes"), "Nintendo SNES-SFC");
        assert_eq!(m.carries()["fields"], json!(["description"]));
    }

    #[test]
    fn a_path_is_made_from_its_place_and_tokens() {
        let places: BTreeMap<String, PathBuf> = [("card".to_string(), PathBuf::from("/media/card"))].into();
        let mut v = system_vars(&built_in()[2], "genesis").unwrap();
        v.insert("stem", "Sonic: The Hedgehog (USA)".into());
        v.insert("ext", "png".into());
        let p = expand("{card}/roms/{folder}/images/{stem}-thumb.{ext}", &places, &v).unwrap().unwrap();
        assert_eq!(p, PathBuf::from("/media/card/roms/megadrive/images/Sonic_ The Hedgehog (USA)-thumb.png"));
        v.insert("bios_file", "dc/dc_boot.bin".into());
        assert_eq!(expand("{card}/bios/{bios_file}", &places, &v).unwrap().unwrap(), PathBuf::from("/media/card/bios/dc/dc_boot.bin"));
        // a place not given (an optional one): nothing is written there
        assert!(expand("{esde}/gamelists/{folder}/gamelist.xml", &places, &v).unwrap().is_none());
        assert!(expand("{card}/{nope}", &places, &v).is_err());
    }

    #[test]
    fn the_check_says_what_is_wrong_and_where() {
        let mut l = built_in()[2].clone();
        l.media.get_mut("cover").unwrap().path = "{card}/roms/{folder}/images/cover.png".into();
        l.media.get_mut("screenshot").unwrap().path = "{sd}/roms/{folder}/images/{stem}.png".into();
        l.media.get_mut("marquee").unwrap().path = "{card}/../{stem}.png".into();
        l.media.get_mut("video").unwrap().path = "{card}/roms/{folder}/images/{stem}-thumb.{ext}".into();
        l.media.insert("poster".into(), Out { path: "{card}/x/{stem}.png".into(), ..Default::default() });
        l.games.path = "{card}/roms/{folder}".into();
        l.gamelist.as_mut().unwrap().fields.insert("description".into(), "my desc".into());
        let p: Vec<(String, String)> = check(&l).into_iter().map(|p| (p.at, p.message)).collect();
        let at = |a: &str| p.iter().filter(|(x, _)| x == a).map(|(_, m)| m.as_str()).collect::<Vec<_>>().join(" | ");
        assert!(at("media.cover").contains("{stem}, {file}, {title} or {label}"), "{p:?}");
        assert!(at("media.screenshot").contains("no place called {sd}"), "{p:?}");
        assert!(at("media.marquee").contains(".."), "{p:?}");
        assert!(at("media.poster").contains("no kind of picture"), "{p:?}");
        assert!(at("games.path").contains("ends in /{file}"), "{p:?}");
        assert!(at("gamelist.fields.description").contains("isn't a tag"), "{p:?}");
        // the video's path is the box's, before the cover changed: the same file twice
        let mut two = built_in()[2].clone();
        two.media.get_mut("video").unwrap().path = "{card}/roms/{folder}/images/{stem}-thumb.{ext}".into();
        assert!(check(&two).iter().any(|p| p.message.contains("same path")), "{:?}", check(&two));
    }

    #[test]
    fn a_built_in_layout_is_changed_by_saving_a_copy_and_a_newer_format_is_refused() {
        let d = std::env::temp_dir().join(format!("gamerepo-layout-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let lib = Library::open(d.join("lib")).unwrap();
        let mut v = to_json(&built_in()[2]);
        v["systems"]["gb"] = json!("GB");
        v["future"] = json!({ "kept": true });
        let saved = save(&lib, &v, false).unwrap();
        assert_eq!(saved.id, "knulli-mine");
        assert_eq!(saved.name, "Knulli (mine)");
        assert_eq!(saved.copied_from.as_ref().unwrap()["id"], "knulli");
        let back = get(Some(&lib), "knulli-mine").unwrap();
        assert_eq!(back.folder("gb").as_deref(), Some("GB"));
        assert_eq!(back.extra["future"], json!({ "kept": true }));
        // the built-in one is as it was
        assert_eq!(get(Some(&lib), "knulli").unwrap().folder("gb").as_deref(), Some("gb"));
        // the user's own is saved in place
        let mut v2 = to_json(&back);
        v2["about"] = json!("Mine.");
        assert_eq!(save(&lib, &v2, false).unwrap().id, "knulli-mine");
        assert_eq!(list(Some(&lib)).len(), BUILT_IN.len() + 1);
        // shared and brought back in: a new id, since this one is taken
        let f = d.join("shared.json");
        write_file(Some(&lib), "knulli-mine", &f).unwrap();
        assert_eq!(import(&lib, &f).unwrap().id, "knulli-mine-2");
        let mut newer = to_json(&back);
        newer["format"] = json!(FORMAT + 1);
        assert!(parse(&newer).unwrap_err().to_string().contains("newer Game Repo"));
        let mut bad = to_json(&back);
        bad["games"]["path"] = json!("{card}/roms");
        assert!(save(&lib, &bad, false).is_err());
        assert!(remove(&lib, "knulli").is_err());
        remove(&lib, "knulli-mine-2").unwrap();
        let _ = std::fs::remove_dir_all(&d);
    }
}
