//! A PC game's metadata in the layout every other game's has (docs/PLAN.md, "PC game page
//! design", D55): `_library/games/pc/<id>/` with `game.json` (its details made from the
//! sources' answers, `edited`, `pinned`, `chosen`, `pinned_media`), `sources/<source>.json`,
//! `media/<kind>.<ext>` with the other sources' at `media/<kind>/<source>.<ext>`, and
//! `docs/`. What only a PC game has stays in `PC/<Title> (<Year>)/game.json` (pcgames.rs),
//! its title and year with it, since its folder is named by them.
//!
//! A PC game from before D55 has its details in `PC/…/game.json` and its artwork already in
//! `media/`, from one source. They're brought across the first time the game's metadata is
//! changed (`ensure`): the details kept as that source's answer, the pictures marked as its,
//! written, and only then taken out of `PC/…/game.json`. Until then they're read from there
//! (`view`), so nothing has to be brought across for the game to be shown.

use crate::identify;
use crate::index::Index;
use crate::library::{now, Library};
use crate::meta::{self, FIELDS};
use crate::pcgames::{self, Entry, Match, PcGame};
use crate::sources::{self, Rules};
use anyhow::{anyhow, Result};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// The source a PC game's details from before are kept as when nothing says which it was.
const EARLIER: &str = "earlier";

/// A PC game's metadata folder: `_library/games/pc/<id>/`.
pub fn dir(lib: &Library, id: &str) -> PathBuf {
    meta::games_root(lib).join("pc").join(id)
}

/// Whether a game's folder is a PC game's.
pub fn is_pc(lib: &Library, dir: &Path) -> bool {
    dir.parent() == Some(meta::games_root(lib).join("pc").as_path())
}

/// The PC games' metadata folders that have been brought across (with a `game.json`).
pub fn dirs(lib: &Library) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(meta::games_root(lib).join("pc"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) && p.join("game.json").is_file())
        .collect();
    v.sort();
    v
}

/// The PC game a metadata folder is for.
fn entry_of(lib: &Library, dir: &Path) -> Result<Entry> {
    let id = dir.file_name().map(|n| n.to_string_lossy().into_owned()).ok_or_else(|| anyhow!("{} isn't a PC game's folder.", dir.display()))?;
    pcgames::get(lib, &id)
}

/// A PC game's title, by its metadata folder (whose name is only its id).
pub fn title_of(lib: &Library, dir: &Path) -> Option<String> {
    is_pc(lib, dir).then(|| entry_of(lib, dir).ok().map(|e| e.game.title)).flatten()
}

/// The source a PC game's details from before came from: the one it was identified by,
/// else its one source's id, else "an earlier scrape".
fn earlier_source(g: &PcGame) -> String {
    if let Some(s) = g.details.get("identified_by").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        return s.to_string();
    }
    let known: Vec<&String> = g.ids.keys().filter(|k| identify::SOURCES.iter().any(|s| s.0 == k.as_str())).collect();
    match known[..] {
        [one] => one.clone(),
        _ => EARLIER.into(),
    }
}

/// The details a `PC/…/game.json` from before still holds.
fn old_details(g: &PcGame) -> Map<String, Value> {
    FIELDS
        .iter()
        .filter_map(|(k, _)| g.details.get(*k).filter(|x| !x.is_null() && *x != "" && !x.as_array().is_some_and(|a| a.is_empty())).map(|x| (k.to_string(), x.clone())))
        .collect()
}

/// What a PC game's details are made by: the source it was last identified by first, then
/// the library's (Settings › Metadata's *Details from*, the scrape's order), then Steam's and
/// GOG's.
pub fn rules(lib: &Library, v: &Value) -> Rules {
    let mut r = Rules::of(lib);
    let mut order: Vec<String> = v["identified_by"].as_str().map(String::from).into_iter().collect();
    order.append(&mut r.order);
    order.extend(["steam".to_string(), "gog".to_string()]);
    r.order = order;
    r
}

/// Make a PC game's details from its sources' answers (its title and year are its own).
pub fn make(lib: &Library, dir: &Path, v: &mut Value) {
    let rules = rules(lib, v);
    sources::make_details(v, dir, &rules);
}

