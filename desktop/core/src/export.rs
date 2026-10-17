//! Exporting a collection (docs/PLAN.md, "Phase 3 design", Export): to ES-DE's ROMs
//! folder (`<ROMs>/<system id>/`, with its gamelists and a custom collection) or to a
//! plain folder (`<folder>/<DAT group name>/`). An export is a change like Organise's
//! copy (planned, previewed, journalled, undone), with every file hardlinked or copied
//! and checked. Each export is remembered in `_library/exports/<id>.json` with the files
//! it placed, so *Update* adds what's new and sets aside only what it placed before.

use crate::archive;
use crate::collection;
use crate::config;
use crate::cue;
use crate::hash::{self, Hashes};
use crate::image;
use crate::index::Index;
use crate::layout;
use crate::library::{now, Library, APP_DIR};
use crate::organise::{self, key, safe_name, Op, Plan, Src};
use crate::platforms;
use crate::roots;
use anyhow::{anyhow, bail, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// What the Export dialog chose (or an export remembered, to update it).
#[derive(Clone, Debug, Default)]
pub struct Options {
    pub collection: String,
    /// The layout (layout.rs); empty: the one `target` names
    pub layout: String,
    /// Where the layout's places are (`card`, `roms`, `esde`…)
    pub places: BTreeMap<String, PathBuf>,
    /// esde | folder: Phase 3's targets, the built-in layouts of those names, with `out`
    /// (ES-DE's ROMs folder, or the folder) and `esde` (ES-DE's own folder) as their places
    pub target: String,
    pub out: PathBuf,
    pub esde: Option<PathBuf>,
    pub always_copy: bool,
    /// Gamelist entries named by the game's title, not its full name
    pub short_names: bool,
    /// The export updated (none: a new one)
    pub export_id: Option<String>,
    /// What it carries besides the games (D30, D34, D40)
    pub carry: Carry,
}

impl Options {
    /// The layout it uses.
    pub fn layout_id(&self) -> String {
        if !self.layout.is_empty() {
            return self.layout.clone();
        }
        if self.target.is_empty() { "esde".into() } else { self.target.clone() }
    }

    /// Where its places are: those given, else Phase 3's `out` and `esde`.
    pub fn place_dirs(&self) -> BTreeMap<String, PathBuf> {
        let mut p = self.places.clone();
        if !self.out.as_os_str().is_empty() {
            let main = if self.layout_id() == "esde" { "roms" } else if self.layout_id() == "folder" { "out" } else { "card" };
            p.entry(main.into()).or_insert_with(|| self.out.clone());
        }
        if let Some(e) = &self.esde {
            p.entry("esde".into()).or_insert_with(|| e.clone());
        }
        p
    }
}

/// What an export carries besides the games' files: the details in the gamelist (or
/// text files), the pictures (and video), manuals and other documents, the systems'
/// BIOS files, and how big pictures may be.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Carry {
    /// meta::FIELDS
    pub fields: Vec<String>,
    /// meta::MEDIA
    pub media: Vec<String>,
    pub manuals: bool,
    /// Every document (a layout with a place for them)
    pub docs: bool,
    /// The longest side of a picture, in pixels (0: as they are)
    pub image_size: u32,
    /// The BIOS files the collection's systems need, those the library has (D40)
    pub bios: bool,
}

/// The pictures an export can make smaller or write in another format.
const PICTURES: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "bmp"];

impl Carry {
    fn wants_game_files(&self) -> bool {
        !self.fields.is_empty() || !self.media.is_empty() || self.manuals || self.docs
    }

}

fn dir(lib: &Library) -> PathBuf {
    lib.root().join(APP_DIR).join("exports")
}

fn path_of(lib: &Library, id: &str) -> Result<PathBuf> {
    crate::library::valid_id(id).map_err(|_| anyhow!("There's no export {id}."))?;
    Ok(dir(lib).join(format!("{id}.json")))
}

/// An export as remembered.
pub fn load(lib: &Library, id: &str) -> Result<Value> {
    let p = path_of(lib, id)?;
    if !p.is_file() {
        bail!("There's no export {id}.");
    }
    Ok(config::read_json_object(&p))
}

/// The options an export was made with, to plan it again.
pub fn options_of(lib: &Library, id: &str) -> Result<Options> {
    let e = load(lib, id)?;
    let layout = e["layout"].as_str().or(e["target"].as_str()).unwrap_or("folder").to_string();
    let mut places: BTreeMap<String, PathBuf> = e["places"].as_object().map(|m| m.iter().filter_map(|(k, v)| Some((k.clone(), PathBuf::from(v.as_str()?)))).collect()).unwrap_or_default();
    if places.is_empty() {
        // an export from before layouts (Phase 3 and 5)
        let main = if layout == "esde" { "roms" } else { "out" };
        places.insert(main.into(), PathBuf::from(e["out"].as_str().unwrap_or("")));
        if let Some(es) = e["esde"].as_str().filter(|s| !s.is_empty()) {
            places.insert("esde".into(), PathBuf::from(es));
        }
    }
    Ok(Options {
        collection: e["collection"].as_str().unwrap_or("").into(),
        layout,
        places,
        always_copy: e["always_copy"].as_bool().unwrap_or(false),
        short_names: e["short_names"].as_bool().unwrap_or(true),
        export_id: Some(id.into()),
        carry: serde_json::from_value(e["carry"].clone()).unwrap_or_default(),
        ..Default::default()
    })
}

/// The exports remembered, newest first (without their files).
pub fn list(lib: &Library) -> Vec<Value> {
    let names: HashMap<String, String> = collection::list(lib).into_iter().map(|c| (c.id, c.name)).collect();
    let mut v: Vec<Value> = std::fs::read_dir(dir(lib))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .map(|e| config::read_json_object(&e.path()))
        .filter(|e| e["id"].is_string())
        .map(|mut e| {
            let files = e["files"].as_array().map(Vec::len).unwrap_or(0);
            let c = e["collection"].as_str().unwrap_or("").to_string();
            if let Some(o) = e.as_object_mut() {
                o.remove("files");
                o.insert("count".into(), json!(files));
                o.insert("collection_name".into(), json!(names.get(&c).cloned().or_else(|| o.get("name").and_then(Value::as_str).map(String::from))));
                o.insert("collection_gone".into(), json!(!names.contains_key(&c)));
            }
            e
        })
        .collect();
    v.sort_by(|a, b| b["updated"].as_str().cmp(&a["updated"].as_str()));
    v
}

/// Forget an export (its files stay where they are).
pub fn forget(lib: &Library, id: &str) -> Result<()> {
    lib.writable()?;
    let p = path_of(lib, id)?;
    std::fs::remove_file(&p).map_err(|_| anyhow!("There's no export {id}."))?;
    Ok(())
}

/// ES-DE's folder, when it's where ES-DE puts it (`%USERPROFILE%/ES-DE`), and its ROMs folder.
pub fn esde_default() -> Value {
    let es = config::home_dir().join("ES-DE");
    if !es.is_dir() {
        return json!({ "esde": null, "roms": null });
    }
    json!({ "esde": es.display().to_string(), "roms": rom_dir(&es).display().to_string() })
}

