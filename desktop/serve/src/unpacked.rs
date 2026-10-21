//! PC games' archives unpacked for devices (D111): a game's install archives (ZIP, 7z or
//! RAR) unpacked once into the server's data folder, each file with its size and SHA-1, so
//! a device fetches the game's files one by one as it fetches any other game's, resumed and
//! checked, and never needs room for the archive too. Kept by the archives' paths, sizes and
//! modified times, the newest up to the same limit as made files.

use anyhow::{anyhow, bail, Result};
use gamerepo_core::archive;
use serde::{Deserialize, Serialize};
use sha1::Digest;
use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// A file of an unpacked game: its path in the game's folder, with `/`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Unfile {
    pub path: String,
    pub size: u64,
    pub sha1: String,
}

pub enum Unpacked {
    Ready(Vec<Unfile>),
    /// Being unpacked, or waiting its turn
    Pending,
    Failed(String),
    /// Not asked for (since the server started)
    Unknown,
}

/// How far an unpacking has got: the bytes written, of its files' whole size (0 until
/// the archives' lists are read)
type Progress = Arc<(AtomicU64, AtomicU64)>;

#[derive(Clone)]
enum Job {
    Waiting(Vec<PathBuf>),
    Unpacking(Progress),
    Failed(String),
}

#[derive(Default)]
struct State {
    queue: VecDeque<String>,
    jobs: HashMap<String, Job>,
}

pub struct Unpacker {
    dir: PathBuf,
    limit: u64,
    state: Mutex<State>,
    cv: Condvar,
    stop: Arc<AtomicBool>,
}

/// The list of an unpacked game's files, beside them.
const LIST: &str = "files.json";

impl Unpacker {
    pub fn new(dir: PathBuf, limit: u64, stop: Arc<AtomicBool>) -> Arc<Self> {
        let u = Arc::new(Self { dir, limit, state: Mutex::default(), cv: Condvar::new(), stop });
        let w = u.clone();
        std::thread::Builder::new().name("unpack".into()).spawn(move || w.work()).expect("a thread");
        u
    }

    /// The id of a game's archives as they are now (each by path, size and modified time).
    pub fn id_of(archives: &[PathBuf]) -> String {
        let a: Vec<(String, Option<(u64, i64)>)> = archives.iter().map(|p| (p.to_string_lossy().into_owned(), crate::made::stamp(p))).collect();
        crate::tls::sha256_hex(serde_json::to_string(&a).unwrap_or_default().as_bytes())[..32].to_string()
    }

    fn valid(id: &str) -> bool {
        id.len() == 32 && id.chars().all(|c| c.is_ascii_hexdigit())
    }

    pub fn status(&self, id: &str) -> Unpacked {
        if !Self::valid(id) {
            return Unpacked::Unknown;
        }
        let at = self.dir.join(id);
        if let Some(list) = std::fs::read(at.join(LIST)).ok().and_then(|b| serde_json::from_slice::<Vec<Unfile>>(&b).ok()) {
            return Unpacked::Ready(list);
        }
        match self.state.lock().unwrap().jobs.get(id) {
            Some(Job::Failed(e)) => Unpacked::Failed(e.clone()),
            Some(_) => Unpacked::Pending,
            None => Unpacked::Unknown,
        }
    }

    /// How far it's got while it's being unpacked (D126): bytes written, of the whole.
    pub fn progress(&self, id: &str) -> Option<(u64, u64)> {
        match self.state.lock().unwrap().jobs.get(id) {
            Some(Job::Unpacking(p)) => Some((p.0.load(Ordering::Relaxed), p.1.load(Ordering::Relaxed))),
            _ => None,
        }
    }

    /// Unpack these archives if they aren't (or are being).
    pub fn want(&self, id: &str, archives: Vec<PathBuf>) {
        let mut s = self.state.lock().unwrap();
        if matches!(s.jobs.get(id), Some(Job::Waiting(_) | Job::Unpacking(_))) {
            return;
        }
        s.jobs.insert(id.to_string(), Job::Waiting(archives));
        s.queue.push_back(id.to_string());
        self.cv.notify_all();
    }

