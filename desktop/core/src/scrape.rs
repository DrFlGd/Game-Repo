//! Scraping (docs/PLAN.md, 3.6, "Phase 5 design", D30): a game's details, pictures,
//! video and manual from the sources, for a system's games in the library, every game
//! in its DAT (owned or not), a collection, or chosen games. The choices (which sources
//! in which order, which details, which pictures and from where, which regions, what to
//! do with games that have something) are the scrape's, saved in library.json `scrape`.
//!
//! ScreenScraper is asked by the ROMs' hashes, which the DAT records, so a game the user
//! doesn't have yet can be scraped; the others by the DAT game's title and system, an
//! answer being taken only when its title is the same. A game no source was sure about
//! keeps the best candidates as its `review` and is listed under *Needs a look*.

use crate::config::AppConfig;
use crate::docs;
use crate::identify::{self, Candidate};
use crate::index::Index;
use crate::launchbox;
use crate::library::Library;
use crate::meta::{self, DatGame};
use crate::pcgames::{norm_title, Match};
use crate::scrapers;
use crate::sources;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// The sources a scrape can ask (Steam and GOG are for PC games, which have their own page).
pub const SCRAPE_SOURCES: &[&str] = &["screenscraper", "igdb", "thegamesdb", "launchbox"];

/// What a scrape is for and what it takes.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Options {
    /// have (the system's games in the library) | all (every game in its DAT) | collection | games
    pub scope: String,
    pub platform: String,
    pub collection: String,
    pub games: Vec<i64>,
    /// The sources, in the order they're asked
    pub sources: Vec<String>,
    /// The details taken (meta::FIELDS)
    pub fields: Vec<String>,
    /// The pictures (meta::MEDIA) and `manual`
    pub media: Vec<String>,
    /// A kind of picture taken from this source when it has one: kind → source
    pub prefer: BTreeMap<String, String>,
    /// ScreenScraper's region codes, after the DAT game's own
    pub regions: Vec<String>,
    pub language: String,
    /// fill (what's missing) | replace | skip (games with anything already)
    pub existing: String,
    /// The kinds of picture kept from every source that has one, not only the chosen (D47)
    pub variants: Vec<String>,
    /// Ask the sources again, passing over their answers kept from the past week (D48)
    pub fresh: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            scope: "have".into(),
            platform: String::new(),
            collection: String::new(),
            games: vec![],
            sources: SCRAPE_SOURCES.iter().map(|s| s.to_string()).collect(),
            fields: meta::FIELDS.iter().map(|(k, _)| k.to_string()).collect(),
            media: meta::MEDIA.iter().map(|(k, _, _)| k.to_string()).chain(["manual".to_string()]).collect(),
            prefer: BTreeMap::new(),
            regions: ["us", "eu", "wor", "jp"].map(String::from).to_vec(),
            language: "en".into(),
            existing: "fill".into(),
            variants: meta::MEDIA.iter().map(|(k, _, _)| k.to_string()).filter(|k| k != "video").collect(),
            fresh: false,
        }
    }
}

/// The choices the last scrape saved (library.json `scrape`), else the defaults; only
/// the choices, not what it was for.
pub fn defaults(lib: &Library) -> Options {
    let mut o: Options = serde_json::from_value(lib.meta()["scrape"].clone()).unwrap_or_default();
    o.scope = "have".into();
    o.games.clear();
    o.fresh = false;
    o.sources.retain(|s| SCRAPE_SOURCES.contains(&s.as_str()));
    o
}

fn save_defaults(lib: &Library, o: &Options) -> Result<()> {
    let mut v = serde_json::to_value(o)?;
    if let Some(m) = v.as_object_mut() {
        for k in ["scope", "platform", "collection", "games", "fresh"] {
            m.remove(k);
        }
    }
    lib.update_meta(json!({ "scrape": v }))?;
    Ok(())
}

