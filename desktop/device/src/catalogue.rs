//! The catalogue kept on the device (docs/PLAN.md, "Phase 9b design", D98): the systems,
//! the collections and every game as the server's list gives them, in SQLite with FTS5, so
//! browsing and search are the device's own and work away from home. It's read whole the
//! first time and then only by what changed (`/v1/changes?since=`), whole again when the
//! server says the marker is too old. Pictures are kept as they're shown, up to a size,
//! the least recently shown going first; and what the server said a device's file is
//! (`/v1/identify`) is kept by the file's path, size and time.

use crate::client::{Refused, Server};
use anyhow::Result;
use gamerepo_wire::api::{Changes, Collection, Game, Page, System};
use rusqlite::{params, params_from_iter, types::Value as Sql, Connection, OptionalExtension};
use serde_json::Value;
use sha1::Digest;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// Games asked for at once when the list is read whole.
pub const READ_AT_ONCE: u64 = 5000;

/// The picture cache's size unless it's set (D98).
pub const PICTURES: u64 = 500 << 20;

const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL) STRICT;
    CREATE TABLE IF NOT EXISTS systems (id TEXT PRIMARY KEY, name TEXT NOT NULL, folder TEXT NOT NULL, extensions TEXT NOT NULL, games INTEGER NOT NULL, size INTEGER NOT NULL) STRICT;
    CREATE TABLE IF NOT EXISTS collections (id TEXT PRIMARY KEY, pos INTEGER NOT NULL, name TEXT NOT NULL, games INTEGER NOT NULL, size INTEGER NOT NULL, groups TEXT NOT NULL) STRICT;
    CREATE TABLE IF NOT EXISTS members (collection TEXT NOT NULL, grp TEXT NOT NULL, key TEXT NOT NULL, PRIMARY KEY (collection, grp, key)) WITHOUT ROWID, STRICT;
    CREATE TABLE IF NOT EXISTS games (
        id INTEGER PRIMARY KEY, key TEXT NOT NULL UNIQUE, system TEXT NOT NULL, title TEXT NOT NULL, sort TEXT NOT NULL,
        year INTEGER, size INTEGER NOT NULL, media TEXT NOT NULL, picture TEXT, discs INTEGER NOT NULL
    ) STRICT;
    -- a page in each order without sorting the whole list, as the server's (a year's own each way round: unknown last);
    -- the system last, so a page of the systems on the device is read in order from the index alone
    CREATE INDEX IF NOT EXISTS games_system ON games (system, sort, key);
    CREATE INDEX IF NOT EXISTS games_sort ON games (sort, key, system);
    CREATE INDEX IF NOT EXISTS games_size ON games (size, sort, key, system);
    CREATE INDEX IF NOT EXISTS games_year ON games (year IS NULL, year, sort, key, system);
    CREATE INDEX IF NOT EXISTS games_year_desc ON games (year IS NULL, year DESC, sort DESC, key DESC, system);
    CREATE INDEX IF NOT EXISTS games_system_size ON games (system, size, sort, key);
    CREATE INDEX IF NOT EXISTS games_system_year ON games (system, year IS NULL, year, sort, key);
    CREATE INDEX IF NOT EXISTS games_system_year_desc ON games (system, year IS NULL, year DESC, sort DESC, key DESC);
    -- the titles' words, kept by hand beside the games (external content, no triggers)
    CREATE VIRTUAL TABLE IF NOT EXISTS games_text USING fts5(title, content='games', content_rowid='id', tokenize='unicode61 remove_diacritics 2');
    CREATE TABLE IF NOT EXISTS details (key TEXT PRIMARY KEY, json TEXT NOT NULL, fetched INTEGER NOT NULL) STRICT;
    CREATE TABLE IF NOT EXISTS pictures (name TEXT PRIMARY KEY, key TEXT NOT NULL, bytes INTEGER NOT NULL, used INTEGER NOT NULL) STRICT;
    CREATE INDEX IF NOT EXISTS pictures_key ON pictures (key);
    CREATE INDEX IF NOT EXISTS pictures_used ON pictures (used);
    CREATE TABLE IF NOT EXISTS identified (path TEXT PRIMARY KEY, size INTEGER NOT NULL, mtime INTEGER NOT NULL, key TEXT NOT NULL) STRICT;
";

pub struct Catalogue {
    dir: PathBuf,
    /// Browsing reads here while a sync writes on the other (WAL: readers see what was
    /// there until the sync's done)
    read: Mutex<Connection>,
    write: Mutex<Connection>,
    pictures: AtomicU64,
}