/// ES-DE's ROMs folder: its setting (`ROMDirectory` in es_settings.xml), else `~/ROMs`.
fn rom_dir(es: &Path) -> PathBuf {
    layout::es_setting(es, "ROMDirectory").unwrap_or_else(|| config::home_dir().join("ROMs"))
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

fn xml_unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// A path below `top`, with `/`.
fn rel_to(top: &Path, p: &Path) -> Option<String> {
    let (t, k) = (key(top), key(p));
    let t = t.trim_end_matches('/');
    k.strip_prefix(t).and_then(|r| r.strip_prefix('/'))?;
    let n = p.to_string_lossy().replace('\\', "/");
    Some(n[n.len() - (k.len() - t.len() - 1)..].to_string())
}

/// A file outside the folders with games, as a change's step reads or sets it aside.
fn outside(top: &Path, p: &Path) -> Src {
    Src {
        file_id: 0, root: format!("dir:{}", top.display()), rel: rel_to(top, p).unwrap_or_default(), path: p.display().to_string(),
        member: None, size: std::fs::metadata(p).map(|m| m.len()).unwrap_or(0), crc32: String::new(), sha1: None,
    }
}

fn text_op(game: &str, to: &Path, top: &Path, text: String, note: &str) -> Op {
    let from = if to.exists() { vec![outside(top, to)] } else { vec![] };
    Op { op: "text".into(), game: game.into(), to: to.display().to_string(), from, size: text.len() as u64, names: vec![text], aside: vec![], note: Some(note.into()) }
}

/// Plan an export (nothing is written).
pub fn plan(ix: &Index, lib: &Library, o: &Options) -> Result<Plan> {
    let conn = ix.conn();
    let l = layout::get(Some(lib), &o.layout_id()).map_err(|_| anyhow!("There's no export layout {}.", o.layout_id()))?;
    if let Some(p) = layout::check(&l).first() {
        bail!("The layout {} can't be used yet: {} ({})", l.name, p.message, p.at);
    }
    let places = layout::resolve_places(&l, &o.place_dirs())?;
    for (id, dir) in &places {
        if l.place(id).is_some_and(|p| p.from.is_some()) {
            continue;
        }
        if key(dir).starts_with(&key(&lib.root().join(APP_DIR))) {
            bail!("The library's own _library folder can't hold an export.");
        }
        for r in roots::list(lib) {
            let (rk, ok) = (key(&r.dir), key(dir));
            let (rk, ok) = (rk.trim_end_matches('/'), ok.trim_end_matches('/'));
            if ok == rk || ok.starts_with(&format!("{rk}/")) {
                bail!("{} is in a folder with games ({}): its copies would be counted as duplicates. Choose a folder outside it.", dir.display(), r.dir.display());
            }
            if rk.starts_with(&format!("{ok}/")) {
                bail!("{} holds a folder with games ({}): its copies would be counted as duplicates. Choose another folder.", dir.display(), r.dir.display());
            }
        }
    }
    // the first place asked for is the export's own folder (its label and what's tidied)
    let out = l.asked().next().and_then(|p| places.get(&p.id)).cloned().ok_or_else(|| anyhow!("Choose where the games go."))?;
    let tops: Vec<PathBuf> = places.values().cloned().collect();
    let top_of = |p: &Path| -> PathBuf { tops.iter().filter(|t| rel_to(t, p).is_some()).max_by_key(|t| t.as_os_str().len()).cloned().unwrap_or_else(|| out.clone()) };
    let record = match &o.export_id {
        Some(id) => Some(load(lib, id)?),
        None => None,
    };
    let recorded: Vec<Value> = record.as_ref().and_then(|r| r["files"].as_array().cloned()).unwrap_or_default();
    let known: HashMap<String, (u64, i64)> = recorded
        .iter()
        .filter_map(|f| Some((key(Path::new(f["path"].as_str()?)), (f["size"].as_u64()?, f["mtime"].as_i64()?))))
        .collect();
    let c = collection::load(lib, &o.collection)?;
    let w = collection::work_out(ix, lib, &c)?;
    let first = record.is_none();
    let export_id = o.export_id.clone().unwrap_or_else(organise::new_id);
    let target_name = if l.id == "folder" { out.display().to_string() } else { format!("{} ({})", l.name, out.display()) };
    let label = if first { format!("Exported {} to {target_name}", c.name) } else { format!("Updated the export of {} to {target_name}", c.name) };
    let mut plan = Plan {
        id: organise::new_id(), created: now(), kind: "export".into(), label,
        dat: json!({ "id": null, "name": c.name, "version": "" }),
        platform: if w.systems.len() == 1 { w.systems[0].platform.clone() } else { String::new() },
        mode: "copy".into(), format: "keep".into(), out: Some(out.display().to_string()),
        ops: vec![], games: 0, in_place: 0, conflicts: vec![], duplicates: vec![], left: vec![], space: vec![], new_root: None,
        present: vec![], export: None,
    };
    let mut wanted: Vec<Value> = vec![];
    // each system's entries in its folder: (the file or folder there, game, title)
    let mut entries: BTreeMap<String, Vec<(String, String, String)>> = BTreeMap::new();
    // each system's games folder, and its games' DAT ids by name
    let mut folders: BTreeMap<String, PathBuf> = BTreeMap::new();
    let mut ids: HashMap<(String, String), i64> = HashMap::new();
    for s in &w.systems {
        if s.games.is_empty() {
            continue;
        }
        let Some(folder) = layout::games_dir(&l, &s.platform, &places)? else {
            plan.left.push(json!({ "path": platforms::name_of(&s.platform), "why": format!("The layout {} leaves this system out (the device doesn't run it).", l.name) }));
            continue;
        };
        let names: Vec<String> = s.games.iter().map(|g| g.name.clone()).collect();
        folders.insert(s.platform.clone(), folder.clone());
        for g in &s.games {
            ids.insert((s.platform.clone(), g.name.clone()), g.id);
        }
        let sub = organise::plan(ix, lib, &organise::Options {
            dat_id: s.dat_id, mode: "copy".into(), out: Some(folder.clone()), format: "keep".into(),
            always_copy: o.always_copy, games: Some(names), esde: l.games.style != "naming", known: known.clone(),
            ..Default::default()
        })?;
        let mut placed: HashSet<String> = HashSet::new();
        let titles: HashMap<&str, &str> = s.games.iter().map(|g| (g.name.as_str(), if g.title.is_empty() { g.name.as_str() } else { g.title.as_str() })).collect();
        let mut seen_entry: HashSet<String> = HashSet::new();
        let mut want = |game: &str, to: &str, placed: &mut HashSet<String>| {
            placed.insert(game.to_string());
            wanted.push(json!({ "path": to, "game": game, "system": s.platform, "dat": s.dat_id }));
            if let Some(rel) = rel_to(&folder, Path::new(to)) {
                let entry = rel.split('/').next().unwrap_or(&rel).to_string();
                if seen_entry.insert(entry.clone()) {
                    let title = if o.short_names { titles.get(game).copied().unwrap_or(game) } else { game };
                    entries.entry(s.platform.clone()).or_default().push((entry, game.to_string(), title.to_string()));
                }
            }
        };
        for op in &sub.ops {
            want(&op.game, &op.to, &mut placed);
        }
        for p in &sub.present {
            want(p["game"].as_str().unwrap_or(""), p["to"].as_str().unwrap_or(""), &mut placed);
        }
        for cf in &sub.conflicts {
            placed.insert(cf["game"].as_str().unwrap_or("").to_string());
        }
        for g in &s.games {
            if !placed.contains(&g.name) {
                let why = if g.status == "have" { "Some of its files aren't in the library." } else { "It isn't in the library." };
                plan.left.push(json!({ "path": format!("{} › {}", platforms::name_of(&s.platform), g.name), "why": why }));
            }
        }
        plan.ops.extend(sub.ops);
        plan.games += sub.games;
        plan.in_place += sub.in_place;
        plan.conflicts.extend(sub.conflicts);
        plan.present.extend(sub.present);
        plan.left.extend(sub.left);
    }
    for p in &w.no_dat {
        plan.left.push(json!({ "path": platforms::name_of(p), "why": "This system has no DAT, so its games aren't known." }));
    }
    // what it carries: each game's pictures, manual, documents and text, its details and
    // tagged pictures for the gamelist, and the systems' BIOS files
    let gamelist = l.gamelist.as_ref();
    let mut details: HashMap<(String, String), Vec<(String, String)>> = HashMap::new();
    let mut files: Vec<(PathBuf, PathBuf, &str, String, String)> = vec![];
    let mut texts: Vec<(PathBuf, String, String, String)> = vec![];
    let pic_as = l.pictures.as_.as_str();
    if o.carry.wants_game_files() {
        for (system, list) in &entries {
            let Some(folder) = folders.get(system) else { continue };
            let Some(svars) = layout::system_vars(&l, system) else { continue };
            for (entry, game, title) in list {
                let Some(id) = ids.get(&(system.clone(), game.clone())) else { continue };
                let g = crate::meta::dat_game(conn, *id)?;
                let Some(dir) = crate::meta::folder_of(conn, &g)?.and_then(|f| lib.resolve(&f).ok()) else { continue };
                let is_dir = folder.join(entry).is_dir() || plan.ops.iter().any(|op| rel_to(folder, Path::new(&op.to)).is_some_and(|r| r.starts_with(&format!("{entry}/"))));
                // pictures are named after the game's file without the extension (a folder keeps its whole name)
                let stem = if is_dir { entry.clone() } else { Path::new(entry).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| entry.clone()) };
                let gj = crate::meta::read_game(&dir);
                let mut vars = svars.clone();
                vars.insert("file", entry.clone());
                vars.insert("stem", stem.clone());
                vars.insert("title", gj["title"].as_str().filter(|t| !t.is_empty()).unwrap_or(title).to_string());
                let mut tags: Vec<(String, String)> = vec![];
                if let Some(gl) = gamelist {
                    if !o.carry.fields.is_empty() {
                        tags = gamelist_fields(&gj["details"], &o.carry.fields, &gl.fields);
                    }
                }
                for kind in &o.carry.media {
                    let Some(out_of) = l.media.get(kind) else { continue };
                    let Some(src) = media_file(&dir.join("media"), kind) else { continue };
                    let src_ext = platforms::ext_of(&src.to_string_lossy());
                    let picture = PICTURES.contains(&src_ext.as_str());
                    // a picture made smaller, or for a firmware that shows only one format, is written as PNG or JPEG
                    let ext = if picture && (pic_as == "png" || pic_as == "jpg") {
                        pic_as.to_string()
                    } else if picture && o.carry.image_size > 0 && ["webp", "gif", "bmp"].contains(&src_ext.as_str()) {
                        "png".to_string()
                    } else {
                        src_ext.clone()
                    };
                    let mut v = vars.clone();
                    v.insert("ext", ext);
                    v.insert("kind", kind.clone());
                    let Some(to) = layout::expand(&out_of.path, &places, &v)? else { continue };
                    if let (Some(tag), Some(rel)) = (&out_of.tag, rel_to(folder, &to)) {
                        if gamelist.is_some() {
                            tags.push((tag.clone(), format!("./{rel}")));
                        }
                    }
                    files.push((src, to, "media", game.clone(), system.clone()));
                }
                if o.carry.manuals {
                    if let (Some(out_of), Some(src)) = (l.media.get("manual"), manual_of(conn, lib, &dir)) {
                        let mut v = vars.clone();
                        v.insert("ext", platforms::ext_of(&src.to_string_lossy()));
                        v.insert("kind", "manual".into());
                        v.insert("doc_file", src.file_name().unwrap_or_default().to_string_lossy().into_owned());
                        if let Some(to) = layout::expand(&out_of.path, &places, &v)? {
                            if let (Some(tag), Some(rel)) = (&out_of.tag, rel_to(folder, &to)) {
                                if gamelist.is_some() {
                                    tags.push((tag.clone(), format!("./{rel}")));
                                }
                            }
                            files.push((src, to, "manual", game.clone(), system.clone()));
                        }
                    }
                }
                if o.carry.docs {
                    if let Some(out_of) = &l.documents {
                        let rel = lib.relative(&dir).unwrap_or_default();
                        for d in crate::docs::of_folder(conn, &rel)? {
                            let Ok(src) = lib.resolve(d["path"].as_str().unwrap_or("")) else { continue };
                            let mut v = vars.clone();
                            v.insert("doc_file", src.file_name().unwrap_or_default().to_string_lossy().into_owned());
                            v.insert("ext", platforms::ext_of(&src.to_string_lossy()));
                            let Some(to) = layout::expand(&out_of.path, &places, &v)? else { continue };
                            if !files.iter().any(|f| key(&f.1) == key(&to)) && !files.iter().any(|f| f.3 == *game && key(&f.0) == key(&src)) {
                                files.push((src, to, "doc", game.clone(), system.clone()));
                            }
                        }
                    }
                }
                for (kind, out_of) in &l.text {
                    if !o.carry.fields.iter().any(|f| f == kind) {
                        continue;
                    }
                    let Some(text) = gj["details"][kind.as_str()].as_str().map(str::trim).filter(|t| !t.is_empty()) else { continue };
                    let mut v = vars.clone();
                    v.insert("ext", "txt".into());
                    v.insert("kind", kind.clone());
                    if let Some(to) = layout::expand(&out_of.path, &places, &v)? {
                        texts.push((to, format!("{text}\n"), game.clone(), system.clone()));
                    }
                }
                if !tags.is_empty() {
                    details.insert((system.clone(), entry.to_lowercase()), tags);
                }
            }
        }
    }
    // the BIOS files the exported systems need, those the library has
    if o.carry.bios {
        if let Some(out_of) = &l.bios {
            for system in folders.keys() {
                let Some(svars) = layout::system_vars(&l, system) else { continue };
                for (name, src) in crate::bios::found_for(conn, lib, system)? {
                    let mut v = svars.clone();
                    v.insert("bios_file", name.clone());
                    let Some(to) = layout::expand(&out_of.path, &places, &v)? else { continue };
                    if !files.iter().any(|f| key(&f.1) == key(&to)) {
                        files.push((src, to, "bios", format!("BIOS › {name}"), system.clone()));
                    }
                }
            }
        }
    }
    let mut carried_there = 0u64;
    for (src, to, what, game, system) in files {
        let top = top_of(&to);
        let size = std::fs::metadata(&src).map(|m| m.len()).unwrap_or(0);
        let src_stamp = organise::file_stamp(&src);
        wanted.push(json!({ "path": to.display().to_string(), "game": game, "system": system, "dat": Value::Null, "carried": what,
                            "from": src.display().to_string(), "from_mtime": src_stamp.map(|s| s.1) }));
        if to.exists() {
            // there already: the same unless the library's copy changed since it was placed
            let was = recorded.iter().find(|f| f["path"].as_str().is_some_and(|p| key(Path::new(p)) == key(&to)));
            let same = match was {
                Some(f) => f["from_mtime"].as_i64() == src_stamp.map(|s| s.1) && known.get(&key(&to)) == organise::file_stamp(&to).as_ref(),
                None => true,
            };
            if same {
                carried_there += size;
                continue;
            }
            let old = outside(&top, &to);
            plan.ops.push(Op { op: "aside".into(), game: game.clone(), to: old.path.clone(), size: 0, from: vec![old], names: vec![], aside: vec![], note: None });
        }
        let mut from = outside(&top, &src);
        from.root = format!("dir:{}", src.parent().unwrap_or(Path::new(".")).display());
        from.rel = src.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let src_ext = platforms::ext_of(&src.to_string_lossy());
        let to_ext = platforms::ext_of(&to.to_string_lossy());
        let image = what == "media" && PICTURES.contains(&src_ext.as_str()) && (o.carry.image_size > 0 || src_ext != to_ext);
        let op = if image {
            "image"
        } else if !o.always_copy && organise::same_drive(&src, &to) {
            "link"
        } else {
            "copy"
        };
        let names = if image { vec![o.carry.image_size.to_string()] } else { vec![] };
        plan.ops.push(Op { op: op.into(), game: game.clone(), to: to.display().to_string(), from: vec![from], names, aside: vec![], size, note: Some(what.into()) });
    }
    for (to, text, game, system) in texts {
        wanted.push(json!({ "path": to.display().to_string(), "game": game, "system": system, "dat": Value::Null, "carried": "text" }));
        if std::fs::read_to_string(&to).ok().as_deref() == Some(text.as_str()) {
            continue;
        }
        plan.ops.push(text_op(&game, &to, &top_of(&to), text, "The game's description, as a text file the device shows."));
    }
    // files placed before that the collection no longer has are set aside (at the top of their place)
    let wanted_keys: HashSet<String> = wanted.iter().filter_map(|f| f["path"].as_str()).map(|p| key(Path::new(p))).collect();
    let mut drop = vec![];
    for f in &recorded {
        let Some(p) = f["path"].as_str().map(PathBuf::from) else { continue };
        if !tops.iter().any(|t| rel_to(t, &p).is_some()) || wanted_keys.contains(&key(&p)) || !p.is_file() {
            continue;
        }
        let src = outside(&top_of(&p), &p);
        plan.ops.push(Op { op: "aside".into(), game: f["game"].as_str().unwrap_or("").into(), to: src.path.clone(), size: 0, from: vec![src], names: vec![], aside: vec![], note: None });
        drop.push(f.clone());
    }
    // each system's gamelist (entries added for games that have none, details and pictures set)
    if let Some(gl) = gamelist {
        for (system, list) in &entries {
            let Some(vars) = layout::system_vars(&l, system) else { continue };
            let Some(at) = layout::expand(&gl.path, &places, &vars)? else { continue };
            let mine: HashMap<String, Vec<(String, String)>> = details.iter().filter(|((s, _), _)| s == system).map(|((_, e), v)| (e.clone(), v.clone())).collect();
            let old = std::fs::read_to_string(&at).ok();
            if let Some(text) = merge_gamelist(old.as_deref(), list, &mine) {
                plan.ops.push(text_op("", &at, &top_of(&at), text, "The system's list of games: entries are added for the games exported, with the details and pictures chosen; the rest of it is kept."));
            }
        }
    }
    // the collection as ES-DE's custom collection
    if l.collection.as_deref() == Some("esde-cfg") {
        if let Some(es) = places.get("esde") {
            let roms = rom_dir(es);
            let games_place = layout::games_place(&l).and_then(|p| places.get(&p)).cloned().unwrap_or_else(|| out.clone());
            let in_roms = key(&games_place).trim_end_matches('/') == key(&roms).trim_end_matches('/');
            let mut lines = vec![];
            for (system, list) in &entries {
                let Some(folder) = folders.get(system) else { continue };
                for (entry, _, _) in list {
                    let rel = rel_to(&games_place, folder).filter(|_| in_roms);
                    lines.push(match rel {
                        Some(r) => format!("%ROMPATH%/{r}/{entry}"),
                        None => folder.join(entry).to_string_lossy().replace('\\', "/"),
                    });
                }
            }
            let at = es.join("collections").join(format!("custom-{}.cfg", safe_name(&c.name)));
            let text = lines.iter().map(|l| format!("{l}\n")).collect::<String>();
            if !lines.is_empty() && std::fs::read_to_string(&at).ok().as_deref() != Some(text.as_str()) {
                plan.ops.push(text_op("", &at, es, text, "The collection, as ES-DE's custom collection."));
            }
        }
    }
    plan.space = organise::space_needed(&plan.ops);
    // how big it is on the card, against the collection's size limit
    let total: u64 = plan.ops.iter().filter(|op| op.op != "aside").map(|op| op.size).sum::<u64>()
        + plan.present.iter().filter_map(|p| p["to"].as_str()).filter_map(|t| std::fs::metadata(t).ok()).map(|m| m.len()).sum::<u64>()
        + carried_there;
    let asked: BTreeMap<String, String> = l.asked().filter_map(|p| places.get(&p.id).map(|d| (p.id.clone(), d.display().to_string()))).collect();
    plan.export = Some(json!({
        "id": export_id, "collection": c.id, "name": c.name, "layout": l.id, "layout_name": l.name, "target": l.id, "places": asked,
        "out": out.display().to_string(), "esde": places.get("esde").map(|e| e.display().to_string()),
        "always_copy": o.always_copy, "short_names": o.short_names, "carry": o.carry,
        "first": first, "files": wanted, "drop": drop, "total": total, "limit": c.size_limit,
    }));
    Ok(plan)
}

