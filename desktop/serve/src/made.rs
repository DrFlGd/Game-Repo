//! Files a plan needs made (D93) and the SHA-1s of the library's files (D93, C11).
//!
//! A made file (an arcade set rebuilt as a ZIP, a ROM taken out of an archive, a cue sheet
//! rewritten, a picture made smaller, a playlist) is made in the server's data folder by
//! the export's own step and kept by its recipe, the newest up to a limit, so asking for a
//! plan again, or another device asking for the same games, finds it made.
//!
//! The index knows a loose file's SHA-1; an archive's or disc image's whole-file SHA-1 is
//! read by the server in the order plans ask for them, beside the downloads, and kept by
//! path, size and modified time.

use anyhow::{anyhow, Result};
use gamerepo_core::organise::Op;
use sha1::Digest;
use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// A file's SHA-1, read through.
pub fn sha1_file(path: &Path, cancel: &AtomicBool) -> Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = sha1::Sha1::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(anyhow!("stopped"));
        }
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

/// A file's size and modified time (milliseconds), as a stamp.
pub fn stamp(path: &Path) -> Option<(u64, i64)> {
    let m = std::fs::metadata(path).ok()?;
    let t = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as i64;
    Some((m.len(), t))
}

// ------------------------------------------------------------ made files

/// Where a made file is.
pub enum Made {
    Ready { path: PathBuf, size: u64, sha1: String },
    /// Being made, or waiting its turn
    Pending,
    Failed(String),
    /// Not asked for (since the server started): ask for the plan again
    Unknown,
}

#[derive(Clone)]
enum Job {
    Waiting(Box<Op>, String),
    Making,
    Failed(String),
}

#[derive(Default)]
struct State {
    queue: VecDeque<String>,
    jobs: HashMap<String, Job>,
}

pub struct Maker {
    dir: PathBuf,
    /// The made files kept, at most, in bytes
    limit: u64,
    state: Mutex<State>,
    cv: Condvar,
    stop: Arc<AtomicBool>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Meta {
    size: u64,
    sha1: String,
    ext: String,
}

impl Maker {
    pub fn new(dir: PathBuf, limit: u64, stop: Arc<AtomicBool>) -> Arc<Self> {
        let m = Arc::new(Self { dir, limit, state: Mutex::default(), cv: Condvar::new(), stop });
        let w = m.clone();
        std::thread::Builder::new().name("make".into()).spawn(move || w.work()).expect("a thread");
        m
    }

    /// A step's recipe (what it does, from what, each source by path, size and modified
    /// time): its id, and the extension of what it makes.
    pub fn recipe(op: &Op) -> (String, String) {
        let ext = Path::new(&op.to).extension().map(|e| e.to_string_lossy().to_lowercase()).filter(|e| e.len() <= 8 && e.chars().all(|c| c.is_ascii_alphanumeric())).unwrap_or_else(|| "bin".into());
        let from: Vec<(&str, &Option<String>, u64, Option<(u64, i64)>)> = op.from.iter().map(|s| (s.path.as_str(), &s.member, s.size, stamp(Path::new(&s.path)))).collect();
        let r = serde_json::to_string(&(&op.op, &op.names, from, &ext)).unwrap_or_default();
        (crate::tls::sha256_hex(r.as_bytes())[..32].to_string(), ext)
    }

    fn paths(&self, id: &str) -> (PathBuf, PathBuf) {
        let d = self.dir.join(&id[..2]);
        (d.join(format!("{id}.file")), d.join(format!("{id}.json")))
    }

    /// Whether a made file is ready.
    pub fn status(&self, id: &str) -> Made {
        if id.len() != 32 || !id.chars().all(|c| c.is_ascii_hexdigit()) {
            return Made::Unknown;
        }
        let (file, meta) = self.paths(id);
        if let Some(m) = std::fs::read(&meta).ok().and_then(|b| serde_json::from_slice::<Meta>(&b).ok()) {
            if std::fs::metadata(&file).is_ok_and(|f| f.len() == m.size) {
                return Made::Ready { path: file, size: m.size, sha1: m.sha1 };
            }
        }
        match self.state.lock().unwrap().jobs.get(id) {
            Some(Job::Failed(e)) => Made::Failed(e.clone()),
            Some(_) => Made::Pending,
            None => Made::Unknown,
        }
    }

