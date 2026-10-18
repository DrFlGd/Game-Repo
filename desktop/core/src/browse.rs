//! The Library page (docs/PLAN.md, "Phase 6 design", D42, D43): every game the library
//! holds, across its systems, as rows of the index's `library` table, rebuilt one system
//! at a time when what that system's games depend on changes; searched, filtered with
//! counts, sorted and paged in SQL. Thumbnails of the games' pictures are a cache on
//! this computer.

use crate::audit::{self, GAMES};
use crate::index::Index;
use crate::library::Library;
use crate::{collection, docs, meta, pcgames, platforms};
use anyhow::{anyhow, bail, Result};
use rusqlite::types::Value as Sql;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex};

/// Bumped when what a row holds changes, so every system is rebuilt once.
const FORMAT: &str = "3";
/// Between the genres in a row's `genres` (a genre may have a comma in it).
const SEP: &str = "\u{1f}";

/// A system's name ("PC" for the PC games, which aren't one of platforms.rs's).
pub(crate) fn system_name(id: &str) -> String {
    if id == "pc" { "PC".into() } else { platforms::name_of(id) }
}

/// The title a game sorts by: lower-case, "The", "A" and "An" at its start put last.
pub fn sort_key(title: &str) -> String {
    let t = title.trim().to_lowercase();
    for a in ["the ", "a ", "an "] {
        if let Some(rest) = t.strip_prefix(a) {
            return format!("{rest}, {}", a.trim());
        }
    }
    t
}

/// The most players a game's `players` detail says ("1-4" is 4).
fn players_of(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.split(|c: char| !c.is_ascii_digit()).filter_map(|x| x.parse::<i64>().ok()).max(),
        _ => None,
    }
}

fn text(v: &Value) -> Option<String> {
    v.as_str().map(str::trim).filter(|s| !s.is_empty()).map(String::from)
}

fn list_of(v: &Value) -> Vec<String> {
    match v {
        Value::Array(a) => a.iter().filter_map(text).collect(),
        Value::String(s) => s.split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect(),
        _ => vec![],
    }
}

fn split(s: &str) -> Vec<String> {
    s.split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect()
}

/// One row of the library, before it's written.
#[derive(Default)]
struct Row {
    key: String,
    kind: &'static str,
    platform: String,
    dat_game: Option<i64>,
    name: String,
    title: String,
    year: Option<i64>,
    facts: Map<String, Value>,
    regions: Vec<String>,
    languages: Vec<String>,
    have: bool,
    folder: Option<String>,
    picture: Option<String>,
    media: String,
    docs: i64,
    review: bool,
    size: i64,
    date: Option<i64>,
    best_all: bool,
    best_have: bool,
    /// How many of its updates, DLC and patches you have (D66)
    addons: i64,
}

fn write_rows(conn: &Connection, platform: &str, rows: &[Row]) -> Result<()> {
    conn.execute("DELETE FROM library_text WHERE rowid IN (SELECT id FROM library WHERE platform = ?1)", [platform])?;
    conn.execute("DELETE FROM library_values WHERE game IN (SELECT id FROM library WHERE platform = ?1)", [platform])?;
    conn.execute("DELETE FROM library WHERE platform = ?1", [platform])?;
    let mut ins = conn.prepare_cached(
        "INSERT INTO library (key, kind, platform, dat_game, name, title, sort, year, developer, publisher, players, rating, regions, have,
                              folder, picture, media, described, docs, review, size, date, best_all, best_have, genre, genres, languages,
                              year_sort, rating_sort, size_sort, date_sort, addons)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27,
                 ?28, ?29, ?30, ?31, ?32)",
    )?;
    let mut val = conn.prepare_cached("INSERT INTO library_values (game, field, value) VALUES (?1, ?2, ?3)")?;
    let mut fts = conn.prepare_cached("INSERT INTO library_text (rowid, title, name, body) VALUES (?1, ?2, ?3, ?4)")?;
    // a key met twice (a PC game's folder copied with its game.json, so the same id) is
    // listed once, not a reason for the whole Library to fail
    let mut keys: HashSet<&str> = HashSet::new();
    for r in rows {
        if !keys.insert(&r.key) {
            continue;
        }
        let f = &r.facts;
        let developer = text(&f.get("developer").cloned().unwrap_or_default());
        let publisher = text(&f.get("publisher").cloned().unwrap_or_default());
        let get = |k: &str| f.get(k).cloned().unwrap_or(Value::Null);
        let description = text(&get("description"));
        let year = r.year.or_else(|| text(&get("released")).and_then(|d| d.get(..4).and_then(|y| y.parse().ok())));
        let genres = list_of(&get("genres"));
        let rating = get("rating").as_f64();
        ins.execute(params![
            r.key, r.kind, r.platform, r.dat_game, r.name, r.title, sort_key(&r.title), year, developer, publisher,
            players_of(&get("players")), rating, r.regions.join(","), r.have as i64, r.folder, r.picture, r.media,
            description.is_some() as i64, r.docs, r.review as i64, r.size, r.date, r.best_all as i64, r.best_have as i64, genres.first(),
            genres.join(SEP), r.languages.join(","),
            year.unwrap_or(i64::MAX), rating.map(|x| -x).unwrap_or(f64::MAX), -r.size, r.date.map(|d| -d).unwrap_or(i64::MAX), r.addons,
        ])?;
        let id = conn.last_insert_rowid();
        for g in &genres {
            val.execute(params![id, "genre", g])?;
        }
        for g in &r.regions {
            val.execute(params![id, "region", g])?;
        }
        for g in &r.languages {
            val.execute(params![id, "language", g])?;
        }
        let body = [developer.as_deref(), publisher.as_deref(), Some(&genres.join(" ")), description.as_deref()].into_iter().flatten().collect::<Vec<_>>().join(" ");
        fts.execute(params![id, r.title, r.name, body])?;
    }
    Ok(())
}

/// The pictures of a game folder's `media/`, as the folder's library path and a file name,
/// with the folder's stamp after it (`?v=`, which the page's server leaves out) so a picture
/// changed under the same name (another source's chosen, D47) isn't shown from the cache.
fn picture_path(folder: &Option<String>, picture: &Option<String>, stamp: i64) -> Option<String> {
    Some(format!("{}/media/{}?v={stamp}", folder.as_ref()?, picture.as_ref()?))
}