/// A gamelist with an entry for each game exported that has none, and the details
/// chosen set in each exported game's entry (its other fields, and other games' entries,
/// are kept). None: nothing changes.
fn merge_gamelist(old: Option<&str>, list: &[(String, String, String)], details: &HashMap<String, Vec<(String, String)>>) -> Option<String> {
    let norm = |p: &str| xml_unescape(p.trim()).replace('\\', "/").trim_start_matches("./").to_lowercase();
    let text = old.unwrap_or("").to_string();
    // the entries there already, with their details set
    let mut have: HashSet<String> = HashSet::new();
    let mut out = String::new();
    let mut rest = text.as_str();
    while let Some(at) = rest.find("<game>").or_else(|| rest.find("<game ")) {
        let Some(end) = rest[at..].find("</game>").map(|e| at + e + "</game>".len()) else { break };
        out.push_str(&rest[..at]);
        let mut block = rest[at..end].to_string();
        let path = block.split("<path>").nth(1).and_then(|s| s.split("</path>").next()).map(norm).unwrap_or_default();
        if !path.is_empty() {
            if let Some(fields) = details.get(&path) {
                for (tag, value) in fields {
                    block = set_tag(&block, tag, value);
                }
            }
            have.insert(path);
        }
        out.push_str(&block);
        rest = &rest[end..];
    }
    out.push_str(rest);
    let add: String = list
        .iter()
        .filter(|(entry, _, _)| !have.contains(&entry.to_lowercase()))
        .map(|(entry, _, title)| {
            let mut g = format!("\t<game>\n\t\t<path>./{}</path>\n\t\t<name>{}</name>\n", xml_escape(entry), xml_escape(title));
            for (tag, value) in details.get(&entry.to_lowercase()).into_iter().flatten() {
                g.push_str(&format!("\t\t<{tag}>{}</{tag}>\n", xml_escape(value)));
            }
            g + "\t</game>\n"
        })
        .collect();
    let merged = if add.is_empty() {
        out
    } else if out.contains("</gameList>") {
        let at = out.rfind("</gameList>").unwrap();
        format!("{}{add}{}", &out[..at], &out[at..])
    } else if out.contains("<gameList/>") {
        out.replacen("<gameList/>", &format!("<gameList>\n{add}</gameList>"), 1)
    } else {
        format!("<?xml version=\"1.0\"?>\n<gameList>\n{add}</gameList>\n")
    };
    (old != Some(merged.as_str())).then_some(merged)
}

/// A `<game>` block with `<tag>` set to `value` (replaced if it's there, else added before `</game>`).
fn set_tag(block: &str, tag: &str, value: &str) -> String {
    let (open, close, empty) = (format!("<{tag}>"), format!("</{tag}>"), format!("<{tag}/>"));
    let new = format!("{open}{}{close}", xml_escape(value));
    if let Some(a) = block.find(&open) {
        if let Some(b) = block[a..].find(&close) {
            return format!("{}{new}{}", &block[..a], &block[a + b + close.len()..]);
        }
    }
    if let Some(a) = block.find(&empty) {
        return format!("{}{new}{}", &block[..a], &block[a + empty.len()..]);
    }
    let at = block.rfind("</game>").unwrap_or(block.len());
    format!("{}\t{new}\n\t{}", block[..at].trim_end_matches(['\t', ' ']), &block[at..])
}