/// What a sync did.
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct Synced {
    /// Read whole (the first time, another server, or a marker too old)
    pub whole: bool,
    /// Games added, changed or gone
    pub changed: u64,
    pub games: u64,
}

/// What a page of the list asks for (`GET /v1/games`'s, and what the device adds).
#[derive(Debug, Default, Clone, serde::Deserialize)]
#[serde(default)]
pub struct Ask {
    pub system: String,
    pub collection: String,
    pub group: Option<String>,
    pub q: String,
    /// title | year | size
    pub sort: String,
    pub desc: bool,
    pub offset: u64,
    pub limit: u64,
    /// Only these systems (those the device has a folder for, C6); none: every one
    #[serde(skip)]
    pub systems: Option<Vec<String>>,
    /// Games not shown (those on the device, C6)
    #[serde(skip)]
    pub hide: Vec<String>,
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// How a title sorts: in lower case.
fn sort_key(title: &str) -> String {
    title.to_lowercase()
}

/// A search's words as FTS5 asks for them: each word as the start of one, all of them.
fn fts(q: &str) -> Option<String> {
    let words: Vec<String> = q.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(|w| format!("\"{w}\"*")).collect();
    (!words.is_empty()).then(|| words.join(" "))
}

/// The server's refusal's status, if the error is one.
pub fn status_of(e: &anyhow::Error) -> u16 {
    e.downcast_ref::<Refused>().map(|r| r.status).unwrap_or(0)
}

/// A query's value, every byte but letters, digits and `-._~` escaped.
pub fn query_value(s: &str) -> String {
    s.bytes().map(|c| if c.is_ascii_alphanumeric() || b"-._~".contains(&c) { (c as char).to_string() } else { format!("%{c:02X}") }).collect()
}

/// A key in a path (`gb/Alpha (Europe)`), each byte that isn't a path's own escaped.
pub fn path_value(s: &str) -> String {
    s.bytes().map(|c| if c.is_ascii_alphanumeric() || b"-._~/".contains(&c) { (c as char).to_string() } else { format!("%{c:02X}") }).collect()
}

fn game_of(r: &rusqlite::Row) -> rusqlite::Result<Game> {
    let media: String = r.get(5)?;
    Ok(Game {
        key: r.get(0)?,
        system: r.get(1)?,
        title: r.get(2)?,
        year: r.get(3)?,
        size: r.get::<_, i64>(4)? as u64,
        media: media.split(',').filter(|m| !m.is_empty()).map(String::from).collect(),
        picture: r.get(6)?,
        discs: r.get::<_, i64>(7)? as u32,
    })
}

const COLS: &str = "g.key, g.system, g.title, g.year, g.size, g.media, g.picture, g.discs";

/// A game put in the list, or put there again as it is now, its words with it.
fn put(tx: &Connection, g: &Game) -> Result<()> {
    if let Some((id, title)) = tx.query_row("SELECT id, title FROM games WHERE key = ?1", [&g.key], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))).optional()? {
        tx.execute("INSERT INTO games_text (games_text, rowid, title) VALUES ('delete', ?1, ?2)", params![id, title])?;
        tx.execute(
            "UPDATE games SET system = ?2, title = ?3, sort = ?4, year = ?5, size = ?6, media = ?7, picture = ?8, discs = ?9 WHERE id = ?1",
            params![id, g.system, g.title, sort_key(&g.title), g.year, g.size as i64, g.media.join(","), g.picture, g.discs as i64],
        )?;
        tx.execute("INSERT INTO games_text (rowid, title) VALUES (?1, ?2)", params![id, g.title])?;
    } else {
        tx.execute(
            "INSERT INTO games (key, system, title, sort, year, size, media, picture, discs) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![g.key, g.system, g.title, sort_key(&g.title), g.year, g.size as i64, g.media.join(","), g.picture, g.discs as i64],
        )?;
        tx.execute("INSERT INTO games_text (rowid, title) VALUES (?1, ?2)", params![tx.last_insert_rowid(), g.title])?;
    }
    Ok(())
}

/// A game taken out of the list, with its words, details and pictures' rows.
fn drop_game(tx: &Connection, key: &str) -> Result<Vec<String>> {
    if let Some((id, title)) = tx.query_row("SELECT id, title FROM games WHERE key = ?1", [key], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))).optional()? {
        tx.execute("INSERT INTO games_text (games_text, rowid, title) VALUES ('delete', ?1, ?2)", params![id, title])?;
        tx.execute("DELETE FROM games WHERE id = ?1", [id])?;
    }
    stale(tx, key)
}