/// What the Scrape dialog offers: the choices, and each source with whether it's ready.
pub fn choices(cfg: &AppConfig, data_dir: &Path, lib: &Library) -> Value {
    let order = identify::order(cfg);
    let sources: Vec<Value> = SCRAPE_SOURCES
        .iter()
        .map(|s| {
            let on = order.iter().find(|(id, _)| id == s).map(|(_, on)| *on).unwrap_or(true);
            let mut needs = scrapers::needs(cfg, s);
            if *s == "launchbox" && launchbox::status(data_dir)["have"] != true {
                needs = Some("Get LaunchBox's database in Settings › Sources first.".into());
            }
            json!({ "id": s, "name": identify::source_name(s), "on": on, "needs": needs, "hashes": *s == "screenscraper" })
        })
        .collect();
    json!({
        "options": defaults(lib),
        "sources": sources,
        "fields": meta::FIELDS.iter().map(|(k, n)| json!({ "id": k, "name": n })).collect::<Vec<_>>(),
        "media": meta::MEDIA.iter().map(|(k, n, _)| json!({ "id": k, "name": n })).chain([json!({ "id": "manual", "name": "Manual" })]).collect::<Vec<_>>(),
        // what Delete metadata… offers (D48)
        "delete": sources::what_choices(),
        // and for a PC game, its kinds of artwork (D55)
        "delete_pc": sources::pc_what_choices(),
    })
}

/// The DAT games a scrape is for.
fn games_of(ix: &Index, lib: &Library, o: &Options) -> Result<Vec<i64>> {
    let conn = ix.conn();
    match o.scope.as_str() {
        "games" => Ok(o.games.clone()),
        "collection" => {
            let c = crate::collection::load(lib, &o.collection)?;
            let w = crate::collection::work_out(ix, lib, &c)?;
            Ok(w.games().filter(|g| g.platform != "pc").map(|g| g.id).collect())
        }
        scope => {
            let Some(dat) = crate::audit::primary_dat(conn, lib, &o.platform)? else {
                bail!("{} has no DAT, so its games aren't known: import one first.", crate::platforms::name_of(&o.platform));
            };
            let filter = if scope == "all" { "1" } else { "status != 'missing'" };
            let mut q = conn.prepare(&format!("WITH {} SELECT id FROM gs WHERE {filter} ORDER BY name", crate::audit::GAMES))?;
            let v = q.query_map([dat], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
            Ok(v)
        }
    }
}

/// Whether a name-only answer is the game: the same title, and the same year when both have one.
fn same_game(g: &DatGame, title: &str, year: Option<i32>) -> bool {
    let t = norm_title(title);
    let ok = t == norm_title(&g.title) || t == norm_title(&g.name);
    let dat_year = crate::dat::parse_name(&g.name).version.as_deref().and_then(identify::year_of);
    ok && (dat_year.is_none() || year.is_none() || dat_year == year)
}

/// Errors a source gives once and for good this run (its keys refused, its quota used).
fn lasting(e: &str) -> bool {
    ["didn't accept", "refused", "used today", "used this month", "Needs ", "doesn't accept", "closed"].iter().any(|w| e.contains(w))
}

struct Asked {
    found: Vec<Match>,
    candidates: Vec<Candidate>,
}

/// Ask the sources about a game, in order: what each was sure of, and the candidates of those that weren't.
fn ask(cfg: &AppConfig, data_dir: &Path, o: &Options, g: &DatGame, off: &mut HashSet<String>, errors: &mut BTreeMap<String, String>) -> Asked {
    let regions = scrapers::regions_for(&g.regions, &o.regions);
    let mut found = vec![];
    let mut candidates = vec![];
    for s in &o.sources {
        if off.contains(s) {
            continue;
        }
        let r: Result<(Option<Match>, Vec<Candidate>)> = (|| match s.as_str() {
            "screenscraper" => Ok((scrapers::ss_lookup(cfg, data_dir, g, &regions, &o.language)?, vec![])),
            "igdb" => {
                // its search answers with everything a game has
                let found = scrapers::igdb_search(cfg, data_dir, &g.title, &g.platform)?;
                match found.iter().position(|m| same_game(g, &m.title, m.year)) {
                    Some(i) => Ok((Some(found[i].clone()), vec![])),
                    None => Ok((
                        None,
                        found
                            .into_iter()
                            .map(|m| Candidate { score: launchbox::score(&g.title, &m.title), cover: m.art.get("cover").cloned(), url: None, source: m.source, platform: g.platform.clone(), id: m.id, title: m.title, year: m.year })
                            .filter(|c| c.score >= 50)
                            .take(3)
                            .collect(),
                    )),
                }
            }
            "thegamesdb" => {
                let found = scrapers::search(cfg, data_dir, s, &g.title, &g.platform)?;
                match found.iter().find(|c| same_game(g, &c.title, c.year)) {
                    Some(c) => Ok((Some(scrapers::details(cfg, data_dir, s, &c.id, &regions)?), vec![])),
                    None => Ok((None, found.into_iter().filter(|c| c.score >= 50).take(3).collect())),
                }
            }
            "launchbox" => {
                if launchbox::status(data_dir)["have"] != true {
                    bail!("Get LaunchBox's database in Settings › Sources first.");
                }
                let lb = launchbox::search(data_dir, &g.title, Some(&g.platform), 20)?;
                match lb.iter().find(|c| same_game(g, &c.name, c.year)) {
                    Some(c) => Ok((Some(identify::details(cfg, data_dir, "launchbox", &c.id.to_string())?), vec![])),
                    None => Ok((
                        None,
                        lb.iter()
                            .filter(|c| c.score >= 50)
                            .take(3)
                            .map(|c| Candidate {
                                source: "launchbox".into(), id: c.id.to_string(), title: c.name.clone(), year: c.year, platform: g.platform.clone(), cover: None,
                                url: Some(format!("https://gamesdb.launchbox-app.com/games/details/{}", c.id)), score: c.score,
                            })
                            .collect(),
                    )),
                }
            }
            _ => Ok((None, vec![])),
        })();
        match r {
            Ok((m, c)) => {
                if let Some(m) = m {
                    found.push(m);
                }
                candidates.extend(c);
            }
            Err(e) => {
                let e = format!("{e:#}");
                if lasting(&e) {
                    off.insert(s.clone());
                }
                errors.entry(s.clone()).or_insert(e);
            }
        }
    }
    Asked { found, candidates }
}

/// The source a kind of picture is chosen from: the one preferred for it, else the first that has it.
fn chosen_from<'a>(o: &Options, found: &'a [Match], kind: &str) -> Option<&'a Match> {
    o.prefer.get(kind).and_then(|p| found.iter().find(|m| &m.source == p && m.art.contains_key(kind))).or_else(|| found.iter().find(|m| m.art.contains_key(kind)))
}

