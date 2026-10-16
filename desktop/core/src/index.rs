//! The library's index (docs/PLAN.md, section 4, "Index: where this app departs
//! from Model Library"): one SQLite file per library on each computer,
//! `<data dir>/index/<library id>.sqlite`, never inside the library. It holds what
//! the app has worked out from the files (hashes, DAT matches, games), so it can
//! always be made again from the library and the folders it points at; nothing
//! the user needs lives only here.
//!
//! Its tables are built by [`MIGRATIONS`], applied in order and counted in
//! SQLite's `user_version`. Each phase adds steps at the end; a step that has
//! shipped is never changed. An index this build can't use (made by a newer
//! version of the app, or belonging to another library) is set aside as
//! `<library id>.old.sqlite` and a fresh one is made.

use crate::library::now;
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

/// The index's schema, one step at a time: (what it adds, SQL). Phase 0 has only
/// the index's own details; the tables for files, DATs and games come with the
/// phases that fill them.
const MIGRATIONS: &[(&str, &str)] = &[(
    "the index's own details",
    "CREATE TABLE index_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL) STRICT;",
)];

/// How many schema steps this build knows.
pub fn schema_version() -> u32 {
    MIGRATIONS.len() as u32
}

/// Where the index of the library with this id is kept on this computer.
pub fn path_for(data_dir: &Path, library_id: &str) -> PathBuf {
    data_dir.join("index").join(format!("{library_id}.sqlite"))
}

pub struct Index {
    conn: Connection,
    path: PathBuf,
    /// Why an earlier index was set aside when this one was opened, if it was.
    pub set_aside: Option<String>,
}

