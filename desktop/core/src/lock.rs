//! The library's lock (docs/PLAN.md, D148): two computers sharing one library (on a network
//! drive, or a folder a sync tool keeps) mustn't change the same files at once. A change (an
//! Organise plan, an import, Undo, finishing one, a custom DAT's change) takes
//! `_library/journal/change.lock`, naming the computer, what it's doing and when it started,
//! and refreshes it every minute while it runs. Game Repo on another computer that finds a
//! fresh lock waits (a change, saying so, until the person stops it) or says who has it (a
//! quick change). A lock not refreshed for ten minutes was left by a computer that stopped
//! (switched off, or the app ended) and is taken over.

use crate::library::{now, Library, APP_DIR};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const FILE: &str = "change.lock";
/// How long a lock lasts without being refreshed, in seconds.
const STALE: u64 = 600;
/// How often a running change refreshes its lock.
const REFRESH: Duration = Duration::from_secs(60);

/// What the lock says.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Held {
    pub computer: String,
    /// This run of the app (one computer may run two)
    pub instance: String,
    /// What's being done: the change's label
    pub what: String,
    /// The Organise change's id, for History
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change: Option<String>,
    pub started: String,
    /// When it was last refreshed, in seconds since 1970 by the clock of the computer holding it
    pub refreshed: u64,
}

fn path(lib: &Library) -> PathBuf {
    lib.root().join(APP_DIR).join("journal").join(FILE)
}

fn secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// This computer's name, as Windows or the system calls it.
pub fn computer() -> String {
    std::env::var("COMPUTERNAME")
        .ok()
        .or_else(|| std::env::var("HOSTNAME").ok())
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "another computer".into())
}

/// This run of the app.
fn instance() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| format!("{}-{:x}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0)))
}

fn read(lib: &Library) -> Option<Held> {
    serde_json::from_slice(&std::fs::read(path(lib)).ok()?).ok()
}

/// Who holds the library's lock, if anyone still does: refreshed in the last ten minutes, by
/// what it says or by the file's time (the computers' clocks may not agree).
pub fn holder(lib: &Library) -> Option<Held> {
    let h = read(lib)?;
    let touched = std::fs::metadata(path(lib)).and_then(|m| m.modified()).map(secs).unwrap_or(0);
    let now = secs(SystemTime::now());
    (h.refreshed + STALE > now || touched + STALE > now).then_some(h)
}

/// The lock, held by another run of the app (on another computer, or another window here).
pub fn elsewhere(lib: &Library) -> Option<Held> {
    holder(lib).filter(|h| h.instance != instance())
}

/// Who's changing the library, as the person reads it.
pub fn says(h: &Held) -> String {
    if h.computer.eq_ignore_ascii_case(&computer()) {
        format!("Game Repo in another window on this computer is changing this library ({})", h.what)
    } else {
        format!("Game Repo on {} is changing this library ({})", h.computer, h.what)
    }
}

/// The lock while it's held: refreshed every minute, and let go when dropped.
pub struct Lock {
    lib: Option<Library>,
    stop: Option<mpsc::Sender<()>>,
    refresher: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Lock {
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(t) = self.refresher.take() {
            let _ = t.join();
        }
        // only if it's still this run's (one taken over isn't removed)
        if let Some(lib) = &self.lib {
            if read(lib).is_some_and(|h| h.instance == instance()) {
                let _ = std::fs::remove_file(path(lib));
            }
        }
    }
}

/// Take the lock, or say who has it.
fn try_take(lib: &Library, what: &str, change: Option<&str>) -> Result<std::result::Result<Lock, Held>> {
    if let Some(h) = holder(lib) {
        if h.instance == instance() {
            // this run has it already (a change within a change): the outer one lets it go
            return Ok(Ok(Lock { lib: None, stop: None, refresher: None }));
        }
        return Ok(Err(h));
    }
    let p = path(lib);
    std::fs::create_dir_all(p.parent().unwrap_or(&p)).with_context(|| format!("couldn't write {}", p.display()))?;
    // one left by a computer that stopped is taken over
    let _ = std::fs::remove_file(&p);
    let held = Held { computer: computer(), instance: instance().into(), what: what.into(), change: change.map(String::from), started: now(), refreshed: secs(SystemTime::now()) };
    match std::fs::OpenOptions::new().write(true).create_new(true).open(&p) {
        Ok(mut f) => f.write_all(&serde_json::to_vec_pretty(&held)?).with_context(|| format!("couldn't write {}", p.display()))?,
        // another computer took it just now
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Ok(Err(read(lib).unwrap_or_default())),
        Err(e) => return Err(e).with_context(|| format!("couldn't write {}", p.display())),
    }
    let (tx, rx) = mpsc::channel::<()>();
    let (keep, lib2) = (held, lib.clone());
    let refresher = std::thread::spawn(move || {
        let mut keep = keep;
        while let Err(mpsc::RecvTimeoutError::Timeout) = rx.recv_timeout(REFRESH) {
            if read(&lib2).is_some_and(|h| h.instance == keep.instance) {
                keep.refreshed = secs(SystemTime::now());
                if let Ok(b) = serde_json::to_vec_pretty(&keep) {
                    let _ = crate::config::write_atomic(&path(&lib2), &b);
                }
            }
        }
    });
    Ok(Ok(Lock { lib: Some(lib.clone()), stop: Some(tx), refresher: Some(refresher) }))
}

