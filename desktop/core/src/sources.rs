//! Each source's answer kept apart, the pictures' variants, and deleting metadata
//! (docs/PLAN.md, "Phase 6b design", D46–D48).
//!
//! A game's folder keeps what each source said in `sources/<source>.json` (its id, title,
//! year and the details taken), and `game.json`'s details, title and year are made from
//! those files field by field: the game's own pick (`pinned`), else Settings › Metadata's
//! *Details from* (library.json `details_from`), else the scrape's order of sources. A
//! hand edit wins over all of them (D44).
//!
//! A kind of picture or video has a variant from each source that has one (one each, Jerred
//! 2026-10-17): the chosen one at `media/<kind>.<ext>`, where the grid, the game page and
//! exports look (an export carries only it, under the usual name), the others in
//! `media/<kind>/<source>.<ext>`. `game.json` `chosen` names the chosen one's source, and
//! `pinned_media` the ones the user picked, kept over later scrapes.

use crate::config::{read_json_object, write_json};
use crate::index::Index;
use crate::library::{now, Library};
use crate::meta::{self, FIELDS, MEDIA};
use crate::organise::{new_id, Op, Plan, Src};
use crate::pcgames::Match;
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

/// The folder in a game's folder with each source's answer.
pub const DIR: &str = "sources";

/// What a folder's details were split into when nothing said which source gave them.
const EARLIER: &str = "earlier";

/// What a source is called, an earlier scrape's and the user's own too.
pub fn source_name(s: &str) -> &str {
    match s {
        EARLIER => "An earlier scrape",
        "you" | "yours" => "You",
        _ => crate::identify::source_name(s),
    }
}

fn valid_source(s: &str) -> bool {
    !s.is_empty() && s.len() <= 40 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The sources' answers kept in a game's folder, by source.
pub fn answers(dir: &Path) -> BTreeMap<String, Value> {
    let mut m = BTreeMap::new();
    for e in std::fs::read_dir(dir.join(DIR)).into_iter().flatten().flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        let Some(s) = n.strip_suffix(".json").filter(|s| valid_source(s)) else { continue };
        let v = read_json_object(&e.path());
        if v.as_object().is_some_and(|o| !o.is_empty()) {
            m.insert(s.to_string(), v);
        }
    }
    m
}

fn write_answer(dir: &Path, source: &str, v: &Value) -> Result<()> {
    if !valid_source(source) {
        bail!("{source} isn't a source.");
    }
    std::fs::create_dir_all(dir.join(DIR))?;
    write_json(&dir.join(DIR).join(format!("{source}.json")), v)
}

fn empty(x: &Value) -> bool {
    x.is_null() || x == "" || x.as_array().is_some_and(|a| a.is_empty())
}

fn edited(v: &Value) -> Vec<String> {
    v["edited"].as_array().into_iter().flatten().filter_map(|e| e.as_str().map(String::from)).collect()
}

/// Keep a source's answer in a game's folder: *Replace* writes the fields taken again
/// (and its id, title and year), *Fill in* adds only what its file lacks.
pub fn keep(dir: &Path, m: &Match, fields: &[String], replace: bool) -> Result<()> {
    let mut a = answers(dir).remove(&m.source).unwrap_or_else(|| json!({}));
    a["source"] = json!(m.source);
    if replace || empty(&a["id"]) {
        a["id"] = json!(m.id);
    }
    if !m.title.is_empty() && (replace || empty(&a["title"])) {
        a["title"] = json!(m.title);
    }
    if m.year.is_some() && (replace || empty(&a["year"])) {
        a["year"] = json!(m.year);
    }
    if !a["details"].is_object() {
        a["details"] = json!({});
    }
    for f in fields {
        let x = m.details.get(f).cloned().unwrap_or(Value::Null);
        let has = !empty(&a["details"][f]);
        if replace && empty(&x) {
            a["details"].as_object_mut().unwrap().remove(f);
        } else if !empty(&x) && (replace || !has) {
            a["details"][f] = x;
        }
    }
    a["when"] = json!(now());
    write_answer(dir, &m.source, &a)
}