/// A gamelist's fields from a game's details: (tag, value), for those chosen, in the
/// tags the layout gives them.
fn gamelist_fields(d: &Value, chosen: &[String], tags: &BTreeMap<String, String>) -> Vec<(String, String)> {
    let mut v = vec![];
    let tag = |k: &str| chosen.iter().any(|c| c == k).then(|| tags.get(k).cloned()).flatten();
    let s = |k: &str| d[k].as_str().map(str::trim).filter(|x| !x.is_empty()).map(String::from);
    if let Some(t) = tag("description") {
        if let Some(x) = s("description") {
            v.push((t, x));
        }
    }
    if let Some(t) = tag("rating") {
        if let Some(r) = d["rating"].as_f64().filter(|r| *r > 0.0) {
            v.push((t, format!("{:.2}", r.min(1.0))));
        }
    }
    if let Some(t) = tag("released") {
        if let Some(x) = s("released").and_then(|r| esde_date(&r)) {
            v.push((t, x));
        }
    }
    for k in ["developer", "publisher", "players"] {
        if let Some(t) = tag(k) {
            if let Some(x) = s(k).or_else(|| d[k].as_u64().map(|n| n.to_string())) {
                v.push((t, x));
            }
        }
    }
    if let Some(t) = tag("genres") {
        let g = match &d["genres"] {
            Value::Array(a) => a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "),
            Value::String(x) => x.clone(),
            _ => String::new(),
        };
        if !g.is_empty() {
            v.push((t, g));
        }
    }
    v
}

/// A release date as ES-DE writes it (`YYYYMMDDT000000`), from "1994-03-19", "1994" or "24 Feb, 1994".
fn esde_date(s: &str) -> Option<String> {
    let digits: Vec<&str> = s.split(|c: char| !c.is_ascii_digit()).filter(|t| !t.is_empty()).collect();
    if digits.len() >= 3 && digits[0].len() == 4 {
        let (m, d): (u32, u32) = (digits[1].parse().ok()?, digits[2].parse().ok()?);
        if (1..=12).contains(&m) && (1..=31).contains(&d) {
            return Some(format!("{}{m:02}{d:02}T000000", digits[0]));
        }
    }
    crate::identify::year_of(s).map(|y| format!("{y}0101T000000"))
}

/// A game's picture of a kind, if it has one.
fn media_file(dir: &Path, kind: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| p.file_name().and_then(|n| n.to_str()).and_then(|n| n.split_once('.')).is_some_and(|(k, _)| k == kind))
}

/// A game's manual to export: the user's PDF manual first, else a scraped one.
fn manual_of(conn: &Connection, lib: &Library, dir: &Path) -> Option<PathBuf> {
    let rel = lib.relative(dir)?;
    let docs = crate::docs::of_folder(conn, &rel).ok()?;
    let pdf = |d: &&Value| d["kind"] == "manual" && d["name"].as_str().is_some_and(|n| n.to_lowercase().ends_with(".pdf"));
    let pick = docs.iter().filter(pdf).find(|d| d["source"].is_null()).or_else(|| docs.iter().find(pdf))?;
    lib.resolve(pick["path"].as_str()?).ok()
}

/// A picture made smaller (its longest side at most `max` pixels), as the bytes of the
/// format `to`'s extension names (PNG for anything but JPEG).
pub fn shrink(src: &Path, max: u32, to: &Path) -> Result<Vec<u8>> {
    let img = ::image::ImageReader::open(src)?.with_guessed_format()?.decode().map_err(|e| anyhow!("couldn't read the picture {}: {e}", src.display()))?;
    let img = if max > 0 && img.width().max(img.height()) > max { img.resize(max, max, ::image::imageops::FilterType::Lanczos3) } else { img };
    let mut out = std::io::Cursor::new(Vec::new());
    let ext = platforms::ext_of(&to.to_string_lossy());
    if ext == "jpg" || ext == "jpeg" {
        let enc = ::image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 88);
        img.to_rgb8().write_with_encoder(enc)?;
    } else {
        img.write_to(&mut out, ::image::ImageFormat::Png)?;
    }
    Ok(out.into_inner())
}

/// Remember what an export placed, once its change has run (or been undone).
pub fn record(lib: &Library, plan: &Plan, undone: bool) -> Result<()> {
    let Some(e) = &plan.export else { return Ok(()) };
    let id = e["id"].as_str().unwrap_or("");
    let path = path_of(lib, id)?;
    if undone && e["first"] == true {
        let _ = std::fs::remove_file(&path);
        return Ok(());
    }
    let old = config::read_json_object(&path);
    let old_files: HashMap<String, Value> = old["files"].as_array().into_iter().flatten().filter_map(|f| Some((key(Path::new(f["path"].as_str()?)), f.clone()))).collect();
    let written: HashSet<String> = plan.ops.iter().filter(|o| o.op != "aside" && o.op != "text").map(|o| key(Path::new(&o.to))).collect();
    let mut files: BTreeMap<String, Value> = BTreeMap::new();
    for f in e["files"].as_array().into_iter().flatten() {
        let Some(p) = f["path"].as_str() else { continue };
        let k = key(Path::new(p));
        // a file there already that the export didn't place isn't its to set aside later
        if !(written.contains(&k) || old_files.contains_key(&k)) {
            continue;
        }
        if let Some((size, mtime)) = organise::file_stamp(Path::new(p)) {
            let mut f = f.clone();
            f["size"] = json!(size);
            f["mtime"] = json!(mtime);
            files.insert(k, f);
        }
    }
    // the rest it placed before, while they're there (set aside ones are back after an Undo)
    for f in old_files.values().chain(e["drop"].as_array().into_iter().flatten()) {
        let Some(p) = f["path"].as_str() else { continue };
        let k = key(Path::new(p));
        if !files.contains_key(&k) && Path::new(p).is_file() {
            files.insert(k, f.clone());
        }
    }
    // folders left empty by files set aside go (within the export's places)
    let mut tops: Vec<PathBuf> = e["places"].as_object().into_iter().flatten().filter_map(|(_, v)| v.as_str()).map(PathBuf::from).collect();
    tops.push(PathBuf::from(e["out"].as_str().unwrap_or("")));
    tops.retain(|t| t.is_absolute());
    for f in e["drop"].as_array().into_iter().flatten() {
        let mut at = f["path"].as_str().map(Path::new).and_then(Path::parent);
        while let Some(d) = at {
            if !tops.iter().any(|t| rel_to(t, d).is_some()) || std::fs::remove_dir(d).is_err() {
                break;
            }
            at = d.parent();
        }
    }
    let mut v = json!({
        "id": id, "collection": e["collection"], "name": e["name"], "layout": e["layout"], "layout_name": e["layout_name"], "places": e["places"],
        "target": e["target"], "out": e["out"], "esde": e["esde"],
        "always_copy": e["always_copy"], "short_names": e["short_names"], "carry": e["carry"],
        "created": old["created"].as_str().map(String::from).unwrap_or_else(now), "updated": now(), "change": plan.id,
        "files": files.into_values().collect::<Vec<_>>(),
    });
    if let Some(checked) = old.get("checked") {
        v["checked"] = checked.clone();
    }
    config::write_json(&path, &v)
}

// ------------------------------------------------------------ the layout editor's preview

/// A few games to show a layout with (D39): the first games of up to three systems of a
/// collection, as they'd be exported, with what the library holds for each; or, with
/// none, a stand-in game for two systems. Worked out once and kept while the editor is open.
pub fn sample(ix: &Index, lib: &Library, collection: Option<&str>) -> Result<Value> {
    let conn = ix.conn();
    let mut systems: Vec<Value> = vec![];
    if let Some(cid) = collection.filter(|c| !c.is_empty()) {
        let c = collection::load(lib, cid)?;
        let w = collection::work_out(ix, lib, &c)?;
        let tmp = std::env::temp_dir().join(format!("gamerepo-sample-{}", organise::new_id()));
        for s in w.systems.iter().filter(|s| s.games.iter().any(|g| g.status == "have")).take(3) {
            let games: Vec<&collection::Game> = s.games.iter().filter(|g| g.status == "have").take(3).collect();
            let names: Vec<String> = games.iter().map(|g| g.name.clone()).collect();
            // the game's entry in its system's folder: as ES-DE and the firmwares want it, and as Settings › Naming says
            let mut entry_of: HashMap<(bool, String), (String, bool)> = HashMap::new();
            for frontend in [true, false] {
                let folder = tmp.join(if frontend { "f" } else { "n" });
                let sub = organise::plan(ix, lib, &organise::Options {
                    dat_id: s.dat_id, mode: "copy".into(), out: Some(folder.clone()), format: "keep".into(), games: Some(names.clone()), esde: frontend, ..Default::default()
                })?;
                for op in &sub.ops {
                    if let Some(rel) = rel_to(&folder, Path::new(&op.to)) {
                        let entry = rel.split('/').next().unwrap_or(&rel).to_string();
                        entry_of.entry((frontend, op.game.clone())).or_insert((entry, rel.contains('/')));
                    }
                }
            }
            let mut out = vec![];
            for g in games {
                let (Some(f), Some(n)) = (entry_of.get(&(true, g.name.clone())), entry_of.get(&(false, g.name.clone()))) else { continue };
                let dg = crate::meta::dat_game(conn, g.id)?;
                let dir = crate::meta::folder_of(conn, &dg)?.and_then(|f| lib.resolve(&f).ok());
                let mut media = serde_json::Map::new();
                let (mut manual, mut docs, mut description) = (Value::Null, vec![], false);
                if let Some(dir) = &dir {
                    for (k, _, _) in crate::meta::MEDIA {
                        if let Some(p) = media_file(&dir.join("media"), k) {
                            media.insert(k.to_string(), json!(platforms::ext_of(&p.to_string_lossy())));
                        }
                    }
                    manual = manual_of(conn, lib, dir).and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).map(Value::from).unwrap_or(Value::Null);
                    let rel = lib.relative(dir).unwrap_or_default();
                    docs = crate::docs::of_folder(conn, &rel)?.iter().filter_map(|d| d["path"].as_str().and_then(|p| p.rsplit('/').next()).map(String::from)).collect();
                    description = crate::meta::read_game(dir)["details"]["description"].as_str().is_some_and(|d| !d.trim().is_empty());
                }
                out.push(json!({
                    "name": g.name, "title": if g.title.is_empty() { &g.name } else { &g.title },
                    "frontend": { "entry": f.0, "dir": f.1 }, "naming": { "entry": n.0, "dir": n.1 },
                    "media": media, "manual": manual, "docs": docs, "description": description,
                }));
            }
            let bios: Vec<Value> = crate::bios::found_for(conn, lib, &s.platform)?.into_iter().map(|(n, _)| json!(n)).collect();
            systems.push(json!({ "system": s.platform, "games": out, "bios": bios }));
        }
    }
    if systems.iter().all(|s| s["games"].as_array().is_none_or(|g| g.is_empty())) {
        let all: serde_json::Map<String, Value> = crate::meta::MEDIA.iter().map(|m| (m.0.to_string(), json!(if m.0 == "video" { "mp4" } else { "png" }))).collect();
        systems = vec![
            json!({ "system": "gb", "stand_in": true, "bios": ["gb_bios.bin"], "games": [{
                "name": "Example Game (World)", "title": "Example Game",
                "frontend": { "entry": "Example Game (World).gb", "dir": false }, "naming": { "entry": "Example Game (World).gb", "dir": false },
                "media": all, "manual": "Example Game Manual.pdf", "docs": ["Example Game Map.png"], "description": true }] }),
            json!({ "system": "psx", "stand_in": true, "bios": ["scph5501.bin"], "games": [{
                "name": "Example Disc (USA)", "title": "Example Disc",
                "frontend": { "entry": "Example Disc (USA).cue", "dir": true }, "naming": { "entry": "Example Disc (USA)", "dir": true },
                "media": all, "manual": "Example Disc Manual.pdf", "docs": [], "description": true }] }),
        ];
    }
    Ok(json!({ "systems": systems }))
}