/// Put what the sources said into a game's folder: each source's answer kept and the
/// details made from them (D46), pictures fetched, one from each source for the kinds
/// that keep them all (D47). Returns how many pictures (and manuals) were kept.
pub fn apply(ix: &Index, lib: &Library, g: &DatGame, found: &[Match], o: &Options, cancel: &AtomicBool) -> Result<usize> {
    let dir = meta::ensure_folder(ix, lib, g)?;
    let mut v = meta::read_game(&dir);
    let replace = o.existing == "replace";
    sources::prepare(&dir, &v)?;
    for m in found {
        sources::keep(&dir, m, &o.fields, replace)?;
    }
    if !found.is_empty() {
        if let Some(obj) = v.as_object_mut() {
            obj.remove("review");
        }
    }
    // what to fetch, kind by kind: the chosen source's, and every source's for the kinds
    // that keep them all; then all of it at once (as many as ScreenScraper's threads)
    let mut kinds: Vec<(&String, Vec<&Match>)> = vec![];
    for kind in &o.media {
        let def = chosen_from(o, found, kind);
        if kind == "manual" {
            let Some(m) = def else { continue };
            if docs::has_users(&dir, "manual") || (!replace && docs::has_scraped(&dir, "manual")) {
                continue;
            }
            kinds.push((kind, vec![m]));
            continue;
        }
        let every = o.variants.iter().any(|k| k == kind);
        if !every && !replace && sources::flat(&dir, kind).is_some() {
            continue;
        }
        let mut want: Vec<&Match> = def.into_iter().collect();
        if every {
            for m in found {
                if m.art.contains_key(kind) && !want.iter().any(|w| w.source == m.source) {
                    want.push(m);
                }
            }
        }
        let have = sources::names(&dir, &v, kind);
        want.retain(|m| replace || !have.contains(&m.source));
        kinds.push((kind, want));
    }
    let list: Vec<(&str, &str)> = if cancel.load(Ordering::Relaxed) { vec![] } else { kinds.iter().flat_map(|(k, ms)| ms.iter().map(|m| (m.art[k.as_str()].as_str(), k.as_str()))).collect() };
    let mut fetched = scrapers::fetch_all(&list).into_iter();
    let mut got = 0;
    for (kind, want) in &kinds {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let kind = *kind;
        let def = chosen_from(o, found, kind);
        if kind == "manual" {
            if let (Some(m), Some(Some((bytes, ext)))) = (want.first(), fetched.next()) {
                docs::save_scraped(lib, &dir, "manual", identify::source_name(&m.source), &m.source, &ext, &bytes)?;
                got += 1;
            }
            continue;
        }
        for m in want {
            let Some(Some((bytes, ext))) = fetched.next() else { continue };
            std::fs::create_dir_all(dir.join("media"))?;
            sources::store(&dir, &v, kind, &m.source, &ext, &bytes)?;
            got += 1;
        }
        if replace {
            // a source asked again that has none of this kind any more
            for m in found.iter().filter(|m| !m.art.contains_key(kind)) {
                sources::remove(&dir, &mut v, kind, &m.source);
            }
        }
        // which is shown: the user's pick, else (replacing, or nothing shown) the chosen source's
        let names = sources::names(&dir, &v, kind);
        let pinned = v["pinned_media"][kind].as_str().filter(|p| names.iter().any(|n| n == p)).map(String::from);
        let shown = sources::flat(&dir, kind).is_some();
        let target = pinned.or_else(|| def.map(|m| m.source.clone()).filter(|s| (replace || !shown) && names.contains(s))).or_else(|| (!shown).then(|| names.first().cloned()).flatten());
        if let Some(t) = target {
            sources::choose_in(&dir, &mut v, kind, &t)?;
        }
    }
    sources::make(&mut v, &dir, &sources::Rules::of(lib));
    meta::write_game(ix, lib, &dir, &v)?;
    Ok(got)
}