/// Bring a PC game's details and artwork from before across, if they aren't yet (D55): the
/// details kept as their source's answer, the pictures marked as its, `game.json` written,
/// and only then the details taken out of `PC/…/game.json`. Run again, it finds nothing to
/// do; interrupted, it picks up where it was. Returns the metadata folder.
pub fn ensure(ix: &Index, lib: &Library, e: &Entry) -> Result<PathBuf> {
    lib.writable()?;
    let d = dir(lib, &e.game.id);
    let old = old_details(&e.game);
    let mut v = meta::read_game(&d);
    if v["by_source"] == true && old.is_empty() && !e.game.details.contains_key("identified_by") {
        return Ok(d);
    }
    let src = earlier_source(&e.game);
    if v["id"].is_null() {
        v["id"] = json!(e.game.id);
        v["platform"] = json!("pc");
        v["created"] = json!(now());
    }
    if !old.is_empty() {
        // filled in, so an answer kept already (an interrupted run's) is left as it is
        let m = Match { source: src.clone(), id: e.game.ids.get(&src).cloned().unwrap_or_default(), details: old.clone(), ..Default::default() };
        sources::keep(&d, &m, &old.keys().cloned().collect::<Vec<_>>(), false)?;
    }
    // the pictures there already came with the details, from the same source
    for (kind, _, _) in meta::all_media() {
        if sources::flat(&d, kind).is_some() && v["chosen"][*kind].is_null() {
            if !v["chosen"].is_object() {
                v["chosen"] = json!({});
            }
            v["chosen"][*kind] = json!(src);
        }
    }
    if v["identified_by"].is_null() && src != EARLIER {
        v["identified_by"] = json!(src);
    }
    make(lib, &d, &mut v);
    std::fs::create_dir_all(&d)?;
    meta::write_game(ix, lib, &d, &v)?;
    // written: now they can leave PC/…/game.json (what isn't a detail, a GOG slug, stays)
    let mut fresh = pcgames::get(lib, &e.game.id)?;
    let before = fresh.game.details.len();
    fresh.game.details.retain(|k, _| !old.contains_key(k) && k != "identified_by");
    if fresh.game.details.len() != before {
        pcgames::write_manifest(&fresh.dir, &fresh.game)?;
    }
    Ok(d)
}

/// `ensure`, by the metadata folder.
pub fn ensure_dir(ix: &Index, lib: &Library, dir: &Path) -> Result<PathBuf> {
    ensure(ix, lib, &entry_of(lib, dir)?)
}

/// Make a PC game's details again from its folder (after Delete metadata… or its Undo, or
/// *Details from* changed). A folder whose game has gone is made from what's in it.
pub fn remake(ix: &Index, lib: &Library, dir: &Path) -> Result<()> {
    if let Ok(e) = entry_of(lib, dir) {
        ensure(ix, lib, &e)?;
    }
    let mut v = meta::read_game(dir);
    if v.as_object().is_none_or(|o| o.is_empty()) {
        return Ok(());
    }
    make(lib, dir, &mut v);
    meta::write_game(ix, lib, dir, &v)
}

/// A PC game's metadata as it is, without writing anything: its folder's `game.json`, with
/// the details still in `PC/…/game.json` from before read as their source's.
pub fn view(lib: &Library, e: &Entry) -> Value {
    let d = dir(lib, &e.game.id);
    let mut v = meta::read_game(&d);
    for k in ["details", "scraped", "pinned", "chosen", "pinned_media"] {
        if !v[k].is_object() {
            v[k] = json!({});
        }
    }
    if !v["edited"].is_array() {
        v["edited"] = json!([]);
    }
    let old = old_details(&e.game);
    if v["by_source"] == true && old.is_empty() {
        return v;
    }
    let src = earlier_source(&e.game);
    for (k, x) in old {
        if v["details"][&k].is_null() {
            v["details"][&k] = x;
            v["scraped"][&k] = json!(src);
        }
    }
    for (kind, _, _) in meta::all_media() {
        if sources::flat(&d, kind).is_some() && v["chosen"][*kind].is_null() {
            v["chosen"][*kind] = json!(src);
        }
    }
    if v["identified_by"].is_null() && src != EARLIER {
        v["identified_by"] = json!(src);
    }
    v
}