/// Rebuild a system's rows: every game of the DAT it's audited against but its updates and
/// DLC (they're counted with their game, D66), and the games no DAT knows that Add a game…
/// kept for it.
fn rebuild_system(conn: &Connection, lib: &Library, platform: &str, dat: Option<i64>) -> Result<()> {
    let mut rows: Vec<Row> = vec![];
    let mut used: HashSet<String> = HashSet::new();
    let addons = crate::addons::have_by_base(conn, lib, platform)?;
    if let Some(dat) = dat {
        let addon_dat = conn.query_row("SELECT name FROM dats WHERE id = ?1", [dat], |r| r.get::<_, String>(0)).map(|n| crate::addons::is_addon_dat(&n))?;
        let sql = format!(
            "WITH {GAMES},
                  fv AS (SELECT r.game_id, mr.folder, COUNT(*) AS n FROM dat_roms r JOIN meta_roms mr ON mr.crc32 = r.crc32 AND mr.size = r.size
                         JOIN meta_games mg ON mg.folder = mr.folder AND mg.platform = ?2 WHERE r.dat_id = ?1 GROUP BY r.game_id, mr.folder),
                  gr AS (SELECT game_id, COUNT(*) AS n FROM dat_roms WHERE dat_id = ?1 AND crc32 IS NOT NULL AND size IS NOT NULL GROUP BY game_id),
                  fr AS (SELECT folder, COUNT(*) AS n FROM meta_roms GROUP BY folder),
                  whole AS (SELECT fv.game_id, MIN(fv.folder) AS folder FROM fv JOIN gr ON gr.game_id = fv.game_id AND gr.n = fv.n
                            JOIN fr ON fr.folder = fv.folder AND fr.n = fv.n GROUP BY fv.game_id),
                  fm AS (SELECT gs.id AS game_id, COALESCE(whole.folder, (SELECT mg.folder FROM meta_games mg WHERE mg.platform = ?2 AND mg.name = gs.name ORDER BY mg.folder LIMIT 1)) AS folder
                         FROM gs LEFT JOIN whole ON whole.game_id = gs.id)
             SELECT gs.id, gs.name, COALESCE(gs.title, ''), COALESCE(gs.regions, ''), COALESCE(g.languages, ''), COALESCE(g.revision, ''),
                    COALESCE(g.version, ''), COALESCE(gs.flags, ''), gs.parent_id, gs.status,
                    mg.folder, mg.title, mg.year, COALESCE(mg.media, ''), COALESCE(mg.review, 0), COALESCE(mg.facts, '{{}}'), mg.picture,
                    (SELECT COUNT(*) FROM docs dc WHERE dc.folder = mg.folder),
                    (SELECT MAX(f.mtime) FROM dat_roms r JOIN matches m ON m.rom_id = r.id JOIN files f ON f.id = m.file_id WHERE r.game_id = gs.id),
                    COALESCE(mg.mtime, 0)
             FROM gs JOIN dat_games g ON g.id = gs.id LEFT JOIN fm ON fm.game_id = gs.id LEFT JOIN meta_games mg ON mg.folder = fm.folder"
        );
        let sizes = collection::sizes(conn, dat)?;
        let mut q = conn.prepare(&sql)?;
        let mut it = q.query(params![dat, platform])?;
        // (row, its 1G1R group, how it ranks in it)
        let mut ranks: Vec<(i64, (usize, bool, std::cmp::Reverse<Vec<u64>>, std::cmp::Reverse<Vec<u64>>, bool, String))> = vec![];
        let order = collection::Rules::default().regions;
        let rank = |regions: &str| split(regions).iter().filter_map(|g| order.iter().position(|x| x.eq_ignore_ascii_case(g))).min().unwrap_or(order.len());
        while let Some(x) = it.next()? {
            let id: i64 = x.get(0)?;
            let name: String = x.get(1)?;
            let dat_title: String = x.get(2)?;
            let regions: String = x.get(3)?;
            let flags: String = x.get(7)?;
            let parent: Option<i64> = x.get(8)?;
            let status: String = x.get(9)?;
            let folder: Option<String> = x.get(10)?;
            let meta_title: Option<String> = x.get(11)?;
            let facts: String = x.get(15)?;
            let picture: Option<String> = x.get(16)?;
            if !addon_dat && crate::addons::kind_of(&name, false).is_some() {
                continue;
            }
            let have_addons = addons.get(&name).copied().unwrap_or(0);
            if let Some(f) = &folder {
                used.insert(f.clone());
            }
            ranks.push((
                parent.unwrap_or(id),
                (rank(&regions), !flags.is_empty(), std::cmp::Reverse(collection::revision_key(&x.get::<_, String>(5)?)), std::cmp::Reverse(collection::revision_key(&x.get::<_, String>(6)?)), parent.is_some(), name.clone()),
            ));
            rows.push(Row {
                key: format!("{platform}/{name}"),
                kind: "dat",
                platform: platform.to_string(),
                dat_game: Some(id),
                title: meta_title.filter(|t| !t.is_empty()).or_else(|| Some(dat_title).filter(|t| !t.is_empty())).unwrap_or_else(|| name.clone()),
                name,
                year: x.get(12)?,
                facts: serde_json::from_str(&facts).unwrap_or_default(),
                regions: split(&regions),
                languages: split(&x.get::<_, String>(4)?),
                have: status != "missing",
                picture: picture_path(&folder, &picture, x.get(19)?),
                folder,
                media: x.get(13)?,
                review: x.get::<_, i64>(14)? == 1,
                docs: x.get(17)?,
                size: sizes.get(&id).map(|s| s.0 as i64).unwrap_or(0),
                date: x.get(18)?,
                addons: have_addons,
                ..Default::default()
            });
        }
        // one per title (1G1R): the best of each group, in the collection rules' region order,
        // among every game and among those you have
        for have_only in [false, true] {
            let mut best: HashMap<i64, usize> = HashMap::new();
            for (i, (group, k)) in ranks.iter().enumerate() {
                if have_only && !rows[i].have {
                    continue;
                }
                match best.get(group) {
                    Some(&b) if ranks[b].1 <= *k => {}
                    _ => {
                        best.insert(*group, i);
                    }
                }
            }
            let chosen: HashSet<usize> = best.into_values().collect();
            for (i, r) in rows.iter_mut().enumerate() {
                if have_only { r.best_have = chosen.contains(&i) } else { r.best_all = chosen.contains(&i) }
            }
        }
    }
    // a DAT can list two different games under one name (`dat::parse` keeps both when
    // their files differ, as libretro's copies do): the later ones by the DAT's order get
    // `~2`, `~3`… after the name, so each has a key of its own
    let mut order: Vec<usize> = (0..rows.len()).collect();
    order.sort_by_key(|&i| rows[i].dat_game);
    let mut keys: HashSet<String> = HashSet::new();
    for i in order {
        let mut n = 1;
        while !keys.insert(rows[i].key.clone()) {
            n += 1;
            rows[i].key = format!("{platform}/{}~{n}", rows[i].name);
        }
    }
    // games no DAT knows (Add a game…): their folder is named by the file's SHA1
    let mut q = conn.prepare("SELECT folder, name, title, year, media, review, facts, picture, (SELECT COUNT(*) FROM docs dc WHERE dc.folder = mg.folder), COALESCE(mtime, 0) FROM meta_games mg WHERE platform = ?1")?;
    let mut it = q.query([platform])?;
    while let Some(x) = it.next()? {
        let folder: String = x.get(0)?;
        let last = folder.rsplit('/').next().unwrap_or("").to_string();
        if used.contains(&folder) || last.len() != 40 || !last.chars().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        let name: Option<String> = x.get(1)?;
        let title: Option<String> = x.get(2)?;
        let picture: Option<String> = x.get(7)?;
        let facts: String = x.get(6)?;
        let folder = Some(folder);
        rows.push(Row {
            key: format!("{platform}/~{last}"),
            kind: "added",
            platform: platform.to_string(),
            name: name.clone().unwrap_or_else(|| last.clone()),
            title: title.filter(|t| !t.is_empty()).or(name).unwrap_or(last),
            year: x.get(3)?,
            facts: serde_json::from_str(&facts).unwrap_or_default(),
            have: true,
            picture: picture_path(&folder, &picture, x.get(9)?),
            folder,
            media: x.get(4)?,
            review: x.get::<_, i64>(5)? == 1,
            docs: x.get(8)?,
            best_all: true,
            best_have: true,
            ..Default::default()
        });
    }
    write_rows(conn, platform, &rows)
}

/// A PC game's metadata folder's stamp: its `game.json`'s size and modified time added to its
/// `media/` folder's, as a DAT game's (`meta::stamp`).
fn pc_stamp(dir: &Path) -> i64 {
    let ms = |p: &Path| std::fs::metadata(p).ok().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64).unwrap_or(0);
    std::fs::metadata(dir.join("game.json")).map(|m| m.len() as i64).unwrap_or(0) + ms(&dir.join("game.json")) + ms(&dir.join("media"))
}

/// Rebuild the PC games' rows: what only a PC game has from `PC/…/game.json`, its details
/// and pictures from its metadata folder (D55).
fn rebuild_pc(conn: &Connection, lib: &Library) -> Result<()> {
    let mut rows = vec![];
    for e in pcgames::list(lib) {
        let g = &e.game;
        let media = pcgames::media(lib, &g.id);
        let dir = crate::pcmeta::dir(lib, &g.id);
        let folder = lib.relative(&dir);
        let stamp = pc_stamp(&dir);
        let picture = ["cover", "box3d", "hero", "banner"].iter().find_map(|k| media.get(*k).and_then(Value::as_str).map(|p| format!("{p}?v={stamp}")));
        let docs: i64 = match &folder {
            Some(f) => conn.query_row("SELECT COUNT(*) FROM docs WHERE folder = ?1", [f], |r| r.get(0))?,
            None => 0,
        };
        rows.push(Row {
            key: format!("pc/{}", g.id),
            kind: "pc",
            platform: "pc".into(),
            name: e.dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            title: g.title.clone(),
            year: g.year.map(i64::from),
            facts: crate::pcmeta::view(lib, &e)["details"].as_object().cloned().unwrap_or_default(),
            have: true,
            media: media.keys().cloned().collect::<Vec<_>>().join(","),
            picture,
            folder,
            docs,
            size: g.files.iter().map(|f| f.size as i64).sum(),
            date: std::fs::metadata(e.dir.join("game.json")).ok().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64),
            best_all: true,
            best_have: true,
            ..Default::default()
        });
    }
    write_rows(conn, "pc", &rows)
}