/// Keep a game's candidates when no source was sure (its folder is made for them).
fn keep_review(ix: &Index, lib: &Library, g: &DatGame, cands: &[Candidate]) -> Result<()> {
    let dir = meta::ensure_folder(ix, lib, g)?;
    let mut v = meta::read_game(&dir);
    let mut list: Vec<&Candidate> = cands.iter().collect();
    list.sort_by(|a, b| b.score.cmp(&a.score));
    v["review"] = json!(list.iter().take(6).map(|c| c.to_json()).collect::<Vec<_>>());
    v["review_asked"] = json!(crate::library::now());
    meta::write_game(ix, lib, &dir, &v)
}

/// Run a scrape (a job). `progress` gets `{ item, items, name }`.
pub fn run(ix: &Index, lib: &Library, cfg: &AppConfig, data_dir: &Path, o: &Options, cancel: &AtomicBool, progress: &mut dyn FnMut(Value)) -> Result<Value> {
    lib.writable()?;
    let mut o = o.clone();
    o.sources.retain(|s| SCRAPE_SOURCES.contains(&s.as_str()));
    if o.sources.is_empty() {
        bail!("Choose at least one source to ask.");
    }
    if o.fields.is_empty() && o.media.is_empty() {
        bail!("Choose at least one detail or picture to take.");
    }
    if o.scope != "games" {
        save_defaults(lib, &o)?;
    }
    meta::sync(ix, lib)?;
    let games = games_of(ix, lib, &o)?;
    // asking again: the answers kept are passed over until the scrape ends
    struct Fresh;
    impl Drop for Fresh {
        fn drop(&mut self) {
            scrapers::FRESH.store(false, Ordering::Relaxed);
        }
    }
    scrapers::FRESH.store(o.fresh, Ordering::Relaxed);
    let _fresh = Fresh;
    let (mut found_n, mut look, mut none, mut skipped, mut pics) = (0, 0, 0, 0, 0);
    let mut off: HashSet<String> = HashSet::new();
    let mut errors: BTreeMap<String, String> = BTreeMap::new();
    let mut stopped = false;
    // ScreenScraper's threads: that many games asked about at once, then put in their
    // folders one after another (only the asking is at once)
    let threads = if o.sources.iter().any(|s| s == "screenscraper") && !games.is_empty() { scrapers::ss_configure(cfg, data_dir) } else { 1 };
    let mut next = 0;
    'all: while next < games.len() {
        let mut batch: Vec<(usize, DatGame)> = vec![];
        while next < games.len() && batch.len() < threads {
            if cancel.load(Ordering::Relaxed) {
                stopped = true;
                break 'all;
            }
            let i = next;
            next += 1;
            let g = meta::dat_game(ix.conn(), games[i])?;
            if o.existing == "skip" {
                if let Some(f) = meta::folder_of(ix.conn(), &g)? {
                    let dir = lib.resolve(&f)?;
                    let v = meta::read_game(&dir);
                    if v["details"].as_object().is_some_and(|d| !d.is_empty()) || !meta::media_kinds(&dir).is_empty() {
                        progress(json!({ "item": i + 1, "items": games.len(), "name": g.name }));
                        skipped += 1;
                        continue;
                    }
                }
            }
            batch.push((i, g));
        }
        if batch.is_empty() {
            continue;
        }
        if o.sources.iter().all(|s| off.contains(s)) {
            break;
        }
        let answers: Vec<(Asked, HashSet<String>, BTreeMap<String, String>)> = if batch.len() == 1 {
            let (mut off1, mut err1) = (off.clone(), BTreeMap::new());
            vec![(ask(cfg, data_dir, &o, &batch[0].1, &mut off1, &mut err1), off1, err1)]
        } else {
            std::thread::scope(|sc| {
                let hs: Vec<_> = batch
                    .iter()
                    .map(|(_, g)| {
                        let (o, off) = (&o, off.clone());
                        sc.spawn(move || {
                            let (mut off, mut errs) = (off, BTreeMap::new());
                            (ask(cfg, data_dir, o, g, &mut off, &mut errs), off, errs)
                        })
                    })
                    .collect();
                hs.into_iter().map(|h| h.join().unwrap_or_else(|_| (Asked { found: vec![], candidates: vec![] }, HashSet::new(), BTreeMap::new()))).collect()
            })
        };
        for ((i, g), (asked, off1, err1)) in batch.iter().zip(answers) {
            off.extend(off1);
            for (k, e) in err1 {
                errors.entry(k).or_insert(e);
            }
            progress(json!({ "item": i + 1, "items": games.len(), "name": g.name }));
            if cancel.load(Ordering::Relaxed) {
                stopped = true;
                break 'all;
            }
            if !asked.found.is_empty() {
                found_n += 1;
                pics += apply(ix, lib, g, &asked.found, &o, cancel)?;
            } else if !asked.candidates.is_empty() {
                look += 1;
                keep_review(ix, lib, g, &asked.candidates)?;
            } else {
                none += 1;
            }
        }
    }
    // the manuals kept, listed with the documents
    docs::sync(ix, lib)?;
    let stopped_by_sources = !stopped && !games.is_empty() && o.sources.iter().all(|s| off.contains(s));
    Ok(json!({
        "games": games.len(), "found": found_n, "look": look, "none": none, "skipped": skipped, "pictures": pics,
        "stopped": stopped, "sources_stopped": stopped_by_sources,
        "errors": errors.iter().map(|(s, e)| json!({ "source": s, "name": identify::source_name(s), "error": e })).collect::<Vec<_>>(),
    }))
}