/// What a PC game's page shows of its metadata, as a DAT game's: its details and where each
/// came from, what each source said, each kind's pictures from each source, its folder.
pub fn page(lib: &Library, e: &Entry) -> Value {
    let d = dir(lib, &e.game.id);
    let v = view(lib, e);
    // its ids: the ones it was imported by (PC/…/game.json) and the sources' answers'
    let mut ids: Map<String, Value> = e.game.ids.iter().map(|(k, x)| (k.clone(), json!(x))).collect();
    for (k, x) in v["ids"].as_object().into_iter().flatten() {
        ids.entry(k.clone()).or_insert_with(|| x.clone());
    }
    let answers: Vec<Value> = sources::answers(&d)
        .into_iter()
        .map(|(s, mut a)| {
            a["name"] = json!(sources::source_name(&s));
            a
        })
        .collect();
    json!({
        "details": v["details"], "scraped": v["scraped"], "edited": v["edited"], "pinned": v["pinned"], "ids": ids,
        "identified_by": v["identified_by"], "sources": answers, "variants": sources::variants(lib, &d, &v),
        "media": identify::media_of(lib, &d.join("media")), "folder": lib.relative(&d),
        "folder_path": d.join("game.json").is_file().then(|| d.join("game.json").display().to_string()),
    })
}

/// Fetch a source's artwork (kind → URL) into a PC game's folder as that source's (D47): it's
/// the one shown when the kind has none yet, or unless another source's was picked on the
/// game's page. The kinds fetched.
fn fetch_art(d: &Path, v: &mut Value, source: &str, art: &BTreeMap<String, String>) -> Vec<String> {
    let mut got = vec![];
    for (kind, u) in art {
        if kind.is_empty() || !kind.chars().all(|c| c.is_ascii_alphanumeric()) {
            continue;
        }
        let Some((bytes, ext)) = identify::fetch_picture(u) else { continue };
        if std::fs::create_dir_all(d.join("media")).is_err() || sources::store(d, v, kind, source, ext, &bytes).is_err() {
            continue;
        }
        let picked = v["pinned_media"][kind].as_str().is_some_and(|p| p != source);
        if sources::flat(d, kind).is_none() || (!picked && sources::chosen_name(v, kind) != source) {
            let _ = sources::choose_in(d, v, kind, source);
        }
        got.push(kind.clone());
    }
    got
}

/// Put a source's answer into a PC game (Identify…): its id with the game's own, its title
/// and year when asked (the folder renamed), its details kept as its answer and the game's
/// made from them, its artwork as its pictures.
pub fn identify(ix: &Index, lib: &Library, id: &str, m: &Match, retitle: bool) -> Result<Entry> {
    lib.writable()?;
    let mut patch = json!({ "ids": { m.source.clone(): m.id } });
    if retitle {
        patch["title"] = json!(m.title);
        if m.year.is_some() {
            patch["year"] = json!(m.year);
        }
    }
    let e = pcgames::update(lib, id, &patch)?;
    let d = ensure(ix, lib, &e)?;
    let fields: Vec<String> = FIELDS.iter().map(|(k, _)| k.to_string()).collect();
    sources::keep(&d, m, &fields, true)?;
    let mut v = meta::read_game(&d);
    v["identified_by"] = json!(m.source);
    fetch_art(&d, &mut v, &m.source, &m.art);
    make(lib, &d, &mut v);
    meta::write_game(ix, lib, &d, &v)?;
    pcgames::get(lib, id)
}

