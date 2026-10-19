//! Scanning the folders with games (docs/PLAN.md, "Phase 1 design", Scanning and
//! hashing): list every file, keep the hashes of what hasn't changed (or only moved),
//! hash what's new in parallel, look inside ZIPs, then match what changed against the
//! DATs. Nothing is written next to the files: what a scan learns goes in the index.

use crate::archive;
use crate::hash::{self, Hashes};
use crate::index::Index;
use crate::library::{now, Library, APP_DIR};
use crate::roots::{self, Root};
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// How a scan reads files (per computer: Settings, or `scan --full --readers n`).
#[derive(Clone, Debug)]
pub struct Options {
    /// Decompress ZIP members and hash them, instead of trusting the CRC32s the ZIP records (D16)
    pub full: bool,
    /// Files read at once
    pub readers: usize,
    /// Only these paths (files or folders, and what's in them), as the folder watcher
    /// reports them; None for every folder with games
    pub only: Option<Vec<PathBuf>>,
}

impl Default for Options {
    fn default() -> Self {
        Self { full: false, readers: 2, only: None }
    }
}

/// More places than this and a scan looks at everything.
const MANY_PLACES: usize = 2000;

/// Hashes are written to the index this many files at a time, so a stopped or
/// crashed scan loses little: the next one carries on from there.
const BATCH: usize = 250;

/// Names that are never games.
const JUNK: &[&str] = &["thumbs.db", "desktop.ini", ".ds_store"];

struct Walked {
    root: usize,
    rel: String,
    name: String,
    size: u64,
    mtime: i64,
}

fn mtime_ms(m: &std::fs::Metadata) -> i64 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Hidden, system and linked entries (symlinks and junctions aren't followed, so a loop can't hang a scan).
fn skipped(name: &str, ft: &std::fs::FileType, _meta: Option<&std::fs::Metadata>) -> bool {
    if name.starts_with('.') || ft.is_symlink() || JUNK.contains(&name.to_ascii_lowercase().as_str()) {
        return true;
    }
    #[cfg(windows)]
    if let Some(m) = _meta {
        use std::os::windows::fs::MetadataExt;
        // hidden, system, reparse point (junctions)
        if m.file_attributes() & (0x2 | 0x4 | 0x400) != 0 {
            return true;
        }
    }
    false
}

/// Every file under `root` (relative paths with `/`), without opening any.
fn walk(lib: &Library, root: &Path, ri: usize, cancel: &AtomicBool, out: &mut Vec<Walked>, errors: &mut Vec<String>) {
    walk_from(lib, root.to_path_buf(), String::new(), ri, cancel, out, errors)
}

/// Every file under `dir`, which is `prefix` in its folder with games.
fn walk_from(lib: &Library, dir: PathBuf, prefix: String, ri: usize, cancel: &AtomicBool, out: &mut Vec<Walked>, errors: &mut Vec<String>) {
    let app_dir = lib.root().join(APP_DIR);
    let mut stack: Vec<(PathBuf, String)> = vec![(dir, prefix)];
    while let Some((dir, prefix)) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let rd = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(e) => {
                errors.push(format!("{}: {e}", dir.display()));
                continue;
            }
        };
        let mut subdirs = vec![];
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(ft) = entry.file_type() else { continue };
            let meta = entry.metadata().ok();
            if skipped(&name, &ft, meta.as_ref()) {
                continue;
            }
            let rel = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
            if ft.is_dir() {
                let path = entry.path();
                if path == app_dir {
                    continue;
                }
                subdirs.push((path, rel));
            } else if ft.is_file() {
                let Some(m) = meta else { continue };
                out.push(Walked { root: ri, rel, name, size: m.len(), mtime: mtime_ms(&m) });
            }
        }
        // depth first, in name order
        subdirs.sort_by(|a, b| b.1.cmp(&a.1));
        stack.extend(subdirs);
    }
}

