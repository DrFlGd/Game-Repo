//! The catalogue devices browse (D92): the Library page's own rows (`browse.rs`'s
//! `library` table) for the DAT games the library has whole (not those had in part, which
//! an export leaves out), with a game on several discs one
//! game (D94), kept in the server's index with a change marker so a client that keeps the
//! catalogue fetches it once and then only what changed. The library's PC games are one
//! more system, `pcgames` (D110), given only to devices that ask for them (`pc=1`).

use anyhow::Result;
use gamerepo_core::index::Index;
use gamerepo_core::library::Library;
use gamerepo_core::{browse, collection, export, layout, platforms};
use gamerepo_wire::api;
use rusqlite::{params, params_from_iter, types::Value as Sql, OptionalExtension};
use std::collections::{BTreeMap, HashMap};

/// The server's own tables, beside the index's (the index is the server's own copy).
pub const TABLES: &str = "
    CREATE TABLE IF NOT EXISTS serve_games (
        key TEXT PRIMARY KEY, system TEXT NOT NULL, title TEXT NOT NULL, sort TEXT NOT NULL, year INTEGER,
        size INTEGER NOT NULL, media TEXT NOT NULL, picture TEXT, discs INTEGER NOT NULL,
        hash TEXT NOT NULL, rev INTEGER NOT NULL, gone INTEGER NOT NULL DEFAULT 0
    ) STRICT;
    -- a page in each order without sorting the whole catalogue (a year's own each way round: unknown last)
    CREATE INDEX IF NOT EXISTS serve_games_system ON serve_games (gone, system, sort, key);
    CREATE INDEX IF NOT EXISTS serve_games_sort ON serve_games (gone, sort, key);
    CREATE INDEX IF NOT EXISTS serve_games_size ON serve_games (gone, size, sort, key);
    CREATE INDEX IF NOT EXISTS serve_games_year ON serve_games (gone, year IS NULL, year, sort, key);
    CREATE INDEX IF NOT EXISTS serve_games_year_desc ON serve_games (gone, year IS NULL, year DESC, sort DESC, key DESC);
    CREATE INDEX IF NOT EXISTS serve_games_system_size ON serve_games (gone, system, size, sort, key);
    CREATE INDEX IF NOT EXISTS serve_games_system_year ON serve_games (gone, system, year IS NULL, year, sort, key);
    CREATE INDEX IF NOT EXISTS serve_games_system_year_desc ON serve_games (gone, system, year IS NULL, year DESC, sort DESC, key DESC);
    CREATE INDEX IF NOT EXISTS serve_games_rev ON serve_games (rev, key);
    -- each of the Library's keys a catalogue game stands for (a game on several discs, each disc's)
    CREATE TABLE IF NOT EXISTS serve_keys (lib_key TEXT PRIMARY KEY, key TEXT NOT NULL, disc INTEGER NOT NULL) STRICT;
    CREATE INDEX IF NOT EXISTS serve_keys_key ON serve_keys (key, disc);
    CREATE TABLE IF NOT EXISTS serve_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL) STRICT;
    CREATE TABLE IF NOT EXISTS serve_hashes (path TEXT PRIMARY KEY, size INTEGER NOT NULL, mtime INTEGER NOT NULL, sha1 TEXT NOT NULL) STRICT;
    -- a PC game's archive's unpacked size, by its path, size and modified time
    CREATE TABLE IF NOT EXISTS serve_sizes (path TEXT PRIMARY KEY, size INTEGER NOT NULL, mtime INTEGER NOT NULL, unpacked INTEGER NOT NULL) STRICT;
";

/// The catalogue's system for the library's PC games (not ES-DE's `pc`, which is DOS, D110).
pub const PC: &str = "pcgames";