/// A folder scraped before D46 has its details in `game.json` alone: they're split into
/// the sources' files by `scraped`, which says where each came from (what it doesn't
/// say goes to the one source in `ids`, else to "an earlier scrape").
fn split(dir: &Path, v: &Value) -> Result<()> {
    if v["by_source"] == true || dir.join(DIR).is_dir() {
        return Ok(());
    }
    let ed = edited(v);
    let ids = v["ids"].as_object().cloned().unwrap_or_default();
    let fallback = if ids.len() == 1 { ids.keys().next().cloned().unwrap_or_default() } else { EARLIER.to_string() };
    let from = |k: &str| v["scraped"][k].as_str().filter(|s| *s != "you" && valid_source(s)).map(String::from);
    let mut by: BTreeMap<String, Value> = BTreeMap::new();
    let mut put = |s: String, k: &str, x: Value, detail: bool| {
        let e = by.entry(s.clone()).or_insert_with(|| json!({ "source": s, "details": {} }));
        if detail {
            e["details"][k] = x;
        } else {
            e[k] = x;
        }
    };
    for (k, _) in FIELDS {
        let x = &v["details"][*k];
        if !empty(x) && !ed.iter().any(|e| e == k) {
            put(from(k).unwrap_or_else(|| fallback.clone()), k, x.clone(), true);
        }
    }
    if let Some(s) = from("title").filter(|_| !ed.iter().any(|e| e == "title")) {
        put(s, "title", v["title"].clone(), false);
    }
    if v["year"].is_number() && !ed.iter().any(|e| e == "year") {
        put(from("year").or(from("title")).unwrap_or_else(|| fallback.clone()), "year", v["year"].clone(), false);
    }
    for (s, id) in &ids {
        if valid_source(s) && !id.is_null() {
            put(s.clone(), "id", id.clone(), false);
        }
    }
    let when = v["scraped"]["when"].clone();
    for (s, mut a) in by {
        if !when.is_null() {
            a["when"] = when.clone();
        }
        write_answer(dir, &s, &a)?;
    }
    Ok(())
}

/// What decides which source a field comes from: Settings › Metadata's choice for it, and
/// the scrape's order of sources.
pub struct Rules {
    pub order: Vec<String>,
    pub from: Map<String, Value>,
}

impl Rules {
    pub fn of(lib: &Library) -> Rules {
        Rules { order: crate::scrape::defaults(lib).sources, from: details_from(lib) }
    }
}

/// Settings › Metadata's *Details from*: field → source.
pub fn details_from(lib: &Library) -> Map<String, Value> {
    lib.meta()["details_from"].as_object().cloned().unwrap_or_default()
}

/// The sources a field is looked for in, in turn.
fn ranked<'a>(field: &str, pinned: &Value, rules: &Rules, have: &'a BTreeMap<String, Value>) -> Vec<&'a str> {
    let mut v: Vec<&str> = vec![];
    let mut add = |s: &str| {
        if let Some((k, _)) = have.get_key_value(s) {
            if !v.contains(&k.as_str()) {
                v.push(k.as_str());
            }
        }
    };
    if let Some(s) = pinned[field].as_str() {
        add(s);
    }
    if let Some(s) = rules.from.get(field).and_then(Value::as_str) {
        add(s);
    }
    for s in &rules.order {
        add(s);
    }
    for s in have.keys() {
        if s != EARLIER {
            add(s);
        }
    }
    add(EARLIER);
    v
}

fn value_of<'a>(a: &'a Value, field: &str) -> &'a Value {
    match field {
        "title" | "year" => &a[field],
        _ => &a["details"][field],
    }
}

/// The title a game has when no source gave one: its DAT name's, else its file's.
fn plain_title(v: &Value) -> Option<String> {
    if let Some(n) = v["name"].as_str().filter(|n| !n.is_empty()) {
        return Some(crate::dat::parse_name(n).title);
    }
    v["files"][0]["name"].as_str().map(|n| Path::new(n).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default())
}

/// Make a game's details, title, year and ids from its sources' answers (D46).
pub fn make(v: &mut Value, dir: &Path, rules: &Rules) {
    let have = answers(dir);
    let ed = edited(v);
    if !v["details"].is_object() {
        v["details"] = json!({});
    }
    if !v["scraped"].is_object() {
        v["scraped"] = json!({});
    }
    let pinned = v["pinned"].clone();
    let fields = FIELDS.iter().map(|(k, _)| *k).chain(["title", "year"]);
    for k in fields {
        if ed.iter().any(|e| e == k) {
            v["scraped"][k] = json!("you");
            continue;
        }
        let pick = ranked(k, &pinned, rules, &have).into_iter().find(|s| !empty(value_of(&have[*s], k)));
        let was_source = v["scraped"][k].as_str().is_some_and(|s| s != "you");
        match pick {
            Some(s) => {
                let x = value_of(&have[s], k).clone();
                match k {
                    "title" | "year" => v[k] = x,
                    _ => v["details"][k] = x,
                }
                v["scraped"][k] = json!(s);
            }
            None => {
                match k {
                    // a title no source gave stays (the DAT's)
                    "title" if was_source => match plain_title(v) {
                        Some(t) => v["title"] = json!(t),
                        None => {
                            v.as_object_mut().unwrap().remove("title");
                        }
                    },
                    "title" => {}
                    "year" => {
                        v.as_object_mut().unwrap().remove("year");
                    }
                    _ => {
                        v["details"].as_object_mut().unwrap().remove(k);
                    }
                }
                v["scraped"].as_object_mut().unwrap().remove(k);
            }
        }
    }
    let ids: Map<String, Value> = have.iter().filter(|(_, a)| !empty(&a["id"])).map(|(s, a)| (s.clone(), a["id"].clone())).collect();
    v["ids"] = Value::Object(ids);
    match have.values().filter_map(|a| a["when"].as_str()).max() {
        Some(w) => v["scraped"]["when"] = json!(w),
        None => {
            v["scraped"].as_object_mut().unwrap().remove("when");
        }
    }
    // what was chosen and picked stays when its files are gone, so an Undo of a deletion
    // brings it all back as it was
    v["by_source"] = json!(true);
}