    /// Make it if it isn't made (or being made).
    pub fn want(&self, id: &str, op: &Op) {
        let (_, ext) = Self::recipe(op);
        let mut s = self.state.lock().unwrap();
        if matches!(s.jobs.get(id), Some(Job::Waiting(..) | Job::Making)) {
            return;
        }
        s.jobs.insert(id.to_string(), Job::Waiting(Box::new(op.clone()), ext));
        s.queue.push_back(id.to_string());
        self.cv.notify_all();
    }

    /// Wait until a made file is ready or has failed, at most `wait`.
    pub fn wait(&self, id: &str, wait: Duration) -> Made {
        let until = std::time::Instant::now() + wait;
        loop {
            let m = self.status(id);
            if !matches!(m, Made::Pending) || std::time::Instant::now() >= until {
                return m;
            }
            let s = self.state.lock().unwrap();
            let _ = self.cv.wait_timeout(s, Duration::from_millis(200)).unwrap();
        }
    }

    fn work(&self) {
        while !self.stop.load(Ordering::Relaxed) {
            let next = {
                let mut s = self.state.lock().unwrap();
                loop {
                    if self.stop.load(Ordering::Relaxed) {
                        return;
                    }
                    if let Some(id) = s.queue.pop_front() {
                        if let Some(Job::Waiting(op, ext)) = s.jobs.get(&id).cloned() {
                            s.jobs.insert(id.clone(), Job::Making);
                            break (id, op, ext);
                        }
                        continue;
                    }
                    s = self.cv.wait_timeout(s, Duration::from_millis(500)).unwrap().0;
                }
            };
            let (id, op, ext) = next;
            let r = self.make(&id, &op, &ext);
            let mut s = self.state.lock().unwrap();
            match r {
                Ok(()) => {
                    s.jobs.remove(&id);
                }
                Err(e) => {
                    s.jobs.insert(id, Job::Failed(format!("{e:#}")));
                }
            }
            drop(s);
            self.cv.notify_all();
        }
    }

    fn make(&self, id: &str, op: &Op, ext: &str) -> Result<()> {
        let (file, meta) = self.paths(id);
        let making = self.dir.join(".making").join(id);
        let _ = std::fs::remove_dir_all(&making);
        std::fs::create_dir_all(&making)?;
        // the step decides how to write some files by their name (a picture as JPEG or PNG)
        let to = making.join(format!("made.{ext}"));
        let r = (|| -> Result<()> {
            gamerepo_core::organise::make(op, &to, &self.stop)?;
            let sha1 = sha1_file(&to, &self.stop)?;
            let size = std::fs::metadata(&to)?.len();
            std::fs::create_dir_all(file.parent().unwrap())?;
            std::fs::rename(&to, &file)?;
            crate::tls::write_new(&meta, &serde_json::to_vec(&Meta { size, sha1, ext: ext.to_string() })?)?;
            Ok(())
        })();
        let _ = std::fs::remove_dir_all(&making);
        r?;
        self.prune();
        Ok(())
    }

    /// Keep the newest made files up to the limit.
    fn prune(&self) {
        let mut all: Vec<(std::time::SystemTime, u64, PathBuf)> = vec![];
        for d in std::fs::read_dir(&self.dir).into_iter().flatten().flatten() {
            if d.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            for e in std::fs::read_dir(d.path()).into_iter().flatten().flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x == "file") {
                    if let Ok(m) = e.metadata() {
                        all.push((m.modified().unwrap_or(std::time::UNIX_EPOCH), m.len(), p));
                    }
                }
            }
        }
        let mut total: u64 = all.iter().map(|a| a.1).sum();
        all.sort();
        for (_, size, p) in all {
            if total <= self.limit {
                break;
            }
            let _ = std::fs::remove_file(p.with_extension("json"));
            let _ = std::fs::remove_file(&p);
            total -= size;
        }
    }
}