/// What a PC game's files come to unpacked, by its id: its `extracted/` folder's, else its
/// install archives' members' (read from their directories, and kept); a game with neither
/// isn't listed.
fn pc_sizes(ix: &Index, lib: &Library) -> Result<HashMap<String, u64>> {
    let conn = ix.conn();
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let mut out = HashMap::new();
    for e in gamerepo_core::pcgames::list(lib) {
        let extracted = e.dir.join("extracted");
        if extracted.is_dir() {
            let mut files = vec![];
            crate::plans::walk(&extracted, &mut files);
            if !files.is_empty() {
                out.insert(e.game.id.clone(), files.iter().filter_map(|p| std::fs::metadata(p).ok()).map(|m| m.len()).sum());
                continue;
            }
        }
        let mut total = 0u64;
        for a in crate::plans::install_archives(&e) {
            let Some((size, mtime)) = crate::made::stamp(&a) else { continue };
            let key = a.to_string_lossy().into_owned();
            let known: Option<i64> = conn
                .query_row("SELECT unpacked FROM serve_sizes WHERE path = ?1 AND size = ?2 AND mtime = ?3", params![key, size as i64, mtime], |r| r.get(0))
                .optional()?;
            let n = match known {
                Some(n) => n as u64,
                None => {
                    let kind = gamerepo_core::archive::kind_of(&key).unwrap_or("zip");
                    let Ok(l) = gamerepo_core::archive::read(&a, kind, false, &cancel) else { continue };
                    let n: u64 = l.members.iter().map(|m| m.hashes.size).sum();
                    conn.execute(
                        "INSERT INTO serve_sizes (path, size, mtime, unpacked) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(path) DO UPDATE SET size = excluded.size, mtime = excluded.mtime, unpacked = excluded.unpacked",
                        params![key, size as i64, mtime, n as i64],
                    )?;
                    n
                }
            };
            total += n;
        }
        if total > 0 {
            out.insert(e.game.id.clone(), total);
        }
    }
    Ok(out)
}

pub fn ensure_tables(ix: &Index) -> Result<()> {
    ix.conn().execute_batch(TABLES)?;
    Ok(())
}

fn meta_get(ix: &Index, key: &str) -> Result<Option<i64>> {
    Ok(ix.conn().query_row("SELECT value FROM serve_meta WHERE key = ?1", [key], |r| r.get::<_, String>(0)).optional()?.and_then(|v| v.parse().ok()))
}

fn meta_set(conn: &rusqlite::Connection, key: &str, v: i64) -> Result<()> {
    conn.execute("INSERT INTO serve_meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value", params![key, v.to_string()])?;
    Ok(())
}

/// The change marker now, and the first one this catalogue had (a marker from before it,
/// from another server or a catalogue made again, can't be followed).
pub fn revs(ix: &Index) -> Result<(i64, i64)> {
    Ok((meta_get(ix, "rev")?.unwrap_or(0), meta_get(ix, "base")?.unwrap_or(0)))
}

struct Row {
    key: String,
    system: String,
    title: String,
    sort: String,
    year: Option<i64>,
    size: i64,
    media: String,
    picture: Option<String>,
    /// The Library's keys it stands for, in disc order
    keys: Vec<String>,
}

impl Row {
    fn hash(&self) -> String {
        let s = serde_json::to_string(&(&self.system, &self.title, &self.sort, self.year, self.size, &self.media, &self.picture, &self.keys)).unwrap_or_default();
        crate::tls::sha256_hex(s.as_bytes())[..32].to_string()
    }
}

/// The picture's kind from its path in the library (`…/media/cover.png?v=…`: cover).
fn kind_of(picture: &str) -> Option<String> {
    let file = picture.split('?').next()?.rsplit('/').next()?;
    Some(file.rsplit_once('.').map(|(s, _)| s).unwrap_or(file).to_string())
}

/// Bring the catalogue up to date with the Library's rows: games changed, added or gone
/// get the next change marker. Returns how many changed.
pub fn refresh(ix: &Index) -> Result<usize> {
    refresh_with(ix, None)
}