/// Read a game's folder, split it if it's from before, make its details again and write it.
pub fn remake(ix: &Index, lib: &Library, dir: &Path, rules: &Rules) -> Result<()> {
    let mut v = meta::read_game(dir);
    if v.as_object().is_none_or(|o| o.is_empty()) || v["platform"] == "pc" {
        return Ok(());
    }
    split(dir, &v)?;
    make(&mut v, dir, rules);
    meta::write_game(ix, lib, dir, &v)
}

/// Before a scrape writes a game's answers: a folder from before is split first.
pub fn prepare(dir: &Path, v: &Value) -> Result<()> {
    split(dir, v)
}

/// Every game folder (each system's, not PC games'), skipping set-aside folders.
fn game_dirs(lib: &Library, platform: Option<&str>) -> Vec<PathBuf> {
    let mut v = vec![];
    for sys in std::fs::read_dir(meta::games_root(lib)).into_iter().flatten().flatten() {
        let name = sys.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || name == "pc" || !sys.path().is_dir() || platform.is_some_and(|p| p != name) {
            continue;
        }
        for g in std::fs::read_dir(sys.path()).into_iter().flatten().flatten() {
            if !g.file_name().to_string_lossy().starts_with('.') && g.path().join("game.json").is_file() {
                v.push(g.path());
            }
        }
    }
    v.sort();
    v
}

/// Make every game's details again (Settings › Metadata's *Details from* changed), from
/// the answers kept, without asking the sources.
pub fn remake_all(ix: &Index, lib: &Library, cancel: &std::sync::atomic::AtomicBool, progress: &mut dyn FnMut(Value)) -> Result<usize> {
    lib.writable()?;
    let rules = Rules::of(lib);
    let dirs = game_dirs(lib, None);
    let tx = ix.conn().unchecked_transaction()?;
    let mut n = 0;
    for (i, d) in dirs.iter().enumerate() {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        if i % 50 == 0 {
            progress(json!({ "item": i + 1, "items": dirs.len(), "name": d.file_name().map(|n| n.to_string_lossy().into_owned()) }));
        }
        remake(ix, lib, d, &rules)?;
        n += 1;
    }
    tx.commit()?;
    Ok(n)
}

/// Choose the source a field comes from (Settings › Metadata's *Details from*): `None` is
/// the scrape's order. Every game's details are made again.
pub fn set_details_from(lib: &Library, field: &str, source: Option<&str>) -> Result<()> {
    if field != "title" && field != "year" && !FIELDS.iter().any(|(k, _)| *k == field) {
        bail!("There's no detail called {field}.");
    }
    let mut m = details_from(lib);
    match source.filter(|s| !s.is_empty()) {
        Some(s) if valid_source(s) => {
            m.insert(field.into(), json!(s));
        }
        Some(s) => bail!("{s} isn't a source."),
        None => {
            m.remove(field);
        }
    }
    lib.update_meta(json!({ "details_from": m }))?;
    Ok(())
}

/// Take one of a game's details from this source (the game page's *Use this*); `None`
/// goes back to the usual order.
pub fn pin(ix: &Index, lib: &Library, dir: &Path, field: &str, source: Option<&str>) -> Result<()> {
    lib.writable()?;
    if field != "title" && field != "year" && !FIELDS.iter().any(|(k, _)| *k == field) {
        bail!("There's no detail called {field}.");
    }
    let mut v = meta::read_game(dir);
    split(dir, &v)?;
    if !v["pinned"].is_object() {
        v["pinned"] = json!({});
    }
    match source {
        Some(s) => {
            if !answers(dir).get(s).is_some_and(|a| !empty(value_of(a, field))) {
                bail!("{} didn't give this game's {field}.", source_name(s));
            }
            v["pinned"][field] = json!(s);
            // a pick over a hand edit means the source's value
            if let Some(a) = v["edited"].as_array_mut() {
                a.retain(|e| e != field);
            }
        }
        None => {
            v["pinned"].as_object_mut().unwrap().remove(field);
        }
    }
    make(&mut v, dir, &Rules::of(lib));
    meta::write_game(ix, lib, dir, &v)
}

// ------------------------------------------------------------ variants (D47)