    /// An unpacked game's file, by its path in the game's folder; never outside it.
    pub fn file(&self, id: &str, path: &str) -> Option<PathBuf> {
        if !Self::valid(id) || path.is_empty() || Path::new(path).components().any(|c| !matches!(c, Component::Normal(_))) {
            return None;
        }
        let p = path.split('/').fold(self.dir.join(id).join("files"), |p, part| p.join(part));
        p.is_file().then_some(p)
    }

    fn work(&self) {
        while !self.stop.load(Ordering::Relaxed) {
            let progress: Progress = Arc::default();
            let (id, archives) = {
                let mut s = self.state.lock().unwrap();
                loop {
                    if self.stop.load(Ordering::Relaxed) {
                        return;
                    }
                    if let Some(id) = s.queue.pop_front() {
                        if let Some(Job::Waiting(a)) = s.jobs.get(&id).cloned() {
                            s.jobs.insert(id.clone(), Job::Unpacking(progress.clone()));
                            break (id, a);
                        }
                        continue;
                    }
                    s = self.cv.wait_timeout(s, Duration::from_millis(500)).unwrap().0;
                }
            };
            let r = self.unpack(&id, &archives, &progress);
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

    fn unpack(&self, id: &str, archives: &[PathBuf], progress: &Progress) -> Result<()> {
        // the whole, from the archives' lists (quick: no file is unpacked to read them)
        let whole: u64 = archives
            .iter()
            .filter_map(|a| archive::read(a, archive::kind_of(&a.to_string_lossy()).unwrap_or("zip"), false, &self.stop).ok())
            .flat_map(|l| l.members)
            .map(|m| m.hashes.size)
            .sum();
        progress.1.store(whole, Ordering::Relaxed);
        let staging = self.dir.join(".unpacking").join(id);
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::create_dir_all(staging.join("files"))?;
        let mut list: Vec<Unfile> = vec![];
        let r = (|| -> Result<()> {
            for a in archives {
                archive::each(a, &self.stop, &mut |name, _, r| {
                    let rel = gamerepo_core::library::rel_inside(name).map_err(|e| std::io::Error::other(e.to_string()))?;
                    let path = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/");
                    let to = staging.join("files").join(&rel);
                    if let Some(d) = to.parent() {
                        std::fs::create_dir_all(d)?;
                    }
                    let mut w = std::fs::File::create(&to)?;
                    let mut h = sha1::Sha1::new();
                    let mut buf = vec![0u8; 1 << 20];
                    let mut size = 0u64;
                    loop {
                        let n = r.read(&mut buf)?;
                        if n == 0 {
                            break;
                        }
                        h.update(&buf[..n]);
                        w.write_all(&buf[..n])?;
                        size += n as u64;
                        progress.0.fetch_add(n as u64, Ordering::Relaxed);
                    }
                    // a later archive's file of the same name takes its place (a second part)
                    list.retain(|f| f.path != path);
                    list.push(Unfile { path, size, sha1: hex::encode(h.finalize()) });
                    Ok(())
                })
                .map_err(|e| anyhow!("couldn't unpack {}: {e}", a.file_name().unwrap_or_default().to_string_lossy()))?;
                if self.stop.load(Ordering::Relaxed) {
                    bail!("stopped");
                }
            }
            if list.is_empty() {
                bail!("its archive has no files in it");
            }
            list.sort_by(|a, b| a.path.cmp(&b.path));
            let at = self.dir.join(id);
            let _ = std::fs::remove_dir_all(&at);
            std::fs::rename(&staging, &at)?;
            crate::tls::write_new(&at.join(LIST), &serde_json::to_vec(&list)?)?;
            Ok(())
        })();
        let _ = std::fs::remove_dir_all(&staging);
        r?;
        self.prune(id);
        Ok(())
    }

    /// Keep the newest unpacked games up to the limit (never the one just unpacked).
    fn prune(&self, keep: &str) {
        let mut all: Vec<(std::time::SystemTime, u64, PathBuf)> = vec![];
        for d in std::fs::read_dir(&self.dir).into_iter().flatten().flatten() {
            let name = d.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name == keep {
                continue;
            }
            let list = d.path().join(LIST);
            let Some(files) = std::fs::read(&list).ok().and_then(|b| serde_json::from_slice::<Vec<Unfile>>(&b).ok()) else { continue };
            let at = std::fs::metadata(&list).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
            all.push((at, files.iter().map(|f| f.size).sum(), d.path()));
        }
        let kept: u64 = std::fs::read(self.dir.join(keep).join(LIST)).ok().and_then(|b| serde_json::from_slice::<Vec<Unfile>>(&b).ok()).map(|f| f.iter().map(|x| x.size).sum()).unwrap_or(0);
        let mut total: u64 = kept + all.iter().map(|a| a.1).sum::<u64>();
        all.sort();
        for (_, size, p) in all {
            if total <= self.limit {
                break;
            }
            // the list first, so a game half removed reads as not unpacked
            let _ = std::fs::remove_file(p.join(LIST));
            let _ = std::fs::remove_dir_all(&p);
            total -= size;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zip(path: &Path, members: &[(&str, &[u8])]) {
        let f = std::fs::File::create(path).unwrap();
        let mut z = zip::ZipWriter::new(f);
        for (name, bytes) in members {
            z.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(bytes).unwrap();
        }
        z.finish().unwrap();
    }

    #[test]
    fn a_games_archive_is_unpacked_once_with_each_files_sha1() {
        let dir = std::env::temp_dir().join(format!("gamerepo-unpack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("Zeta.zip");
        zip(&a, &[("Zeta/zeta.exe", b"MZ program"), ("Zeta/data/level1.dat", b"level one")]);
        let stop = Arc::new(AtomicBool::new(false));
        let u = Unpacker::new(dir.join("unpacked"), 1 << 30, stop.clone());
        let id = Unpacker::id_of(&[a.clone()]);
        assert!(matches!(u.status(&id), Unpacked::Unknown));
        u.want(&id, vec![a.clone()]);
        let end = std::time::Instant::now() + Duration::from_secs(20);
        let list = loop {
            match u.status(&id) {
                Unpacked::Ready(l) => break l,
                Unpacked::Failed(e) => panic!("{e}"),
                _ if std::time::Instant::now() > end => panic!("not unpacked"),
                _ => std::thread::sleep(Duration::from_millis(50)),
            }
        };
        assert_eq!(list.iter().map(|f| (f.path.as_str(), f.size)).collect::<Vec<_>>(), [("Zeta/data/level1.dat", 9), ("Zeta/zeta.exe", 10)]);
        assert_eq!(list[1].sha1, hex::encode(sha1::Sha1::digest(b"MZ program")));
        assert_eq!(std::fs::read(u.file(&id, "Zeta/zeta.exe").unwrap()).unwrap(), b"MZ program");
        assert!(u.file(&id, "../files.json").is_none() && u.file(&id, "Zeta/../../x").is_none());
        // the archive changed: another id, unpacked again when asked
        std::thread::sleep(Duration::from_millis(20));
        zip(&a, &[("Zeta/zeta.exe", b"MZ program 2")]);
        assert_ne!(Unpacker::id_of(&[a]), id);
        stop.store(true, Ordering::Relaxed);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn how_far_it_has_got_is_counted_in_bytes_of_the_whole() {
        let dir = std::env::temp_dir().join(format!("gamerepo-unpack-progress-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("Omega.zip");
        zip(&a, &[("Omega/omega.exe", &[7u8; 3000]), ("Omega/omega.dat", &[1u8; 5000])]);
        let stop = Arc::new(AtomicBool::new(false));
        let u = Unpacker::new(dir.join("unpacked"), 1 << 30, stop.clone());
        let id = Unpacker::id_of(&[a.clone()]);
        assert_eq!(u.progress(&id), None);
        let p: Progress = Arc::default();
        u.unpack(&id, &[a], &p).unwrap();
        assert_eq!((p.0.load(Ordering::Relaxed), p.1.load(Ordering::Relaxed)), (8000, 8000));
        // only while it's being unpacked
        assert!(matches!(u.status(&id), Unpacked::Ready(_)) && u.progress(&id).is_none());
        stop.store(true, Ordering::Relaxed);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