/// [`refresh`], with the library's PC games by their unpacked size when it's given (else
/// by their files').
pub fn refresh_with(ix: &Index, lib: Option<&Library>) -> Result<usize> {
    ensure_tables(ix)?;
    let conn = ix.conn();
    let mut by_system: BTreeMap<String, Vec<(String, String, String, String, Option<i64>, i64, String, Option<String>)>> = BTreeMap::new();
    {
        // (updates and DLC are their games', not games to download on their own: D163; a game
        // on several discs is the Library's one row, D174, had whole when every disc is, and
        // its other discs are read for their keys and sizes)
        let mut others: HashMap<String, Vec<(String, String, String, i64)>> = HashMap::new();
        let mut q = conn.prepare("SELECT base, key, name, title, size FROM library WHERE kind = 'dat' AND addon = 'disc' AND have = 1 AND partial = 0")?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            others.entry(r.get(0)?).or_default().push((r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?));
        }
        let mut q = conn.prepare("SELECT key, platform, name, title, sort, year, size, media, picture, discs FROM library WHERE kind = 'dat' AND have = 1 AND partial = 0 AND addon = ''")?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            let platform: String = r.get(1)?;
            let key: String = r.get(0)?;
            let (name, title, sort, year, mut size, media, picture): (String, String, String, Option<i64>, i64, String, Option<String>) =
                (r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?);
            let list = by_system.entry(platform.clone()).or_default();
            let rest = if r.get::<_, i64>(9)? > 0 { others.remove(&key).unwrap_or_default() } else { vec![] };
            // (each disc by its own size and title, laid out below as one game or, where the
            // system takes no playlists, a game each)
            size -= rest.iter().map(|d| d.3).sum::<i64>();
            list.push((key, name, title.clone(), sort.clone(), year, size, media.clone(), picture.clone()));
            for (k, n, t, z) in rest {
                list.push((k, n, if platforms::m3u_folders(&platform) { title.clone() } else { t }, sort.clone(), year, z, media.clone(), picture.clone()));
            }
        }
    }
    let mut rows: Vec<Row> = vec![];
    for (system, games) in by_system {
        // a game on several discs is one game, by its first disc (as the ES-DE export lays it out)
        let discs = if platforms::m3u_folders(&system) { export::disc_sets(games.iter().map(|g| g.1.as_str())) } else { export::Discs::default() };
        let by_name: HashMap<&str, usize> = games.iter().enumerate().map(|(i, g)| (g.1.as_str(), i)).collect();
        for (_, _, names) in &discs.sets {
            let first = &games[by_name[names[0].as_str()]];
            let all: Vec<&_> = names.iter().map(|n| &games[by_name[n.as_str()]]).collect();
            rows.push(Row {
                key: first.0.clone(),
                system: system.clone(),
                title: first.2.clone(),
                sort: first.3.clone(),
                year: first.4,
                size: all.iter().map(|g| g.5).sum(),
                media: first.6.clone(),
                picture: first.7.clone(),
                keys: all.iter().map(|g| g.0.clone()).collect(),
            });
        }
        for g in &games {
            if discs.of.contains_key(&g.1) {
                continue;
            }
            rows.push(Row { key: g.0.clone(), system: system.clone(), title: g.2.clone(), sort: g.3.clone(), year: g.4, size: g.5, media: g.6.clone(), picture: g.7.clone(), keys: vec![g.0.clone()] });
        }
    }
    // the PC games (D110), each by its id
    {
        let sizes = match lib {
            Some(l) => pc_sizes(ix, l)?,
            None => HashMap::new(),
        };
        let mut q = conn.prepare("SELECT key, title, sort, year, size, media, picture FROM library WHERE kind = 'pc'")?;
        let mut found = q.query([])?;
        while let Some(r) = found.next()? {
            let key: String = r.get(0)?;
            let Some(id) = key.strip_prefix("pc/") else { continue };
            if lib.is_some() && !sizes.contains_key(id) {
                continue;
            }
            let size: i64 = sizes.get(id).map(|&s| s as i64).unwrap_or(r.get(4)?);
            rows.push(Row { key: format!("{PC}/{id}"), system: PC.into(), title: r.get(1)?, sort: r.get(2)?, year: r.get(3)?, size, media: r.get(5)?, picture: r.get(6)?, keys: vec![key.clone()] });
        }
    }
    let have: HashMap<String, (String, bool)> = conn
        .prepare("SELECT key, hash, gone FROM serve_games")?
        .query_map([], |r| Ok((r.get::<_, String>(0)?, (r.get::<_, String>(1)?, r.get::<_, i64>(2)? == 1))))?
        .collect::<rusqlite::Result<_>>()?;
    let (rev, base) = revs(ix)?;
    let tx = rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)?;
    // the first marker: the time, so a catalogue made again starts after any marker from before
    let next = if base == 0 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(1) } else { rev + 1 };
    let mut changed = 0;
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    tx.execute("DELETE FROM serve_keys", [])?;
    {
        let mut put = tx.prepare(
            "INSERT INTO serve_games (key, system, title, sort, year, size, media, picture, discs, hash, rev, gone) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 0)
             ON CONFLICT(key) DO UPDATE SET system = excluded.system, title = excluded.title, sort = excluded.sort, year = excluded.year, size = excluded.size,
                 media = excluded.media, picture = excluded.picture, discs = excluded.discs, hash = excluded.hash, rev = excluded.rev, gone = 0",
        )?;
        let mut keys = tx.prepare("INSERT OR REPLACE INTO serve_keys (lib_key, key, disc) VALUES (?1, ?2, ?3)")?;
        for r in &rows {
            seen.insert(&r.key);
            for (i, k) in r.keys.iter().enumerate() {
                keys.execute(params![k, r.key, i as i64])?;
            }
            let h = r.hash();
            if have.get(&r.key).is_some_and(|(old, gone)| *old == h && !gone) {
                continue;
            }
            let discs = if r.keys.len() > 1 { r.keys.len() as i64 } else { 0 };
            put.execute(params![r.key, r.system, r.title, r.sort, r.year, r.size, r.media, r.picture, discs, h, next])?;
            changed += 1;
        }
    }
    for (k, (_, gone)) in &have {
        if !gone && !seen.contains(k.as_str()) {
            tx.execute("UPDATE serve_games SET gone = 1, rev = ?2 WHERE key = ?1", params![k, next])?;
            changed += 1;
        }
    }
    if base == 0 {
        meta_set(&tx, "base", next)?;
        meta_set(&tx, "rev", next)?;
    } else if changed > 0 {
        meta_set(&tx, "rev", next)?;
    }
    tx.commit()?;
    Ok(changed)
}