/// A kind's chosen file: `media/<kind>.<ext>`.
pub fn flat(dir: &Path, kind: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir.join("media")).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_file()).find(|p| p.file_name().and_then(|n| n.to_str()).and_then(|n| n.split_once('.')).is_some_and(|(k, _)| k == kind))
}

/// The other variants of a kind: (name, file), by name.
fn others(dir: &Path, kind: &str) -> Vec<(String, PathBuf)> {
    let mut v: Vec<(String, PathBuf)> = std::fs::read_dir(dir.join("media").join(kind))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_file())
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            let (stem, _) = n.rsplit_once('.')?;
            (!stem.is_empty() && !n.ends_with(".gamerepo-tmp")).then(|| (stem.to_string(), e.path()))
        })
        .collect();
    v.sort();
    v
}

/// The chosen variant's source: `chosen`, else (a folder from before) the one it came
/// from, else the user's own ("yours").
pub fn chosen_name(v: &Value, kind: &str) -> String {
    if let Some(n) = v["chosen"][kind].as_str() {
        return n.to_string();
    }
    match v["scraped"]["media"][kind].as_str() {
        Some(s) if s != "you" && valid_source(s) => s.to_string(),
        _ => "yours".into(),
    }
}

/// The other variants' sources.
fn variant_names(dir: &Path, kind: &str) -> Vec<String> {
    others(dir, kind).into_iter().map(|(n, _)| n).collect()
}

/// Every variant's source a kind has, the chosen one first.
pub fn names(dir: &Path, v: &Value, kind: &str) -> Vec<String> {
    let mut n = vec![];
    if flat(dir, kind).is_some() {
        n.push(chosen_name(v, kind));
    }
    n.extend(variant_names(dir, kind));
    n
}

/// A game's variants of each kind it has, as its page shows them.
pub fn variants(lib: &Library, dir: &Path, v: &Value) -> Map<String, Value> {
    let mut out = Map::new();
    for (kind, _, _) in MEDIA {
        let entry = |name: &str, p: &Path, chosen: bool| {
            json!({
                "name": name, "source_name": source_name(name), "chosen": chosen, "pinned": v["pinned_media"][*kind] == name,
                "path": lib.relative(p), "size": std::fs::metadata(p).map(|m| m.len()).unwrap_or(0),
            })
        };
        let mut list = vec![];
        if let Some(f) = flat(dir, kind) {
            list.push(entry(&chosen_name(v, kind), &f, true));
        }
        for (n, p) in others(dir, kind) {
            list.push(entry(&n, &p, false));
        }
        if !list.is_empty() {
            out.insert(kind.to_string(), json!(list));
        }
    }
    out
}

/// Keep the picture a source sent: over the chosen one if it's that source's, else
/// beside the others (in place of that source's earlier one).
pub fn store(dir: &Path, v: &Value, kind: &str, name: &str, ext: &str, bytes: &[u8]) -> Result<()> {
    if flat(dir, kind).is_some() && chosen_name(v, kind) == name {
        crate::identify::save_media(&dir.join("media"), kind, ext, bytes)?;
        return Ok(());
    }
    let sub = dir.join("media").join(kind);
    std::fs::create_dir_all(&sub)?;
    for (n, p) in others(dir, kind) {
        if n == name {
            let _ = std::fs::remove_file(p);
        }
    }
    crate::config::write_atomic(&sub.join(format!("{name}.{ext}")), bytes)?;
    Ok(())
}

/// Remove a source's variant (a scrape found the source no longer has one).
pub fn remove(dir: &Path, v: &mut Value, kind: &str, name: &str) {
    if chosen_name(v, kind) == name {
        if let Some(f) = flat(dir, kind) {
            let _ = std::fs::remove_file(f);
            if let Some(o) = v["chosen"].as_object_mut() {
                o.remove(kind);
            }
        }
    }
    for (n, p) in others(dir, kind) {
        if n == name {
            let _ = std::fs::remove_file(p);
        }
    }
    let _ = std::fs::remove_dir(dir.join("media").join(kind));
}

/// Make a variant the one shown: it and the chosen one change places.
pub fn choose_in(dir: &Path, v: &mut Value, kind: &str, name: &str) -> Result<()> {
    let current = flat(dir, kind);
    let cur = chosen_name(v, kind);
    if current.is_some() && cur == name {
        return Ok(());
    }
    let sub = dir.join("media").join(kind);
    let (_, from) = others(dir, kind).into_iter().find(|(n, _)| n == name).ok_or_else(|| anyhow!("This game has no picture from {}.", source_name(name)))?;
    if let Some(f) = current {
        let ext = f.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
        for (n, p) in others(dir, kind) {
            if n == cur {
                let _ = std::fs::remove_file(p);
            }
        }
        std::fs::rename(&f, sub.join(format!("{cur}.{ext}")))?;
    }
    let ext = from.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
    std::fs::rename(&from, dir.join("media").join(format!("{kind}.{ext}")))?;
    let _ = std::fs::remove_dir(&sub);
    if !v["chosen"].is_object() {
        v["chosen"] = json!({});
    }
    v["chosen"][kind] = json!(name);
    if !v["scraped"].is_object() {
        v["scraped"] = json!({});
    }
    if !v["scraped"]["media"].is_object() {
        v["scraped"]["media"] = json!({});
    }
    v["scraped"]["media"][kind] = json!(if name == "yours" { "you" } else { name });
    Ok(())
}