impl Index {
    /// Open the index at `path` for the library `library_id`, making it (or
    /// bringing its tables up to date) as needed.
    pub fn open(path: &Path, library_id: &str) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("couldn't create {}", dir.display()))?;
        }
        let mut set_aside = None;
        if path.is_file() {
            if let Some(why) = unusable(path, library_id)? {
                move_aside(path)?;
                set_aside = Some(why);
            }
        }
        let conn = Connection::open(path)
            .with_context(|| format!("couldn't open the index {}", path.display()))?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let mut ix = Self {
            conn,
            path: path.to_path_buf(),
            set_aside,
        };
        ix.migrate()?;
        let created: Option<String> = ix.meta("created")?;
        if created.is_none() {
            ix.set_meta("library_id", library_id)?;
            ix.set_meta("created", &now())?;
        }
        Ok(ix)
    }

    /// Apply the schema steps this index doesn't have yet, each in its own transaction.
    fn migrate(&mut self) -> Result<()> {
        let have = self.version()?;
        for (i, (what, sql)) in MIGRATIONS.iter().enumerate().skip(have as usize) {
            let tx = self.conn.transaction()?;
            tx.execute_batch(sql)
                .with_context(|| format!("couldn't add {what} to the index"))?;
            tx.pragma_update(None, "user_version", (i + 1) as u32)?;
            tx.commit()?;
        }
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// The schema step this index is at.
    pub fn version(&self) -> Result<u32> {
        Ok(self
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))?)
    }

    pub fn meta(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT value FROM index_meta WHERE key = ?1",
                params![key],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO index_meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Rows in each of the index's tables, by name.
    pub fn counts(&self) -> Result<Map<String, Value>> {
        let mut names: Vec<String> = vec![];
        let mut q = self.conn.prepare(
            "SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )?;
        for name in q.query_map([], |r| r.get::<_, String>(0))? {
            names.push(name?);
        }
        let mut out = Map::new();
        for name in names {
            let n: i64 = self.conn.query_row(
                &format!("SELECT COUNT(*) FROM \"{}\"", name.replace('"', "\"\"")),
                [],
                |r| r.get(0),
            )?;
            out.insert(name, json!(n));
        }
        Ok(out)
    }

    /// The index file and its write-ahead log, in bytes.
    pub fn size(&self) -> u64 {
        [self.path.clone(), sidecar(&self.path, "-wal")]
            .iter()
            .filter_map(|p| std::fs::metadata(p).ok())
            .map(|m| m.len())
            .sum()
    }

    /// What the page shows about the index.
    pub fn info(&self) -> Result<Value> {
        Ok(json!({
            "path": self.path.display().to_string(),
            "version": self.version()?,
            "schema_version": schema_version(),
            "created": self.meta("created")?,
            "size": self.size(),
            "tables": self.counts()?,
            "set_aside": self.set_aside,
        }))
    }

    /// SQLite's own check of the file: Ok(empty) when it's sound, else what it found.
    pub fn check(&self) -> Result<Vec<String>> {
        let mut q = self.conn.prepare("PRAGMA integrity_check")?;
        let found: Vec<String> = q
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(if found == ["ok"] { vec![] } else { found })
    }
}

/// `index.sqlite` -> `index.sqlite-wal` (SQLite's own files beside the index).
fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// Why the index at `path` can't be used for `library_id` (None: it can).
fn unusable(path: &Path, library_id: &str) -> Result<Option<String>> {
    let conn = match Connection::open(path) {
        Ok(c) => c,
        Err(e) => return Ok(Some(format!("it couldn't be opened ({e})"))),
    };
    let version: u32 = match conn.pragma_query_value(None, "user_version", |r| r.get(0)) {
        Ok(v) => v,
        Err(e) => return Ok(Some(format!("it couldn't be read ({e})"))),
    };
    if version > schema_version() {
        return Ok(Some(format!(
            "it was made by a newer version of Game Repo (index step {version}; this version knows {})",
            schema_version()
        )));
    }
    if version == 0 {
        return Ok(None);
    }
    let owner: Option<String> = conn
        .query_row(
            "SELECT value FROM index_meta WHERE key = 'library_id'",
            [],
            |r| r.get(0),
        )
        .optional()
        .unwrap_or(None);
    match owner {
        Some(id) if id != library_id => Ok(Some(format!("it belongs to another library ({id})"))),
        _ => Ok(None),
    }
}

/// Move an index (and SQLite's files beside it) to `<name>.old.sqlite`, replacing
/// an older one set aside before.
fn move_aside(path: &Path) -> Result<()> {
    let old = path.with_extension("old.sqlite");
    for suffix in ["", "-wal", "-shm"] {
        let from = sidecar(path, suffix);
        let to = sidecar(&old, suffix);
        let _ = std::fs::remove_file(&to);
        if from.exists() {
            if let Err(e) = std::fs::rename(&from, &to) {
                bail!("couldn't set aside the old index {}: {e}", from.display());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::tests::temp_dir;

    #[test]
    fn makes_an_index_with_every_step() {
        let dir = temp_dir("index-new");
        let path = path_for(&dir, "lib-1");
        let ix = Index::open(&path, "lib-1").unwrap();
        assert!(path.is_file());
        assert_eq!(ix.version().unwrap(), schema_version());
        assert_eq!(ix.meta("library_id").unwrap().as_deref(), Some("lib-1"));
        let info = ix.info().unwrap();
        assert_eq!(info["tables"]["index_meta"], json!(2));
        assert!(info["set_aside"].is_null());
        assert!(ix.check().unwrap().is_empty());
        let created = ix.meta("created").unwrap();
        drop(ix);
        let again = Index::open(&path, "lib-1").unwrap();
        assert_eq!(again.meta("created").unwrap(), created);
        assert!(again.set_aside.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_index_from_a_newer_app_is_set_aside() {
        let dir = temp_dir("index-newer");
        let path = path_for(&dir, "lib-1");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let c = Connection::open(&path).unwrap();
        c.execute_batch("CREATE TABLE future (x); PRAGMA user_version = 999;")
            .unwrap();
        drop(c);
        let ix = Index::open(&path, "lib-1").unwrap();
        assert!(ix.set_aside.as_deref().unwrap().contains("newer version"));
        assert_eq!(ix.version().unwrap(), schema_version());
        assert!(ix.counts().unwrap().get("future").is_none());
        assert!(path.with_extension("old.sqlite").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn another_librarys_index_is_set_aside() {
        let dir = temp_dir("index-other");
        let path = path_for(&dir, "lib-1");
        drop(Index::open(&path, "lib-2").unwrap());
        let ix = Index::open(&path, "lib-1").unwrap();
        assert!(ix.set_aside.as_deref().unwrap().contains("another library"));
        assert_eq!(ix.meta("library_id").unwrap().as_deref(), Some("lib-1"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