/// Take the library's lock for a quick change, or say who has it.
pub fn take(lib: &Library, what: &str) -> Result<Lock> {
    match try_take(lib, what, None)? {
        Ok(l) => Ok(l),
        Err(h) => bail!("{}: try again when it's finished.", says(&h)),
    }
}

/// Take the library's lock for a change (`change`: its id), waiting while another computer
/// has it, which `progress` says, until it's free or the person stops.
pub fn wait(lib: &Library, what: &str, change: Option<&str>, cancel: &AtomicBool, progress: &mut dyn FnMut(Value)) -> Result<Lock> {
    loop {
        match try_take(lib, what, change)? {
            Ok(l) => return Ok(l),
            Err(h) => {
                progress(json!({ "text": format!("waiting: {}", says(&h)), "waiting": h.computer }));
                for _ in 0..20 {
                    if cancel.load(Ordering::Relaxed) {
                        bail!("stopped");
                    }
                    std::thread::sleep(Duration::from_millis(250));
                }
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A lock as another computer leaves it, refreshed `ago` seconds back.
    pub(crate) fn other_computer(lib: &Library, what: &str, change: Option<&str>, ago: u64) {
        let h = Held { computer: "OTHER-PC".into(), instance: "1-1".into(), what: what.into(), change: change.map(String::from), started: now(), refreshed: secs(SystemTime::now()) - ago };
        std::fs::create_dir_all(path(lib).parent().unwrap()).unwrap();
        std::fs::write(path(lib), serde_json::to_vec(&h).unwrap()).unwrap();
        // the file's time too, as that computer wrote it
        let f = std::fs::File::options().write(true).open(path(lib)).unwrap();
        f.set_modified(SystemTime::now() - Duration::from_secs(ago)).unwrap();
    }

    #[test]
    fn a_change_takes_the_lock_and_another_computers_fresh_one_is_waited_for_a_stale_one_taken_over() {
        let dir = std::env::temp_dir().join(format!("gamerepo-test-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let lib = Library::open(dir.join("Lib")).unwrap();
        {
            let _l = take(&lib, "Organising").unwrap();
            let h = holder(&lib).unwrap();
            assert_eq!((h.computer, h.what.as_str()), (computer(), "Organising"));
            assert!(elsewhere(&lib).is_none());
            // a change within it is let through, and doesn't let the lock go
            drop(take(&lib, "Undoing").unwrap());
            assert!(holder(&lib).is_some());
        }
        assert!(!path(&lib).exists(), "let go when it ends");

        other_computer(&lib, "Importing games", Some("c-1"), 30);
        let e = take(&lib, "Organising").err().unwrap().to_string();
        assert_eq!(e, "Game Repo on OTHER-PC is changing this library (Importing games): try again when it's finished.");
        assert_eq!(elsewhere(&lib).unwrap().change.as_deref(), Some("c-1"));
        // a change waits, saying so, until the person stops it
        let stop = AtomicBool::new(false);
        let mut said = vec![];
        let e = wait(&lib, "Organising", None, &stop, &mut |p| {
            said.push(p["text"].as_str().unwrap_or("").to_string());
            stop.store(true, Ordering::Relaxed);
        });
        assert_eq!(e.err().map(|e| e.to_string()).as_deref(), Some("stopped"));
        assert_eq!(said, ["waiting: Game Repo on OTHER-PC is changing this library (Importing games)"]);
        assert_eq!(read(&lib).unwrap().computer, "OTHER-PC", "theirs is left alone");

        // one not refreshed for ten minutes is taken over
        other_computer(&lib, "Importing games", None, 700);
        assert!(holder(&lib).is_none());
        let l = wait(&lib, "Organising", Some("c-2"), &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(holder(&lib).unwrap().change.as_deref(), Some("c-2"));
        drop(l);
        assert!(!path(&lib).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