/// What a system's rows depend on, so they're rebuilt only when it changes.
fn stamp(conn: &Connection, lib: &Library, platform: &str) -> Result<(String, Option<i64>)> {
    if platform == "pc" {
        let mut parts = vec![];
        for e in std::fs::read_dir(pcgames::folder(lib)).into_iter().flatten().flatten() {
            if let Ok(m) = std::fs::metadata(e.path().join("game.json")) {
                parts.push(format!("{}:{}:{:?}", e.file_name().to_string_lossy(), m.len(), m.modified().ok()));
            }
        }
        // and each one's metadata folder (its details and pictures, D55)
        for e in pcgames::list(lib) {
            parts.push(format!("{}:{}", e.game.id, pc_stamp(&crate::pcmeta::dir(lib, &e.game.id))));
        }
        parts.sort();
        let media: i64 = conn.query_row("SELECT COUNT(*) FROM docs WHERE platform = 'pc'", [], |r| r.get(0))?;
        let shown: usize = pcgames::list(lib).iter().map(|e| pcgames::media(lib, &e.game.id).len()).sum();
        return Ok((format!("{FORMAT}|{}|{media}|{shown}", parts.join(",")), None));
    }
    let dat = audit::primary_dat(conn, lib, platform)?;
    let matched: (i64, i64) = match dat {
        Some(d) => conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(m.file_id), 0) FROM dat_roms r JOIN matches m ON m.rom_id = r.id WHERE r.dat_id = ?1",
            [d],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?,
        None => (0, 0),
    };
    let games: (i64, i64, i64) = conn.query_row("SELECT COUNT(*), COALESCE(SUM(mtime), 0), COALESCE(SUM(size), 0) FROM meta_games WHERE platform = ?1", [platform], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    let pictures: String = conn.query_row("SELECT COALESCE(group_concat(COALESCE(picture, '') || media, '|'), '') FROM meta_games WHERE platform = ?1", [platform], |r| r.get(0))?;
    let docs: (i64, i64) = conn.query_row("SELECT COUNT(*), COALESCE(SUM(mtime), 0) FROM docs WHERE platform = ?1", [platform], |r| Ok((r.get(0)?, r.get(1)?)))?;
    let pics = {
        use sha2::Digest;
        hex::encode(&sha2::Sha256::digest(pictures.as_bytes())[..8])
    };
    // and its updates and DLC (in its other DATs, and linked by hand)
    let others: (i64, i64) = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(m.file_id), 0) FROM dat_roms r JOIN matches m ON m.rom_id = r.id JOIN dats d ON d.id = r.dat_id WHERE d.platform = ?1 AND d.current = 1",
        [platform],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let hand = lib.meta()["addons"][platform].to_string();
    Ok((format!("{FORMAT}|{dat:?}|{matched:?}|{games:?}|{pics}|{docs:?}|{others:?}|{hand}"), dat))
}

/// Bring the library's rows up to date: each system whose games, files, game folders or
/// documents changed is rebuilt (in its own transaction), the others are left. Returns
/// the systems rebuilt.
pub fn refresh(ix: &Index, lib: &Library) -> Result<Vec<String>> {
    let conn = ix.conn();
    let mut systems: Vec<String> = conn
        .prepare("SELECT DISTINCT platform FROM dats WHERE current = 1 AND platform != ?1 UNION SELECT DISTINCT platform FROM meta_games WHERE platform != 'pc'")?
        .query_map([crate::bios::ID], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    systems.push("pc".into());
    let mut done = vec![];
    for p in &systems {
        let (st, dat) = stamp(conn, lib, p)?;
        let key = format!("library:{p}");
        let old: Option<String> = conn.query_row("SELECT value FROM index_meta WHERE key = ?1", [&key], |r| r.get(0)).optional()?;
        if old.as_deref() == Some(st.as_str()) {
            continue;
        }
        // IMMEDIATE: a rebuild reads before it writes, and a deferred transaction can't
        // take the write lock once another connection (a job's) has written in between
        let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
        if p == "pc" { rebuild_pc(&tx, lib)? } else { rebuild_system(&tx, lib, p, dat)? }
        tx.execute("INSERT INTO index_meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value", params![key, st])?;
        tx.commit()?;
        done.push(p.clone());
    }
    if !done.is_empty() {
        // the planner's statistics, for the filters' queries
        conn.execute_batch("PRAGMA optimize")?;
    }
    // systems that have gone (their last DAT removed)
    let gone: Vec<String> = conn.prepare("SELECT DISTINCT platform FROM library")?.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?.into_iter().filter(|p| !systems.contains(p)).collect();
    for p in gone {
        let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
        write_rows(&tx, &p, &[])?;
        tx.execute("DELETE FROM index_meta WHERE key = ?1", [format!("library:{p}")])?;
        tx.commit()?;
    }
    Ok(done)
}

// ------------------------------------------------------------ the query

/// What the page asks for: the filters (values of one filter OR'd, filters AND'd), the
/// search, the sort and a page.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Query {
    pub search: String,
    /// Every game in the DATs, not only the games you have
    pub all: bool,
    pub one_per_title: bool,
    pub platform: Vec<String>,
    pub genre: Vec<String>,
    pub decade: Vec<i64>,
    pub developer: Vec<String>,
    pub publisher: Vec<String>,
    pub region: Vec<String>,
    pub language: Vec<String>,
    /// At least this many players
    pub players: i64,
    /// At least this rating (0 to 1)
    pub rating: f64,
    /// pictures | description | manual | video | documents | addons (each wanted)
    pub has: Vec<String>,
    /// Only games needing a look
    pub look: bool,
    /// Only a collection's games (its id)
    pub collection: String,
    /// title | system | year (oldest first) | rating (highest first) | size (biggest first) | date (newest first)
    pub sort: String,
    pub desc: bool,
    /// system | genre (its first) | decade | developer | publisher (sorted by it first; each row says its group)
    pub group: String,
    pub offset: i64,
    pub limit: i64,
    /// With the filters' counts
    pub facets: bool,
}

/// The words of a search as an FTS5 query: each a prefix, all wanted.
fn fts_query(s: &str) -> Option<String> {
    let words: Vec<String> = s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(|w| format!("\"{}\"*", w.to_lowercase())).collect();
    (!words.is_empty()).then(|| words.join(" "))
}

fn json_list<T: serde::Serialize>(v: &[T]) -> Sql {
    Sql::Text(serde_json::to_string(v).unwrap_or_else(|_| "[]".into()))
}

/// The WHERE of a query, leaving out the filter `skip` (a filter's own counts are worked
/// out without it, so its other values can still be chosen).
fn where_of(q: &Query, keys: &Option<Vec<String>>, skip: &str) -> (String, Vec<Sql>) {
    let mut w: Vec<String> = vec![];
    let mut p: Vec<Sql> = vec![];
    if !q.all {
        w.push("l.have = 1".into());
    }
    if q.one_per_title {
        w.push(if q.all { "l.best_all = 1" } else { "l.best_have = 1" }.into());
    }
    if let Some(f) = fts_query(&q.search) {
        w.push("l.id IN (SELECT rowid FROM library_text WHERE library_text MATCH ?)".into());
        p.push(Sql::Text(f));
    }
    let list = |name: &str, col: &str, v: &[String], w: &mut Vec<String>, p: &mut Vec<Sql>| {
        if v.is_empty() || skip == name {
            return;
        }
        // one value as `=`, so the planner can use its index
        if let [one] = v {
            w.push(format!("{col} = ?"));
            p.push(Sql::Text(one.clone()));
        } else {
            w.push(format!("{col} IN (SELECT value FROM json_each(?))"));
            p.push(json_list(v));
        }
    };
    list("platform", "l.platform", &q.platform, &mut w, &mut p);
    list("developer", "l.developer", &q.developer, &mut w, &mut p);
    list("publisher", "l.publisher", &q.publisher, &mut w, &mut p);
    for (name, v) in [("genre", &q.genre), ("region", &q.region), ("language", &q.language)] {
        if !v.is_empty() && skip != name {
            w.push(format!("l.id IN (SELECT game FROM library_values WHERE field = '{name}' AND value IN (SELECT value FROM json_each(?)))"));
            p.push(json_list(v));
        }
    }
    if !q.decade.is_empty() && skip != "decade" {
        w.push("(l.year / 10) * 10 IN (SELECT value FROM json_each(?))".into());
        p.push(json_list(&q.decade));
    }
    if q.players > 0 && skip != "players" {
        w.push("l.players >= ?".into());
        p.push(Sql::Integer(q.players));
    }
    if q.rating > 0.0 && skip != "rating" {
        w.push("l.rating >= ?".into());
        p.push(Sql::Real(q.rating - 1e-9));
    }
    if skip != "has" {
        for h in &q.has {
            match h.as_str() {
                "pictures" => w.push("l.picture IS NOT NULL".into()),
                "description" => w.push("l.described = 1".into()),
                "documents" => w.push("l.docs > 0".into()),
                "manual" | "video" => w.push(format!("(',' || l.media || ',') LIKE '%,{h},%'")),
                "addons" => w.push("l.addons > 0".into()),
                _ => {}
            }
        }
    }
    if q.look && skip != "look" {
        w.push("l.review = 1".into());
    }
    if let Some(k) = keys {
        w.push("l.key IN (SELECT value FROM json_each(?))".into());
        p.push(json_list(k));
    }
    (if w.is_empty() { "1".into() } else { w.join(" AND ") }, p)
}

/// The DAT games of a system with these details (a collection's rules by details).
pub(crate) fn matching(conn: &Connection, platform: &str, d: &collection::Details) -> Result<HashSet<i64>> {
    let q = Query {
        all: true,
        platform: vec![platform.to_string()],
        search: d.search.clone(),
        genre: d.genre.clone(),
        decade: d.decade.clone(),
        developer: d.developer.clone(),
        publisher: d.publisher.clone(),
        players: d.players,
        rating: d.rating,
        has: d.has.clone(),
        look: d.look,
        ..Default::default()
    };
    let (w, p) = where_of(&q, &None, "");
    let ids = conn
        .prepare(&format!("SELECT l.dat_game FROM library l WHERE {w} AND l.dat_game IS NOT NULL"))?
        .query_map(params_from_iter(p.iter()), |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(ids)
}

fn group_col(group: &str) -> Option<&'static str> {
    match group {
        "system" => Some("l.platform"),
        "genre" => Some("l.genre"),
        "decade" => Some("(l.year / 10) * 10"),
        "developer" => Some("l.developer"),
        "publisher" => Some("l.publisher"),
        _ => None,
    }
}

