//! Noticing changes made outside the app (docs/PLAN.md, "Phase 2 design", Folder
//! watcher): the folders with games on local drives are watched while the app is
//! open. Changes are gathered until the folders have been quiet for a moment, then
//! `on_change` gets the paths that changed, once, and only those are scanned again.
//! Hidden entries (the set-aside and staging folders among them) and the library's
//! own `_library/` are left out. Network drives don't report changes reliably, so
//! they're scanned on demand ([`is_network`]).
//!
//! Adapted from Model Library's `watch.rs`, which watches one library folder.

use notify::{Event, EventKind, RecursiveMode, Watcher as _};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

/// How long the folders must be quiet before what changed is scanned.
pub const SETTLE: Duration = Duration::from_secs(3);
/// More changed paths than this and every folder is scanned instead.
const MANY: usize = 5000;

/// Watches some folders until it's dropped.
pub struct Watcher {
    dirs: Vec<PathBuf>,
    _inner: notify::RecommendedWatcher,
}

impl Watcher {
    pub fn dirs(&self) -> &[PathBuf] {
        &self.dirs
    }
}

/// Does a change at `p` matter to a scan? Not in a hidden folder, nor in `skip`.
fn matters(dirs: &[PathBuf], skip: &Path, p: &Path) -> bool {
    if p.starts_with(skip) {
        return false;
    }
    dirs.iter().any(|d| match p.strip_prefix(d) {
        Ok(rel) => !rel.components().any(|c| c.as_os_str().to_string_lossy().starts_with('.')),
        Err(_) => false,
    })
}

/// Watch each of `dirs` and everything in them (except under `skip`, the library's
/// own folder). Once changes have settled for `settle`, `on_change` gets the paths
/// that changed (None: too many to list, or the system lost track, so scan everything).
pub fn watch(
    dirs: &[PathBuf],
    skip: &Path,
    settle: Duration,
    on_change: impl Fn(Option<Vec<PathBuf>>) + Send + 'static,
) -> notify::Result<Watcher> {
    let (tx, rx) = mpsc::channel::<Option<Vec<PathBuf>>>();
    let (base, skip) = (dirs.to_vec(), skip.to_path_buf());
    let mut w = notify::recommended_watcher(move |res: notify::Result<Event>| {
        let _ = match res {
            Ok(ev) if matches!(ev.kind, EventKind::Access(_)) => Ok(()),
            Ok(ev) if ev.need_rescan() => tx.send(None),
            Ok(ev) => {
                let paths: Vec<PathBuf> = ev.paths.into_iter().filter(|p| matters(&base, &skip, p)).collect();
                if paths.is_empty() {
                    Ok(())
                } else {
                    tx.send(Some(paths))
                }
            }
            Err(_) => tx.send(None),
        };
    })?;
    for d in dirs {
        w.watch(d, RecursiveMode::Recursive)?;
    }
    std::thread::Builder::new().name("folder-watch".into()).spawn(move || {
        // a change; then more until none come for `settle`; then report them
        while let Ok(first) = rx.recv() {
            let mut all: Option<HashSet<PathBuf>> = first.map(|p| p.into_iter().collect());
            loop {
                match rx.recv_timeout(settle) {
                    Ok(more) => match (all.as_mut(), more) {
                        (Some(set), Some(p)) => {
                            set.extend(p);
                            if set.len() > MANY {
                                all = None;
                            }
                        }
                        _ => all = None,
                    },
                    Err(mpsc::RecvTimeoutError::Timeout) => break,
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                }
            }
            on_change(all.map(|s| s.into_iter().collect()));
        }
    })?;
    Ok(Watcher { dirs: dirs.to_vec(), _inner: w })
}

/// Is `dir` on a network drive (a mapped drive or `\\server\share` on Windows; an
/// NFS, SMB or similar mount elsewhere)? Those are scanned on demand, not watched.
pub fn is_network(dir: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use std::path::{Component, Prefix};
        let root = match dir.components().next() {
            Some(Component::Prefix(p)) => match p.kind() {
                Prefix::UNC(..) | Prefix::VerbatimUNC(..) => return true,
                Prefix::Disk(d) | Prefix::VerbatimDisk(d) => format!("{}:\\", d as char),
                _ => return false,
            },
            _ => return false,
        };
        let wide: Vec<u16> = std::ffi::OsStr::new(&root).encode_wide().chain(Some(0)).collect();
        // DRIVE_REMOTE
        unsafe { windows_sys::Win32::Storage::FileSystem::GetDriveTypeW(wide.as_ptr()) == 4 }
    }
    #[cfg(not(windows))]
    {
        // the deepest mount the folder is on, from /proc/mounts (Linux; elsewhere, local)
        let Ok(mounts) = std::fs::read_to_string("/proc/mounts") else { return false };
        let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
        let mut best: Option<(usize, bool)> = None;
        for line in mounts.lines() {
            let mut f = line.split_whitespace();
            let (Some(_), Some(at), Some(fs)) = (f.next(), f.next(), f.next()) else { continue };
            let at = at.replace("\\040", " ");
            if dir.starts_with(&at) && best.is_none_or(|(n, _)| at.len() >= n) {
                let net = matches!(fs, "nfs" | "nfs4" | "cifs" | "smb3" | "smbfs" | "9p" | "afs" | "ceph" | "fuse.sshfs" | "fuse.rclone");
                best = Some((at.len(), net));
            }
        }
        best.is_some_and(|(_, net)| net)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::tests::temp_dir;
    use std::sync::{Arc, Mutex};

    #[test]
    fn reports_settled_changes_once_and_skips_hidden_folders() {
        let base = temp_dir("watch");
        let (a, b, app) = (base.join("Carts"), base.join("More"), base.join("Carts/_library"));
        for d in [&a, &b, &app, &a.join(".gamerepo-set-aside")] {
            std::fs::create_dir_all(d).unwrap();
        }
        let seen: Arc<Mutex<Vec<Option<Vec<PathBuf>>>>> = Arc::default();
        let s = seen.clone();
        let w = watch(&[a.clone(), b.clone()], &app, Duration::from_millis(800), move |p| s.lock().unwrap().push(p)).unwrap();
        assert_eq!(w.dirs(), [a.clone(), b.clone()]);
        std::fs::write(app.join("library.json"), "{}").unwrap();
        std::fs::write(a.join(".gamerepo-set-aside/old.gb"), "x").unwrap();
        std::thread::sleep(Duration::from_millis(1500));
        assert!(seen.lock().unwrap().is_empty(), "the library's own folder and hidden ones are left out");
        std::fs::create_dir_all(a.join("Game Boy")).unwrap();
        for i in 0..5 {
            std::fs::write(a.join(format!("Game Boy/g{i}.gb")), "rom").unwrap();
            std::thread::sleep(Duration::from_millis(50));
        }
        std::fs::write(b.join("other.gb"), "rom").unwrap();
        let t = std::time::Instant::now();
        while seen.lock().unwrap().is_empty() && t.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(50));
        }
        std::thread::sleep(Duration::from_millis(1500));
        let got = seen.lock().unwrap().clone();
        assert_eq!(got.len(), 1, "one report for the lot: {got:?}");
        let paths = got[0].clone().unwrap();
        assert!(paths.iter().any(|p| p.ends_with("Game Boy/g4.gb")), "{paths:?}");
        assert!(paths.iter().any(|p| p.ends_with("More/other.gb")), "{paths:?}");
        drop(w);
        assert!(!is_network(&base));
        let _ = std::fs::remove_dir_all(&base);
    }
}