/// Show this variant of a kind (the game page's *Use this*), kept over later scrapes.
pub fn choose(ix: &Index, lib: &Library, dir: &Path, kind: &str, name: &str) -> Result<()> {
    lib.writable()?;
    if !MEDIA.iter().any(|(k, _, _)| *k == kind) {
        bail!("There's no kind of picture called {kind}.");
    }
    let mut v = meta::read_game(dir);
    choose_in(dir, &mut v, kind, name)?;
    if !v["pinned_media"].is_object() {
        v["pinned_media"] = json!({});
    }
    v["pinned_media"][kind] = json!(name);
    meta::write_game(ix, lib, dir, &v)
}

// ------------------------------------------------------------ sizes

/// What the library's metadata takes, for each system and each kind: details (the
/// sources' answers and `game.json`), each kind of picture and video, scraped manuals,
/// and the user's documents.
pub fn sizes(lib: &Library) -> Value {
    fn walk(p: &Path) -> u64 {
        match std::fs::metadata(p) {
            Ok(m) if m.is_file() => m.len(),
            Ok(m) if m.is_dir() => std::fs::read_dir(p).into_iter().flatten().flatten().map(|e| walk(&e.path())).sum(),
            _ => 0,
        }
    }
    let mut systems: Vec<Value> = vec![];
    let mut total: BTreeMap<String, u64> = BTreeMap::new();
    let mut by: BTreeMap<String, (BTreeMap<String, u64>, u64)> = BTreeMap::new();
    for sys in std::fs::read_dir(meta::games_root(lib)).into_iter().flatten().flatten() {
        let platform = sys.file_name().to_string_lossy().into_owned();
        if platform.starts_with('.') || !sys.path().is_dir() {
            continue;
        }
        let (kinds, games) = by.entry(platform).or_default();
        for g in std::fs::read_dir(sys.path()).into_iter().flatten().flatten() {
            let dir = g.path();
            if !dir.join("game.json").is_file() {
                continue;
            }
            *games += 1;
            let mut add = |k: &str, n: u64| {
                if n > 0 {
                    *kinds.entry(k.to_string()).or_default() += n;
                }
            };
            add("details", walk(&dir.join("game.json")) + walk(&dir.join(DIR)));
            for e in std::fs::read_dir(dir.join("media")).into_iter().flatten().flatten() {
                let n = e.file_name().to_string_lossy().into_owned();
                let kind = n.split_once('.').map(|(k, _)| k.to_string()).unwrap_or(n);
                add(&kind, walk(&e.path()));
            }
            let m = read_json_object(&dir.join("docs").join(crate::docs::MANIFEST));
            for e in std::fs::read_dir(dir.join("docs")).into_iter().flatten().flatten() {
                let n = e.file_name().to_string_lossy().into_owned();
                if n == crate::docs::MANIFEST {
                    continue;
                }
                let scraped = m["files"][&n]["source"].as_str().is_some_and(|s| !s.is_empty());
                add(if scraped { "manual" } else { "docs" }, walk(&e.path()));
            }
        }
    }
    for (platform, (kinds, games)) in by.iter().filter(|(_, (_, games))| *games > 0) {
        for (k, n) in kinds {
            *total.entry(k.clone()).or_default() += n;
        }
        systems.push(json!({
            "platform": platform, "name": crate::platforms::name_of(platform), "games": games,
            "total": kinds.values().sum::<u64>(), "kinds": kinds,
        }));
    }
    let names: Vec<Value> = [("details", "Details")]
        .into_iter()
        .chain(MEDIA.iter().map(|(k, n, _)| (*k, *n)))
        .chain([("manual", "Scraped manuals"), ("docs", "Your documents")])
        .map(|(k, n)| json!({ "id": k, "name": n }))
        .collect();
    json!({ "systems": systems, "total": total.values().sum::<u64>(), "kinds": total, "names": names })
}

// ------------------------------------------------------------ deleting metadata (D48)

/// What can be deleted: details, each kind of picture and video, scraped manuals.
pub fn what_choices() -> Vec<Value> {
    [("details", "Details from sources")]
        .into_iter()
        .chain(MEDIA.iter().map(|(k, n, _)| (*k, *n)))
        .chain([("manual", "Scraped manuals")])
        .map(|(k, n)| json!({ "id": k, "name": n }))
        .collect()
}