const COLS: &str = "g.key, g.system, g.title, g.year, g.size, g.media, g.picture, g.discs";

fn game_of(r: &rusqlite::Row) -> rusqlite::Result<api::Game> {
    let media: String = r.get(5)?;
    let picture: Option<String> = r.get(6)?;
    Ok(api::Game {
        key: r.get(0)?,
        system: r.get(1)?,
        title: r.get(2)?,
        year: r.get(3)?,
        size: r.get::<_, i64>(4)? as u64,
        media: media.split(',').filter(|m| !m.is_empty()).map(String::from).collect(),
        picture: picture.as_deref().and_then(kind_of),
        discs: r.get::<_, i64>(7)? as u32,
    })
}

/// What `GET /v1/games` asks for.
#[derive(Clone, Default)]
pub struct Ask {
    pub system: String,
    /// Only a collection's games (and with `group`, only that group's)
    pub collection: String,
    pub group: Option<String>,
    pub search: String,
    /// title | year | size
    pub sort: String,
    pub desc: bool,
    pub offset: u64,
    pub limit: u64,
    /// The PC games too (a device that asks for them, D110)
    pub pc: bool,
}

/// The games a page gives unless asked for more.
pub const PAGE: u64 = 500;

/// Most games a page gives, for a client reading the whole list (D98).
pub const MOST: u64 = 5000;