/// What an export in this layout would write for the sample games, as files under each
/// place, each saying what it is and which part of the layout put it there (`at`), with
/// the layout's problems. Nothing is read or written: it's worked out from the sample.
pub fn preview(l: &layout::Layout, sample: &Value) -> Value {
    let problems = layout::check(l);
    let places: BTreeMap<String, PathBuf> = {
        let mut m: BTreeMap<String, PathBuf> = l.asked().map(|p| (p.id.clone(), PathBuf::from(format!("/{}", p.id)))).collect();
        for p in l.places.iter().filter(|p| p.from.is_some()) {
            if let Ok(Some(d)) = layout::expand(p.from.as_deref().unwrap_or(""), &m, &layout::Vars::new()) {
                m.insert(p.id.clone(), d);
            }
        }
        m
    };
    let shown = |p: &Path| p.to_string_lossy().replace('\\', "/").trim_start_matches('/').to_string();
    let mut items: Vec<Value> = vec![];
    let mut left: Vec<Value> = vec![];
    let push = |items: &mut Vec<Value>, p: Option<PathBuf>, kind: &str, at: &str, what: &str, system: &str, game: &str, faint: bool| {
        if let Some(p) = p {
            items.push(json!({ "path": shown(&p), "kind": kind, "at": at, "what": what, "system": system, "game": game, "faint": faint }));
        }
    };
    let names: HashMap<&str, &str> = crate::meta::MEDIA.iter().map(|m| (m.0, m.1)).collect();
    let style = if l.games.style == "naming" { "naming" } else { "frontend" };
    for s in sample["systems"].as_array().into_iter().flatten() {
        let system = s["system"].as_str().unwrap_or("");
        let Some(svars) = layout::system_vars(l, system) else {
            left.push(json!({ "system": system, "system_name": platforms::name_of(system), "at": format!("systems.{system}") }));
            continue;
        };
        let Ok(Some(folder)) = layout::games_dir(l, system, &places) else { continue };
        items.push(json!({ "path": shown(&folder), "kind": "system", "at": format!("systems.{system}"), "what": format!("{}'s games", platforms::name_of(system)), "system": system, "game": "", "faint": false, "dir": true }));
        for g in s["games"].as_array().into_iter().flatten() {
            let e = &g[style];
            let entry = e["entry"].as_str().unwrap_or("").to_string();
            let is_dir = e["dir"].as_bool().unwrap_or(false);
            let game = g["name"].as_str().unwrap_or("");
            let stem = if is_dir { entry.clone() } else { Path::new(&entry).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| entry.clone()) };
            let mut vars = svars.clone();
            vars.insert("file", entry.clone());
            vars.insert("stem", stem);
            vars.insert("title", g["title"].as_str().unwrap_or(game).to_string());
            let to = folder.join(&entry);
            push(&mut items, Some(if is_dir { to.join("…") } else { to }), "game", "games.path", if is_dir { "The game (a folder of its files)" } else { "The game" }, system, game, false);
            for (kind, out_of) in &l.media {
                let mut v = vars.clone();
                let have = if kind == "manual" { g["manual"].as_str().map(|_| "pdf".to_string()) } else { g["media"][kind.as_str()].as_str().map(String::from) };
                let src_ext = have.clone().unwrap_or_else(|| if kind == "video" { "mp4".into() } else if kind == "manual" { "pdf".into() } else { "png".into() });
                let ext = if kind != "manual" && kind != "video" && (l.pictures.as_ == "png" || l.pictures.as_ == "jpg") { l.pictures.as_.clone() } else { src_ext };
                v.insert("ext", ext);
                v.insert("kind", kind.clone());
                v.insert("doc_file", g["manual"].as_str().unwrap_or("Manual.pdf").to_string());
                let what = if kind == "manual" { "Manual".to_string() } else { names.get(kind.as_str()).copied().unwrap_or(kind).to_string() };
                let what = match &out_of.tag { Some(t) => format!("{what} (gamelist <{t}>)"), None => what };
                push(&mut items, layout::expand(&out_of.path, &places, &v).ok().flatten(), if kind == "manual" { "manual" } else { "media" }, &format!("media.{kind}"), &what, system, game, have.is_none());
            }
            for (kind, out_of) in &l.text {
                let mut v = vars.clone();
                v.insert("ext", "txt".into());
                v.insert("kind", kind.clone());
                push(&mut items, layout::expand(&out_of.path, &places, &v).ok().flatten(), "text", &format!("text.{kind}"), "Description (text)", system, game, !g["description"].as_bool().unwrap_or(false));
            }
            if let Some(d) = &l.documents {
                let docs: Vec<String> = g["docs"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect();
                let faint = docs.is_empty();
                for name in if faint { vec!["Guide.pdf".to_string()] } else { docs } {
                    let mut v = vars.clone();
                    v.insert("ext", platforms::ext_of(&name));
                    v.insert("doc_file", name);
                    push(&mut items, layout::expand(&d.path, &places, &v).ok().flatten(), "doc", "documents", "Document", system, game, faint);
                }
            }
        }
        if let Some(gl) = &l.gamelist {
            push(&mut items, layout::expand(&gl.path, &places, &svars).ok().flatten(), "gamelist", "gamelist.path", "The system's gamelist", system, "", false);
        }
        if let Some(b) = &l.bios {
            let names: Vec<String> = s["bios"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect();
            let faint = names.is_empty();
            for name in if faint { vec!["bios.bin".to_string()] } else { names } {
                let mut v = svars.clone();
                v.insert("bios_file", name);
                push(&mut items, layout::expand(&b.path, &places, &v).ok().flatten(), "bios", "bios", "BIOS file", system, "", faint);
            }
        }
    }
    if l.collection.as_deref() == Some("esde-cfg") {
        push(&mut items, places.get("esde").map(|e| e.join("collections").join("custom-<collection>.cfg")), "collection", "collection", "The collection, as ES-DE's custom collection", "", "", false);
    }
    // the same file twice (two kinds, or two games, with one name)
    let clash: HashSet<String> = {
        let mut count: HashMap<String, usize> = HashMap::new();
        for it in items.iter().filter(|i| i["kind"] != "system") {
            *count.entry(it["path"].as_str().unwrap_or("").to_lowercase()).or_default() += 1;
        }
        count.into_iter().filter(|(_, n)| *n > 1).map(|(k, _)| k).collect()
    };
    for it in items.iter_mut() {
        if clash.contains(&it["path"].as_str().unwrap_or("").to_lowercase()) && it["kind"] != "system" {
            it["clash"] = json!(true);
        }
    }
    items.sort_by(|a, b| a["path"].as_str().unwrap_or("").to_lowercase().cmp(&b["path"].as_str().unwrap_or("").to_lowercase()));
    json!({
        "places": l.asked().map(|p| json!({ "id": p.id, "label": p.label })).chain(l.places.iter().filter(|p| p.from.is_some()).map(|p| json!({ "id": p.id, "label": p.label, "from": p.from }))).collect::<Vec<_>>(),
        "items": items, "left": left, "problems": problems,
        "stand_in": sample["systems"].as_array().is_some_and(|s| s.iter().any(|x| x["stand_in"] == true)),
    })
}

// ------------------------------------------------------------ the check

/// What a file holds: its members (an archive or a disc image) or itself, hashed.
fn contents(path: &Path, cancel: &AtomicBool) -> Result<Vec<(String, Hashes)>> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if let Some(kind) = image::kind_of(&name) {
        return Ok(image::read(path, kind, cancel)?.members.into_iter().map(|m| (m.name, m.hashes)).collect());
    }
    if let Some(kind) = archive::kind_of(&name) {
        return Ok(archive::read(path, kind, true, cancel)?.members.into_iter().map(|m| (m.name, m.hashes)).collect());
    }
    let size = std::fs::metadata(path)?.len();
    let h = hash::hash_reader(&mut std::fs::File::open(path)?, size, &name, cancel)?;
    Ok(vec![(name, h)])
}

struct DatRom {
    name: String,
    size: Option<u64>,
    crc32: Option<String>,
    sha1: Option<String>,
}

impl DatRom {
    fn matches(&self, h: &Hashes) -> bool {
        let one = |h: &Hashes| match (&self.sha1, &h.sha1) {
            (Some(want), Some(have)) => want.eq_ignore_ascii_case(have),
            _ => self.size == Some(h.size) && self.crc32.as_deref().is_some_and(|c| c.eq_ignore_ascii_case(&h.crc32)),
        };
        one(h) || h.hl.as_deref().is_some_and(one)
    }
}

/// A game's ROMs: in the DAT the export used, else the system's current DAT of that name.
fn game_roms(conn: &Connection, dat: Option<i64>, system: &str, game: &str) -> Result<Vec<DatRom>> {
    let mut gid: Option<i64> = None;
    if let Some(d) = dat {
        gid = conn.query_row("SELECT id FROM dat_games WHERE dat_id = ?1 AND name = ?2", params![d, game], |r| r.get(0)).optional()?;
    }
    if gid.is_none() {
        gid = conn
            .query_row(
                "SELECT g.id FROM dat_games g JOIN dats d ON d.id = g.dat_id WHERE d.platform = ?1 AND g.name = ?2 ORDER BY d.current DESC, d.id DESC LIMIT 1",
                params![system, game],
                |r| r.get(0),
            )
            .optional()?;
    }
    let Some(gid) = gid else { return Ok(vec![]) };
    let mut q = conn.prepare("SELECT name, size, crc32, sha1 FROM dat_roms WHERE game_id = ?1")?;
    let v = q
        .query_map([gid], |r| Ok(DatRom { name: r.get(0)?, size: r.get::<_, Option<i64>>(1)?.map(|s| s as u64), crc32: r.get(2)?, sha1: r.get(3)? }))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(v)
}