/// The games a query finds, a page at a time, with how many there are and (with
/// `facets`) each filter's values and counts.
pub fn query(ix: &Index, lib: &Library, q: &Query) -> Result<Value> {
    let conn = ix.conn();
    let keys = if q.collection.is_empty() {
        None
    } else {
        let c = collection::load(lib, &q.collection)?;
        let w = collection::work_out(ix, lib, &c)?;
        Some(w.games().map(|g| g.key()).collect::<Vec<_>>())
    };
    let (w, p) = where_of(q, &keys, "");
    let total: i64 = conn.query_row(&format!("SELECT COUNT(*) FROM library l WHERE {w}"), params_from_iter(p.iter()), |r| r.get(0))?;
    // each sort as an index has it (so a page deep in the list is quick), and descending
    // its exact reverse
    let d = if q.desc { "DESC" } else { "ASC" };
    let order = match q.sort.as_str() {
        "system" => format!("l.platform {d}, l.sort {d}, l.id {d}"),
        "year" => format!("l.year_sort {d}, l.sort {d}, l.id {d}"),
        "rating" => format!("l.rating_sort {d}, l.sort {d}, l.id {d}"),
        "size" => format!("l.size_sort {d}, l.sort {d}, l.id {d}"),
        "date" => format!("l.date_sort {d}, l.sort {d}, l.id {d}"),
        _ => format!("l.sort {d}, l.id {d}"),
    };
    let (order, group) = match group_col(&q.group) {
        Some(g) => (format!("{g} IS NULL, {g}, {order}"), g),
        None => (order, "NULL"),
    };
    let mut pp = p.clone();
    pp.push(Sql::Integer(q.limit.clamp(1, 1000)));
    pp.push(Sql::Integer(q.offset.max(0)));
    // the page's ids first (the sort holds only them), then its rows
    let ids: Vec<i64> = conn
        .prepare(&format!("SELECT l.id FROM library l WHERE {w} ORDER BY {order} LIMIT ? OFFSET ?"))?
        .query_map(params_from_iter(pp.iter()), |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut st = conn.prepare(&format!(
        "SELECT l.id, l.key, l.kind, l.platform, l.dat_game, l.name, l.title, l.year, l.developer, l.publisher, l.players, l.rating, l.regions,
                l.have, l.picture, l.media, l.described, l.docs, l.review, l.size, l.date, {group}, l.genres, l.addons
         FROM library l WHERE l.id IN (SELECT value FROM json_each(?1))"
    ))?;
    let mut by: HashMap<i64, Value> = st
        .query_map([serde_json::to_string(&ids)?], |r| {
            let platform: String = r.get(3)?;
            let media: String = r.get(15)?;
            let group: Option<Sql> = r.get(21)?;
            let genres: String = r.get(22)?;
            Ok((r.get::<_, i64>(0)?, json!({
                "key": r.get::<_, String>(1)?, "kind": r.get::<_, String>(2)?, "platform_name": system_name(&platform), "platform": platform,
                "dat_game": r.get::<_, Option<i64>>(4)?, "name": r.get::<_, String>(5)?, "title": r.get::<_, String>(6)?, "year": r.get::<_, Option<i64>>(7)?,
                "developer": r.get::<_, Option<String>>(8)?, "publisher": r.get::<_, Option<String>>(9)?, "players": r.get::<_, Option<i64>>(10)?,
                "rating": r.get::<_, Option<f64>>(11)?, "regions": r.get::<_, String>(12)?, "have": r.get::<_, i64>(13)? == 1,
                "picture": r.get::<_, Option<String>>(14)?, "media": split(&media), "described": r.get::<_, i64>(16)? == 1,
                "docs": r.get::<_, i64>(17)?, "review": r.get::<_, i64>(18)? == 1, "size": r.get::<_, i64>(19)?, "date": r.get::<_, Option<i64>>(20)?,
                "group": match group { Some(Sql::Text(t)) => json!(t), Some(Sql::Integer(n)) => json!(n), _ => Value::Null },
                "genres": genres.split(SEP).filter(|g| !g.is_empty()).collect::<Vec<_>>(), "addons": r.get::<_, i64>(23)?,
            })))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut items: Vec<Value> = ids.iter().filter_map(|id| by.remove(id)).collect();
    if q.group == "system" {
        for it in &mut items {
            it["group_name"] = json!(system_name(it["platform"].as_str().unwrap_or("")));
        }
    }
    let mut out = json!({ "total": total, "offset": q.offset.max(0), "items": items });
    if group != "NULL" && q.offset <= 0 {
        // the groups in the list's order, with their counts, so the page can lay them out
        let mut st = conn.prepare(&format!("SELECT {group} AS g, COUNT(*) FROM library l WHERE {w} GROUP BY g ORDER BY g IS NULL, g"))?;
        let groups: Vec<Value> = st
            .query_map(params_from_iter(p.iter()), |r| {
                let v: Sql = r.get(0)?;
                let value = match v { Sql::Text(t) => json!(t), Sql::Integer(n) => json!(n), _ => Value::Null };
                let name = if q.group == "system" { json!(system_name(value.as_str().unwrap_or(""))) } else { Value::Null };
                Ok(json!({ "value": value, "name": name, "count": r.get::<_, i64>(1)? }))
            })?
            .collect::<rusqlite::Result<_>>()?;
        out["groups"] = json!(groups);
    }
    if q.facets {
        out["facets"] = facets(conn, q, &keys)?;
    }
    Ok(out)
}

/// What the filters count: each filter's values over the games the other filters find.
#[derive(Default)]
struct Counts {
    values: HashMap<&'static str, HashMap<String, i64>>,
    decade: HashMap<i64, i64>,
    players: [i64; 4],
    rating: [i64; 3],
    has: [i64; 6],
    look: i64,
}

const FILTERS: [&str; 11] = ["platform", "genre", "decade", "developer", "publisher", "region", "language", "players", "rating", "has", "look"];
const RATINGS: [f64; 3] = [0.4, 0.6, 0.8];

/// Whether the query uses a filter.
fn in_use(q: &Query, name: &str) -> bool {
    match name {
        "platform" => !q.platform.is_empty(),
        "genre" => !q.genre.is_empty(),
        "decade" => !q.decade.is_empty(),
        "developer" => !q.developer.is_empty(),
        "publisher" => !q.publisher.is_empty(),
        "region" => !q.region.is_empty(),
        "language" => !q.language.is_empty(),
        "players" => q.players > 0,
        "rating" => q.rating > 0.0,
        "has" => !q.has.is_empty(),
        "look" => q.look,
        _ => false,
    }
}

/// Each filter's values and counts. The filters not in use are counted in one pass over
/// the games the query finds; each one in use, in a pass leaving it out, so its other
/// values can still be chosen.
fn facets(conn: &Connection, q: &Query, keys: &Option<Vec<String>>) -> Result<Value> {
    let mut c = Counts::default();
    let passes: Vec<(&str, Vec<&str>)> = std::iter::once(("", FILTERS.iter().copied().filter(|f| !in_use(q, f)).collect()))
        .chain(FILTERS.iter().copied().filter(|f| in_use(q, f)).map(|f| (f, vec![f])))
        .collect();
    for (skip, fields) in passes {
        if fields.is_empty() {
            continue;
        }
        let want = |f: &str| fields.contains(&f);
        let (w, p) = where_of(q, keys, skip);
        let mut st = conn.prepare_cached(&format!(
            "SELECT l.platform, l.year, l.developer, l.publisher, l.players, l.rating, l.picture IS NOT NULL, l.described, l.media, l.docs > 0, l.review,
                    l.genres, l.regions, l.languages, l.addons > 0 FROM library l WHERE {w}"
        ))?;
        let mut rows = st.query(params_from_iter(p.iter()))?;
        while let Some(r) = rows.next()? {
            let mut add = |field: &'static str, v: &str| {
                if !v.is_empty() {
                    let m = c.values.entry(field).or_default();
                    // a new String only for a value not seen yet
                    match m.get_mut(v) {
                        Some(n) => *n += 1,
                        None => {
                            m.insert(v.to_string(), 1);
                        }
                    }
                }
            };
            if want("platform") {
                add("platform", r.get_ref(0)?.as_str()?);
            }
            for (i, f) in [(2, "developer"), (3, "publisher")] {
                if want(f) {
                    if let Some(v) = r.get_ref(i)?.as_str_or_null()? {
                        add(f, v);
                    }
                }
            }
            for (i, f, sep) in [(11, "genre", SEP), (12, "region", ","), (13, "language", ",")] {
                if want(f) {
                    for v in r.get_ref(i)?.as_str()?.split(sep) {
                        add(f, v);
                    }
                }
            }
            if want("decade") {
                if let Some(y) = r.get::<_, Option<i64>>(1)? {
                    *c.decade.entry(y / 10 * 10).or_default() += 1;
                }
            }
            if want("players") {
                let n = r.get::<_, Option<i64>>(4)?.unwrap_or(0);
                for (i, x) in c.players.iter_mut().enumerate() {
                    *x += (n > i as i64) as i64;
                }
            }
            if want("rating") {
                if let Some(v) = r.get::<_, Option<f64>>(5)? {
                    for (x, at) in c.rating.iter_mut().zip(RATINGS) {
                        *x += (v >= at - 1e-9) as i64;
                    }
                }
            }
            if want("has") {
                let media = r.get_ref(8)?.as_str()?;
                let kinds = media.split(',').collect::<Vec<_>>();
                for (x, yes) in c.has.iter_mut().zip([r.get::<_, i64>(6)? == 1, r.get::<_, i64>(7)? == 1, kinds.contains(&"manual"), kinds.contains(&"video"), r.get::<_, i64>(9)? == 1, r.get::<_, i64>(14)? == 1]) {
                    *x += yes as i64;
                }
            }
            if want("look") {
                c.look += r.get::<_, i64>(10)?;
            }
        }
    }
    // each filter's 500 most-used values, which its Find box searches
    const TOP: usize = 500;
    let top = |field: &str| -> Vec<Value> {
        let mut v: Vec<(&String, &i64)> = c.values.get(field).map(|m| m.iter().collect()).unwrap_or_default();
        v.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
        v.into_iter()
            .take(TOP)
            .map(|(k, n)| if field == "platform" { json!({ "value": k, "count": n, "name": system_name(k) }) } else { json!({ "value": k, "count": n }) })
            .collect()
    };
    let mut decade: Vec<(&i64, &i64)> = c.decade.iter().collect();
    decade.sort();
    Ok(json!({
        "platform": top("platform"), "genre": top("genre"), "developer": top("developer"), "publisher": top("publisher"),
        "region": top("region"), "language": top("language"),
        "decade": decade.into_iter().map(|(d, n)| json!({ "value": d, "count": n })).collect::<Vec<_>>(),
        "players": c.players.iter().enumerate().map(|(i, n)| json!({ "value": (i + 1) as f64, "count": n })).collect::<Vec<_>>(),
        "rating": RATINGS.iter().zip(c.rating).map(|(v, n)| json!({ "value": v, "count": n })).collect::<Vec<_>>(),
        "has": { "pictures": c.has[0], "description": c.has[1], "manual": c.has[2], "video": c.has[3], "documents": c.has[4], "addons": c.has[5] },
        "look": c.look,
    }))
}

/// The keys of every game a query finds (for "everything selected"), with what each
/// action needs: its DAT game and system.
pub fn keys(ix: &Index, lib: &Library, q: &Query) -> Result<Value> {
    let conn = ix.conn();
    let keys = match q.collection.as_str() {
        "" => None,
        id => Some(collection::work_out(ix, lib, &collection::load(lib, id)?)?.games().map(|g| g.key()).collect::<Vec<_>>()),
    };
    let (w, p) = where_of(q, &keys, "");
    let v: Vec<Value> = conn
        .prepare(&format!("SELECT l.key, l.dat_game, l.platform, l.kind, l.name FROM library l WHERE {w} ORDER BY l.sort, l.id"))?
        .query_map(params_from_iter(p.iter()), |r| {
            Ok(json!({ "key": r.get::<_, String>(0)?, "dat_game": r.get::<_, Option<i64>>(1)?, "platform": r.get::<_, String>(2)?, "kind": r.get::<_, String>(3)?, "name": r.get::<_, String>(4)? }))
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(json!(v))
}

/// How many games the library has, and what's in it (the page's first look), with the
/// DAT each system's games are of.
pub fn overview(ix: &Index, lib: &Library) -> Result<Value> {
    let conn = ix.conn();
    let mut dats = Map::new();
    let systems: Vec<String> = conn.prepare("SELECT DISTINCT platform FROM library WHERE kind = 'dat'")?.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    for p in systems {
        if let Some(d) = audit::primary_dat(conn, lib, &p)? {
            dats.insert(p, json!(d));
        }
    }
    let (have, all, systems): (i64, i64, i64) = conn.query_row(
        "SELECT COALESCE(SUM(have), 0), COUNT(*), COUNT(DISTINCT CASE WHEN have = 1 THEN platform END) FROM library",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    Ok(json!({ "have": have, "all": all, "systems": systems, "dats": dats }))
}

// ------------------------------------------------------------ a game

/// One of the library's games by its key, as its page shows it: what the Metadata panel
/// shows for a DAT game (or the PC games page for a PC game), with its files and the
/// collections it's in.
pub fn game(ix: &Index, lib: &Library, key: &str) -> Result<Value> {
    let conn = ix.conn();
    let row: Option<(String, String, Option<i64>, Option<String>, String)> = conn
        .query_row("SELECT kind, platform, dat_game, folder, title FROM library WHERE key = ?1", [key], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
        .optional()?;
    let Some((kind, platform, dat_game, folder, title)) = row else { bail!("There's no game {key} in the library (it may have gone since the list was shown).") };
    let mut v = match (kind.as_str(), dat_game) {
        ("dat", Some(id)) => {
            let mut v = meta::game(ix, lib, id)?;
            v["dat_id"] = json!(audit::primary_dat(conn, lib, &platform)?);
            let name: String = conn.query_row("SELECT name FROM library WHERE key = ?1", [key], |r| r.get(0))?;
            v["addons"] = crate::addons::of_game(conn, lib, &platform, &name)?;
            v
        }
        ("pc", _) => {
            // its metadata as any game's (D55), and what only a PC game has in `pc`
            let id = key.strip_prefix("pc/").unwrap_or(key);
            let e = pcgames::get(lib, id)?;
            let mut v = crate::pcmeta::page(lib, &e);
            let mut pc = e.to_json(lib);
            pc["programs"] = json!(pcgames::programs(lib, id).unwrap_or_default());
            pc["innoextract"] = json!(crate::tools::find("innoextract").is_some());
            v["title"] = json!(e.game.title);
            v["year"] = json!(e.game.year);
            v["platform"] = json!("pc");
            v["platform_name"] = json!(system_name("pc"));
            v["have"] = json!(true);
            v["docs"] = json!(match &folder { Some(f) => docs::of_folder(conn, f)?, None => vec![] });
            v["pc_id"] = json!(id);
            // its patches and DLC are listed as other systems' updates and DLC are (D66)
            let items: Vec<Value> = e
                .game
                .files
                .iter()
                .filter(|f| f.kind == "patch" || f.kind == "dlc")
                .map(|f| json!({ "name": f.path.rsplit('/').next().unwrap_or(&f.path), "kind": f.kind, "version": f.version, "status": "have", "by_hand": false }))
                .collect();
            v["addons"] = json!({ "items": items, "have": items.len(), "loose": [] });
            v["pc"] = pc;
            v
        }
        _ => {
            let f = folder.clone().ok_or_else(|| anyhow!("The game {key} has no folder."))?;
            let dir = lib.resolve(&f)?;
            let j = meta::read_game(&dir);
            json!({
                "platform": platform, "platform_name": system_name(&platform), "name": j["name"], "title": title, "year": j["year"],
                "details": j["details"], "ids": j["ids"], "scraped": j["scraped"], "edited": j["edited"], "folder": f, "have": true, "pinned": j["pinned"],
                "sources": crate::sources::answers(&dir).into_iter().map(|(s, mut a)| { a["name"] = json!(crate::sources::source_name(&s)); a }).collect::<Vec<_>>(),
                "variants": crate::sources::variants(lib, &dir, &j),
                "media": crate::identify::media_of(lib, &dir.join("media")), "docs": docs::of_folder(conn, &f)?, "roms": j["files"],
            })
        }
    };
    v["key"] = json!(key);
    v["kind"] = json!(kind);
    match (kind.as_str(), dat_game) {
        ("dat", Some(id)) => v["files"] = json!(files_of(conn, lib, id)?),
        // a PC game's files are its folder's (pcgames.rs)
        ("pc", _) => {}
        _ => v["files"] = json!([]),
    }
    v["collections"] = json!(collections_of(ix, lib, key)?);
    Ok(v)
}

/// Change one of a game's details by hand, by its key (D44): kept over later scrapes.
/// `None` takes the edit mark off. A PC game's title and year are its `PC/…/game.json`'s
/// own (its folder is renamed); its other details are kept as any game's (D55).
pub fn edit(ix: &Index, lib: &Library, key: &str, field: &str, value: Option<&Value>) -> Result<Value> {
    lib.writable()?;
    let row: Option<(String, Option<i64>, Option<String>)> =
        ix.conn().query_row("SELECT kind, dat_game, folder FROM library WHERE key = ?1", [key], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional()?;
    let Some((kind, dat_game, folder)) = row else { bail!("There's no game {key} in the library (it may have gone since the list was shown).") };
    match (kind.as_str(), dat_game) {
        ("dat", Some(id)) => {
            meta::edit(ix, lib, id, field, value)?;
        }
        ("pc", _) => {
            let id = key.strip_prefix("pc/").unwrap_or(key);
            match field {
                "title" | "year" => {
                    // a title or year is the game's own: there's no mark to take off
                    if let Some(x) = value {
                        pcgames::update(lib, id, &json!({ field: meta::edit_value(field, x)? }))?;
                        // the index names its documents by it
                        let dir = crate::pcmeta::dir(lib, id);
                        if dir.join("game.json").is_file() {
                            meta::touch(ix, lib, &dir)?;
                        }
                    }
                }
                _ => {
                    let dir = crate::pcmeta::ensure(ix, lib, &pcgames::get(lib, id)?)?;
                    meta::edit_dir(ix, lib, &dir, field, value)?;
                }
            }
        }
        _ => {
            let f = folder.ok_or_else(|| anyhow!("The game {key} has no folder."))?;
            meta::edit_dir(ix, lib, &lib.resolve(&f)?, field, value)?;
        }
    }
    refresh(ix, lib)?;
    game(ix, lib, key)
}

/// A DAT game's files: where each is (an archive or image once, with the ROMs in it), its
/// kind and size.
pub fn files_of(conn: &Connection, lib: &Library, dat_game: i64) -> Result<Vec<Value>> {
    let dirs: HashMap<String, PathBuf> = crate::roots::list(lib).into_iter().map(|r| (r.id.clone(), r.dir.clone())).collect();
    let mut q = conn.prepare(
        "SELECT COALESCE(p.root_id, f.root_id), COALESCE(p.rel_path, f.rel_path), COALESCE(p.kind, f.kind), COALESCE(p.size, f.size), r.name, f.missing_since IS NOT NULL
         FROM dat_roms r JOIN matches m ON m.rom_id = r.id JOIN files f ON f.id = m.file_id LEFT JOIN files p ON p.id = f.parent_id
         WHERE r.game_id = ?1 ORDER BY 2, r.name",
    )?;
    let mut out: Vec<Value> = vec![];
    let rows: Vec<(String, String, String, i64, String, bool)> = q.query_map([dat_game], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?.collect::<rusqlite::Result<_>>()?;
    for (root, rel, kind, size, rom, gone) in rows {
        let path = dirs.get(&root).map(|d| rel.split('/').fold(d.clone(), |p, x| p.join(x)).display().to_string()).unwrap_or_else(|| rel.clone());
        match out.iter_mut().find(|x| x["path"] == json!(path)) {
            Some(x) => x["roms"].as_array_mut().unwrap().push(json!(rom)),
            None => out.push(json!({ "path": path, "kind": kind, "size": size, "roms": [rom], "missing": gone })),
        }
    }
    Ok(out)
}

/// The collections a game is in: (id, name).
fn collections_of(ix: &Index, lib: &Library, key: &str) -> Result<Vec<Value>> {
    let mut v = vec![];
    for c in collection::list(lib) {
        let w = collection::work_out(ix, lib, &c)?;
        if w.games().any(|g| g.key() == key) {
            v.push(json!({ "id": c.id, "name": c.name }));
        }
    }
    Ok(v)
}

// ------------------------------------------------------------ thumbnails (D43)

/// Thumbnails made at once, at most.
static MAKING: (Mutex<usize>, Condvar) = (Mutex::new(0), Condvar::new());
const AT_ONCE: usize = 4;
/// Thumbnails kept, at most (the oldest go).
const KEEP: usize = 50_000;

/// A thumbnail of a picture in the library (`rel`), at most `size` pixels on its long
/// side, made the first time it's asked for and kept in `<data dir>/thumbs/<library id>/`.
pub fn thumb(lib: &Library, data_dir: &Path, rel: &str, size: u32) -> Result<PathBuf> {
    if ![128, 256, 512].contains(&size) {
        bail!("Thumbnails are 128, 256 or 512 pixels.");
    }
    let src = lib.resolve(rel)?;
    let m = std::fs::metadata(&src)?;
    let stamp = format!("{rel}|{}|{:?}|{size}", m.len(), m.modified().ok());
    let name = {
        use sha2::Digest;
        hex::encode(&sha2::Sha256::digest(stamp.as_bytes())[..16])
    };
    let dir = data_dir.join("thumbs").join(lib.id().unwrap_or_else(|| "library".into())).join(&name[..2]);
    let out = dir.join(format!("{name}.jpg"));
    if out.is_file() {
        return Ok(out);
    }
    {
        let (lock, cv) = &MAKING;
        let mut n = lock.lock().unwrap();
        while *n >= AT_ONCE {
            n = cv.wait(n).unwrap();
        }
        *n += 1;
    }
    let made = (|| -> Result<()> {
        if out.is_file() {
            return Ok(());
        }
        let img = ::image::ImageReader::open(&src)?.with_guessed_format()?.decode().map_err(|e| anyhow!("couldn't read the picture {rel}: {e}"))?;
        let img = if img.width().max(img.height()) > size { img.thumbnail(size, size) } else { img };
        let rgb = ::image::DynamicImage::ImageRgb8(img.to_rgb8());
        std::fs::create_dir_all(&dir)?;
        let tmp = dir.join(format!("{name}.part"));
        let mut buf = std::io::Cursor::new(Vec::new());
        ::image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 82).encode_image(&rgb)?;
        std::fs::write(&tmp, buf.into_inner())?;
        std::fs::rename(&tmp, &out)?;
        Ok(())
    })();
    {
        let (lock, cv) = &MAKING;
        *lock.lock().unwrap() -= 1;
        cv.notify_one();
    }
    made?;
    if name.ends_with("00") {
        // now and then (1 in 256), keep the cache to its size
        let _ = prune(&data_dir.join("thumbs"), KEEP);
    }
    Ok(out)
}

/// Keep the newest `keep` thumbnails.
pub fn prune(dir: &Path, keep: usize) -> Result<usize> {
    let mut all: Vec<(std::time::SystemTime, PathBuf)> = vec![];
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(t) = e.metadata().and_then(|m| m.modified()) {
                all.push((t, p));
            }
        }
    }
    if all.len() <= keep {
        return Ok(0);
    }
    all.sort();
    let n = all.len() - keep;
    for (_, p) in all.into_iter().take(n) {
        let _ = std::fs::remove_file(p);
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collection::tests::fixture;

    fn ask(ix: &Index, lib: &Library, f: impl FnOnce(&mut Query)) -> Value {
        let mut q = Query { limit: 100, ..Default::default() };
        f(&mut q);
        query(ix, lib, &q).unwrap()
    }

    fn titles(v: &Value) -> Vec<String> {
        v["items"].as_array().unwrap().iter().map(|g| g["name"].as_str().unwrap().to_string()).collect()
    }

    #[test]
    fn the_library_lists_every_game_with_one_per_title_and_rebuilds_only_what_changed() {
        let (dir, lib, ix) = fixture("browse-list");
        meta::sync(&ix, &lib).unwrap();
        assert_eq!(refresh(&ix, &lib).unwrap(), ["gb", "pc"]);
        // nothing changed: nothing rebuilt
        assert!(refresh(&ix, &lib).unwrap().is_empty());
        let v = ask(&ix, &lib, |_| {});
        assert_eq!(v["total"], 8);
        assert_eq!(v["items"][0]["key"], "gb/Alpha (Europe)");
        assert_eq!(v["items"][0]["platform_name"], system_name("gb"));
        // every game in the DAT, and one per title in the collection rules' region order
        assert_eq!(ask(&ix, &lib, |q| q.all = true)["total"], 10);
        assert_eq!(titles(&ask(&ix, &lib, |q| q.one_per_title = true)), ["Alpha (Europe)", "Beta (Europe) (Rev 1)", "Delta (World)", "Epsilon (Japan) (En,Ja)", "Gamma (USA) (Beta)"]);
        assert_eq!(
            titles(&ask(&ix, &lib, |q| { q.one_per_title = true; q.all = true })),
            ["Alpha (USA)", "Beta (Europe) (Rev 1)", "Delta (World)", "Epsilon (Japan) (En,Ja)", "Gamma (USA) (Beta)", "Zeta (Japan)"]
        );
        // regions and languages from the DAT names, with counts that leave their own filter out
        let v = ask(&ix, &lib, |q| { q.region = vec!["Japan".into()]; q.facets = true });
        assert_eq!(titles(&v), ["Alpha (Japan)", "Beta (Japan)", "Epsilon (Japan) (En,Ja)"]);
        let japan = v["facets"]["region"].as_array().unwrap().iter().find(|x| x["value"] == "Japan").unwrap()["count"].clone();
        assert_eq!(japan, 3);
        assert!(v["facets"]["region"].as_array().unwrap().iter().any(|x| x["value"] == "Europe"));
        assert_eq!(ask(&ix, &lib, |q| q.language = vec!["Ja".into()])["total"], 1);
        // a collection's games only
        let mut rules = collection::Rules::default();
        rules.platforms = vec!["gb".into()];
        let c = collection::Collection { id: String::new(), name: "Mine".into(), rules, added: vec![], removed: vec![], size_limit: None, made_from: None, created: String::new(), modified: String::new() };
        let c = collection::save(&lib, c).unwrap();
        assert_eq!(ask(&ix, &lib, |q| q.collection = c.id.clone())["total"], 4);
        // paging
        let v = ask(&ix, &lib, |q| { q.offset = 6; q.limit = 5 });
        assert_eq!((v["total"].as_i64(), v["items"].as_array().unwrap().len()), (Some(8), 2));
        // a game's page
        let g = game(&ix, &lib, "gb/Delta (World)").unwrap();
        assert_eq!((g["have"].as_bool(), g["files"].as_array().unwrap().len()), (Some(true), 1));
        assert_eq!(g["collections"][0]["name"], "Mine");
        assert!(game(&ix, &lib, "gb/Nothing").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn details_edited_by_hand_are_searched_filtered_and_kept_over_a_scrape() {
        let (dir, lib, ix) = fixture("browse-edit");
        meta::sync(&ix, &lib).unwrap();
        refresh(&ix, &lib).unwrap();
        let key = "gb/Delta (World)";
        edit(&ix, &lib, key, "developer", Some(&json!("Acme Soft"))).unwrap();
        edit(&ix, &lib, key, "genres", Some(&json!("Puzzle, Action"))).unwrap();
        edit(&ix, &lib, key, "year", Some(&json!("1991"))).unwrap();
        edit(&ix, &lib, key, "players", Some(&json!("1-2"))).unwrap();
        let g = edit(&ix, &lib, key, "rating", Some(&json!(0.8))).unwrap();
        assert_eq!(g["edited"], json!(["developer", "genres", "year", "players", "rating"]));
        assert_eq!(g["details"]["genres"], json!(["Puzzle", "Action"]));
        assert_eq!(g["scraped"]["developer"], "you");
        assert!(edit(&ix, &lib, key, "rating", Some(&json!(4))).is_err());
        assert!(edit(&ix, &lib, key, "colour", Some(&json!("red"))).is_err());
        // the library follows: search, filters, counts, group and sort
        assert_eq!(titles(&ask(&ix, &lib, |q| q.search = "acme".into())), ["Delta (World)"]);
        assert_eq!(titles(&ask(&ix, &lib, |q| q.search = "delt wor".into())), ["Delta (World)"]);
        assert_eq!(ask(&ix, &lib, |q| q.genre = vec!["Action".into(), "Racing".into()])["total"], 1);
        assert_eq!(ask(&ix, &lib, |q| { q.decade = vec![1990]; q.players = 2; q.rating = 0.8 })["total"], 1);
        assert_eq!(ask(&ix, &lib, |q| q.players = 3)["total"], 0);
        let v = ask(&ix, &lib, |q| { q.facets = true; q.group = "genre".into() });
        let f = &v["facets"];
        assert_eq!(f["genre"], json!([{ "value": "Action", "count": 1 }, { "value": "Puzzle", "count": 1 }]));
        assert_eq!(f["developer"], json!([{ "value": "Acme Soft", "count": 1 }]));
        assert_eq!(f["decade"], json!([{ "value": 1990, "count": 1 }]));
        assert_eq!(f["players"][1], json!({ "value": 2.0, "count": 1 }));
        assert_eq!(v["items"][0]["group"], "Puzzle");
        assert_eq!(titles(&ask(&ix, &lib, |q| { q.sort = "year".into(); q.limit = 1 })), ["Delta (World)"]);
        // a scrape leaves what was edited, and fills the rest
        let dir_of = lib.resolve(g["folder"].as_str().unwrap()).unwrap();
        let mut v = meta::read_game(&dir_of);
        let found: Map<String, Value> = serde_json::from_value(json!({ "developer": "Other", "publisher": "Big Co" })).unwrap();
        let set = meta::merge_details(&mut v, &meta::Found { source: "test", id: "1", title: Some("Delta!"), year: Some(1995), details: &found }, &["developer".into(), "publisher".into()], true);
        assert_eq!(set, ["publisher"]);
        assert_eq!((v["details"]["developer"].as_str(), v["year"].as_i64(), v["title"].as_str()), (Some("Acme Soft"), Some(1991), Some("Delta!")));
        // a collection by details keeps up with them
        let mut rules = collection::Rules::default();
        rules.platforms = vec!["gb".into()];
        rules.details.genre = vec!["Puzzle".into()];
        rules.details.decade = vec![1990];
        let c = collection::Collection { id: String::new(), name: "Puzzles".into(), rules, added: vec![], removed: vec![], size_limit: None, made_from: None, created: String::new(), modified: String::new() };
        let names = |c: &collection::Collection| collection::work_out(&ix, &lib, c).unwrap().games().map(|g| g.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&c), ["Delta (World)"]);
        edit(&ix, &lib, "gb/Epsilon (Japan) (En,Ja)", "genres", Some(&json!(["Puzzle"]))).unwrap();
        edit(&ix, &lib, "gb/Epsilon (Japan) (En,Ja)", "year", Some(&json!(1994))).unwrap();
        assert_eq!(names(&c), ["Delta (World)", "Epsilon (Japan) (En,Ja)"]);
        let saved = collection::save(&lib, c).unwrap();
        assert_eq!(collection::load(&lib, &saved.id).unwrap().rules.details.genre, ["Puzzle"]);
        // the mark taken off: the source's value comes back (D46)
        let m = crate::pcgames::Match { source: "igdb".into(), id: "1".into(), details: found.clone(), ..Default::default() };
        crate::sources::keep(&dir_of, &m, &["developer".into()], false).unwrap();
        let g = edit(&ix, &lib, key, "developer", None).unwrap();
        assert_eq!(g["edited"], json!(["genres", "year", "players", "rating"]));
        assert_eq!((g["details"]["developer"].as_str(), g["scraped"]["developer"].as_str()), (Some("Other"), Some("igdb")), "{g}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn two_games_a_dat_lists_under_one_name_each_get_a_key() {
        use crate::audit::tests::{dat_xml, rom, rom_xml};
        let (dir, lib, ix) = fixture("browse-same-name");
        // libretro's copies list some discs twice under one name with different files
        let games = vec![("Twin (USA)", None, rom_xml("Twin (USA).gb", &rom(71, 512))), ("Twin (USA)", None, rom_xml("Twin (USA).gb", &rom(72, 512))), ("Solo (USA)", None, rom_xml("Solo (USA).gb", &rom(73, 512)))];
        std::fs::write(dir.join("twins.dat"), dat_xml("Nintendo - Game Boy", "20261201", &games)).unwrap();
        crate::dat::import(&ix, &lib, &dir.join("twins.dat"), &Map::new()).unwrap();
        let gb: i64 = ix.conn().query_row("SELECT id FROM dats WHERE version = '20261201'", [], |r| r.get(0)).unwrap();
        audit::choose_dat(ix.conn(), &lib, gb).unwrap();
        meta::sync(&ix, &lib).unwrap();
        refresh(&ix, &lib).unwrap();
        let mut keys: Vec<String> = ask(&ix, &lib, |q| q.all = true)["items"].as_array().unwrap().iter().map(|g| g["key"].as_str().unwrap().to_string()).collect();
        keys.sort();
        assert_eq!(keys, ["gb/Solo (USA)", "gb/Twin (USA)", "gb/Twin (USA)~2"]);
        assert_eq!(game(&ix, &lib, "gb/Twin (USA)~2").unwrap()["name"], "Twin (USA)");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_game_folder_is_renamed_to_its_new_dat_name_as_a_change_that_can_be_undone() {
        use crate::audit::tests::{dat_xml, rom, rom_xml};
        let (dir, lib, ix) = fixture("browse-rename");
        meta::sync(&ix, &lib).unwrap();
        refresh(&ix, &lib).unwrap();
        for k in ["gb/Delta (World)", "gb/Beta (Japan)"] {
            edit(&ix, &lib, k, "developer", Some(&json!("Acme"))).unwrap();
        }
        let plan = meta::rename_folders_plan(&ix, &lib).unwrap();
        assert_eq!((plan.ops.len(), plan.in_place), (0, 2));
        // the DAT's next version renames Delta
        let games = vec![("Delta (World) (Rev 1)", None, rom_xml("Delta (World) (Rev 1).gb", &rom(8, 1024))), ("Beta (Japan)", None, rom_xml("Beta (Japan).gb", &rom(4, 1024)))];
        std::fs::write(dir.join("gb2.dat"), dat_xml("Nintendo - Game Boy", "20261101", &games)).unwrap();
        crate::dat::import(&ix, &lib, &dir.join("gb2.dat"), &Map::new()).unwrap();
        let gb: i64 = ix.conn().query_row("SELECT id FROM dats WHERE name = 'Nintendo - Game Boy' AND current = 1", [], |r| r.get(0)).unwrap();
        audit::choose_dat(ix.conn(), &lib, gb).unwrap();
        // until it's renamed, the folder is still found by its ROMs
        refresh(&ix, &lib).unwrap();
        let g = game(&ix, &lib, "gb/Delta (World) (Rev 1)").unwrap();
        assert_eq!(g["details"]["developer"], "Acme");
        let plan = meta::rename_folders_plan(&ix, &lib).unwrap();
        assert_eq!((plan.ops.len(), plan.in_place, plan.label.as_str()), (1, 1, "Renamed 1 game's library folder to its DAT name"));
        assert!(plan.ops[0].to.ends_with("Delta (World) (Rev 1)"));
        let old = PathBuf::from(&plan.ops[0].from[0].path);
        let none = std::sync::atomic::AtomicBool::new(false);
        let r = crate::organise::run(&ix, &lib, &plan, &none, &mut |_| {}).unwrap();
        assert_eq!(r["done"], 1);
        assert!(!old.exists() && Path::new(&plan.ops[0].to).join("game.json").is_file());
        meta::sync(&ix, &lib).unwrap();
        refresh(&ix, &lib).unwrap();
        assert_eq!(game(&ix, &lib, "gb/Delta (World) (Rev 1)").unwrap()["folder"], "_library/games/gb/Delta (World) (Rev 1)");
        assert!(meta::rename_folders_plan(&ix, &lib).unwrap().ops.is_empty());
        // and back
        crate::organise::undo(&ix, &lib, &plan.id, &none, &mut |_| {}).unwrap();
        assert!(old.join("game.json").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn thumbnails_are_made_once_kept_on_this_computer_and_pruned() {
        let (dir, lib, ix) = fixture("browse-thumb");
        let key = "gb/Delta (World)";
        meta::sync(&ix, &lib).unwrap();
        refresh(&ix, &lib).unwrap();
        let g = edit(&ix, &lib, key, "developer", Some(&json!("Acme"))).unwrap();
        let media = lib.resolve(g["folder"].as_str().unwrap()).unwrap().join("media");
        std::fs::create_dir_all(&media).unwrap();
        ::image::RgbImage::from_pixel(600, 300, ::image::Rgb([200, 30, 30])).save(media.join("cover.png")).unwrap();
        meta::sync(&ix, &lib).unwrap();
        assert_eq!(refresh(&ix, &lib).unwrap(), ["gb"]);
        let v = ask(&ix, &lib, |q| { q.has = vec!["pictures".into()]; q.facets = true });
        assert_eq!(titles(&v), ["Delta (World)"]);
        assert_eq!(v["facets"]["has"]["pictures"], 1);
        let rel = v["items"][0]["picture"].as_str().unwrap().to_string();
        assert!(rel.contains("/media/cover.png?v="), "{rel}");
        let rel = rel.split('?').next().unwrap().to_string();
        let data = dir.join("data");
        let t = thumb(&lib, &data, &rel, 256).unwrap();
        assert!(t.starts_with(data.join("thumbs")));
        let img = ::image::open(&t).unwrap();
        assert_eq!((img.width(), img.height()), (256, 128));
        let made = std::fs::metadata(&t).unwrap().modified().unwrap();
        assert_eq!(thumb(&lib, &data, &rel, 256).unwrap(), t);
        assert_eq!(std::fs::metadata(&t).unwrap().modified().unwrap(), made);
        assert_ne!(thumb(&lib, &data, &rel, 128).unwrap(), t);
        assert!(thumb(&lib, &data, &rel, 300).is_err());
        assert!(thumb(&lib, &data, "../outside.png", 256).is_err());
        assert_eq!(prune(&data.join("thumbs"), 1).unwrap(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // The performance targets of docs/PLAN.md ("Phase 6 design"), on generated data. Run
    // with `cargo test --release -- --ignored timing` (CI does, on Windows).
    #[test]
    #[ignore]
    fn timing_the_library_of_200k_games() {
        let dir = crate::library::tests::temp_dir("timing-library");
        let lib = Library::open(dir.join("Lib")).unwrap();
        let ix = Index::open(&dir.join("index.sqlite"), "lib-x").unwrap();
        let genres = ["Action", "Puzzle", "Racing", "Sports", "Platform", "Shooter", "Role-playing", "Strategy"];
        let regions = ["USA", "Europe", "Japan", "World"];
        // titles of two of 1,000 made-up words
        let syl = ["ka", "zo", "ri", "mu", "te", "lo", "shi", "an", "bel", "dor"];
        let word = |n: usize| format!("{}{}{}", syl[n % 10], syl[n / 10 % 10], syl[n / 100 % 10]);
        let t = std::time::Instant::now();
        for p in 0..8 {
            let rows: Vec<Row> = (0..25_000)
                .map(|i| Row {
                    key: format!("s{p}/Game {i}"),
                    kind: "dat",
                    platform: format!("s{p}"),
                    name: format!("Game {i} ({})", regions[i % 4]),
                    title: format!("{} {} {}", ["The", "Super", "Mega", "Hyper"][i % 4], word(i * 7919 % 1000), word((i / 3 + p * 17) % 1000)),
                    year: (i % 7 != 0).then_some(1980 + (i % 40) as i64),
                    facts: serde_json::from_value(json!({
                        "developer": format!("Studio {}", i % 900), "publisher": format!("Publisher {}", i % 300),
                        "genres": [genres[i % 8], genres[(i / 8) % 8]], "players": format!("1-{}", 1 + i % 4), "rating": (i % 10) as f64 / 10.0,
                        "description": format!("A game about {} and the {}.", word(i % 1000), word(i * 31 % 1000)),
                    }))
                    .unwrap(),
                    regions: vec![regions[i % 4].into()],
                    languages: vec!["En".into()],
                    have: i % 3 != 0,
                    picture: (i % 2 == 0).then(|| format!("_library/games/s{p}/{i}/media/cover.png")),
                    size: i as i64 * 1000,
                    best_all: i % 4 == 0,
                    best_have: i % 4 == 1,
                    ..Default::default()
                })
                .collect();
            let tx = ix.conn().unchecked_transaction().unwrap();
            write_rows(&tx, &format!("s{p}"), &rows).unwrap();
            tx.commit().unwrap();
        }
        let built = t.elapsed();
        ix.conn().execute_batch("ANALYZE").unwrap();
        let best = |f: &dyn Fn() -> Value| {
            let mut took = std::time::Duration::MAX;
            let mut v = Value::Null;
            for _ in 0..3 {
                let t = std::time::Instant::now();
                v = f();
                took = took.min(t.elapsed());
            }
            (took, v)
        };
        let (first, v) = best(&|| ask(&ix, &lib, |_| {}));
        assert_eq!(v["total"], 8 * 16_666);
        let mut deep = std::time::Duration::ZERO;
        for (sort, desc) in [("year", false), ("year", true), ("rating", false), ("size", false), ("date", false), ("title", true), ("system", false)] {
            let (took, v) = best(&|| ask(&ix, &lib, |q| { q.offset = 120_000; q.sort = sort.into(); q.desc = desc }));
            assert_eq!(v["items"].as_array().unwrap().len(), 100);
            eprintln!("  page at 120,000 by {sort}{}: {took:?}", if desc { " (descending)" } else { "" });
            deep = deep.max(took);
        }
        let (system, _) = best(&|| ask(&ix, &lib, |q| { q.platform = vec!["s5".into()]; q.offset = 10_000 }));
        let (search, v) = best(&|| ask(&ix, &lib, |q| q.search = word(123)));
        assert!(v["total"].as_i64().unwrap() > 0);
        let (filtered, v) = best(&|| ask(&ix, &lib, |q| { q.genre = vec!["Puzzle".into()]; q.platform = vec!["s3".into(), "s4".into()]; q.search = format!("{} {}", word(7), word(400)) }));
        assert!(v["total"].as_i64().is_some());
        // every game matches (the first letter typed): the worst case
        let (everything, _) = best(&|| ask(&ix, &lib, |q| q.search = "a".into()));
        let (counts, v) = best(&|| ask(&ix, &lib, |q| { q.facets = true; q.limit = 1 }));
        assert_eq!(v["facets"]["platform"].as_array().unwrap().len(), 8);
        let (counts_filtered, _) = best(&|| ask(&ix, &lib, |q| { q.facets = true; q.limit = 1; q.genre = vec!["Action".into()]; q.decade = vec![1990]; q.one_per_title = true }));
        eprintln!(
            "200,000 games written in {built:?}; first page {first:?}, deepest page {deep:?}, a system's {system:?}, searched {search:?}, filtered and searched {filtered:?}, \
             searched for everything {everything:?}; counts {counts:?}, filtered counts {counts_filtered:?}"
        );
        for (what, took, most) in [
            ("a page", first, 0.1), ("a deep page", deep, 0.1), ("a system's page", system, 0.1), ("a search", search, 0.1), ("a filtered page", filtered, 0.1),
            ("a search finding everything", everything, 0.3), ("the counts", counts, 0.3), ("the filtered counts", counts_filtered, 0.3),
        ] {
            assert!(took.as_secs_f64() < most, "{what} took {took:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn titles_sort_without_their_article_and_players_are_the_most_it_says() {
        assert_eq!(sort_key("The Legend of Zelda"), "legend of zelda, the");
        assert_eq!(sort_key("Alpha"), "alpha");
        assert_eq!(players_of(&json!("1-4")), Some(4));
        assert_eq!(players_of(&json!(2)), Some(2));
        assert_eq!(fts_query("zel 'da"), Some("\"zel\"* \"da\"*".into()));
        assert_eq!(fts_query("  "), None);
    }
}
