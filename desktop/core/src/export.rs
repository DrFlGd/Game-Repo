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
    /// esde | folder
    pub target: String,
    /// ES-DE's ROMs folder, or the folder
    pub out: PathBuf,
    /// ES-DE's own folder (its gamelists and collections), when given
    pub esde: Option<PathBuf>,
    pub always_copy: bool,
    /// Gamelist entries named by the game's title, not its full name
    pub short_names: bool,
    /// The export updated (none: a new one)
    pub export_id: Option<String>,
    /// What it carries besides the games (D30, D34)
    pub carry: Carry,
}

/// What an export carries besides the games' files: the details in ES-DE's gamelist,
/// the pictures (and video), manuals and other documents, and how big pictures may be.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Carry {
    /// meta::FIELDS
    pub fields: Vec<String>,
    /// meta::MEDIA
    pub media: Vec<String>,
    pub manuals: bool,
    /// Every document (a plain folder only)
    pub docs: bool,
    /// The longest side of a picture, in pixels (0: as they are)
    pub image_size: u32,
}

impl Carry {
    fn any(&self) -> bool {
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
    Ok(Options {
        collection: e["collection"].as_str().unwrap_or("").into(),
        target: e["target"].as_str().unwrap_or("folder").into(),
        out: PathBuf::from(e["out"].as_str().unwrap_or("")),
        esde: e["esde"].as_str().filter(|s| !s.is_empty()).map(PathBuf::from),
        always_copy: e["always_copy"].as_bool().unwrap_or(false),
        short_names: e["short_names"].as_bool().unwrap_or(true),
        export_id: Some(id.into()),
        carry: serde_json::from_value(e["carry"].clone()).unwrap_or_default(),
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
    let text = std::fs::read_to_string(es.join("settings").join("es_settings.xml")).unwrap_or_default();
    let set = text.lines().find(|l| l.contains("name=\"ROMDirectory\"")).and_then(|l| l.split("value=\"").nth(1)).and_then(|v| v.split('"').next()).map(xml_unescape).unwrap_or_default();
    let home = config::home_dir();
    match set.trim() {
        "" => home.join("ROMs"),
        s if s.starts_with('~') => home.join(s.trim_start_matches('~').trim_start_matches(['/', '\\'])),
        s if s.starts_with("%ESPATH%") => es.join(s.trim_start_matches("%ESPATH%").trim_start_matches(['/', '\\'])),
        s => PathBuf::from(s),
    }
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
    if !["esde", "folder"].contains(&o.target.as_str()) {
        bail!("There's no export to {}.", o.target);
    }
    let out = &o.out;
    if out.as_os_str().is_empty() || !out.is_absolute() {
        bail!("Choose the folder the games go to.");
    }
    if key(out).starts_with(&key(&lib.root().join(APP_DIR))) {
        bail!("The library's own _library folder can't hold an export.");
    }
    for r in roots::list(lib) {
        let (rk, ok) = (key(&r.dir), key(out));
        let (rk, ok) = (rk.trim_end_matches('/'), ok.trim_end_matches('/'));
        if ok == rk || ok.starts_with(&format!("{rk}/")) {
            bail!("{} is in a folder with games ({}): its copies would be counted as duplicates. Choose a folder outside it.", out.display(), r.dir.display());
        }
        if rk.starts_with(&format!("{ok}/")) {
            bail!("{} holds a folder with games ({}): its copies would be counted as duplicates. Choose another folder.", out.display(), r.dir.display());
        }
    }
    let esde = if o.target == "esde" { o.esde.clone() } else { None };
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
    let target_name = if o.target == "esde" { format!("ES-DE ({})", out.display()) } else { out.display().to_string() };
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
    // ES-DE's entries for each system: (the file or folder in the system's folder, game, title)
    let mut entries: BTreeMap<String, Vec<(String, String, String)>> = BTreeMap::new();
    // each system's folder, and its games' DAT ids by name
    let mut folders: BTreeMap<String, PathBuf> = BTreeMap::new();
    let mut ids: HashMap<(String, String), i64> = HashMap::new();
    for s in &w.systems {
        if s.games.is_empty() {
            continue;
        }
        let folder = if o.target == "esde" {
            out.join(&s.platform)
        } else {
            let group = platforms::by_id(&s.platform).and_then(|p| p.dat_names.first().copied()).unwrap_or(&s.platform);
            out.join(safe_name(group))
        };
        let names: Vec<String> = s.games.iter().map(|g| g.name.clone()).collect();
        folders.insert(s.platform.clone(), folder.clone());
        for g in &s.games {
            ids.insert((s.platform.clone(), g.name.clone()), g.id);
        }
        let sub = organise::plan(ix, lib, &organise::Options {
            dat_id: s.dat_id, mode: "copy".into(), out: Some(folder.clone()), format: "keep".into(),
            always_copy: o.always_copy, games: Some(names), esde: o.target == "esde", known: known.clone(),
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
    // what it carries: each game's pictures, manual and documents, and its details for the gamelist
    let media_root = esde.as_ref().map(|e| media_dir(e));
    let mut details: HashMap<String, Vec<(String, String)>> = HashMap::new();
    if o.carry.any() {
        for (system, list) in &entries {
            let Some(folder) = folders.get(system) else { continue };
            for (entry, game, _) in list {
                let Some(id) = ids.get(&(system.clone(), game.clone())) else { continue };
                let g = crate::meta::dat_game(conn, *id)?;
                let Some(dir) = crate::meta::folder_of(conn, &g)?.and_then(|f| lib.resolve(&f).ok()) else { continue };
                let is_dir = folder.join(entry).is_dir() || plan.ops.iter().any(|op| rel_to(folder, Path::new(&op.to)).is_some_and(|r| r.starts_with(&format!("{entry}/"))));
                // ES-DE names a game's pictures after its file without the extension (a folder keeps its whole name)
                let stem = if is_dir { entry.clone() } else { Path::new(entry).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| entry.clone()) };
                let gj = crate::meta::read_game(&dir);
                if o.target == "esde" && !o.carry.fields.is_empty() {
                    details.insert(entry.to_lowercase(), gamelist_fields(&gj["details"], &o.carry.fields));
                }
                let mut files: Vec<(PathBuf, PathBuf, &str)> = vec![];
                for kind in &o.carry.media {
                    let Some(src) = media_file(&dir.join("media"), kind) else { continue };
                    let ext = platforms::ext_of(&src.to_string_lossy());
                    // made smaller, a picture is written as JPEG or PNG
                    let ext = if o.carry.image_size > 0 && ["webp", "gif", "bmp"].contains(&ext.as_str()) { "png".to_string() } else { ext };
                    let to_dir = match (&media_root, o.target.as_str()) {
                        (Some(m), "esde") => m.join(system).join(crate::meta::MEDIA.iter().find(|x| x.0 == kind).map(|x| x.2).unwrap_or("custom")),
                        (None, "esde") => continue,
                        _ => folder.join(&stem),
                    };
                    let name = if o.target == "esde" { stem.clone() } else { kind.clone() };
                    files.push((src, to_dir.join(format!("{name}.{ext}")), "media"));
                }
                if o.carry.manuals {
                    if let Some(src) = manual_of(conn, lib, &dir) {
                        let to = match (&media_root, o.target.as_str()) {
                            (Some(m), "esde") => m.join(system).join("manuals").join(format!("{stem}.pdf")),
                            (None, "esde") => PathBuf::new(),
                            _ => folder.join(&stem).join(src.file_name().unwrap_or_default()),
                        };
                        if !to.as_os_str().is_empty() {
                            files.push((src, to, "manual"));
                        }
                    }
                }
                if o.carry.docs && o.target == "folder" {
                    let rel = lib.relative(&dir).unwrap_or_default();
                    for d in crate::docs::of_folder(conn, &rel)? {
                        let Ok(src) = lib.resolve(d["path"].as_str().unwrap_or("")) else { continue };
                        let to = folder.join(&stem).join("docs").join(src.file_name().unwrap_or_default());
                        if !files.iter().any(|(_, t, _)| key(t) == key(&to)) && !files.iter().any(|(s2, _, _)| key(s2) == key(&src)) {
                            files.push((src, to, "doc"));
                        }
                    }
                }
                for (src, to, what) in files {
                    let top = if o.target == "esde" && rel_to(out, &to).is_none() { es_top(&esde, &to).unwrap_or_else(|| out.clone()) } else { out.clone() };
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
                            continue;
                        }
                        let old = outside(&top, &to);
                        plan.ops.push(Op { op: "aside".into(), game: game.clone(), to: old.path.clone(), size: 0, from: vec![old], names: vec![], aside: vec![], note: None });
                    }
                    let mut from = outside(&top, &src);
                    from.root = format!("dir:{}", src.parent().unwrap_or(Path::new(".")).display());
                    from.rel = src.file_name().unwrap_or_default().to_string_lossy().into_owned();
                    let image = what == "media" && o.carry.image_size > 0 && ["png", "jpg", "jpeg", "webp", "gif", "bmp"].contains(&platforms::ext_of(&src.to_string_lossy()).as_str());
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
            }
        }
    }
    // files placed before that the collection no longer has are set aside (at the top of the export folder)
    let wanted_keys: HashSet<String> = wanted.iter().filter_map(|f| f["path"].as_str()).map(|p| key(Path::new(p))).collect();
    let mut drop = vec![];
    for f in &recorded {
        let Some(p) = f["path"].as_str().map(PathBuf::from) else { continue };
        let top = if rel_to(out, &p).is_some() { Some(out.clone()) } else { es_top(&esde, &p) };
        let Some(top) = top else { continue };
        if wanted_keys.contains(&key(&p)) || !p.is_file() {
            continue;
        }
        let src = outside(&top, &p);
        plan.ops.push(Op { op: "aside".into(), game: f["game"].as_str().unwrap_or("").into(), to: src.path.clone(), size: 0, from: vec![src], names: vec![], aside: vec![], note: None });
        drop.push(f.clone());
    }
    // ES-DE's gamelists (entries added for games that have none) and the custom collection
    if let Some(es) = &esde {
        for (system, list) in &entries {
            let at = es.join("gamelists").join(system).join("gamelist.xml");
            let old = std::fs::read_to_string(&at).ok();
            if let Some(text) = merge_gamelist(old.as_deref(), list, &details) {
                plan.ops.push(text_op("", &at, es, text, "ES-DE's list of the system's games: entries are added for the games exported, with the details chosen; the rest of it is kept."));
            }
        }
        let roms = rom_dir(es);
        let in_roms = key(out).trim_end_matches('/') == key(&roms).trim_end_matches('/');
        let mut lines = vec![];
        for (system, list) in &entries {
            for (entry, _, _) in list {
                lines.push(if in_roms { format!("%ROMPATH%/{system}/{entry}") } else { out.join(system).join(entry).to_string_lossy().replace('\\', "/") });
            }
        }
        let at = es.join("collections").join(format!("custom-{}.cfg", safe_name(&c.name)));
        let text = lines.iter().map(|l| format!("{l}\n")).collect::<String>();
        if !lines.is_empty() && std::fs::read_to_string(&at).ok().as_deref() != Some(text.as_str()) {
            plan.ops.push(text_op("", &at, es, text, "The collection, as ES-DE's custom collection."));
        }
    }
    plan.space = organise::space_needed(&plan.ops);
    plan.export = Some(json!({
        "id": export_id, "collection": c.id, "name": c.name, "target": o.target, "out": out.display().to_string(),
        "esde": esde.as_ref().map(|e| e.display().to_string()), "always_copy": o.always_copy, "short_names": o.short_names, "carry": o.carry,
        "first": first, "files": wanted, "drop": drop,
    }));
    let _ = conn;
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

/// ES-DE's gamelist fields from a game's details: (tag, value), for those chosen.
fn gamelist_fields(d: &Value, chosen: &[String]) -> Vec<(String, String)> {
    let mut v = vec![];
    let want = |k: &str| chosen.iter().any(|c| c == k);
    let s = |k: &str| d[k].as_str().map(str::trim).filter(|x| !x.is_empty()).map(String::from);
    if want("description") {
        if let Some(x) = s("description") {
            v.push(("desc".into(), x));
        }
    }
    if want("rating") {
        if let Some(r) = d["rating"].as_f64().filter(|r| *r > 0.0) {
            v.push(("rating".into(), format!("{:.2}", r.min(1.0))));
        }
    }
    if want("released") {
        if let Some(x) = s("released").and_then(|r| esde_date(&r)) {
            v.push(("releasedate".into(), x));
        }
    }
    for (k, tag) in [("developer", "developer"), ("publisher", "publisher"), ("players", "players")] {
        if want(k) {
            if let Some(x) = s(k).or_else(|| d[k].as_u64().map(|n| n.to_string())) {
                v.push((tag.into(), x));
            }
        }
    }
    if want("genres") {
        let g = match &d["genres"] {
            Value::Array(a) => a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "),
            Value::String(x) => x.clone(),
            _ => String::new(),
        };
        if !g.is_empty() {
            v.push(("genre".into(), g));
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

/// ES-DE's media folder: its setting (`MediaDirectory`), else `<ES-DE>/downloaded_media`.
fn media_dir(es: &Path) -> PathBuf {
    let text = std::fs::read_to_string(es.join("settings").join("es_settings.xml")).unwrap_or_default();
    let set = text.lines().find(|l| l.contains("name=\"MediaDirectory\"")).and_then(|l| l.split("value=\"").nth(1)).and_then(|v| v.split('"').next()).map(xml_unescape).unwrap_or_default();
    match set.trim() {
        "" => es.join("downloaded_media"),
        s if s.starts_with('~') => config::home_dir().join(s.trim_start_matches('~').trim_start_matches(['/', '\\'])),
        s if s.starts_with("%ESPATH%") => es.join(s.trim_start_matches("%ESPATH%").trim_start_matches(['/', '\\'])),
        s => PathBuf::from(s),
    }
}

/// The top folder a file outside the export folder is under: ES-DE's own folder, or its media folder.
fn es_top(esde: &Option<PathBuf>, p: &Path) -> Option<PathBuf> {
    let es = esde.as_ref()?;
    if rel_to(es, p).is_some() {
        return Some(es.clone());
    }
    let m = media_dir(es);
    rel_to(&m, p).is_some().then_some(m)
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
    // folders left empty by files set aside go
    let out = PathBuf::from(e["out"].as_str().unwrap_or(""));
    for f in e["drop"].as_array().into_iter().flatten() {
        let mut at = f["path"].as_str().map(Path::new).and_then(Path::parent);
        while let Some(d) = at {
            if rel_to(&out, d).is_none() || std::fs::remove_dir(d).is_err() {
                break;
            }
            at = d.parent();
        }
    }
    let mut v = json!({
        "id": id, "collection": e["collection"], "name": e["name"], "target": e["target"], "out": e["out"], "esde": e["esde"],
        "always_copy": e["always_copy"], "short_names": e["short_names"], "carry": e["carry"],
        "created": old["created"].as_str().map(String::from).unwrap_or_else(now), "updated": now(), "change": plan.id,
        "files": files.into_values().collect::<Vec<_>>(),
    });
    if let Some(checked) = old.get("checked") {
        v["checked"] = checked.clone();
    }
    config::write_json(&path, &v)
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
        let carry = Carry { fields: vec!["description".into(), "developer".into(), "released".into(), "rating".into()], media: vec!["cover".into(), "screenshot".into()], manuals: true, docs: false, image_size: 320 };
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
}
