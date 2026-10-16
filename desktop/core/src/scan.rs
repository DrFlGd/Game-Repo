//! Scanning the folders with games (docs/PLAN.md, "Phase 1 design", Scanning and
//! hashing): list every file, keep the hashes of what hasn't changed (or only moved),
//! hash what's new in parallel, look inside ZIPs, then match what changed against the
//! DATs. Nothing is written next to the files: what a scan learns goes in the index.

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
}

impl Default for Options {
    fn default() -> Self {
        Self { full: false, readers: 2 }
    }
}

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
    let app_dir = lib.root().join(APP_DIR);
    let mut stack: Vec<(PathBuf, String)> = vec![(root.to_path_buf(), String::new())];
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

fn is_zip(name: &str) -> bool {
    crate::platforms::ext_of(name) == "zip"
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

/// A file to hash.
struct Work {
    file_id: i64,
    root: usize,
    path: PathBuf,
    rel: String,
    name: String,
    size: u64,
    mtime: i64,
    zip: bool,
}

struct Member {
    name: String,
    hashes: Hashes,
    full: bool,
}

enum Outcome {
    File(Hashes),
    Zip(Vec<Member>),
    Failed(String),
    Stopped,
}

fn hash_file(w: &Work, cancel: &AtomicBool) -> std::io::Result<Hashes> {
    let mut f = std::fs::File::open(&w.path)?;
    hash::hash_reader(&mut f, w.size, &w.name, cancel)
}

/// A ZIP's members: by the CRC32s it records (quick), or decompressed and hashed (full,
/// or when its central directory can't be read).
fn read_zip(path: &Path, full: bool, cancel: &AtomicBool) -> std::io::Result<Vec<Member>> {
    let io = |e: zip::result::ZipError| std::io::Error::other(e.to_string());
    let f = std::io::BufReader::new(std::fs::File::open(path)?);
    let mut zip = match zip::ZipArchive::new(f) {
        Ok(z) => z,
        Err(_) => return read_zip_stream(path, cancel),
    };
    let mut members = vec![];
    for i in 0..zip.len() {
        if cancel.load(Ordering::Relaxed) {
            return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "stopped"));
        }
        if full {
            let mut m = zip.by_index(i).map_err(io)?;
            if m.is_dir() {
                continue;
            }
            let name = m.name().map_err(io)?.into_owned();
            let size = m.size();
            let hashes = hash::hash_reader(&mut m, size, &name, cancel)?;
            members.push(Member { name, hashes, full: true });
        } else {
            let m = zip.by_index_raw(i).map_err(io)?;
            if m.is_dir() {
                continue;
            }
            let name = m.name().map_err(io)?.into_owned();
            members.push(Member { hashes: Hashes::quick(m.size(), m.crc32()), name, full: false });
        }
    }
    Ok(members)
}

/// A ZIP read from its local headers, when its central directory is damaged.
fn read_zip_stream(path: &Path, cancel: &AtomicBool) -> std::io::Result<Vec<Member>> {
    let mut f = std::io::BufReader::new(std::fs::File::open(path)?);
    let mut members = vec![];
    while let Some(mut m) = zip::read::read_zipfile_from_stream(&mut f).map_err(|e| std::io::Error::other(e.to_string()))? {
        if m.is_dir() {
            continue;
        }
        let name = m.name().map_err(|e| std::io::Error::other(e.to_string()))?.into_owned();
        let size = m.size();
        let hashes = hash::hash_reader(&mut m, size, &name, cancel)?;
        members.push(Member { name, hashes, full: true });
    }
    if members.is_empty() {
        return Err(std::io::Error::other("not a ZIP that can be read"));
    }
    Ok(members)
}

/// Every member of the ZIP at `path`, decompressed and hashed: (name, hashes).
pub fn zip_members(path: &Path, cancel: &AtomicBool) -> std::io::Result<Vec<(String, Hashes)>> {
    Ok(read_zip(path, true, cancel)?.into_iter().map(|m| (m.name, m.hashes)).collect())
}