/// Identify settled for a DAT game: the source's details and pictures, as the scrape
/// defaults choose (replacing what's there), and its candidates gone.
pub fn pick(ix: &Index, lib: &Library, cfg: &AppConfig, data_dir: &Path, game: i64, source: &str, id: &str) -> Result<Value> {
    let g = meta::dat_game(ix.conn(), game)?;
    let regions = scrapers::regions_for(&g.regions, &defaults(lib).regions);
    let m = if SCRAPE_SOURCES.contains(&source) && source != "launchbox" { scrapers::details(cfg, data_dir, source, id, &regions)? } else { identify::details(cfg, data_dir, source, id)? };
    let mut o = defaults(lib);
    o.existing = "replace".into();
    o.prefer.clear();
    apply(ix, lib, &g, &[m], &o, &AtomicBool::new(false))?;
    docs::sync(ix, lib)?;
    meta::game(ix, lib, game)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::{Read, Write};

    /// A web server on 127.0.0.1 answering each request with the first body whose key is
    /// in the request (its line, headers and body); 404 otherwise.
    pub(crate) fn serve(routes: Vec<(String, Vec<u8>)>) -> String {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let mut s = s;
                let _ = s.set_read_timeout(Some(std::time::Duration::from_millis(500)));
                let mut req = vec![];
                let mut buf = [0u8; 16384];
                // the head, then a body if it says it has one
                loop {
                    let n = s.read(&mut buf).unwrap_or(0);
                    req.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&req).to_string();
                    if let Some(h) = text.find("\r\n\r\n") {
                        let len = text.lines().find_map(|l| l.to_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                        if req.len() >= h + 4 + len {
                            break;
                        }
                    }
                    if n == 0 {
                        break;
                    }
                }
                let text = String::from_utf8_lossy(&req).to_string();
                let line = text.lines().next().unwrap_or("").to_string();
                let body = routes.iter().find(|(k, _)| line.contains(k.as_str()) || (k.starts_with("BODY:") && text.contains(&k[5..]))).map(|(_, b)| b.clone());
                let (code, body) = match body {
                    Some(b) => ("200 OK", b),
                    None => ("404 Not Found", b"Erreur : Rom/Iso/Dossier non trouv\xc3\xa9e !".to_vec()),
                };
                let _ = write!(s, "HTTP/1.1 {code}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", body.len());
                let _ = s.write_all(&body);
            }
        });
        format!("http://{addr}")
    }

    pub(crate) fn png(w: u32, h: u32) -> Vec<u8> {
        let img = ::image::RgbImage::from_pixel(w, h, ::image::Rgb([200, 30, 30]));
        let mut out = std::io::Cursor::new(vec![]);
        img.write_to(&mut out, ::image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    /// The sources' stand-ins, and a config with every source pointed at them and keys set.
    pub(crate) fn sources() -> AppConfig {
        let crc = format!("{:08X}", crc32fast::hash(&crate::audit::tests::rom(2, 1024)));
        let ss = json!({ "header": { "success": "true" }, "response": { "ssuser": { "maxthreads": "1" }, "jeu": {
            "id": "1001", "noms": [{ "region": "ss", "text": "Alpha SS" }, { "region": "eu", "text": "Alpha" }],
            "synopsis": [{ "langue": "fr", "text": "En français" }, { "langue": "en", "text": "Alpha&#039;s quest." }],
            "editeur": { "text": "Alpha Pub" }, "developpeur": { "id": "3", "text": "Alpha Dev" }, "joueurs": { "text": "1-2" }, "note": { "text": "16" },
            "dates": [{ "region": "eu", "text": "1999-02-24" }],
            "genres": [{ "noms": [{ "langue": "en", "text": "Platform" }, { "langue": "fr", "text": "Plateforme" }] }],
            "medias": [
                { "type": "box-2D", "parent": "jeu", "region": "jp", "url": "BASE/media/cover-jp.png", "format": "png" },
                { "type": "box-2D", "parent": "jeu", "region": "eu", "url": "BASE/media/cover.png", "format": "png" },
                { "type": "ss", "parent": "jeu", "url": "BASE/media/shot.png", "format": "png" },
                { "type": "manuel", "parent": "jeu", "region": "eu", "url": "BASE/media/manual.pdf", "format": "pdf" },
                { "type": "box-2D", "parent": "roms", "region": "eu", "url": "BASE/media/nope.png" }
            ] } } });
        let pdf = [b"%PDF-1.4\n".to_vec(), vec![b' '; 200]].concat();
        let routes = vec![
            // the account: three threads at once
            ("ssuserInfos.php".to_string(), br#"{"response":{"ssuser":{"id":"me","maxthreads":"3","maxrequestspermin":"600"}}}"#.to_vec()),
            (format!("crc={crc}"), ss.to_string().into_bytes()),
            ("/media/cover.png".to_string(), png(600, 400)),
            ("/media/shot.png".to_string(), png(320, 240)),
            ("/media/manual.pdf".to_string(), pdf),
            ("/tw/oauth2/token".to_string(), br#"{"access_token":"tok","expires_in":5000,"token_type":"bearer"}"#.to_vec()),
            ("BODY:search \"Beta\"".to_string(), br#"[{"id":77,"name":"Beta","summary":"Beta from IGDB.","first_release_date":946684800,"cover":{"image_id":"co77"},"genres":[{"name":"Puzzle"}],"involved_companies":[{"company":{"name":"Beta Co"},"developer":true,"publisher":true}]}]"#.to_vec()),
            ("BODY:search \"Alpha\"".to_string(), br#"[{"id":55,"name":"Alpha","summary":"Alpha from IGDB.","cover":{"image_id":"co77"}}]"#.to_vec()),
            ("BODY:search \"Delta\"".to_string(), br#"[{"id":88,"name":"Delta Force","summary":"Not quite."}]"#.to_vec()),
            ("BODY:where id = 88".to_string(), br#"[{"id":88,"name":"Delta Force","summary":"Picked by hand."}]"#.to_vec()),
            ("BODY:search ".to_string(), b"[]".to_vec()),
            ("/img/t_1080p/co77.jpg".to_string(), {
                let img = ::image::RgbImage::from_pixel(30, 40, ::image::Rgb([1, 2, 3]));
                let mut out = std::io::Cursor::new(vec![]);
                img.write_to(&mut out, ::image::ImageFormat::Jpeg).unwrap();
                out.into_inner()
            }),
        ];
        // the media URLs name the server, which isn't known until it's started: a second one forwards
        let base = serve(routes.clone());
        let ss = ss.to_string().replace("BASE", &base);
        let mut routes = routes;
        routes[1].1 = ss.into_bytes();
        let base = serve(routes);
        let mut cfg = AppConfig::default();
        for (k, u) in [("screenscraper", "/ss"), ("igdb", "/igdb"), ("twitch", "/tw"), ("igdb_images", "/img"), ("thegamesdb", "/tgdb")] {
            cfg.source_urls.insert(k.into(), format!("{base}{u}"));
        }
        cfg.source_keys.insert("screenscraper".into(), [("devid", "d"), ("devpassword", "p")].map(|(a, b)| (a.to_string(), b.to_string())).into_iter().collect());
        cfg.source_keys.insert("igdb".into(), [("client_id", "c"), ("client_secret", "s")].map(|(a, b)| (a.to_string(), b.to_string())).into_iter().collect());
        cfg
    }

    #[test]
    fn a_systems_games_are_scraped_by_hash_and_by_name_and_the_unsure_need_a_look() {
        let (dir, lib, ix) = crate::collection::tests::fixture("scrape");
        let data = dir.join("data");
        let mut cfg = sources();
        let o = Options { platform: "gb".into(), sources: vec!["screenscraper".into(), "igdb".into(), "thegamesdb".into()], ..Default::default() };
        let cancel = AtomicBool::new(false);
        let mut items = vec![];
        let r = run(&ix, &lib, &cfg, &data, &o, &cancel, &mut |p| items.push(p["item"].as_u64().unwrap())).unwrap();
        // asked three games at a time, as the account allows, and each game said once, in order
        assert_eq!(scrapers::ss_configure(&cfg, &data), 3);
        assert_eq!(items, (1..=8).collect::<Vec<u64>>());
        // fewer, if Settings says so
        cfg.source_keys.get_mut("screenscraper").unwrap().insert("threads".into(), "2".into());
        assert_eq!(scrapers::ss_configure(&cfg, &data), 2);
        cfg.source_keys.get_mut("screenscraper").unwrap().remove("threads");
        // eight games on disk: Alpha (Europe) by its hash (and by name), the other Alpha and the three Betas by name, Delta unsure
        assert_eq!((r["games"].as_u64(), r["found"].as_u64(), r["look"].as_u64()), (Some(8), Some(5), Some(1)), "{r}");
        // TheGamesDB has no key: said once, the others still asked
        assert_eq!(r["errors"][0]["source"], "thegamesdb", "{r}");
        let alpha = dir.join("lib/_library/games/gb/Alpha (Europe)");
        let v = meta::read_game(&alpha);
        assert_eq!((v["title"].as_str(), v["details"]["description"].as_str()), (Some("Alpha"), Some("Alpha's quest.")), "{v}");
        assert_eq!((v["details"]["rating"].as_f64(), v["details"]["genres"][0].as_str(), v["ids"]["screenscraper"].as_str()), (Some(0.8), Some("Platform"), Some("1001")));
        assert_eq!(v["roms"][0]["name"], "Alpha (Europe).gb");
        assert_eq!(meta::media_kinds(&alpha), ["cover", "screenshot"]);
        // the European box, not the Japanese; IGDB's kept beside it, and what it said
        assert_eq!(std::fs::read(alpha.join("media/cover.png")).unwrap(), png(600, 400));
        assert!(alpha.join("media/cover/igdb.jpg").is_file());
        assert_eq!(crate::sources::answers(&alpha)["igdb"]["details"]["description"], "Alpha from IGDB.");
        assert!(alpha.join("docs/Manual (ScreenScraper).pdf").is_file());
        let beta = meta::read_game(&dir.join("lib/_library/games/gb/Beta (Japan)"));
        assert_eq!((beta["details"]["developer"].as_str(), beta["year"].as_i64()), (Some("Beta Co"), Some(2000)));
        // the page's list and counts
        let l = meta::list(&ix, &lib, "gb", "have", "", 0, 50).unwrap();
        assert_eq!((l["counts"]["have"].as_u64(), l["counts"]["details"].as_u64(), l["counts"]["look"].as_u64()), (Some(8), Some(5), Some(1)), "{l}");
        let look = meta::list(&ix, &lib, "gb", "look", "", 0, 50).unwrap();
        assert_eq!(look["items"][0]["name"], "Delta (World)");
        let a = l["items"].as_array().unwrap().iter().find(|x| x["name"] == "Alpha (Europe)").unwrap();
        assert_eq!((a["docs"].as_i64(), a["media"].as_array().unwrap().len()), (Some(1), 2));
        let gid = a["id"].as_i64().unwrap();
        let g = meta::game(&ix, &lib, gid).unwrap();
        assert_eq!((g["docs"][0]["kind"].as_str(), g["docs"][0]["source"].as_str()), (Some("manual"), Some("screenscraper")), "{g}");
        assert!(g["media"]["cover"].as_str().unwrap().ends_with("media/cover.png"));
        assert_eq!((g["sources"].as_array().map(Vec::len), g["variants"]["cover"].as_array().map(Vec::len)), (Some(2), Some(2)), "{g}");
        // IGDB's description and box chosen on its page; scraping again keeps them
        crate::sources::pin(&ix, &lib, &alpha, "description", Some("igdb")).unwrap();
        crate::sources::choose(&ix, &lib, &alpha, "cover", "igdb").unwrap();
        let again = Options { scope: "games".into(), games: vec![gid], existing: "replace".into(), fresh: true, ..o.clone() };
        let r = run(&ix, &lib, &cfg, &data, &again, &cancel, &mut |_| {}).unwrap();
        assert_eq!(r["found"], 1, "{r}");
        assert!(!scrapers::FRESH.load(Ordering::Relaxed));
        let v = meta::read_game(&alpha);
        assert_eq!((v["details"]["description"].as_str(), v["chosen"]["cover"].as_str()), (Some("Alpha from IGDB."), Some("igdb")), "{v}");
        assert!(alpha.join("media/cover.jpg").is_file() && alpha.join("media/cover/screenscraper.png").is_file());
        // again: filling in what's missing fetches nothing new
        let r = run(&ix, &lib, &cfg, &data, &o, &cancel, &mut |_| {}).unwrap();
        assert_eq!(r["pictures"], 0, "{r}");
        // Delta settled by hand, then Alpha cleared (its manual goes; a document of the user's would stay)
        let delta = look["items"][0]["id"].as_i64().unwrap();
        let d = pick(&ix, &lib, &cfg, &data, delta, "igdb", "88").unwrap();
        assert_eq!((d["details"]["description"].as_str(), d["review"].as_array().map(Vec::len)), (Some("Picked by hand."), Some(0)), "{d}");
        assert_eq!(meta::list(&ix, &lib, "gb", "look", "", 0, 50).unwrap()["total"], 0);
        let c = meta::clear(&ix, &lib, gid).unwrap();
        assert!(c["media"].as_object().unwrap().is_empty() && c["docs"].as_array().unwrap().is_empty(), "{c}");
        // every game in the DAT: Alpha (USA) isn't on disk but ScreenScraper knows its hashes (it doesn't here)
        let all = Options { scope: "all".into(), existing: "skip".into(), ..o.clone() };
        let r = run(&ix, &lib, &cfg, &data, &all, &cancel, &mut |_| {}).unwrap();
        assert_eq!((r["games"].as_u64(), r["skipped"].as_u64()), (Some(10), Some(5)), "{r}");
        // the choices are saved for next time, without what they were for
        assert_eq!(defaults(&lib).sources, o.sources);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