/// A collection's games (of one group, or all), as the catalogue's keys.
pub fn collection_keys(ix: &Index, lib: &Library, id: &str, group: Option<&str>) -> Result<Vec<String>> {
    let c = collection::load(lib, id)?;
    let w = collection::work_out(ix, lib, &c)?;
    let lib_keys: Vec<String> = w
        .systems
        .iter()
        .filter(|s| group.is_none_or(|g| w.groups.get(s.group).is_some_and(|x| x.id == g)))
        .flat_map(|s| s.games.iter().map(collection::Game::key))
        .collect();
    let mut q = ix.conn().prepare("SELECT DISTINCT key FROM serve_keys WHERE lib_key IN (SELECT value FROM json_each(?1))")?;
    let v = q.query_map([serde_json::to_string(&lib_keys)?], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    Ok(v)
}

/// A page of games.
pub fn games(ix: &Index, lib: &Library, a: &Ask) -> Result<api::Page> {
    let mut w = vec!["g.gone = 0".to_string()];
    let mut p: Vec<Sql> = vec![];
    if !a.pc {
        w.push(format!("g.system != '{PC}'"));
    }
    if !a.system.is_empty() {
        w.push("g.system = ?".into());
        p.push(Sql::Text(a.system.clone()));
    }
    if !a.collection.is_empty() {
        w.push("g.key IN (SELECT value FROM json_each(?))".into());
        p.push(Sql::Text(serde_json::to_string(&collection_keys(ix, lib, &a.collection, a.group.as_deref())?)?));
    }
    if let Some(f) = browse::fts_query(&a.search) {
        w.push("g.key IN (SELECT k.key FROM serve_keys k JOIN library l ON l.key = k.lib_key WHERE l.id IN (SELECT rowid FROM library_text WHERE library_text MATCH ?))".into());
        p.push(Sql::Text(f));
    }
    let w = w.join(" AND ");
    let conn = ix.conn();
    let total: i64 = conn.query_row(&format!("SELECT COUNT(*) FROM serve_games g WHERE {w}"), params_from_iter(p.iter()), |r| r.get(0))?;
    let d = if a.desc { "DESC" } else { "ASC" };
    let order = match a.sort.as_str() {
        "year" => format!("g.year IS NULL, g.year {d}, g.sort {d}, g.key {d}"),
        "size" => format!("g.size {d}, g.sort {d}, g.key {d}"),
        _ => format!("g.sort {d}, g.key {d}"),
    };
    let mut pp = p.clone();
    pp.push(Sql::Integer(a.limit.clamp(1, MOST) as i64));
    pp.push(Sql::Integer(a.offset as i64));
    let items = conn
        .prepare(&format!("SELECT {COLS} FROM serve_games g WHERE {w} ORDER BY {order} LIMIT ? OFFSET ?"))?
        .query_map(params_from_iter(pp.iter()), game_of)?
        .collect::<rusqlite::Result<_>>()?;
    Ok(api::Page { items, total: total as u64, offset: a.offset, rev: revs(ix)?.0 })
}

/// Why a change marker can't be followed: the client fetches the whole list again (410).
pub struct TooOld;

/// The games changed since `since`, and the keys gone, a page at a time (the PC games with `pc`).
pub fn changes(ix: &Index, since: i64, offset: u64, pc: bool) -> Result<std::result::Result<api::Changes, TooOld>> {
    let (rev, base) = revs(ix)?;
    if since < base || since > rev {
        return Ok(Err(TooOld));
    }
    let conn = ix.conn();
    let rows: Vec<(api::Game, bool)> = conn
        .prepare(&format!("SELECT {COLS}, g.gone FROM serve_games g WHERE g.rev > ?1 AND (?4 OR g.system != '{PC}') ORDER BY g.rev, g.key LIMIT ?2 OFFSET ?3"))?
        .query_map(params![since, (PAGE + 1) as i64, offset as i64, pc], |r| Ok((game_of(r)?, r.get::<_, i64>(8)? == 1)))?
        .collect::<rusqlite::Result<_>>()?;
    let more = rows.len() as u64 > PAGE;
    let mut c = api::Changes { rev, more, ..Default::default() };
    for (g, gone) in rows.into_iter().take(PAGE as usize) {
        if gone {
            c.gone.push(g.key);
        } else {
            c.items.push(g);
        }
    }
    Ok(Ok(c))
}

/// The extensions of the files ES-DE lists as a system's games: those the library knows
/// for it, ZIP and 7-Zip archives for cartridge and arcade systems, CHD for disc systems,
/// and `.m3u` playlists where a game on several discs is one game (D94).
pub fn extensions(id: &str) -> Vec<String> {
    let mut v: Vec<String> = platforms::by_id(id).map(|p| p.exts.iter().map(|e| e.to_string()).collect()).unwrap_or_default();
    let more: &[&str] = match platforms::media(id) {
        platforms::Media::Cartridge | platforms::Media::Arcade => &["zip", "7z"],
        platforms::Media::Cd => &["chd"],
        _ => &[],
    };
    v.extend(more.iter().map(|e| e.to_string()));
    if platforms::m3u_folders(id) {
        v.push("m3u".into());
    }
    let mut seen = std::collections::HashSet::new();
    v.retain(|e| seen.insert(e.clone()));
    v
}

/// Each system with games, with its ES-DE folder; with `all`, every system Game Repo knows
/// besides (none of whose games are in the catalogue), each with the extensions ES-DE lists
/// as its games, so a device can tell what ES-DE has in a folder the library has nothing
/// for (D100). The PC games only with `pc`, with no folder (they go in the device's own).
pub fn systems(ix: &Index, lib: &Library, all: bool, pc: bool) -> Result<Vec<api::System>> {
    let esde = layout::get(Some(lib), "esde").ok();
    let folder = |id: &str| esde.as_ref().and_then(|l| l.folder(id)).unwrap_or_else(|| id.to_string());
    let mut q = ix.conn().prepare(&format!("SELECT system, COUNT(*), SUM(size) FROM serve_games WHERE gone = 0 AND (?1 OR system != '{PC}') GROUP BY system ORDER BY system"))?;
    let mut v: Vec<api::System> = q
        .query_map([pc], |r| {
            let id: String = r.get(0)?;
            let is_pc = id == PC;
            Ok(api::System {
                name: if is_pc { "PC games".into() } else { platforms::name_of(&id) },
                folder: if is_pc { String::new() } else { folder(&id) },
                games: r.get::<_, i64>(1)? as u64,
                size: r.get::<_, i64>(2)? as u64,
                extensions: if all && !is_pc { extensions(&id) } else { vec![] },
                id,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    if all {
        for p in platforms::all() {
            if p.id == "bios" || v.iter().any(|s| s.id == p.id) {
                continue;
            }
            v.push(api::System { id: p.id.to_string(), name: platforms::name_of(p.id), folder: folder(p.id), extensions: extensions(p.id), ..Default::default() });
        }
        v.sort_by(|a, b| a.id.cmp(&b.id));
    }
    Ok(v)
}

/// Each collection with its groups, counted in the catalogue's games.
pub fn collections(ix: &Index, lib: &Library) -> Result<Vec<api::Collection>> {
    let count = |keys: &[String]| -> Result<(u64, u64)> {
        Ok(ix.conn().query_row(
            "SELECT COUNT(*), COALESCE(SUM(size), 0) FROM serve_games WHERE gone = 0 AND key IN (SELECT value FROM json_each(?1))",
            [serde_json::to_string(keys)?],
            |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u64)),
        )?)
    };
    let mut out = vec![];
    for c in collection::list(lib) {
        let w = collection::work_out(ix, lib, &c)?;
        let mut groups = vec![];
        for g in &w.groups {
            let keys = collection_keys(ix, lib, &c.id, Some(&g.id))?;
            let (games, size) = count(&keys)?;
            groups.push(api::Group { id: g.id.clone(), name: g.name.clone(), games, size });
        }
        let (games, size) = count(&collection_keys(ix, lib, &c.id, None)?)?;
        out.push(api::Collection { id: c.id.clone(), name: c.name.clone(), games, size, groups });
    }
    Ok(out)
}

/// A catalogue game's Library keys, in disc order (one for a game on one disc); None: not
/// in the catalogue.
pub fn keys_of(ix: &Index, key: &str) -> Result<Option<Vec<String>>> {
    let v: Vec<String> = ix.conn().prepare("SELECT lib_key FROM serve_keys WHERE key = ?1 ORDER BY disc")?.query_map([key], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    Ok((!v.is_empty()).then_some(v))
}

/// The catalogue game a Library key belongs to.
pub fn game_for(ix: &Index, lib_key: &str) -> Result<Option<String>> {
    Ok(ix.conn().query_row("SELECT key FROM serve_keys WHERE lib_key = ?1", [lib_key], |r| r.get(0)).optional()?)
}

/// One game as the list gives it.
pub fn game(ix: &Index, key: &str) -> Result<Option<api::Game>> {
    Ok(ix.conn().query_row(&format!("SELECT {COLS} FROM serve_games g WHERE g.key = ?1 AND g.gone = 0"), [key], game_of).optional()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// The best of three runs.
    fn best<T>(f: &dyn Fn() -> T) -> (Duration, T) {
        let mut took = Duration::MAX;
        let mut v = None;
        for _ in 0..3 {
            let t = Instant::now();
            v = Some(f());
            took = took.min(t.elapsed());
        }
        (took, v.unwrap())
    }

    // The targets of docs/PLAN.md ("Phase 9a design"), on generated data. Run with
    // `cargo test --release -- --ignored timing` (CI does, on Windows).
    #[test]
    #[ignore]
    fn timing_a_page_of_a_catalogue_of_100k_games_and_its_changes() {
        let dir = std::env::temp_dir().join(format!("gamerepo-serve-timing-catalogue-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let lib = Library::open(dir.join("Lib")).unwrap();
        let ix = Index::open(&dir.join("index.sqlite"), "lib-x").unwrap();
        let syl = ["ka", "zo", "ri", "mu", "te", "lo", "shi", "an", "bel", "dor"];
        let word = |n: usize| format!("{}{}{}", syl[n % 10], syl[n / 10 % 10], syl[n / 100 % 10]);
        {
            let tx = ix.conn().unchecked_transaction().unwrap();
            let mut put = tx
                .prepare(
                    "INSERT INTO library (id, key, kind, platform, name, title, sort, year, have, media, picture, size, year_sort, rating_sort, size_sort, date_sort)
                     VALUES (?1, ?2, 'dat', ?3, ?4, ?5, ?6, ?7, 1, 'cover', ?8, ?9, 0, 0, 0, 0)",
                )
                .unwrap();
            let mut text = tx.prepare("INSERT INTO library_text (rowid, title, name, body) VALUES (?1, ?2, ?3, '')").unwrap();
            for i in 0..100_000usize {
                let system = format!("s{}", i % 10);
                let title = format!("{} {} {}", ["The", "Super", "Mega", "Hyper"][i % 4], word(i * 7919 % 1000), word(i / 3 % 1000));
                let name = format!("{title} (Game {i})");
                put.execute(params![
                    i as i64 + 1,
                    format!("{system}/{name}"),
                    system,
                    name,
                    title,
                    title.to_lowercase(),
                    (i % 7 != 0).then_some(1980 + (i % 40) as i64),
                    (i % 2 == 0).then(|| format!("_library/games/{system}/{i}/media/cover.png")),
                    (i * 1000) as i64
                ])
                .unwrap();
                text.execute(params![i as i64 + 1, title, name]).unwrap();
            }
            drop(put);
            drop(text);
            tx.commit().unwrap();
        }
        let t = Instant::now();
        assert_eq!(refresh(&ix).unwrap(), 100_000);
        let made = t.elapsed();
        ix.conn().execute_batch("ANALYZE").unwrap();
        let (ixr, libr) = (&ix, &lib);
        let page = |a: Ask| move || serde_json::to_vec(&games(ixr, libr, &a).unwrap()).unwrap();
        let ask = Ask { sort: "title".into(), limit: PAGE, ..Default::default() };
        let (first, v) = best(&page(Ask { ..ask.clone() }));
        let first_page: api::Page = serde_json::from_slice(&v).unwrap();
        assert_eq!((first_page.total, first_page.items.len() as u64), (100_000, PAGE));
        let mut deep = Duration::ZERO;
        for (sort, desc) in [("title", true), ("size", false), ("size", true), ("year", false), ("year", true)] {
            let (took, v) = best(&page(Ask { offset: 90_000, sort: sort.into(), desc, ..ask.clone() }));
            assert_eq!(serde_json::from_slice::<api::Page>(&v).unwrap().items.len() as u64, PAGE);
            eprintln!("  a page at 90,000 by {sort}{}: {took:?}", if desc { " (descending)" } else { "" });
            deep = deep.max(took);
        }
        for (sort, desc) in [("size", true), ("year", false), ("year", true)] {
            let (took, _) = best(&page(Ask { system: "s3".into(), offset: 9_000, sort: sort.into(), desc, ..ask.clone() }));
            eprintln!("  a system's page at 9,000 by {sort}{}: {took:?}", if desc { " (descending)" } else { "" });
            deep = deep.max(took);
        }
        let (system, v) = best(&page(Ask { system: "s3".into(), offset: 5_000, ..ask.clone() }));
        assert_eq!(serde_json::from_slice::<api::Page>(&v).unwrap().items.len() as u64, PAGE);
        let (search, v) = best(&page(Ask { search: word(123), ..ask.clone() }));
        assert!(serde_json::from_slice::<api::Page>(&v).unwrap().total > 0);
        // the same games again: nothing changed; then 1,000 changed and 500 gone
        let rev = revs(&ix).unwrap().0;
        assert_eq!(refresh(&ix).unwrap(), 0);
        ix.conn().execute_batch("UPDATE library SET size = size + 1 WHERE id % 100 = 0; DELETE FROM library WHERE id % 200 = 1;").unwrap();
        let t = Instant::now();
        assert_eq!(refresh(&ix).unwrap(), 1_500);
        let again = t.elapsed();
        let (changed, c) = best(&|| {
            let Ok(Ok(c)) = changes(&ix, rev, 0, false) else { panic!("the marker is followed") };
            serde_json::to_vec(&c).unwrap();
            c
        });
        assert_eq!((c.items.len() + c.gone.len(), c.more), (PAGE as usize, true));
        eprintln!(
            "100,000 games: catalogue made in {made:?}, made again after 1,500 changes in {again:?}; first page {first:?}, the slowest deep page {deep:?}, \
             a system's page {system:?}, searched {search:?}; changes {changed:?}"
        );
        for (what, took, most) in [("the first page", first, 0.2), ("a deep page", deep, 0.2), ("a system's page", system, 0.2), ("a search", search, 0.2), ("the changes", changed, 0.1)] {
            assert!(took.as_secs_f64() < most, "{what} took {took:?} (at most {most} s)");
        }
        drop(ix);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