fn work_on(w: &Work, full: bool, cancel: &AtomicBool) -> Outcome {
    let r = if w.zip {
        read_zip(&w.path, full, cancel).map(Outcome::Zip).or_else(|e| {
            if e.kind() == std::io::ErrorKind::Interrupted {
                Err(e)
            } else {
                // not a ZIP after all: an ordinary file
                hash_file(w, cancel).map(Outcome::File)
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
fn blob_id(conn: &Connection, h: &Hashes) -> Result<i64> {
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
            conn.prepare_cached("UPDATE files SET kind = 'file', blob_id = ?2, hashed = 2, matched = 0 WHERE id = ?1")?
                .execute(params![w.file_id, blob])?;
        }
        Outcome::Zip(members) => {
            conn.prepare_cached("DELETE FROM files WHERE parent_id = ?1")?.execute([w.file_id])?;
            let all_full = members.iter().all(|m| m.full);
            let mut ins = conn.prepare_cached(
                "INSERT OR REPLACE INTO files (root_id, parent_id, rel_path, name, kind, size, mtime, blob_id, hashed, matched, platform, seen)
                 VALUES (?1, ?2, ?3, ?4, 'member', ?5, ?6, ?7, ?8, 0, ?9, ?10)",
            )?;
            for m in &members {
                let blob = blob_id(conn, &m.hashes)?;
                ins.execute(params![
                    root.id, w.file_id, format!("{}/{}", w.rel, m.name), m.name, m.hashes.size as i64, w.mtime, blob,
                    if m.full { 2 } else { 1 }, guess_platform(root, &w.rel, Some(&m.name)), scan_id
                ])?;
            }
            conn.prepare_cached("UPDATE files SET kind = 'zip', blob_id = NULL, hashed = ?2, matched = 1 WHERE id = ?1")?
                .execute(params![w.file_id, if all_full { 2 } else { 1 }])?;
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

    // the roots table follows library.json; files of removed roots go
    {
        let tx = conn.unchecked_transaction()?;
        tx.execute("DELETE FROM roots", [])?;
        for r in &roots {
            tx.execute("INSERT INTO roots (id, path, platform) VALUES (?1, ?2, ?3)", params![r.id, r.path, r.platform])?;
        }
        tx.execute("DELETE FROM files WHERE root_id NOT IN (SELECT id FROM roots)", [])?;
        tx.commit()?;
    }
    conn.execute("INSERT INTO scans (started, full) VALUES (?1, ?2)", params![now(), opts.full as i64])?;
    let scan_id = conn.last_insert_rowid();
    let started = Instant::now();

    // 1. walk
    progress(json!({ "text": "Listing files" }));
    let mut walked = vec![];
    let mut errors: Vec<String> = vec![];
    let mut missing_roots = vec![];
    for (i, r) in roots.iter().enumerate() {
        if r.found() {
            walk(lib, &r.dir, i, cancel, &mut walked, &mut errors);
            conn.execute("UPDATE files SET missing_since = NULL WHERE root_id = ?1 AND missing_since IS NOT NULL", [&r.id])?;
        } else {
            missing_roots.push(r.id.clone());
            conn.execute("UPDATE files SET missing_since = COALESCE(missing_since, ?2) WHERE root_id = ?1", params![r.id, now()])?;
        }
    }
    let found_ids: Vec<&str> = roots.iter().filter(|r| r.found()).map(|r| r.id.as_str()).collect();
    if cancel.load(Ordering::Relaxed) {
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
        let mut q = conn.prepare("SELECT id, root_id, rel_path, name, size, mtime, kind, hashed FROM files WHERE parent_id IS NULL")?;
        let rows = q.query_map([], |r| {
            Ok(((r.get::<_, String>(1)?, r.get::<_, String>(2)?), Row {
                id: r.get(0)?, name: r.get(3)?, size: r.get::<_, i64>(4)? as u64, mtime: r.get(5)?, kind: r.get(6)?, hashed: r.get(7)?,
            }))
        })?;
        for row in rows {
            let (k, v) = row?;
            if found_ids.contains(&k.0.as_str()) {
                known.insert(k, v);
            }
        }
    }
    let needs_hash = |row: &Row| row.hashed == 0 || (opts.full && row.kind == "zip" && row.hashed < 2);
    let mut queue: Vec<Work> = vec![];
    let mut fresh: Vec<Walked> = vec![];
    let (mut changed, mut added, mut moved) = (0usize, 0usize, 0usize);
    let tx = conn.unchecked_transaction()?;
    let work = |id: i64, w: &Walked| Work {
        file_id: id, root: w.root, path: roots[w.root].dir.join(&w.rel), rel: w.rel.clone(), name: w.name.clone(),
        size: w.size, mtime: w.mtime, zip: is_zip(&w.name),
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
                    tx.execute("UPDATE files SET platform = ?2 WHERE id = ?1", params![id, guess_platform(root, &w.rel, Some(&name))])?;
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
            params![root.id, w.rel, w.name, if is_zip(&w.name) { "zip" } else { "file" }, w.size as i64, w.mtime, platform, scan_id],
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
        std::thread::scope(|s| -> Result<()> {
            for _ in 0..readers {
                let (send, next, queue) = (send.clone(), &next, &queue);
                s.spawn(move || loop {
                    if cancel.load(Ordering::Relaxed) {
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
            let mut tx = Some(conn.unchecked_transaction()?);
            let (mut in_batch, mut last) = (0usize, Instant::now());
            // record every result, even after Stop: what was hashed is kept
            for (i, outcome) in recv.iter() {
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
                if tx.is_none() {
                    tx = Some(conn.unchecked_transaction()?);
                }
                record(tx.as_ref().unwrap(), w, &roots[w.root], scan_id, outcome)?;
                in_batch += 1;
                if in_batch >= BATCH {
                    tx.take().unwrap().commit()?;
                    in_batch = 0;
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

    // 4. match what changed against the DATs; drop hashes no file has any more
    progress(json!({ "text": "Matching" }));
    let matched = crate::audit::match_files(conn)?;
    crate::audit::changed(conn)?;
    conn.execute("DELETE FROM blobs WHERE id NOT IN (SELECT blob_id FROM files WHERE blob_id IS NOT NULL)", [])?;

    let summary = json!({
        "scan": scan_id, "files": seen, "added": added, "changed": changed, "moved": moved, "removed": removed,
        "hashed": done, "bytes": bytes, "failed": failed, "errors": errors, "matched": matched, "stopped": stopped,
        "roots": roots.len(), "roots_missing": missing_roots, "full": opts.full,
        "seconds": (started.elapsed().as_secs_f64() * 10.0).round() / 10.0,
    });
    finish(conn, scan_id, &summary, done, bytes, seen as usize)
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