/// After an import: each game it wrote (`games`, their `PC/…` folders) has its details kept
/// as the answer of the source that found it, and the artwork that source offered (kept in
/// `game.json` until now) fetched as its pictures. The titles that got some.
pub fn settle_imports(ix: &Index, lib: &Library, games: &[PathBuf], cancel: &AtomicBool) -> Vec<String> {
    let mut got = vec![];
    for e in pcgames::list(lib) {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if !games.iter().any(|g| crate::organise::key(g) == crate::organise::key(&e.dir)) {
            continue;
        }
        let art: Option<BTreeMap<String, String>> = e.game.extra.get("art").and_then(Value::as_object).map(|a| a.iter().filter_map(|(k, x)| Some((k.clone(), x.as_str()?.to_string()))).collect());
        let from = e.game.extra.get("art_from").and_then(Value::as_str).map(String::from).unwrap_or_else(|| earlier_source(&e.game));
        let Ok(d) = ensure(ix, lib, &e) else { continue };
        if let Some(art) = art {
            let mut v = meta::read_game(&d);
            if !fetch_art(&d, &mut v, &from, &art).is_empty() {
                got.push(e.game.title.clone());
            }
            let _ = meta::write_game(ix, lib, &d, &v);
            // fetched: the addresses leave PC/…/game.json
            if let Ok(mut g) = pcgames::get(lib, &e.game.id) {
                g.game.extra.remove("art");
                g.game.extra.remove("art_from");
                let _ = pcgames::write_manifest(&g.dir, &g.game);
            }
        }
    }
    got
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::tests::temp_dir;

    /// A library with a PC game as Phase 4 left it: its details in `PC/…/game.json`, its
    /// cover in `_library/games/pc/<id>/media/`.
    fn from_before(name: &str) -> (PathBuf, Library, Index, String) {
        let dir = temp_dir(name);
        let lib = Library::open(dir.join("lib")).unwrap();
        let ix = Index::open(&dir.join("index.sqlite"), &lib.id().unwrap()).unwrap();
        let game_dir = pcgames::folder(&lib).join("Zeta Quest (1999)");
        std::fs::create_dir_all(&game_dir).unwrap();
        let g = PcGame {
            id: "pc-1".into(), title: "Zeta Quest".into(), year: Some(1999), source: "gog".into(),
            ids: [("gog".to_string(), "1207658930".to_string())].into(),
            details: serde_json::from_value(json!({ "description": "Quest on.", "developer": "Zeta Soft", "gog_slug": "zeta_quest", "identified_by": "gog" })).unwrap(),
            ..Default::default()
        };
        pcgames::write_manifest(&game_dir, &g).unwrap();
        pcgames::save_media(&lib, "pc-1", "cover", "jpg", b"a cover from gog").unwrap();
        (dir, lib, ix, "pc-1".into())
    }

    #[test]
    fn a_pc_game_from_before_is_read_where_it_was_then_brought_across_once() {
        let (dir, lib, ix, id) = from_before("pcmeta-across");
        let e = pcgames::get(&lib, &id).unwrap();
        // read where it was, nothing written
        let v = view(&lib, &e);
        assert_eq!((v["details"]["developer"].as_str(), v["scraped"]["developer"].as_str(), v["chosen"]["cover"].as_str()), (Some("Zeta Soft"), Some("gog"), Some("gog")));
        let d = dir_of(&lib, &id);
        assert!(!d.join("game.json").exists());
        // brought across: the source's answer, the cover marked as its, then out of PC/…/game.json
        ensure(&ix, &lib, &e).unwrap();
        let a = sources::answers(&d);
        assert_eq!((a["gog"]["details"]["description"].as_str(), a["gog"]["id"].as_str()), (Some("Quest on."), Some("1207658930")));
        let v = meta::read_game(&d);
        assert_eq!((v["details"]["developer"].as_str(), v["chosen"]["cover"].as_str(), v["identified_by"].as_str(), v["platform"].as_str()), (Some("Zeta Soft"), Some("gog"), Some("gog"), Some("pc")), "{v}");
        let e = pcgames::get(&lib, &id).unwrap();
        assert_eq!(e.game.details, serde_json::from_value::<Map<String, Value>>(json!({ "gog_slug": "zeta_quest" })).unwrap());
        assert_eq!((e.game.title.as_str(), e.game.ids["gog"].as_str()), ("Zeta Quest", "1207658930"));
        // again: nothing to do, nothing changed
        let before = std::fs::read(d.join("game.json")).unwrap();
        ensure(&ix, &lib, &e).unwrap();
        assert_eq!(std::fs::read(d.join("game.json")).unwrap(), before);
        assert_eq!(view(&lib, &e)["details"]["description"], "Quest on.");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn dir_of(lib: &Library, id: &str) -> PathBuf {
        dir(lib, id)
    }

    #[test]
    fn an_interrupted_bringing_across_keeps_what_was_written_and_finishes() {
        let (dir, lib, ix, id) = from_before("pcmeta-interrupted");
        let d = dir_of(&lib, &id);
        // the answer was written, then it stopped: the details are still in PC/…/game.json
        let m = Match { source: "gog".into(), id: "1207658930".into(), details: serde_json::from_value(json!({ "description": "Kept first." })).unwrap(), ..Default::default() };
        sources::keep(&d, &m, &["description".into()], false).unwrap();
        ensure(&ix, &lib, &pcgames::get(&lib, &id).unwrap()).unwrap();
        let a = sources::answers(&d);
        assert_eq!((a["gog"]["details"]["description"].as_str(), a["gog"]["details"]["developer"].as_str()), (Some("Kept first."), Some("Zeta Soft")));
        assert!(pcgames::get(&lib, &id).unwrap().game.details.get("developer").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_pc_game_is_edited_given_another_sources_answer_and_its_pictures_chosen_like_any_game() {
        let (dir, lib, ix, id) = from_before("pcmeta-page");
        let d = dir_of(&lib, &id);
        // Steam's answer and pictures beside GOG's, as Identify keeps them
        let base = crate::identify::tests::serve(crate::identify::tests::answers());
        let cfg = crate::identify::tests::config_at(&base);
        let m = crate::identify::details(&cfg, &dir, "steam", "4000").unwrap();
        let e = identify(&ix, &lib, &id, &m, false).unwrap();
        assert_eq!((e.game.ids["steam"].as_str(), e.game.ids["gog"].as_str(), e.dir.file_name().unwrap().to_str()), ("4000", "1207658930", Some("Zeta Quest (1999)")));
        let v = meta::read_game(&d);
        // the source it was identified by comes first; GOG's description stays an answer
        assert_eq!((v["details"]["description"].as_str(), v["scraped"]["description"].as_str(), v["details"]["publisher"].as_str()), (Some("Quest & more."), Some("steam"), Some("Zeta Pub")), "{v}");
        assert_eq!(sources::answers(&d)["gog"]["details"]["description"], "Quest on.");
        // Steam's cover is shown now, GOG's kept beside it; Steam's logo is the only logo
        assert_eq!((v["chosen"]["cover"].as_str(), v["chosen"]["logo"].as_str()), (Some("steam"), Some("steam")));
        assert!(d.join("media/cover/gog.jpg").is_file() && d.join("media/logo.png").is_file());
        let p = page(&lib, &e);
        assert_eq!(p["variants"]["cover"].as_array().unwrap().len(), 2, "{p}");
        assert_eq!(p["folder"], format!("_library/games/pc/{id}"));
        // GOG's cover picked on its page, kept over Identify again
        sources::choose(&ix, &lib, &d, "cover", "gog").unwrap();
        identify(&ix, &lib, &id, &m, false).unwrap();
        assert_eq!(meta::read_game(&d)["chosen"]["cover"], "gog");
        assert_eq!(std::fs::read(d.join("media/cover.jpg")).unwrap(), b"a cover from gog");
        // GOG's description used instead, then a hand edit over both, then the mark taken off
        sources::pin(&ix, &lib, &d, "description", Some("gog")).unwrap();
        assert_eq!(meta::read_game(&d)["details"]["description"], "Quest on.");
        assert!(sources::pin(&ix, &lib, &d, "title", Some("steam")).is_err(), "a PC game's title is its own");
        meta::edit_dir(&ix, &lib, &d, "developer", Some(&json!("Mine"))).unwrap();
        let v = meta::read_game(&d);
        assert_eq!((v["details"]["developer"].as_str(), v["scraped"]["developer"].as_str()), (Some("Mine"), Some("you")));
        meta::edit_dir(&ix, &lib, &d, "developer", None).unwrap();
        assert_eq!(meta::read_game(&d)["details"]["developer"], "Zeta Soft");
        // the PC game's own file still says only what's its own
        let e = pcgames::get(&lib, &id).unwrap();
        assert!(e.game.details.get("description").is_none() && e.game.title == "Zeta Quest");
        // Delete metadata… for it, then Undo
        let plan = sources::delete_plan(&ix, &lib, None, Some(&format!("_library/games/pc/{id}")), &["details".into(), "cover".into()]).unwrap();
        assert!(plan.label.contains("Zeta Quest"), "{}", plan.label);
        let cancel = AtomicBool::new(false);
        crate::organise::run(&ix, &lib, &plan, &cancel, &mut |_| {}).unwrap();
        let v = meta::read_game(&d);
        assert!(v["details"].as_object().unwrap().is_empty() && sources::flat(&d, "cover").is_none() && sources::flat(&d, "logo").is_some(), "{v}");
        crate::organise::undo(&ix, &lib, &plan.id, &cancel, &mut |_| {}).unwrap();
        assert_eq!(meta::read_game(&d)["details"]["description"], "Quest on.");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_imported_game_keeps_its_details_and_artwork_as_the_source_that_found_it() {
        let dir = temp_dir("pcmeta-import");
        let lib = Library::open(dir.join("lib")).unwrap();
        let ix = Index::open(&dir.join("index.sqlite"), &lib.id().unwrap()).unwrap();
        let inbox = dir.join("in");
        std::fs::create_dir_all(&inbox).unwrap();
        crate::pcgames::tests::zip_of(&inbox.join("Zeta Quest (1999).zip"), &[("zeta.exe", b"exe")]);
        let base = crate::identify::tests::serve(crate::identify::tests::answers());
        let mut cfg = crate::identify::tests::config_at(&base);
        cfg.sources = vec![crate::config::SourceSetting { id: "gog".into(), on: false }, crate::config::SourceSetting { id: "launchbox".into(), on: false }];
        let cancel = AtomicBool::new(false);
        let search = |t: &str, y: Option<i32>| crate::identify::best(&cfg, &dir, t, y);
        let items = pcgames::read_items(&lib, &pcgames::inbox_paths(&inbox).unwrap(), &search, &cancel, &mut |_| {}).unwrap();
        assert_eq!((items[0].found.as_str(), items[0].ids.get("steam").map(String::as_str)), ("search", Some("4000")));
        let plan = pcgames::plan_import(&lib, &items, "copy", true).unwrap();
        crate::organise::run(&ix, &lib, &plan, &cancel, &mut |_| {}).unwrap();
        let games: Vec<PathBuf> = plan.ops.iter().filter(|o| o.op == "text").filter_map(|o| Path::new(&o.to).parent().map(Path::to_path_buf)).collect();
        assert_eq!(settle_imports(&ix, &lib, &games, &cancel), ["Zeta Quest"]);
        let e = &pcgames::list(&lib)[0];
        let d = dir_of(&lib, &e.game.id);
        // Steam's answer, its cover and logo as Steam's; PC/…/game.json keeps only what's its own
        assert_eq!(sources::answers(&d)["steam"]["details"]["developer"], "Zeta Soft");
        let v = meta::read_game(&d);
        assert_eq!((v["details"]["description"].as_str(), v["chosen"]["cover"].as_str(), v["identified_by"].as_str()), (Some("Quest & more."), Some("steam"), Some("steam")), "{v}");
        assert!(d.join("media/cover.jpg").is_file() && d.join("media/logo.png").is_file());
        assert!(e.game.details.is_empty() && !e.game.extra.contains_key("art"), "{:?}", e.game);
        // the Library lists it with them
        meta::sync(&ix, &lib).unwrap();
        crate::browse::refresh(&ix, &lib).unwrap();
        let q = crate::browse::Query { platform: vec!["pc".into()], limit: 10, ..Default::default() };
        let r = crate::browse::query(&ix, &lib, &q).unwrap();
        assert_eq!((r["items"][0]["developer"].as_str(), r["items"][0]["described"].as_bool()), (Some("Zeta Soft"), Some(true)), "{r}");
        assert!(r["items"][0]["picture"].as_str().unwrap().contains("/media/cover.jpg?v="));
        let g = crate::browse::game(&ix, &lib, &format!("pc/{}", e.game.id)).unwrap();
        assert_eq!((g["pc"]["programs"].as_array().map(|a| a.len()), g["details"]["publisher"].as_str(), g["variants"]["cover"][0]["name"].as_str()), (Some(0), Some("Zeta Pub"), Some("steam")), "{g}");
        // its title edited on its page renames its folder; a detail is kept as yours
        crate::browse::edit(&ix, &lib, &format!("pc/{}", e.game.id), "title", Some(&json!("Zeta Quest Gold"))).unwrap();
        let g = crate::browse::edit(&ix, &lib, &format!("pc/{}", e.game.id), "genres", Some(&json!("Puzzle"))).unwrap();
        assert_eq!((g["title"].as_str(), g["details"]["genres"].clone(), g["edited"].clone()), (Some("Zeta Quest Gold"), json!(["Puzzle"]), json!(["genres"])));
        assert!(pcgames::folder(&lib).join("Zeta Quest Gold (1999)/game.json").is_file());
        // and the index names its documents by the new title
        let rel = lib.relative(&d).unwrap();
        let t: String = ix.conn().query_row("SELECT title FROM meta_games WHERE folder = ?1", [&rel], |r| r.get(0)).unwrap();
        assert_eq!(t, "Zeta Quest Gold");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