/// A change setting aside a system's or a game's metadata: the sources' answers, the
/// kinds of picture and video chosen, the manuals sources gave. The user's documents and
/// hand edits stay. After it (and after its Undo) the games' details are made again.
pub fn delete_plan(ix: &Index, lib: &Library, platform: Option<&str>, folder: Option<&str>, what: &[String]) -> Result<Plan> {
    lib.writable()?;
    if what.is_empty() {
        bail!("Choose what to delete.");
    }
    for w in what {
        if !what_choices().iter().any(|c| c["id"] == w.as_str()) {
            bail!("There's no metadata called {w}.");
        }
    }
    let root = meta::games_root(lib);
    let dirs: Vec<PathBuf> = match (folder, platform) {
        (Some(f), _) => vec![meta::resolve(lib, f)?],
        (None, Some(p)) => {
            crate::platforms::valid_id(p)?;
            game_dirs(lib, Some(p))
        }
        (None, None) => bail!("Choose a system or a game."),
    };
    let rules = Rules::of(lib);
    let mut ops = vec![];
    let mut games = 0;
    for dir in &dirs {
        let rel_game = dir.strip_prefix(&root).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default();
        let game = lib.relative(dir).unwrap_or_default();
        let before = ops.len();
        let mut aside = |p: PathBuf| {
            let Ok(rel) = p.strip_prefix(&root) else { return };
            let size = if p.is_dir() { std::fs::read_dir(&p).into_iter().flatten().flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum() } else { std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0) };
            let src = Src { file_id: 0, root: format!("dir:{}", root.display()), rel: rel.to_string_lossy().replace('\\', "/"), path: p.display().to_string(), member: None, size, crc32: String::new(), sha1: None };
            ops.push(Op { op: "aside".into(), game: game.clone(), to: p.display().to_string(), from: vec![src], names: vec![], aside: vec![], size: 0, note: None });
        };
        if what.iter().any(|w| w == "details") {
            // a folder from before is split into its sources first, so they can go
            let v = meta::read_game(dir);
            if v["by_source"] != true {
                remake(ix, lib, dir, &rules)?;
            }
            if dir.join(DIR).is_dir() {
                aside(dir.join(DIR));
            }
        }
        for (kind, _, _) in MEDIA {
            if !what.iter().any(|w| w == kind) {
                continue;
            }
            if let Some(f) = flat(dir, kind) {
                aside(f);
            }
            if dir.join("media").join(kind).is_dir() {
                aside(dir.join("media").join(kind));
            }
        }
        if what.iter().any(|w| w == "manual") {
            let m = read_json_object(&dir.join("docs").join(crate::docs::MANIFEST));
            for e in std::fs::read_dir(dir.join("docs")).into_iter().flatten().flatten() {
                let n = e.file_name().to_string_lossy().into_owned();
                if n != crate::docs::MANIFEST && m["files"][&n]["source"].as_str().is_some_and(|s| !s.is_empty()) {
                    aside(e.path());
                }
            }
        }
        if ops.len() > before {
            games += 1;
        }
        let _ = rel_game;
    }
    let label_what = if what.len() == 1 { what_choices().into_iter().find(|c| c["id"] == what[0].as_str()).and_then(|c| c["name"].as_str().map(str::to_lowercase)).unwrap_or_default() } else { "metadata".into() };
    let whose = match (folder, platform) {
        (Some(f), _) => f.rsplit('/').next().unwrap_or(f).to_string(),
        (None, Some(p)) => format!("{} game{} on {}", games, if games == 1 { "" } else { "s" }, crate::platforms::name_of(p)),
        _ => String::new(),
    };
    Ok(Plan {
        id: new_id(),
        created: now(),
        kind: "metadata".into(),
        label: format!("Deleted the {label_what} of {whose}"),
        title: format!("Delete the {label_what} of {whose}"),
        dat: Value::Null,
        platform: platform.map(String::from).or_else(|| dirs.first().and_then(|d| d.parent()).and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned())).unwrap_or_default(),
        mode: "aside".into(),
        format: "keep".into(),
        out: Some(root.display().to_string()),
        ops,
        games,
        in_place: 0,
        conflicts: vec![],
        duplicates: vec![],
        left: vec![],
        space: vec![],
        new_root: None,
        present: vec![],
        export: None,
    })
}