// ------------------------------------------------------------ SHA-1s

pub struct Hashes {
    db: PathBuf,
    queue: Mutex<VecDeque<PathBuf>>,
    cv: Condvar,
    stop: Arc<AtomicBool>,
}

fn open(db: &Path) -> Result<rusqlite::Connection> {
    let c = rusqlite::Connection::open(db)?;
    c.busy_timeout(Duration::from_secs(10))?;
    Ok(c)
}

impl Hashes {
    pub fn new(db: PathBuf, stop: Arc<AtomicBool>) -> Arc<Self> {
        let h = Arc::new(Self { db, queue: Mutex::default(), cv: Condvar::new(), stop });
        let w = h.clone();
        std::thread::Builder::new().name("sha1".into()).spawn(move || w.work()).expect("a thread");
        h
    }

    /// A file's SHA-1, if it's been read since it last changed.
    pub fn known(conn: &rusqlite::Connection, path: &Path) -> Option<String> {
        let (size, mtime) = stamp(path)?;
        conn.query_row(
            "SELECT sha1 FROM serve_hashes WHERE path = ?1 AND size = ?2 AND mtime = ?3",
            rusqlite::params![path.to_string_lossy(), size as i64, mtime],
            |r| r.get(0),
        )
        .ok()
    }

    /// Read these files' SHA-1s, after those asked for before.
    pub fn want(&self, paths: impl IntoIterator<Item = PathBuf>) {
        let mut q = self.queue.lock().unwrap();
        for p in paths {
            if !q.contains(&p) {
                q.push_back(p);
            }
        }
        self.cv.notify_all();
    }

    /// A file's SHA-1, read now ahead of the others if it isn't known, waiting at most `wait`.
    pub fn wait(&self, path: &Path, wait: Duration) -> Result<Option<String>> {
        let conn = open(&self.db)?;
        if let Some(h) = Self::known(&conn, path) {
            return Ok(Some(h));
        }
        {
            let mut q = self.queue.lock().unwrap();
            q.retain(|p| p != path);
            q.push_front(path.to_path_buf());
            self.cv.notify_all();
        }
        let until = std::time::Instant::now() + wait;
        loop {
            if let Some(h) = Self::known(&conn, path) {
                return Ok(Some(h));
            }
            if std::time::Instant::now() >= until {
                return Ok(None);
            }
            let q = self.queue.lock().unwrap();
            let _ = self.cv.wait_timeout(q, Duration::from_millis(200)).unwrap();
        }
    }

    fn work(&self) {
        let Ok(conn) = open(&self.db) else { return };
        while !self.stop.load(Ordering::Relaxed) {
            let next = {
                let mut q = self.queue.lock().unwrap();
                loop {
                    if self.stop.load(Ordering::Relaxed) {
                        return;
                    }
                    if let Some(p) = q.pop_front() {
                        break p;
                    }
                    q = self.cv.wait_timeout(q, Duration::from_millis(500)).unwrap().0;
                }
            };
            if Self::known(&conn, &next).is_some() {
                continue;
            }
            let Some((size, mtime)) = stamp(&next) else { continue };
            if let Ok(sha1) = sha1_file(&next, &self.stop) {
                // the file may have changed while it was read: then it's read again when asked
                if stamp(&next) == Some((size, mtime)) {
                    let _ = conn.execute(
                        "INSERT INTO serve_hashes (path, size, mtime, sha1) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(path) DO UPDATE SET size = excluded.size, mtime = excluded.mtime, sha1 = excluded.sha1",
                        rusqlite::params![next.to_string_lossy(), size as i64, mtime, sha1],
                    );
                }
            }
            self.cv.notify_all();
        }
    }
}