/// A game's kept details and pictures, no longer as the server has them: the pictures'
/// files to delete.
fn stale(tx: &Connection, key: &str) -> Result<Vec<String>> {
    tx.execute("DELETE FROM details WHERE key = ?1", [key])?;
    let names: Vec<String> = tx.prepare("SELECT name FROM pictures WHERE key = ?1")?.query_map([key], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    tx.execute("DELETE FROM pictures WHERE key = ?1", [key])?;
    Ok(names)
}

impl Catalogue {
    /// The catalogue in `dir` (made the first time).
    pub fn open(dir: &Path) -> Result<Catalogue> {
        std::fs::create_dir_all(dir.join("pictures"))?;
        let path = dir.join("catalogue.sqlite");
        let write = Connection::open(&path)?;
        write.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")?;
        write.execute_batch(SCHEMA)?;
        let read = Connection::open(&path)?;
        Ok(Catalogue { dir: dir.to_path_buf(), read: Mutex::new(read), write: Mutex::new(write), pictures: AtomicU64::new(PICTURES) })
    }

    fn reader(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.read.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn writer(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.write.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn meta(&self, key: &str) -> Option<String> {
        self.reader().query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0)).optional().ok().flatten()
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.writer().execute("INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value", params![key, value])?;
        Ok(())
    }

    /// How many games it holds, and the server they're from.
    pub fn counts(&self) -> (u64, Option<String>) {
        let n: i64 = self.reader().query_row("SELECT COUNT(*) FROM games", [], |r| r.get(0)).unwrap_or(0);
        (n as u64, self.meta("server"))
    }

    /// Everything forgotten: the device was paired with another library, or unpaired.
    pub fn forget(&self) -> Result<()> {
        self.writer().execute_batch(
            "DELETE FROM meta; DELETE FROM systems; DELETE FROM collections; DELETE FROM members; DELETE FROM games;
             INSERT INTO games_text (games_text) VALUES ('delete-all'); DELETE FROM details; DELETE FROM pictures; DELETE FROM identified;",
        )?;
        for e in std::fs::read_dir(self.dir.join("pictures")).into_iter().flatten().flatten() {
            let _ = std::fs::remove_file(e.path());
        }
        Ok(())
    }

    /// Brought up to date with the server: its systems and collections, then the games,
    /// whole the first time (`progress` told how many of how many), else only what changed.
    pub fn sync(&self, s: &Server, progress: &dyn Fn(u64, u64)) -> Result<Synced> {
        if self.meta("server").as_deref() != Some(s.server.as_str()) {
            self.forget()?;
            self.set_meta("server", &s.server)?;
        }
        let systems: Vec<System> = s.get("/v1/systems?all=1")?;
        let collections: Vec<Collection> = s.get("/v1/collections")?;
        // each collection's games, group by group, as keys
        let mut members: Vec<(String, String, String)> = vec![];
        for c in &collections {
            let groups: Vec<String> = if c.groups.is_empty() { vec![String::new()] } else { c.groups.iter().map(|g| g.id.clone()).collect() };
            for g in groups {
                let mut offset = 0;
                loop {
                    let page: Page = s.get(&format!("/v1/games?collection={}&group={}&limit={READ_AT_ONCE}&offset={offset}", query_value(&c.id), query_value(&g)))?;
                    offset += page.items.len() as u64;
                    members.extend(page.items.into_iter().map(|x| (c.id.clone(), g.clone(), x.key)));
                    if offset >= page.total || offset == 0 {
                        break;
                    }
                }
            }
        }
        {
            let mut w = self.writer();
            let tx = w.transaction()?;
            tx.execute("DELETE FROM systems", [])?;
            for x in &systems {
                tx.execute(
                    "INSERT INTO systems (id, name, folder, extensions, games, size) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![x.id, x.name, x.folder, serde_json::to_string(&x.extensions)?, x.games as i64, x.size as i64],
                )?;
            }
            tx.execute("DELETE FROM collections", [])?;
            for (i, c) in collections.iter().enumerate() {
                tx.execute(
                    "INSERT INTO collections (id, pos, name, games, size, groups) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![c.id, i as i64, c.name, c.games as i64, c.size as i64, serde_json::to_string(&c.groups)?],
                )?;
            }
            tx.execute("DELETE FROM members", [])?;
            for (c, g, k) in &members {
                tx.execute("INSERT OR IGNORE INTO members (collection, grp, key) VALUES (?1, ?2, ?3)", params![c, g, k])?;
            }
            tx.commit()?;
        }
        let rev: i64 = self.meta("rev").and_then(|r| r.parse().ok()).unwrap_or(0);
        let mut out = Synced::default();
        let rev = if rev == 0 { self.read_whole(s, progress, &mut out)? } else { rev };
        match self.changes(s, rev, &mut out) {
            Err(e) if status_of(&e) == 410 => {
                let rev = self.read_whole(s, progress, &mut out)?;
                self.changes(s, rev, &mut out)?;
            }
            r => r?,
        }
        out.games = self.counts().0;
        self.set_meta("synced", &now().to_string())?;
        Ok(out)
    }

    /// The whole list read again, a page at a time: the change marker it was read at.
    fn read_whole(&self, s: &Server, progress: &dyn Fn(u64, u64), out: &mut Synced) -> Result<i64> {
        out.whole = true;
        let mut w = self.writer();
        let tx = w.transaction()?;
        tx.execute_batch("DELETE FROM games; INSERT INTO games_text (games_text) VALUES ('delete-all');")?;
        let (mut offset, mut rev) = (0u64, None);
        loop {
            let page: Page = s.get(&format!("/v1/games?limit={READ_AT_ONCE}&offset={offset}"))?;
            rev.get_or_insert(page.rev);
            for g in &page.items {
                put(&tx, g)?;
            }
            offset += page.items.len() as u64;
            progress(offset.min(page.total), page.total);
            out.changed += page.items.len() as u64;
            if page.items.is_empty() || offset >= page.total {
                break;
            }
        }
        let rev = rev.unwrap_or(0);
        // the details and pictures kept of games still there stay (the same library)
        let mut gone: Vec<String> = tx.prepare("SELECT name FROM pictures WHERE key NOT IN (SELECT key FROM games)")?.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        tx.execute_batch("DELETE FROM pictures WHERE key NOT IN (SELECT key FROM games); DELETE FROM details WHERE key NOT IN (SELECT key FROM games);")?;
        tx.execute("INSERT INTO meta (key, value) VALUES ('rev', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value", [rev.to_string()])?;
        tx.commit()?;
        drop(w);
        gone.retain(|n| !n.is_empty());
        self.delete_pictures(&gone);
        Ok(rev)
    }

    /// What changed since `rev`, a page at a time, taken in.
    fn changes(&self, s: &Server, rev: i64, out: &mut Synced) -> Result<()> {
        let mut offset = 0u64;
        let mut seen: HashSet<String> = HashSet::new();
        let mut to_delete = vec![];
        let mut w = self.writer();
        let tx = w.transaction()?;
        let new_rev = loop {
            let c: Changes = s.get(&format!("/v1/changes?since={rev}&offset={offset}"))?;
            for g in &c.items {
                to_delete.extend(stale(&tx, &g.key)?);
                put(&tx, g)?;
                seen.insert(g.key.clone());
            }
            for k in &c.gone {
                to_delete.extend(drop_game(&tx, k)?);
                seen.insert(k.clone());
            }
            offset += (c.items.len() + c.gone.len()) as u64;
            if !c.more {
                break c.rev;
            }
        };
        tx.execute("INSERT INTO meta (key, value) VALUES ('rev', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value", [new_rev.to_string()])?;
        tx.commit()?;
        drop(w);
        out.changed += seen.len() as u64;
        self.delete_pictures(&to_delete);
        Ok(())
    }

    pub fn systems(&self) -> Result<Vec<System>> {
        let r = self.reader();
        let v = r
            .prepare("SELECT id, name, folder, extensions, games, size FROM systems ORDER BY name")?
            .query_map([], |r| {
                let ext: String = r.get(3)?;
                Ok(System { id: r.get(0)?, name: r.get(1)?, folder: r.get(2)?, extensions: serde_json::from_str(&ext).unwrap_or_default(), games: r.get::<_, i64>(4)? as u64, size: r.get::<_, i64>(5)? as u64 })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(v)
    }

    pub fn collections(&self) -> Result<Vec<Collection>> {
        let r = self.reader();
        let v = r
            .prepare("SELECT id, name, games, size, groups FROM collections ORDER BY pos")?
            .query_map([], |r| {
                let groups: String = r.get(4)?;
                Ok(Collection { id: r.get(0)?, name: r.get(1)?, games: r.get::<_, i64>(2)? as u64, size: r.get::<_, i64>(3)? as u64, groups: serde_json::from_str(&groups).unwrap_or_default() })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(v)
    }

    /// A page of games, from the device's own copy.
    pub fn games(&self, a: &Ask) -> Result<Page> {
        let mut w = vec!["1".to_string()];
        let mut p: Vec<Sql> = vec![];
        if !a.system.is_empty() {
            w.push("g.system = ?".into());
            p.push(Sql::Text(a.system.clone()));
        }
        if let Some(systems) = &a.systems {
            // the page reads its order's index and passes over other systems' games (`+`:
            // not by the system's index, which would sort what it finds); the count uses it
            w.push("{+}g.system IN (SELECT value FROM json_each(?))".into());
            p.push(Sql::Text(serde_json::to_string(systems)?));
        }
        if !a.collection.is_empty() {
            match &a.group {
                Some(g) => {
                    w.push("g.key IN (SELECT key FROM members WHERE collection = ? AND grp = ?)".into());
                    p.push(Sql::Text(a.collection.clone()));
                    p.push(Sql::Text(g.clone()));
                }
                None => {
                    w.push("g.key IN (SELECT key FROM members WHERE collection = ?)".into());
                    p.push(Sql::Text(a.collection.clone()));
                }
            }
        }
        if let Some(f) = fts(&a.q) {
            w.push("g.id IN (SELECT rowid FROM games_text WHERE games_text MATCH ?)".into());
            p.push(Sql::Text(f));
        }
        if !a.hide.is_empty() {
            w.push("g.key NOT IN (SELECT value FROM json_each(?))".into());
            p.push(Sql::Text(serde_json::to_string(&a.hide)?));
        }
        let w = w.join(" AND ");
        let r = self.reader();
        let total: i64 = r.query_row(&format!("SELECT COUNT(*) FROM games g WHERE {}", w.replace("{+}", "")), params_from_iter(p.iter()), |r| r.get(0))?;
        let w = w.replace("{+}", "+");
        let d = if a.desc { "DESC" } else { "ASC" };
        let order = match a.sort.as_str() {
            "year" => format!("g.year IS NULL, g.year {d}, g.sort {d}, g.key {d}"),
            "size" => format!("g.size {d}, g.sort {d}, g.key {d}"),
            _ => format!("g.sort {d}, g.key {d}"),
        };
        let mut pp = p.clone();
        pp.push(Sql::Integer(if a.limit == 0 { 100 } else { a.limit.min(READ_AT_ONCE) } as i64));
        pp.push(Sql::Integer(a.offset as i64));
        let items = r.prepare(&format!("SELECT {COLS} FROM games g WHERE {w} ORDER BY {order} LIMIT ? OFFSET ?"))?.query_map(params_from_iter(pp.iter()), game_of)?.collect::<rusqlite::Result<_>>()?;
        let rev: Option<String> = r.query_row("SELECT value FROM meta WHERE key = 'rev'", [], |r| r.get(0)).optional()?;
        let rev = rev.and_then(|r| r.parse().ok()).unwrap_or(0);
        Ok(Page { items, total: total as u64, offset: a.offset, rev })
    }

    /// One game as the list has it.
    pub fn game(&self, key: &str) -> Option<Game> {
        self.reader().query_row(&format!("SELECT {COLS} FROM games g WHERE g.key = ?1"), [key], game_of).optional().ok().flatten()
    }

    /// A game's details (`GET /v1/games/<key>`): the server's when it answers, kept for when
    /// it doesn't.
    pub fn details(&self, s: Option<&Server>, key: &str) -> Result<Option<Value>> {
        if let Some(s) = s {
            match s.get::<Value>(&format!("/v1/games/{}", path_value(key))) {
                Ok(v) => {
                    self.writer().execute(
                        "INSERT INTO details (key, json, fetched) VALUES (?1, ?2, ?3) ON CONFLICT(key) DO UPDATE SET json = excluded.json, fetched = excluded.fetched",
                        params![key, v.to_string(), now()],
                    )?;
                    return Ok(Some(v));
                }
                Err(e) if status_of(&e) == 404 => return Ok(None),
                Err(_) => {}
            }
        }
        let kept: Option<String> = self.reader().query_row("SELECT json FROM details WHERE key = ?1", [key], |r| r.get(0)).optional()?;
        Ok(kept.and_then(|j| serde_json::from_str(&j).ok()))
    }

    /// The picture cache's size.
    pub fn set_picture_limit(&self, bytes: u64) {
        self.pictures.store(bytes.max(16 << 20), Ordering::SeqCst);
        let _ = self.prune();
    }

    fn picture_path(&self, name: &str) -> PathBuf {
        self.dir.join("pictures").join(name)
    }

    fn delete_pictures(&self, names: &[String]) {
        for n in names {
            let _ = std::fs::remove_file(self.picture_path(n));
        }
    }

    /// A game's picture of a kind at a size (`GET /v1/pictures/…`): kept on the device once
    /// it's been shown; None when the server can't be reached and it isn't kept.
    pub fn picture(&self, s: Option<&Server>, key: &str, kind: &str, size: u32) -> Result<Option<Vec<u8>>> {
        let name = format!("{}.jpg", &hex::encode(sha1::Sha1::digest(format!("{key}\0{kind}\0{size}").as_bytes()))[..24]);
        let path = self.picture_path(&name);
        if let Ok(b) = std::fs::read(&path) {
            let _ = self.writer().execute("UPDATE pictures SET used = ?2 WHERE name = ?1", params![name, now()]);
            return Ok(Some(b));
        }
        let Some(s) = s else { return Ok(None) };
        let (head, mut body) = match s.open_request("GET", &format!("/v1/pictures/{}/{}?size={size}", path_value(key), query_value(kind))) {
            Ok(x) => x,
            Err(_) => return Ok(None),
        };
        if head.status != 200 {
            return Ok(None);
        }
        let mut b = vec![0u8; head.length as usize];
        std::io::Read::read_exact(&mut body, &mut b)?;
        let tmp = path.with_extension("jpg.part");
        std::fs::write(&tmp, &b)?;
        std::fs::rename(&tmp, &path)?;
        self.writer().execute(
            "INSERT INTO pictures (name, key, bytes, used) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(name) DO UPDATE SET bytes = excluded.bytes, used = excluded.used",
            params![name, key, b.len() as i64, now()],
        )?;
        self.prune()?;
        Ok(Some(b))
    }

    /// The pictures shown longest ago deleted, until the cache is under nine tenths of its size.
    fn prune(&self) -> Result<()> {
        let limit = self.pictures.load(Ordering::SeqCst) as i64;
        let w = self.writer();
        let total: i64 = w.query_row("SELECT COALESCE(SUM(bytes), 0) FROM pictures", [], |r| r.get(0))?;
        if total <= limit {
            return Ok(());
        }
        let mut over = total - limit * 9 / 10;
        let oldest: Vec<(String, i64)> = w.prepare("SELECT name, bytes FROM pictures ORDER BY used, name")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        for (name, bytes) in oldest {
            if over <= 0 {
                break;
            }
            w.execute("DELETE FROM pictures WHERE name = ?1", [&name])?;
            let _ = std::fs::remove_file(self.picture_path(&name));
            over -= bytes;
        }
        Ok(())
    }

    /// The pictures kept: how many, and their size.
    pub fn pictures_kept(&self) -> (u64, u64) {
        self.reader().query_row("SELECT COUNT(*), COALESCE(SUM(bytes), 0) FROM pictures", [], |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u64))).unwrap_or((0, 0))
    }

    /// What the server said a device's file is, when it's as it was then: Some(None) not a
    /// game of the library's; None: not asked yet.
    pub fn identified(&self, path: &str, size: u64, mtime: i64) -> Option<Option<String>> {
        let row: Option<(i64, i64, String)> = self.reader().query_row("SELECT size, mtime, key FROM identified WHERE path = ?1", [path], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional().ok().flatten();
        match row {
            Some((s, m, k)) if s as u64 == size && m == mtime => Some((!k.is_empty()).then_some(k)),
            _ => None,
        }
    }

    pub fn set_identified(&self, rows: &[(String, u64, i64, Option<String>)]) -> Result<()> {
        let mut w = self.writer();
        let tx = w.transaction()?;
        for (path, size, mtime, key) in rows {
            tx.execute(
                "INSERT INTO identified (path, size, mtime, key) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(path) DO UPDATE SET size = excluded.size, mtime = excluded.mtime, key = excluded.key",
                params![path, *size as i64, mtime, key.clone().unwrap_or_default()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}