/// The places a scan of `only` looks at: (folder with games, path in it, "" for all of it),
/// none inside another. None when there are too many to be worth it.
fn places(roots: &[Root], only: &[PathBuf]) -> Option<Vec<(usize, String)>> {
    let mut out: Vec<(usize, String)> = vec![];
    for p in only {
        // the folder with games it's in (the deepest, if one is inside another)
        let Some((ri, rel)) = roots
            .iter()
            .enumerate()
            .filter(|(_, r)| r.found())
            .filter_map(|(i, r)| p.strip_prefix(&r.dir).ok().map(|rel| (i, r.dir.components().count(), rel)))
            .max_by_key(|(_, depth, _)| *depth)
            .map(|(i, _, rel)| (i, rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/")))
        else {
            continue;
        };
        out.push((ri, rel));
    }
    if out.len() > MANY_PLACES {
        return None;
    }
    // a place inside another is covered by it
    out.sort();
    out.dedup();
    let mut kept: Vec<(usize, String)> = vec![];
    for (ri, rel) in out {
        if kept.iter().any(|(r, k)| *r == ri && within(&rel, k)) {
            continue;
        }
        kept.push((ri, rel));
    }
    Some(kept)
}

/// Is `rel` at or under `place` ("" is the whole folder)?
fn within(rel: &str, place: &str) -> bool {
    place.is_empty() || rel == place || (rel.starts_with(place) && rel.as_bytes().get(place.len()) == Some(&b'/'))
}

/// The files at one place: what's under it if it's a folder, itself if it's a file,
/// nothing if it's gone (or hidden, or the app's own).
fn walk_place(lib: &Library, root: &Root, ri: usize, rel: &str, cancel: &AtomicBool, out: &mut Vec<Walked>, errors: &mut Vec<String>) {
    if rel.is_empty() {
        return walk(lib, &root.dir, ri, cancel, out, errors);
    }
    let path = root.dir.join(rel);
    if path.starts_with(lib.root().join(APP_DIR)) {
        return;
    }
    let Ok(meta) = std::fs::symlink_metadata(&path) else { return };
    let ft = meta.file_type();
    // hidden anywhere on the way: scans skip it
    let parts: Vec<&str> = rel.split('/').collect();
    if parts[..parts.len() - 1].iter().any(|d| d.starts_with('.')) || skipped(parts[parts.len() - 1], &ft, Some(&meta)) {
        return;
    }
    if ft.is_dir() {
        walk_from(lib, path, rel.to_string(), ri, cancel, out, errors);
    } else if ft.is_file() {
        out.push(Walked { root: ri, rel: rel.to_string(), name: parts[parts.len() - 1].to_string(), size: meta.len(), mtime: mtime_ms(&meta) });
    }
}

/// A file's platform when no DAT says: see [`crate::platforms::guess`].
fn guess_platform(root: &Root, rel: &str, member: Option<&str>) -> Option<String> {
    let root_name = root.dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut dirs: Vec<&str> = vec![root_name.as_str()];
    let parts: Vec<&str> = rel.split('/').collect();
    dirs.extend(&parts[..parts.len() - 1]);
    let name = match member {
        Some(m) => {
            let mparts: Vec<&str> = m.split('/').collect();
            dirs.extend(&mparts[..mparts.len() - 1]);
            *mparts.last().unwrap()
        }
        None => parts.last().copied().unwrap_or(""),
    };
    crate::platforms::guess(root.platform.as_deref(), &dirs, name)
}

/// The system of a file in an archive or disc image (`kind` the container's): a disc
/// image's tracks have the image's.
fn member_platform(root: &Root, container_rel: &str, kind: &str, name: &str) -> Option<String> {
    guess_platform(root, container_rel, (!crate::image::KINDS.contains(&kind)).then_some(name))
}

/// Guess again the system of every file of a folder with games (its hint or its place
/// changed). Returns how many files' systems changed.
fn reguess(conn: &Connection, root: &Root) -> Result<usize> {
    type Row = (i64, String, String, Option<String>, Option<(String, String)>);
    let rows: Vec<Row> = conn
        .prepare(
            "SELECT f.id, f.rel_path, f.name, f.platform, p.rel_path, p.kind
             FROM files f LEFT JOIN files p ON p.id = f.parent_id WHERE f.root_id = ?1",
        )?
        .query_map([&root.id], |r| {
            let parent: Option<String> = r.get(4)?;
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, parent.map(|p| (p, r.get::<_, String>(5).unwrap_or_default()))))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut set = conn.prepare_cached("UPDATE files SET platform = ?2 WHERE id = ?1")?;
    let mut n = 0;
    for (id, rel, name, old, parent) in rows {
        let new = match &parent {
            None => guess_platform(root, &rel, None),
            Some((prel, kind)) => member_platform(root, prel, kind, &name),
        };
        if new != old {
            set.execute(params![id, new])?;
            n += 1;
        }
    }
    Ok(n)
}

/// After a folder's system hint or place is changed: what's already scanned in it takes
/// the new guess (review M10), without waiting for its files to change.
pub fn reguess_root(ix: &Index, lib: &Library, id: &str) -> Result<usize> {
    let Some(root) = roots::list(lib).into_iter().find(|r| r.id == id) else { return Ok(0) };
    let conn = ix.conn();
    let tx = conn.unchecked_transaction()?;
    tx.execute("UPDATE roots SET path = ?2, platform = ?3 WHERE id = ?1", params![root.id, root.path, root.platform])?;
    let n = reguess(&tx, &root)?;
    if n > 0 {
        crate::audit::changed(&tx)?;
    }
    tx.commit()?;
    Ok(n)
}

/// A file to hash.
struct Work {
    file_id: i64,
    root: usize,
    path: PathBuf,
    rel: String,
    name: String,
    size: u64,
    mtime: i64,
    /// the archive or image kind ("zip", "7z", "rar", "chd", "cso", "zso", "rvz") when it's one
    archive: Option<&'static str>,
}

enum Outcome {
    File(Hashes),
    Archive(&'static str, archive::Listing),
    /// An RVZ when dolphin-tool isn't found: checked by a scan once it is
    NeedsTool(&'static str),
    Failed(String),
    Stopped,
}

fn hash_file(w: &Work, cancel: &AtomicBool) -> std::io::Result<Hashes> {
    let mut f = std::fs::File::open(&w.path)?;
    hash::hash_reader(&mut f, w.size, &w.name, cancel)
}

fn work_on(w: &Work, full: bool, cancel: &AtomicBool) -> Outcome {
    let r = if let Some(kind) = w.archive {
        archive::read(&w.path, kind, full, cancel).map(|l| Outcome::Archive(kind, l)).or_else(|e| {
            if e.kind() == std::io::ErrorKind::Interrupted {
                Err(e)
            } else if kind == "rvz" && e.kind() == std::io::ErrorKind::NotFound {
                Ok(Outcome::NeedsTool(kind))
            } else if matches!(e.kind(), std::io::ErrorKind::Other | std::io::ErrorKind::InvalidData | std::io::ErrorKind::UnexpectedEof) {
                // not an archive after all (it can't be opened as one): an ordinary file
                hash_file(w, cancel).map(Outcome::File)
            } else {
                // the file couldn't be read just now (held by another program, a drive
                // that went away): it isn't hashed, so the next scan tries again
                Err(e)
            }
        })
    } else {
        hash_file(w, cancel).map(Outcome::File)
    };
    match r {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::Interrupted && cancel.load(Ordering::Relaxed) => Outcome::Stopped,
        Err(e) => Outcome::Failed(format!("{}: {e}", w.path.display())),
    }
}

/// The blob for these hashes (shared by files with the same contents).
pub(crate) fn blob_id(conn: &Connection, h: &Hashes) -> Result<i64> {
    let quick = h.sha1.is_none();
    let found: Option<i64> = if quick {
        conn.prepare_cached("SELECT id FROM blobs WHERE crc32 = ?1 AND size = ?2 AND quick = 1")?
            .query_row(params![h.crc32, h.size as i64], |r| r.get(0))
            .optional()?
    } else {
        conn.prepare_cached("SELECT id FROM blobs WHERE sha1 = ?1 AND size = ?2 AND quick = 0 AND header IS ?3")?
            .query_row(params![h.sha1, h.size as i64, h.header], |r| r.get(0))
            .optional()?
    };
    if let Some(id) = found {
        return Ok(id);
    }
    let hl = h.hl.as_deref();
    conn.prepare_cached(
        "INSERT INTO blobs (size, crc32, md5, sha1, header, hl_size, hl_crc32, hl_md5, hl_sha1, quick)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
    )?
    .execute(params![
        h.size as i64, h.crc32, h.md5, h.sha1, h.header,
        hl.map(|x| x.size as i64), hl.map(|x| &x.crc32), hl.and_then(|x| x.md5.as_ref()), hl.and_then(|x| x.sha1.as_ref()),
        quick as i64
    ])?;
    Ok(conn.last_insert_rowid())
}

/// Write what hashing found for one file.
fn record(conn: &Connection, w: &Work, root: &Root, scan_id: i64, outcome: Outcome) -> Result<()> {
    match outcome {
        Outcome::File(h) => {
            let blob = blob_id(conn, &h)?;
            conn.prepare_cached("DELETE FROM files WHERE parent_id = ?1")?.execute([w.file_id])?;
            conn.prepare_cached("UPDATE files SET kind = 'file', blob_id = ?2, hashed = 2, matched = 0, locked = 0, chd_sha1 = NULL WHERE id = ?1")?
                .execute(params![w.file_id, blob])?;
        }
        Outcome::Archive(kind, listing) => {
            conn.prepare_cached("DELETE FROM files WHERE parent_id = ?1")?.execute([w.file_id])?;
            let members = listing.members;
            let all_full = members.iter().all(|m| m.full);
            let mut ins = conn.prepare_cached(
                "INSERT OR REPLACE INTO files (root_id, parent_id, rel_path, name, kind, size, mtime, blob_id, hashed, matched, platform, seen)
                 VALUES (?1, ?2, ?3, ?4, 'member', ?5, ?6, ?7, ?8, 0, ?9, ?10)",
            )?;
            for m in &members {
                let blob = blob_id(conn, &m.hashes)?;
                ins.execute(params![
                    root.id, w.file_id, format!("{}/{}", w.rel, m.name), m.name, m.hashes.size as i64, w.mtime, blob,
                    if m.full { 2 } else { 1 },
                    // a disc image's tracks have the image's platform
                    guess_platform(root, &w.rel, (!crate::image::KINDS.contains(&kind)).then_some(m.name.as_str())), scan_id
                ])?;
            }
            conn.prepare_cached("UPDATE files SET kind = ?3, blob_id = NULL, hashed = ?2, matched = 1, locked = ?4, chd_sha1 = NULL WHERE id = ?1")?
                .execute(params![w.file_id, if all_full { 2 } else { 1 }, kind, listing.locked as i64])?;
        }
        Outcome::NeedsTool(kind) => {
            conn.prepare_cached("DELETE FROM files WHERE parent_id = ?1")?.execute([w.file_id])?;
            conn.prepare_cached("UPDATE files SET kind = ?2, blob_id = NULL, hashed = 0, matched = 1, locked = 2 WHERE id = ?1")?
                .execute(params![w.file_id, kind])?;
        }
        Outcome::Failed(_) | Outcome::Stopped => {}
    }
    Ok(())
}

/// Scan every folder with games of `lib` into `ix`. `progress` gets what the page
/// shows; setting `cancel` stops the scan, keeping what's been hashed.
pub fn scan(ix: &Index, lib: &Library, opts: &Options, cancel: &AtomicBool, progress: &mut dyn FnMut(Value)) -> Result<Value> {
    let conn = ix.conn();
    let roots = roots::list(lib);
    crate::dat::sync_library(ix, lib)?;

    // the roots table follows library.json; files of removed roots go, and those of a
    // root whose hint or place changed take the new guess (review M10)
    // (what changed decides whether the audit's summaries are worked out again: M13)
    let mut altered = 0usize;
    {
        let tx = conn.unchecked_transaction()?;
        let before: HashMap<String, (String, Option<String>)> = tx
            .prepare("SELECT id, path, platform FROM roots")?
            .query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?
            .collect::<rusqlite::Result<_>>()?;
        tx.execute("DELETE FROM roots", [])?;
        for r in &roots {
            tx.execute("INSERT INTO roots (id, path, platform) VALUES (?1, ?2, ?3)", params![r.id, r.path, r.platform])?;
        }
        altered += tx.execute("DELETE FROM files WHERE root_id NOT IN (SELECT id FROM roots)", [])?;
        for r in &roots {
            if before.get(&r.id).is_some_and(|(path, platform)| *path != r.path || *platform != r.platform) {
                altered += reguess(&tx, r)?;
            }
        }
        tx.commit()?;
    }
    conn.execute("INSERT INTO scans (started, full) VALUES (?1, ?2)", params![now(), opts.full as i64])?;
    let scan_id = conn.last_insert_rowid();
    let started = Instant::now();

    // 1. walk (everything, or only the places the watcher saw change)
    progress(json!({ "text": "Listing files" }));
    let places = opts.only.as_deref().and_then(|only| places(&roots, only));
    let mut walked = vec![];
    let mut errors: Vec<String> = vec![];
    let mut missing_roots = vec![];
    for (i, r) in roots.iter().enumerate() {
        if r.found() {
            match &places {
                Some(places) => {
                    for (_, rel) in places.iter().filter(|(ri, _)| *ri == i) {
                        walk_place(lib, r, i, rel, cancel, &mut walked, &mut errors);
                    }
                }
                None => walk(lib, &r.dir, i, cancel, &mut walked, &mut errors),
            }
            altered += conn.execute("UPDATE files SET missing_since = NULL WHERE root_id = ?1 AND missing_since IS NOT NULL", [&r.id])?;
        } else {
            missing_roots.push(r.id.clone());
            altered += conn.execute("UPDATE files SET missing_since = ?2 WHERE root_id = ?1 AND missing_since IS NULL", params![r.id, now()])?;
        }
    }
    let found_ids: Vec<&str> = roots.iter().filter(|r| r.found()).map(|r| r.id.as_str()).collect();
    if cancel.load(Ordering::Relaxed) {
        if altered > 0 {
            crate::audit::changed(conn)?;
        }
        return finish(conn, scan_id, &json!({ "stopped": true }), 0, 0, walked.len());
    }

    // 2. compare with what the index has
    struct Row {
        id: i64,
        name: String,
        size: u64,
        mtime: i64,
        kind: String,
        hashed: i64,
    }
    let mut known: HashMap<(String, String), Row> = HashMap::new();
    {
        let read = |r: &rusqlite::Row| {
            Ok(((r.get::<_, String>(1)?, r.get::<_, String>(2)?), Row {
                id: r.get(0)?, name: r.get(3)?, size: r.get::<_, i64>(4)? as u64, mtime: r.get(5)?, kind: r.get(6)?, hashed: r.get(7)?,
            }))
        };
        const COLUMNS: &str = "SELECT id, root_id, rel_path, name, size, mtime, kind, hashed FROM files";
        match &places {
            // only the rows under each place, by the (root_id, rel_path) index (review M13)
            Some(places) => {
                let mut q = conn.prepare(&format!(
                    "{COLUMNS} WHERE parent_id IS NULL AND root_id = ?1
                       AND (?2 = '' OR rel_path = ?2 OR (rel_path >= ?2 || '/' AND rel_path < ?2 || '0'))"
                ))?;
                for (ri, place) in places {
                    if !roots[*ri].found() {
                        continue;
                    }
                    for row in q.query_map(params![roots[*ri].id, place], read)? {
                        let (k, v) = row?;
                        if within(&k.1, place) {
                            known.insert(k, v);
                        }
                    }
                }
            }
            None => {
                let mut q = conn.prepare(&format!("{COLUMNS} WHERE parent_id IS NULL"))?;
                for row in q.query_map([], read)? {
                    let (k, v) = row?;
                    if found_ids.contains(&k.0.as_str()) {
                        known.insert(k, v);
                    }
                }
            }
        }
    }
    let needs_hash = |row: &Row| row.hashed == 0 || (opts.full && archive::KINDS.contains(&row.kind.as_str()) && row.hashed < 2);
    let mut queue: Vec<Work> = vec![];
    let mut fresh: Vec<Walked> = vec![];
    let (mut changed, mut added, mut moved) = (0usize, 0usize, 0usize);
    let tx = conn.unchecked_transaction()?;
    let work = |id: i64, w: &Walked| Work {
        file_id: id, root: w.root, path: roots[w.root].dir.join(&w.rel), rel: w.rel.clone(), name: w.name.clone(),
        size: w.size, mtime: w.mtime, archive: archive::container_of(&w.name),
    };
    for w in walked {
        let key = (roots[w.root].id.clone(), w.rel.clone());
        match known.remove(&key) {
            Some(row) if row.size == w.size && row.mtime == w.mtime => {
                if needs_hash(&row) {
                    queue.push(work(row.id, &w));
                }
            }
            Some(row) => {
                // changed: hashed again
                tx.execute("DELETE FROM files WHERE parent_id = ?1", [row.id])?;
                tx.execute("DELETE FROM matches WHERE file_id = ?1", [row.id])?;
                tx.execute(
                    "UPDATE files SET size = ?2, mtime = ?3, blob_id = NULL, hashed = 0, matched = 0 WHERE id = ?1",
                    params![row.id, w.size as i64, w.mtime],
                )?;
                changed += 1;
                queue.push(work(row.id, &w));
            }
            None => fresh.push(w),
        }
    }
    // what vanished and appeared with the same name, size and time was moved: its hashes go with it
    let mut gone: HashMap<(String, u64, i64), Vec<(String, Row)>> = HashMap::new();
    for ((_, rel), row) in known.drain() {
        gone.entry((row.name.clone(), row.size, row.mtime)).or_default().push((rel, row));
    }
    for w in fresh {
        let root = &roots[w.root];
        let platform = guess_platform(root, &w.rel, None);
        if let Some((old_rel, row)) = gone.get_mut(&(w.name.clone(), w.size, w.mtime)).and_then(|v| v.pop()).filter(|(_, r)| r.hashed > 0) {
            tx.execute(
                "UPDATE files SET root_id = ?2, rel_path = ?3 || substr(rel_path, ?4), platform = ?5, seen = ?6 WHERE parent_id = ?1",
                params![row.id, root.id, w.rel, old_rel.chars().count() as i64 + 1, platform, scan_id],
            )?;
            // the members' platforms are guessed again from the new place
            {
                let mut q = tx.prepare("SELECT id, name FROM files WHERE parent_id = ?1")?;
                let members: Vec<(i64, String)> = q.query_map([row.id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
                for (id, name) in members {
                    tx.execute("UPDATE files SET platform = ?2 WHERE id = ?1", params![id, member_platform(root, &w.rel, &row.kind, &name)])?;
                }
            }
            tx.execute(
                "UPDATE files SET root_id = ?2, rel_path = ?3, platform = ?4, seen = ?5 WHERE id = ?1",
                params![row.id, root.id, w.rel, platform, scan_id],
            )?;
            moved += 1;
            if needs_hash(&row) {
                queue.push(work(row.id, &w));
            }
            continue;
        }
        tx.execute(
            "INSERT INTO files (root_id, rel_path, name, kind, size, mtime, platform, seen) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![root.id, w.rel, w.name, archive::container_of(&w.name).unwrap_or("file"), w.size as i64, w.mtime, platform, scan_id],
        )?;
        added += 1;
        queue.push(work(tx.last_insert_rowid(), &w));
    }
    let mut removed = 0usize;
    for (_, rows) in gone {
        for (_, row) in rows {
            tx.execute("DELETE FROM files WHERE id = ?1", [row.id])?;
            removed += 1;
        }
    }
    tx.commit()?;
    let seen: i64 = conn.query_row("SELECT COUNT(*) FROM files WHERE parent_id IS NULL", [], |r| r.get(0))?;

    // 3. hash what's new or changed, a few files at once
    let total_bytes: u64 = queue.iter().map(|w| w.size).sum();
    let items = queue.len();
    let (mut done, mut bytes, mut failed) = (0usize, 0u64, 0usize);
    let mut stopped = false;
    if !queue.is_empty() {
        let next = AtomicUsize::new(0);
        let (send, recv) = mpsc::channel::<(usize, Outcome)>();
        let readers = opts.readers.clamp(1, 16).min(queue.len());
        // set when the results can't be recorded: the readers stop rather than hash on
        let halt = AtomicBool::new(false);
        std::thread::scope(|s| -> Result<()> {
            for _ in 0..readers {
                let (send, next, queue, halt) = (send.clone(), &next, &queue, &halt);
                s.spawn(move || loop {
                    if cancel.load(Ordering::Relaxed) || halt.load(Ordering::Relaxed) {
                        break;
                    }
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(w) = queue.get(i) else { break };
                    if send.send((i, work_on(w, opts.full, cancel))).is_err() {
                        break;
                    }
                });
            }
            drop(send);
            let mut tx: Option<rusqlite::Transaction> = None;
            // committed every BATCH files, every second, and whenever no result comes for
            // half a second, so other writers (the audit's cache, edits, DAT imports)
            // aren't kept waiting behind large files
            let (mut in_batch, mut last, mut opened) = (0usize, Instant::now(), Instant::now());
            // record every result, even after Stop: what was hashed is kept
            loop {
                let (i, outcome) = match recv.recv_timeout(Duration::from_millis(500)) {
                    Ok(r) => r,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        // nothing new for a while (a large file being read): what's recorded is committed
                        if let Some(t) = tx.take() {
                            if let Err(e) = t.commit() {
                                halt.store(true, Ordering::Relaxed);
                                return Err(e.into());
                            }
                            in_batch = 0;
                        }
                        continue;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                };
                let w = &queue[i];
                match &outcome {
                    Outcome::Failed(e) => {
                        failed += 1;
                        if errors.len() < 50 {
                            errors.push(e.clone());
                        }
                    }
                    Outcome::Stopped => stopped = true,
                    _ => {
                        done += 1;
                        bytes += w.size;
                    }
                }
                let recorded = (|| -> Result<()> {
                    if tx.is_none() {
                        tx = Some(conn.unchecked_transaction()?);
                        opened = Instant::now();
                    }
                    record(tx.as_ref().unwrap(), w, &roots[w.root], scan_id, outcome)?;
                    in_batch += 1;
                    if in_batch >= BATCH || opened.elapsed() > Duration::from_secs(1) {
                        tx.take().unwrap().commit()?;
                        in_batch = 0;
                    }
                    Ok(())
                })();
                if let Err(e) = recorded {
                    halt.store(true, Ordering::Relaxed);
                    return Err(e);
                }
                if last.elapsed() > Duration::from_millis(100) {
                    last = Instant::now();
                    progress(json!({ "text": "Hashing", "item": done + failed, "items": items, "bytes": bytes, "total_bytes": total_bytes, "name": w.rel }));
                }
            }
            if let Some(t) = tx.take() {
                t.commit()?;
            }
            Ok(())
        })?;
    }
    stopped |= cancel.load(Ordering::Relaxed);

    // 4. each CHD's own SHA1, from its header, for arcade disks (D58)
    let chd = chd_headers(conn, &roots)?;

    // 5. match what changed against the DATs; drop hashes no file has any more
    progress(json!({ "text": "Matching" }));
    let matched = crate::audit::match_files(conn)?;
    // match_files marks a change itself when it matched anything again; a scan that found
    // nothing new (most of the watcher's) leaves the audit's summaries as they are (M13)
    if altered + added + changed + moved + removed + chd > 0 {
        crate::audit::changed(conn)?;
    }
    conn.execute("DELETE FROM blobs WHERE id NOT IN (SELECT blob_id FROM files WHERE blob_id IS NOT NULL)", [])?;

    let summary = json!({
        "scan": scan_id, "files": seen, "added": added, "changed": changed, "moved": moved, "removed": removed,
        "hashed": done, "bytes": bytes, "failed": failed, "errors": errors, "matched": matched, "stopped": stopped,
        "roots": roots.len(), "roots_missing": missing_roots, "full": opts.full, "places": places.as_ref().map(Vec::len),
        "seconds": (started.elapsed().as_secs_f64() * 10.0).round() / 10.0,
    });
    finish(conn, scan_id, &summary, done, bytes, seen as usize)
}

/// Read the header of each CHD whose own SHA1 isn't known yet ("" for one that can't be read).
fn chd_headers(conn: &Connection, roots: &[Root]) -> Result<usize> {
    let rows: Vec<(i64, String, String)> = conn
        .prepare("SELECT id, root_id, rel_path FROM files WHERE kind = 'chd' AND parent_id IS NULL AND chd_sha1 IS NULL AND missing_since IS NULL")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    if rows.is_empty() {
        return Ok(0);
    }
    let n = rows.len();
    let tx = conn.unchecked_transaction()?;
    for (id, root, rel) in rows {
        let Some(r) = roots.iter().find(|r| r.id == root) else { continue };
        let path = rel.split('/').fold(r.dir.clone(), |p, part| p.join(part));
        let sha1 = crate::image::chd_sha1(&path).unwrap_or_default();
        tx.execute("UPDATE files SET chd_sha1 = ?2 WHERE id = ?1", params![id, sha1])?;
    }
    tx.commit()?;
    Ok(n)
}

fn finish(conn: &Connection, scan_id: i64, summary: &Value, hashed: usize, bytes: u64, seen: usize) -> Result<Value> {
    conn.execute(
        "UPDATE scans SET finished = ?2, files_seen = ?3, files_hashed = ?4, bytes_hashed = ?5, stopped = ?6 WHERE id = ?1",
        params![scan_id, now(), seen as i64, hashed as i64, bytes as i64, summary["stopped"] == true],
    )
    .context("couldn't record the scan")?;
    Ok(summary.clone())
}

/// The last scan, as Home shows it.
pub fn last(ix: &Index) -> Result<Value> {
    Ok(ix
        .conn()
        .query_row(
            "SELECT id, started, finished, full, files_seen, files_hashed, bytes_hashed, stopped FROM scans ORDER BY id DESC LIMIT 1",
            [],
            |r| {
                Ok(json!({
                    "id": r.get::<_, i64>(0)?, "started": r.get::<_, String>(1)?, "finished": r.get::<_, Option<String>>(2)?,
                    "full": r.get::<_, i64>(3)? == 1, "files": r.get::<_, i64>(4)?, "hashed": r.get::<_, i64>(5)?,
                    "bytes": r.get::<_, i64>(6)?, "stopped": r.get::<_, i64>(7)? == 1,
                }))
            },
        )
        .optional()?
        .unwrap_or(Value::Null))
}