/// Read every file an export placed back: missing, changed (no longer what its game's
/// DAT says) or as it should be. `progress` gets `{ item, items, name }`.
pub fn check(ix: &Index, lib: &Library, id: &str, cancel: &AtomicBool, progress: &mut dyn FnMut(Value)) -> Result<Value> {
    let e = load(lib, id)?;
    let conn = ix.conn();
    let files: Vec<Value> = e["files"].as_array().cloned().unwrap_or_default();
    let (mut ok, mut missing, mut changed) = (0usize, vec![], vec![]);
    let mut roms_of: HashMap<(String, String), Vec<DatRom>> = HashMap::new();
    for (i, f) in files.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let p = PathBuf::from(f["path"].as_str().unwrap_or(""));
        let (game, system) = (f["game"].as_str().unwrap_or(""), f["system"].as_str().unwrap_or(""));
        progress(json!({ "item": i + 1, "items": files.len(), "name": game }));
        let shown = json!({ "path": p.display().to_string(), "game": game, "system": system, "system_name": platforms::name_of(system) });
        if !p.is_file() {
            missing.push(shown);
            continue;
        }
        if f["carried"].is_string() {
            // a picture, manual or document: checked by being there
            ok += 1;
            continue;
        }
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if cue::is_sheet(&name) {
            // a sheet is rewritten for its tracks' names, so it's checked by being there
            ok += 1;
            continue;
        }
        let k = (system.to_string(), game.to_string());
        if !roms_of.contains_key(&k) {
            roms_of.insert(k.clone(), game_roms(conn, f["dat"].as_i64(), system, game)?);
        }
        let roms = &roms_of[&k];
        let verdict = match contents(&p, cancel) {
            Err(e) if cancel.load(Ordering::Relaxed) => {
                let _ = e;
                break;
            }
            Err(e) => Some(format!("It couldn't be read: {e:#}")),
            Ok(_) if roms.is_empty() => Some("Its game isn't in a DAT any more.".to_string()),
            Ok(have) => {
                let whole = image::kind_of(&name).is_some() || archive::kind_of(&name).is_some();
                let matched: Vec<bool> = roms.iter().map(|r| have.iter().any(|(_, h)| r.matches(h))).collect();
                let good = if whole {
                    // every ROM of the game is in it (a disc image holds no cue sheet)
                    roms.iter().zip(&matched).all(|(r, m)| *m || cue::is_sheet(&r.name))
                } else {
                    matched.iter().any(|m| *m)
                };
                if good { None } else { Some("Its contents don't match its game in the DAT.".to_string()) }
            }
        };
        match verdict {
            None => ok += 1,
            Some(why) => {
                let mut v = shown;
                v["why"] = json!(why);
                changed.push(v);
            }
        }
    }
    let stopped = cancel.load(Ordering::Relaxed);
    let result = json!({ "id": id, "files": files.len(), "ok": ok, "missing": missing, "changed": changed, "stopped": stopped, "checked": now() });
    if !stopped && lib.writable().is_ok() {
        let p = path_of(lib, id)?;
        let mut rec = config::read_json_object(&p);
        rec["checked"] = json!({ "at": now(), "ok": ok, "missing": missing.len(), "changed": changed.len() });
        config::write_json(&p, &rec)?;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collection::tests::fixture;
    use crate::collection::{Collection, Rules};

    fn collection(lib: &Library, f: impl FnOnce(&mut Rules)) -> Collection {
        let mut c = Collection { id: String::new(), name: "Best Of".into(), rules: Rules::default(), added: vec![], removed: vec![], size_limit: None, created: String::new(), modified: String::new() };
        c.rules.platforms = vec!["gb".into()];
        f(&mut c.rules);
        collection::save(lib, c).unwrap()
    }

    fn tree(dir: &Path) -> Vec<String> {
        let mut v = vec![];
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                if e.file_type().unwrap().is_dir() {
                    stack.push(e.path());
                } else {
                    v.push(rel_to(dir, &e.path()).unwrap());
                }
            }
        }
        v.sort();
        v
    }

    fn run(ix: &Index, lib: &Library, o: &Options) -> Plan {
        let plan = plan(ix, lib, o).unwrap();
        let r = organise::run(ix, lib, &plan, &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(r["failed"], json!([]), "{r}");
        plan
    }

    #[test]
    fn a_collection_is_exported_to_es_de_updated_undone_and_checked() {
        let _turn = platforms::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (dir, lib, ix) = fixture("export-esde");
        let c = collection(&lib, |_| {});
        let es = dir.join("ES-DE");
        // (not "ROMs": on Windows that is the fixture's "Roms", a folder with games)
        let roms = dir.join("ES-ROMs");
        std::fs::create_dir_all(es.join("settings")).unwrap();
        std::fs::write(es.join("settings").join("es_settings.xml"), format!("<?xml version=\"1.0\"?>\n<string name=\"ROMDirectory\" value=\"{}\" />\n", roms.display())).unwrap();
        // a gamelist there already, with an entry of its own
        std::fs::create_dir_all(es.join("gamelists").join("gb")).unwrap();
        std::fs::write(es.join("gamelists/gb/gamelist.xml"), "<?xml version=\"1.0\"?>\n<gameList>\n\t<game>\n\t\t<path>./Delta (World).gb</path>\n\t\t<name>My Delta</name>\n\t</game>\n</gameList>\n").unwrap();
        // refused inside a folder with games
        let bad = Options { collection: c.id.clone(), target: "esde".into(), out: dir.join("Roms").join("x"), ..Default::default() };
        assert!(plan(&ix, &lib, &bad).is_err());
        let o = Options { collection: c.id.clone(), target: "esde".into(), out: roms.clone(), esde: Some(es.clone()), short_names: true, ..Default::default() };
        let p = run(&ix, &lib, &o);
        assert_eq!(p.games, 4);
        assert_eq!(tree(&roms), ["gb/Alpha (Europe).gb", "gb/Beta (Europe) (Rev 1).gb", "gb/Delta (World).gb", "gb/Epsilon (Japan) (En,Ja).gb"]);
        let gl = std::fs::read_to_string(es.join("gamelists/gb/gamelist.xml")).unwrap();
        assert!(gl.contains("<name>My Delta</name>") && gl.matches("<path>./Delta (World).gb</path>").count() == 1, "{gl}");
        assert!(gl.contains("<path>./Alpha (Europe).gb</path>\n\t\t<name>Alpha</name>"), "{gl}");
        let cfg = std::fs::read_to_string(es.join("collections/custom-Best Of.cfg")).unwrap();
        assert_eq!(cfg.lines().next(), Some("%ROMPATH%/gb/Alpha (Europe).gb"));
        let exports = list(&lib);
        assert_eq!(exports.len(), 1);
        assert_eq!(exports[0]["count"], 4);
        let eid = exports[0]["id"].as_str().unwrap().to_string();
        // a file of the user's in the export folder is never touched
        std::fs::write(roms.join("gb").join("mine.gb"), b"mine").unwrap();
        // the rules change: Japan first, so Alpha (Japan) and Beta (Japan) replace the European ones
        let mut c2 = collection::load(&lib, &c.id).unwrap();
        c2.rules.regions = vec!["Japan".into(), "USA".into(), "World".into(), "Europe".into()];
        collection::save(&lib, c2).unwrap();
        let again = options_of(&lib, &eid).unwrap();
        let p2 = plan(&ix, &lib, &again).unwrap();
        assert_eq!(p2.present.len(), 2, "Delta and Epsilon are there already");
        assert_eq!(p2.ops.iter().filter(|o| o.op == "aside").count(), 2);
        organise::run(&ix, &lib, &p2, &AtomicBool::new(false), &mut |_| {}).unwrap();
        let mut now_there = tree(&roms);
        now_there.retain(|p| p.starts_with("gb/"));
        assert_eq!(now_there, ["gb/Alpha (Japan).gb", "gb/Beta (Japan).gb", "gb/Delta (World).gb", "gb/Epsilon (Japan) (En,Ja).gb", "gb/mine.gb"]);
        assert!(roms.join(organise::SET_ASIDE).join(&p2.id).join("gb").join("Alpha (Europe).gb").is_file());
        assert_eq!(list(&lib)[0]["count"], 4);
        // the check: all as they should be, then one changed and one gone
        let r = check(&ix, &lib, &eid, &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!((r["ok"].as_u64(), r["missing"].as_array().unwrap().len(), r["changed"].as_array().unwrap().len()), (Some(4), 0, 0), "{r}");
        // (the export is copies when hardlinks are the same file as the library's: write a new file)
        let delta = roms.join("gb").join("Delta (World).gb");
        std::fs::remove_file(&delta).unwrap();
        std::fs::write(&delta, b"not delta").unwrap();
        std::fs::remove_file(roms.join("gb").join("Beta (Japan).gb")).unwrap();
        let r = check(&ix, &lib, &eid, &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!((r["ok"].as_u64(), r["missing"].as_array().unwrap().len(), r["changed"].as_array().unwrap().len()), (Some(2), 1, 1), "{r}");
        // Undo of the update puts the European ones back and the record with them
        std::fs::remove_file(&delta).unwrap();
        std::fs::hard_link(dir.join("Roms").join("Delta (World).gb"), &delta).unwrap();
        organise::undo(&ix, &lib, &p2.id, &AtomicBool::new(false), &mut |_| {}).unwrap();
        let mut back = tree(&roms);
        back.retain(|p| p.starts_with("gb/"));
        assert_eq!(back, ["gb/Alpha (Europe).gb", "gb/Beta (Europe) (Rev 1).gb", "gb/Delta (World).gb", "gb/Epsilon (Japan) (En,Ja).gb", "gb/mine.gb"]);
        let rec = load(&lib, &eid).unwrap();
        let mut paths: Vec<String> = rec["files"].as_array().unwrap().iter().map(|f| rel_to(&roms, Path::new(f["path"].as_str().unwrap())).unwrap()).collect();
        paths.sort();
        assert_eq!(paths, ["gb/Alpha (Europe).gb", "gb/Beta (Europe) (Rev 1).gb", "gb/Delta (World).gb", "gb/Epsilon (Japan) (En,Ja).gb"]);
        // Undo of the first export removes it all but the user's file, and forgets it
        organise::undo(&ix, &lib, &p.id, &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(tree(&roms), ["gb/mine.gb"]);
        assert!(list(&lib).is_empty());
        let gl = std::fs::read_to_string(es.join("gamelists/gb/gamelist.xml")).unwrap();
        assert!(!gl.contains("Alpha"), "the gamelist is as it was");
        assert!(!es.join("collections/custom-Best Of.cfg").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_disc_game_of_several_files_is_a_folder_for_es_de_and_named_for_a_plain_folder() {
        let _turn = platforms::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = crate::library::tests::temp_dir("export-disc");
        let lib = Library::open(dir.join("lib")).unwrap();
        let src = dir.join("PSX");
        std::fs::create_dir_all(&src).unwrap();
        let t1: Vec<u8> = (0..2352 * 4).map(|i| (i % 251) as u8).collect();
        let t2: Vec<u8> = (0..2352 * 2).map(|i| (i % 13) as u8).collect();
        let cue_text = "FILE \"Disc (USA) (Track 1).bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\nFILE \"Disc (USA) (Track 2).bin\" BINARY\n  TRACK 02 AUDIO\n    INDEX 01 00:00:00\n";
        std::fs::write(src.join("Disc (USA) (Track 1).bin"), &t1).unwrap();
        std::fs::write(src.join("Disc (USA) (Track 2).bin"), &t2).unwrap();
        std::fs::write(src.join("Disc (USA).cue"), cue_text).unwrap();
        let rom = |name: &str, b: &[u8]| {
            let h = hash::hash_reader(&mut &b[..], b.len() as u64, name, &AtomicBool::new(false)).unwrap();
            format!("<rom name=\"{name}\" size=\"{}\" crc=\"{}\" sha1=\"{}\"/>", b.len(), h.crc32, h.sha1.unwrap())
        };
        let dat = format!(
            "<?xml version=\"1.0\"?>\n<datafile><header><name>Sony - PlayStation</name><version>1</version></header>\n<game name=\"Disc (USA)\"><description>Disc (USA)</description>{}{}{}</game>\n</datafile>\n",
            rom("Disc (USA).cue", cue_text.as_bytes()), rom("Disc (USA) (Track 1).bin", &t1), rom("Disc (USA) (Track 2).bin", &t2)
        );
        std::fs::write(dir.join("psx.dat"), dat).unwrap();
        let ix = Index::open(&dir.join("index.sqlite"), &lib.id().unwrap()).unwrap();
        roots::add(&lib, &src, Some("psx")).unwrap();
        crate::scan::scan(&ix, &lib, &crate::scan::Options::default(), &AtomicBool::new(false), &mut |_| {}).unwrap();
        crate::dat::import(&ix, &lib, &dir.join("psx.dat"), &serde_json::Map::new()).unwrap();
        let mut c = Collection { id: String::new(), name: "Discs".into(), rules: Rules::default(), added: vec![], removed: vec![], size_limit: None, created: String::new(), modified: String::new() };
        c.rules.platforms = vec!["psx".into()];
        let c = collection::save(&lib, c).unwrap();
        let roms = dir.join("ROMs");
        run(&ix, &lib, &Options { collection: c.id.clone(), target: "esde".into(), out: roms.clone(), always_copy: true, ..Default::default() });
        assert_eq!(tree(&roms), ["psx/Disc (USA).cue/Disc (USA) (Track 1).bin", "psx/Disc (USA).cue/Disc (USA) (Track 2).bin", "psx/Disc (USA).cue/Disc (USA).cue"]);
        let flat = dir.join("Flat");
        run(&ix, &lib, &Options { collection: c.id.clone(), target: "folder".into(), out: flat.clone(), ..Default::default() });
        assert_eq!(tree(&flat), ["Sony - PlayStation/Disc (USA)/Disc (USA) (Track 1).bin", "Sony - PlayStation/Disc (USA)/Disc (USA) (Track 2).bin", "Sony - PlayStation/Disc (USA)/Disc (USA).cue"]);
        let e = list(&lib).into_iter().find(|e| e["target"] == "esde").unwrap();
        let r = check(&ix, &lib, e["id"].as_str().unwrap(), &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(r["ok"], 3, "{r}");
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn an_export_carries_details_smaller_pictures_and_the_manual_and_they_are_undone() {
        let _turn = platforms::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (dir, lib, ix) = fixture("export-carry");
        let c = collection(&lib, |_| {});
        // Alpha's folder in the library: its details, a big cover and a PDF manual of the user's
        let g = crate::meta::dat_game_named(ix.conn(), &lib, "gb", "Alpha (Europe)").unwrap().unwrap();
        let gdir = crate::meta::ensure_folder(&ix, &lib, &g).unwrap();
        let mut gj = crate::meta::read_game(&gdir);
        gj["details"] = json!({ "description": "A & B", "developer": "Dev Co", "released": "1990-04-21", "rating": 0.8 });
        crate::meta::write_game(&ix, &lib, &gdir, &gj).unwrap();
        std::fs::create_dir_all(gdir.join("media")).unwrap();
        std::fs::write(gdir.join("media").join("cover.png"), crate::scrape::tests::png(800, 600)).unwrap();
        std::fs::create_dir_all(gdir.join("docs")).unwrap();
        std::fs::write(gdir.join("docs").join("Alpha Manual.pdf"), b"%PDF-1.4\n%%EOF\n").unwrap();
        crate::meta::sync(&ix, &lib).unwrap();
        let es = dir.join("ES-DE");
        let roms = dir.join("ES-ROMs");
        std::fs::create_dir_all(es.join("settings")).unwrap();
        std::fs::write(es.join("settings").join("es_settings.xml"), format!("<?xml version=\"1.0\"?>\n<string name=\"ROMDirectory\" value=\"{}\" />\n", roms.display())).unwrap();
        let carry = Carry { fields: vec!["description".into(), "developer".into(), "released".into(), "rating".into()], media: vec!["cover".into(), "screenshot".into()], manuals: true, docs: false, image_size: 320, ..Default::default() };
        let o = Options { collection: c.id.clone(), target: "esde".into(), out: roms.clone(), esde: Some(es.clone()), carry, ..Default::default() };
        let p = run(&ix, &lib, &o);
        let gl = std::fs::read_to_string(es.join("gamelists/gb/gamelist.xml")).unwrap();
        assert!(gl.contains("<desc>A &amp; B</desc>") && gl.contains("<developer>Dev Co</developer>") && gl.contains("<releasedate>19900421T000000</releasedate>") && gl.contains("<rating>0.80</rating>"), "{gl}");
        let media = es.join("downloaded_media").join("gb");
        assert_eq!(tree(&media), ["covers/Alpha (Europe).png", "manuals/Alpha (Europe).pdf"]);
        let cover = ::image::open(media.join("covers/Alpha (Europe).png")).unwrap();
        assert_eq!((cover.width(), cover.height()), (320, 240));
        // nothing changed: nothing to do
        let eid = list(&lib)[0]["id"].as_str().unwrap().to_string();
        let again = plan(&ix, &lib, &options_of(&lib, &eid).unwrap()).unwrap();
        assert!(again.ops.iter().all(|op| op.op == "text"), "{:?}", again.ops.iter().map(|o| (&o.op, &o.to)).collect::<Vec<_>>());
        let r = check(&ix, &lib, &eid, &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(r["missing"], json!([]), "{r}");
        // Undo takes the pictures and the manual away too
        organise::undo(&ix, &lib, &p.id, &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert!(tree(&media).is_empty());
        assert!(gdir.join("media").join("cover.png").is_file(), "the library's own are kept");
        // a plain folder: the pictures and every document beside the game, as they are
        let flat = dir.join("Flat");
        let carry = Carry { media: vec!["cover".into()], docs: true, ..Default::default() };
        run(&ix, &lib, &Options { collection: c.id.clone(), target: "folder".into(), out: flat.clone(), carry, ..Default::default() });
        let t = tree(&flat);
        assert!(t.iter().any(|p| p.ends_with("/Alpha (Europe)/cover.png")) && t.iter().any(|p| p.ends_with("/Alpha (Europe)/docs/Alpha Manual.pdf")), "{t:?}");
        assert_eq!(::image::open(flat.join(t.iter().find(|p| p.ends_with("cover.png")).unwrap())).unwrap().width(), 800);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The files under a folder, without what was set aside.
    fn kept(dir: &Path) -> Vec<String> {
        tree(dir).into_iter().filter(|p| !p.starts_with(organise::SET_ASIDE)).collect()
    }

    /// Alpha's folder in the library with a description, a cover, a screenshot (JPEG) and a manual.
    fn alpha_with_media(ix: &Index, lib: &Library) -> PathBuf {
        let g = crate::meta::dat_game_named(ix.conn(), lib, "gb", "Alpha (Europe)").unwrap().unwrap();
        let gdir = crate::meta::ensure_folder(ix, lib, &g).unwrap();
        let mut gj = crate::meta::read_game(&gdir);
        gj["details"] = json!({ "description": "A & B", "developer": "Dev Co" });
        crate::meta::write_game(ix, lib, &gdir, &gj).unwrap();
        std::fs::create_dir_all(gdir.join("media")).unwrap();
        std::fs::write(gdir.join("media").join("cover.png"), crate::scrape::tests::png(800, 600)).unwrap();
        let mut jpg = std::io::Cursor::new(Vec::new());
        ::image::RgbImage::new(200, 150).write_to(&mut jpg, ::image::ImageFormat::Jpeg).unwrap();
        std::fs::write(gdir.join("media").join("screenshot.jpg"), jpg.into_inner()).unwrap();
        std::fs::create_dir_all(gdir.join("docs")).unwrap();
        std::fs::write(gdir.join("docs").join("Alpha Manual.pdf"), b"%PDF-1.4\n%%EOF\n").unwrap();
        crate::meta::sync(ix, lib).unwrap();
        gdir
    }

    fn on(place: &str, dir: &Path) -> BTreeMap<String, PathBuf> {
        [(place.to_string(), dir.to_path_buf())].into()
    }

    #[test]
    fn a_collection_goes_to_a_card_in_knullis_and_muoss_layouts_and_follows_a_changed_layout() {
        let _turn = platforms::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (dir, lib, ix) = fixture("export-layouts");
        let c = collection(&lib, |_| {});
        alpha_with_media(&ix, &lib);
        let card = dir.join("Card");
        let carry = Carry { fields: vec!["description".into(), "developer".into()], media: vec!["cover".into(), "screenshot".into(), "video".into()], manuals: true, image_size: 640, ..Default::default() };
        let o = Options { collection: c.id.clone(), layout: "knulli".into(), places: on("card", &card), carry: carry.clone(), ..Default::default() };
        let p = run(&ix, &lib, &o);
        assert!(p.label.contains("Knulli"), "{}", p.label);
        let t = kept(&card);
        for f in ["roms/gb/Alpha (Europe).gb", "roms/gb/images/Alpha (Europe)-thumb.png", "roms/gb/images/Alpha (Europe)-image.jpg", "roms/gb/manuals/Alpha (Europe)-manual.pdf", "roms/gb/gamelist.xml", "roms/gb/Delta (World).gb"] {
            assert!(t.iter().any(|x| x == f), "{f} in {t:?}");
        }
        let gl = std::fs::read_to_string(card.join("roms/gb/gamelist.xml")).unwrap();
        for tag in ["<thumbnail>./images/Alpha (Europe)-thumb.png</thumbnail>", "<image>./images/Alpha (Europe)-image.jpg</image>", "<manual>./manuals/Alpha (Europe)-manual.pdf</manual>", "<desc>A &amp; B</desc>", "<developer>Dev Co</developer>"] {
            assert!(gl.contains(tag), "{tag} in {gl}");
        }
        let cover = ::image::open(card.join("roms/gb/images/Alpha (Europe)-thumb.png")).unwrap();
        assert_eq!((cover.width(), cover.height()), (640, 480));
        // the layout is copied and changed: Game Boy games in GameBoy/; Update moves them there
        let mut v = layout::to_json(&layout::get(Some(&lib), "knulli").unwrap());
        v["systems"]["gb"] = json!("GameBoy");
        let mine = layout::save(&lib, &v, false).unwrap();
        let eid = list(&lib)[0]["id"].as_str().unwrap().to_string();
        let mut again = options_of(&lib, &eid).unwrap();
        assert_eq!(again.layout, "knulli");
        assert_eq!(again.places["card"], card);
        again.layout = mine.id.clone();
        run(&ix, &lib, &again);
        let t = kept(&card);
        assert!(t.iter().any(|x| x == "roms/GameBoy/Alpha (Europe).gb") && t.iter().any(|x| x == "roms/GameBoy/images/Alpha (Europe)-thumb.png"), "{t:?}");
        assert!(!t.iter().any(|x| x.starts_with("roms/gb/") && x != "roms/gb/gamelist.xml"), "the old ones are set aside: {t:?}");
        assert_eq!(list(&lib)[0]["layout"], json!(mine.id));
        // muOS: the catalogue's box art (as PNG, even from a JPEG) and description, no gamelist
        let card2 = dir.join("Card2");
        let carry = Carry { fields: vec!["description".into()], media: vec!["cover".into(), "screenshot".into()], ..Default::default() };
        run(&ix, &lib, &Options { collection: c.id.clone(), layout: "muos".into(), places: on("card", &card2), carry, ..Default::default() });
        let t = kept(&card2);
        let cat = "MUOS/info/catalogue/Nintendo Game Boy";
        for f in ["ROMS/gb/Alpha (Europe).gb".to_string(), format!("{cat}/box/Alpha (Europe).png"), format!("{cat}/preview/Alpha (Europe).png"), format!("{cat}/text/Alpha (Europe).txt")] {
            assert!(t.contains(&f), "{f} in {t:?}");
        }
        assert!(!t.iter().any(|x| x.ends_with("gamelist.xml")), "{t:?}");
        let shot = std::fs::read(card2.join(format!("{cat}/preview/Alpha (Europe).png"))).unwrap();
        assert_eq!(::image::guess_format(&shot).unwrap(), ::image::ImageFormat::Png);
        assert_eq!(std::fs::read_to_string(card2.join(format!("{cat}/text/Alpha (Europe).txt"))).unwrap(), "A & B\n");
        // a layout that leaves the system out says so, and exports nothing of it
        let mut none = layout::to_json(&layout::get(Some(&lib), "muos").unwrap());
        none["systems"]["gb"] = Value::Null;
        let none = layout::save(&lib, &none, false).unwrap();
        let p = plan(&ix, &lib, &Options { collection: c.id.clone(), layout: none.id, places: on("card", &dir.join("Card3")), ..Default::default() }).unwrap();
        assert!(p.ops.is_empty() && p.left.iter().any(|l| l["why"].as_str().unwrap().contains("leaves this system out")), "{:?}", p.left);
        // the editor's preview of the collection: its own games' names, and where each would go
        let s = sample(&ix, &lib, Some(&c.id)).unwrap();
        assert_eq!(s["systems"][0]["system"], "gb");
        let pv = preview(&layout::get(Some(&lib), "knulli").unwrap(), &s);
        let paths: Vec<&str> = pv["items"].as_array().unwrap().iter().map(|i| i["path"].as_str().unwrap()).collect();
        assert!(paths.contains(&"card/roms/gb/images/Alpha (Europe)-thumb.png") && paths.contains(&"card/roms/gb/gamelist.xml"), "{paths:?}");
        let faint: Vec<&Value> = pv["items"].as_array().unwrap().iter().filter(|i| i["at"] == "media.video").collect();
        assert!(faint.iter().all(|i| i["faint"] == true), "no game has a video: {faint:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_editors_preview_shows_a_stand_in_and_files_that_would_clash() {
        let lib_less = |id: &str| layout::built_in().into_iter().find(|l| l.id == id).unwrap();
        let s = {
            // no library: the stand-in
            let dir = crate::library::tests::temp_dir("preview-stand-in");
            let lib = Library::open(dir.join("lib")).unwrap();
            let ix = Index::open(&dir.join("index.sqlite"), &lib.id().unwrap()).unwrap();
            let s = sample(&ix, &lib, None).unwrap();
            let _ = std::fs::remove_dir_all(&dir);
            s
        };
        let pv = preview(&lib_less("muos"), &s);
        let paths: Vec<&str> = pv["items"].as_array().unwrap().iter().map(|i| i["path"].as_str().unwrap()).collect();
        assert!(pv["stand_in"] == true && paths.contains(&"card/MUOS/info/catalogue/Nintendo Game Boy/box/Example Game (World).png") && paths.contains(&"card/MUOS/bios/scph5501.bin"), "{paths:?}");
        let mut l = lib_less("knulli");
        l.media.get_mut("screenshot").unwrap().path = "{card}/roms/{folder}/images/{stem}-thumb.{ext}".into();
        let pv = preview(&l, &s);
        assert!(pv["items"].as_array().unwrap().iter().any(|i| i["clash"] == true));
        assert!(pv["problems"].as_array().unwrap().iter().any(|p| p["message"].as_str().unwrap().contains("same path")));
    }

    #[test]
    fn bios_files_are_audited_gathered_and_exported_with_the_games() {
        let _turn = platforms::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (dir, lib, ix) = fixture("export-bios");
        let c = collection(&lib, |_| {});
        let gb_bios: Vec<u8> = (0..256u32).map(|i| (i * 7 % 256) as u8).collect();
        let dc_boot: Vec<u8> = (0..512u32).map(|i| (i * 3 % 256) as u8).collect();
        std::fs::write(dir.join("Roms").join("gb_bios.bin"), &gb_bios).unwrap();
        // a file named as Dreamcast's BIOS, but not it
        std::fs::write(dir.join("Roms").join("dc_boot.bin"), b"not the bios").unwrap();
        crate::scan::scan(&ix, &lib, &crate::scan::Options::default(), &AtomicBool::new(false), &mut |_| {}).unwrap();
        let line = |name: &str, b: &[u8]| {
            let h = hash::hash_reader(&mut &b[..], b.len() as u64, name, &AtomicBool::new(false)).unwrap();
            format!("\trom ( name {name} size {} crc {} sha1 {} )\n", b.len(), h.crc32, h.sha1.unwrap())
        };
        let text = format!(
            "clrmamepro (\n\tname \"System\"\n\tdescription \"System\"\n\tversion v1.19.0\n\tauthor \"libretro\"\n)\n\ngame (\n\tname \"System\"\n\tcomment \"System\"\n\n\tcomment \"Nintendo - Gameboy\"\n{}{}\n\tcomment \"Sega - Dreamcast\"\n{})\n",
            line("gb_bios.bin", &gb_bios), line("sgb_bios.bin", &[1, 2, 3]), line("dc/dc_boot.bin", &dc_boot)
        );
        std::fs::write(dir.join("System.dat"), text).unwrap();
        let r = crate::dat::import(&ix, &lib, &dir.join("System.dat"), &serde_json::Map::new()).unwrap();
        let crate::dat::Imported::Loaded(v) = &r[0] else { panic!("no platform for System.dat") };
        assert_eq!((v["platform"].as_str(), v["games"].as_u64()), (Some("bios"), Some(2)), "{v}");
        let s = crate::bios::summary(&ix, &lib).unwrap();
        let sys = s["systems"].as_array().unwrap();
        assert_eq!(sys[0]["name"], "Nintendo - Gameboy", "the library's own systems first: {s}");
        assert_eq!((sys[0]["have"].as_i64(), sys[0]["missing"].as_i64(), sys[0]["yours"].as_bool()), (Some(1), Some(1), Some(true)), "{s}");
        assert_eq!((sys[1]["wrong"].as_i64(), sys[1]["yours"].as_bool()), (Some(1), Some(false)), "{s}");
        assert!(crate::bios::report(&ix, &lib).unwrap().contains("sgb_bios.bin"));
        // gathered into the library's BIOS folder, which becomes a folder with games
        let gp = crate::bios::gather_plan(&ix, &lib).unwrap();
        assert!(gp.new_root.is_some());
        let r = organise::run(&ix, &lib, &gp, &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(r["failed"], json!([]), "{r}");
        assert!(lib.root().join("BIOS").join("Nintendo - Gameboy").join("gb_bios.bin").is_file(), "{:?}", tree(&lib.root().join("BIOS")));
        // exported with the collection, into Knulli's bios folder
        let card = dir.join("Card");
        run(&ix, &lib, &Options { collection: c.id.clone(), layout: "knulli".into(), places: on("card", &card), carry: Carry { bios: true, ..Default::default() }, ..Default::default() });
        assert_eq!(std::fs::read(card.join("bios").join("gb_bios.bin")).unwrap(), gb_bios);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