/// After a metadata change ran or was undone: its games' details made again from what's
/// left, and the documents listed again.
pub fn after(ix: &Index, lib: &Library, plan: &Plan) -> Result<()> {
    let rules = Rules::of(lib);
    let mut seen = HashSet::new();
    for op in &plan.ops {
        if op.game.is_empty() || !seen.insert(op.game.clone()) {
            continue;
        }
        let Ok(dir) = meta::resolve(lib, &op.game) else { continue };
        if dir.join("game.json").is_file() {
            remake(ix, lib, &dir, &rules)?;
        }
    }
    crate::docs::sync(ix, lib)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game_with_answers(name: &str) -> (PathBuf, Library, Index, PathBuf) {
        let (dir, lib, ix) = crate::collection::tests::fixture(name);
        let g = meta::dat_game_named(ix.conn(), &lib, "gb", "Alpha (Europe)").unwrap().unwrap();
        let gdir = meta::ensure_folder(&ix, &lib, &g).unwrap();
        (dir, lib, ix, gdir)
    }

    fn answer(source: &str, title: &str, year: i32, details: Value) -> Match {
        Match { source: source.into(), id: format!("{source}-1"), title: title.into(), year: Some(year), details: details.as_object().cloned().unwrap(), ..Default::default() }
    }

    #[test]
    fn details_are_made_from_each_sources_answer_by_field() {
        let (dir, lib, ix, gdir) = game_with_answers("by-source");
        let all: Vec<String> = FIELDS.iter().map(|(k, _)| k.to_string()).collect();
        keep(&gdir, &answer("igdb", "Alpha IGDB", 1999, json!({ "description": "From IGDB.", "developer": "IGDB Dev" })), &all, false).unwrap();
        keep(&gdir, &answer("screenscraper", "Alpha", 1998, json!({ "description": "From SS.", "players": "1-2" })), &["description".into()], false).unwrap();
        // only the details chosen are kept
        assert!(answers(&gdir)["screenscraper"]["details"].get("players").is_none());
        let rules = Rules { order: vec!["screenscraper".into(), "igdb".into()], from: Map::new() };
        let mut v = meta::read_game(&gdir);
        make(&mut v, &gdir, &rules);
        assert_eq!((v["title"].as_str(), v["year"].as_i64(), v["details"]["description"].as_str()), (Some("Alpha"), Some(1998), Some("From SS.")), "{v}");
        assert_eq!((v["details"]["developer"].as_str(), v["scraped"]["developer"].as_str()), (Some("IGDB Dev"), Some("igdb")));
        assert_eq!(v["ids"], json!({ "igdb": "igdb-1", "screenscraper": "screenscraper-1" }));
        // Settings › Metadata says descriptions come from IGDB; the game's own pick wins over that
        let mut from = Map::new();
        from.insert("description".into(), json!("igdb"));
        let rules = Rules { order: rules.order, from };
        make(&mut v, &gdir, &rules);
        assert_eq!(v["details"]["description"], "From IGDB.");
        v["pinned"] = json!({ "description": "screenscraper", "title": "igdb" });
        make(&mut v, &gdir, &rules);
        assert_eq!((v["details"]["description"].as_str(), v["title"].as_str()), (Some("From SS."), Some("Alpha IGDB")));
        // a hand edit wins over all of them
        v["edited"] = json!(["description"]);
        v["details"]["description"] = json!("Mine.");
        make(&mut v, &gdir, &rules);
        assert_eq!((v["details"]["description"].as_str(), v["scraped"]["description"].as_str()), (Some("Mine."), Some("you")));
        // fill in keeps what a source's file has; replace writes the fields taken again
        keep(&gdir, &answer("igdb", "Alpha IGDB", 1999, json!({ "description": "Newer." })), &all, false).unwrap();
        assert_eq!(answers(&gdir)["igdb"]["details"]["description"], "From IGDB.");
        keep(&gdir, &answer("igdb", "Alpha IGDB", 1999, json!({ "description": "Newer." })), &all, true).unwrap();
        let a = answers(&gdir);
        assert_eq!((a["igdb"]["details"]["description"].as_str(), a["igdb"]["details"].get("developer")), (Some("Newer."), None));
        // with no source left: the DAT's title, no year, no ids
        std::fs::remove_dir_all(gdir.join(DIR)).unwrap();
        v["edited"] = json!([]);
        make(&mut v, &gdir, &rules);
        assert_eq!((v["title"].as_str(), v.get("year"), v["details"].as_object().map(|d| d.len())), (Some("Alpha"), None, Some(0)), "{v}");
        assert!(v["ids"].as_object().unwrap().is_empty());
        let _ = (lib, ix);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_folder_from_before_is_split_by_where_its_details_came_from() {
        let (dir, lib, ix, gdir) = game_with_answers("split");
        let mut v = meta::read_game(&gdir);
        v["title"] = json!("Alpha!");
        v["year"] = json!(1999);
        v["details"] = json!({ "description": "Old.", "developer": "Dev", "players": "2" });
        v["scraped"] = json!({ "description": "screenscraper", "developer": "igdb", "title": "screenscraper", "players": "you", "when": "2026-10-01T00:00:00Z" });
        v["edited"] = json!(["players"]);
        v["ids"] = json!({ "screenscraper": "1001", "igdb": "77" });
        meta::write_game(&ix, &lib, &gdir, &v).unwrap();
        remake(&ix, &lib, &gdir, &Rules::of(&lib)).unwrap();
        let a = answers(&gdir);
        assert_eq!((a["screenscraper"]["details"]["description"].as_str(), a["screenscraper"]["title"].as_str(), a["screenscraper"]["year"].as_i64()), (Some("Old."), Some("Alpha!"), Some(1999)));
        assert_eq!((a["igdb"]["details"]["developer"].as_str(), a["igdb"]["id"].as_str()), (Some("Dev"), Some("77")));
        let v = meta::read_game(&gdir);
        assert_eq!((v["title"].as_str(), v["details"]["players"].as_str(), v["by_source"].as_bool()), (Some("Alpha!"), Some("2"), Some(true)), "{v}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn variants_are_kept_chosen_and_deleted_with_undo() {
        let (dir, lib, ix, gdir) = game_with_answers("variants");
        let mut v = meta::read_game(&gdir);
        let png = crate::scrape::tests::png(4, 4);
        let jpg = b"\xff\xd8 a jpeg".to_vec();
        std::fs::create_dir_all(gdir.join("media")).unwrap();
        // the chosen one first, then the other sources' (one each: a second from IGDB replaces its first)
        store(&gdir, &v, "cover", "screenscraper", "png", &png).unwrap();
        choose_in(&gdir, &mut v, "cover", "screenscraper").unwrap();
        store(&gdir, &v, "cover", "thegamesdb", "jpg", &jpg).unwrap();
        store(&gdir, &v, "cover", "igdb", "png", &png).unwrap();
        store(&gdir, &v, "cover", "igdb", "jpg", &jpg).unwrap();
        assert_eq!(meta::media_kinds(&gdir), ["cover"]);
        assert_eq!(std::fs::read(gdir.join("media/cover.png")).unwrap(), png);
        assert_eq!(names(&gdir, &v, "cover"), ["screenscraper", "igdb", "thegamesdb"]);
        assert!(!gdir.join("media/cover/igdb.png").exists());
        meta::write_game(&ix, &lib, &gdir, &v).unwrap();
        // TheGamesDB's box shown instead: the two change places
        choose(&ix, &lib, &gdir, "cover", "thegamesdb").unwrap();
        let v = meta::read_game(&gdir);
        assert_eq!((v["chosen"]["cover"].as_str(), v["pinned_media"]["cover"].as_str(), v["scraped"]["media"]["cover"].as_str()), (Some("thegamesdb"), Some("thegamesdb"), Some("thegamesdb")));
        assert!(gdir.join("media/cover.jpg").is_file() && gdir.join("media/cover/screenscraper.png").is_file() && !gdir.join("media/cover.png").exists());
        let page = variants(&lib, &gdir, &v);
        assert_eq!(page["cover"].as_array().unwrap().len(), 3);
        assert_eq!((page["cover"][0]["name"].as_str(), page["cover"][0]["chosen"].as_bool(), page["cover"][0]["pinned"].as_bool()), (Some("thegamesdb"), Some(true), Some(true)));
        // its details too
        keep(&gdir, &answer("screenscraper", "Alpha SS", 1999, json!({ "description": "SS." })), &["description".into()], false).unwrap();
        remake(&ix, &lib, &gdir, &Rules::of(&lib)).unwrap();
        assert_eq!(meta::read_game(&gdir)["title"], "Alpha SS");
        let s = sizes(&lib);
        assert!(s["kinds"]["cover"].as_u64().unwrap() > 0 && s["systems"][0]["platform"] == "gb", "{s}");
        // delete the system's details and boxes: set aside, then put back by Undo
        let plan = delete_plan(&ix, &lib, Some("gb"), None, &["details".into(), "cover".into()]).unwrap();
        assert_eq!((plan.games, plan.ops.len()), (1, 3), "{:?}", plan.ops);
        let cancel = std::sync::atomic::AtomicBool::new(false);
        crate::organise::run(&ix, &lib, &plan, &cancel, &mut |_| {}).unwrap();
        let v = meta::read_game(&gdir);
        assert!(!gdir.join(DIR).exists() && meta::media_kinds(&gdir).is_empty() && !gdir.join("media/cover").exists());
        assert_eq!(v["title"].as_str(), Some("Alpha"), "{v}");
        // the set-aside folder isn't taken for a system
        meta::sync(&ix, &lib).unwrap();
        crate::organise::undo(&ix, &lib, &plan.id, &cancel, &mut |_| {}).unwrap();
        let v = meta::read_game(&gdir);
        assert_eq!((v["title"].as_str(), meta::media_kinds(&gdir)), (Some("Alpha SS"), vec!["cover".to_string()]), "{v}");
        assert!(gdir.join("media/cover/igdb.jpg").is_file());
        assert_eq!((v["chosen"]["cover"].as_str(), v["scraped"]["media"]["cover"].as_str()), (Some("thegamesdb"), Some("thegamesdb")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
