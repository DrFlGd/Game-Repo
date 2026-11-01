//! A PC game's folder checked against how it was installed (docs/PLAN.md, "Mods on
//! installed games", D207–D208): each file the install put there with its size and SHA-1,
//! compared with what's there now, so a change is found whoever made it (a mod put in by
//! hand, a patcher, a file deleted) and the game can be put back from the library. Game
//! Repo Go checks against its record; Game Repo, against the library's unpacked copy.
//!
//! What a check learns is kept in the game's own `.gamerepo-mods/state.json`: each file's
//! size, time and SHA-1 as last read (so a later check reads again only what changed), and
//! the files the user said are the game's own (`keep`), which aren't listed again.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The folder in a game's folder that Game Repo keeps its own things in.
pub const DIR: &str = ".gamerepo-mods";
/// In [`DIR`]: what a check learnt.
pub const STATE: &str = "state.json";
/// In [`DIR`]: files a put back took away, by when (`put-aside/<when>/<path>`).
pub const PUT_ASIDE: &str = "put-aside";

/// A file as last read.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Seen {
    pub size: u64,
    /// Its modified time (nanoseconds since 1970)
    pub modified: u64,
    pub sha1: String,
    /// When it was read (nanoseconds since 1970): a file read within two seconds of its
    /// time is read again next time, as a change in the same tick of the clock (FAT's two
    /// seconds, Windows' sixteenth of one) wouldn't change its time
    pub read_at: u64,
}

/// Two seconds, in nanoseconds.
const SETTLED: u64 = 2_000_000_000;

/// `.gamerepo-mods/state.json`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct State {
    /// Files the user keeps as they are (paths in the game's folder): one added stays the
    /// game's own, one changed or taken away stays so
    pub keep: Vec<String>,
    /// Each file of the game's folder as last read, by its path
    pub seen: BTreeMap<String, Seen>,
    /// The library's copy's files as last read (Game Repo's check, by their paths there)
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub library: BTreeMap<String, Seen>,
    /// The mods in the game now, in order, as Game Repo Go put them in (D209); none: as the
    /// library ticks them when it was installed, before Go kept this
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mods: Option<Vec<String>>,
    /// Which mod gave each file it gave (by its path in the game's folder)
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub owners: BTreeMap<String, String>,
}

impl State {
    pub fn path(game: &Path) -> PathBuf {
        game.join(DIR).join(STATE)
    }

    pub fn read(game: &Path) -> State {
        std::fs::read(Self::path(game)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn write(&self, game: &Path) -> std::io::Result<()> {
        let p = Self::path(game);
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(p, serde_json::to_vec_pretty(self).unwrap_or_default())
    }

    /// Whether the user keeps this file as the game's own.
    pub fn keeps(&self, rel: &str) -> bool {
        self.keep.iter().any(|k| k.eq_ignore_ascii_case(rel))
    }
}

/// A file a game makes as it's played, never listed by a check: logs, crash dumps, shader
/// caches, temporary files, and Game Repo's own folders.
pub fn made_by_playing(rel: &str) -> bool {
    let lower = rel.to_lowercase();
    let parts: Vec<&str> = lower.split('/').collect();
    let name = parts.last().copied().unwrap_or("");
    if parts.iter().any(|p| p.starts_with(".gamerepo-")) {
        return true;
    }
    if parts[..parts.len().saturating_sub(1)].iter().any(|p| ["logs", "log", "crashdumps", "crashes", "crash reports", "shadercache", "shader_cache", "shadercaches", "dxcache", "glcache", "nvidia"].contains(p)) {
        return true;
    }
    [".log", ".dmp", ".mdmp", ".tmp", ".crash"].iter().any(|x| name.ends_with(x)) || name == "thumbs.db" || name == "desktop.ini" || name == ".ds_store"
}

/// A file as the install put it: its path in the game's folder, size and SHA-1 (empty: not
/// known, so only its size is compared).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Want {
    pub path: String,
    pub size: u64,
    pub sha1: String,
}

/// Files put aside by a put back, at one time.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Aside {
    /// Its folder's name in `put-aside/` (when, `YYYYMMDD-HHMMSS`)
    pub when: String,
    pub files: usize,
    pub size: u64,
}

/// What a check found.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Check {
    /// The game's folder
    pub folder: String,
    /// `as_installed` | `changed` | `not_there`
    pub state: String,
    pub changed: Vec<String>,
    pub added: Vec<String>,
    pub missing: Vec<String>,
    /// Files kept as the game's own (`keep`)
    pub kept: usize,
    /// Files the game made as it was played, not listed
    pub played: usize,
    /// What put backs put aside, newest first
    pub put_aside: Vec<Aside>,
    /// The mods Game Repo Go put in, in order (D209), when it knows
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mods: Option<Vec<String>>,
    /// Which mod gave a file, for each changed file a mod gave
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub owners: BTreeMap<String, String>,
}

impl Check {
    pub fn is_changed(&self) -> bool {
        !(self.changed.is_empty() && self.added.is_empty() && self.missing.is_empty())
    }
}

/// Every file under `dir` (but Game Repo's own folders), as its slashed path in it.
pub fn files_in(dir: &Path) -> Vec<(String, std::fs::Metadata)> {
    let mut out = vec![];
    let mut stack = vec![(dir.to_path_buf(), String::new())];
    while let Some((d, rel)) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let r = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
            let Ok(m) = std::fs::metadata(e.path()) else { continue };
            if m.is_dir() {
                if !name.starts_with(".gamerepo-") {
                    stack.push((e.path(), r));
                }
            } else {
                out.push((r, m));
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

pub fn modified_of(m: &std::fs::Metadata) -> u64 {
    m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos() as u64).unwrap_or(0)
}

/// A file's SHA-1, read again only when its size or time changed since `seen` had it.
pub fn sha1_seen(seen: &mut BTreeMap<String, Seen>, rel: &str, path: &Path, m: &std::fs::Metadata, hash: &dyn Fn(&Path) -> std::io::Result<String>) -> std::io::Result<String> {
    let (size, modified) = (m.len(), modified_of(m));
    if let Some(s) = seen.get(rel).filter(|s| s.size == size && s.modified == modified && !s.sha1.is_empty() && s.read_at > modified.saturating_add(SETTLED)) {
        return Ok(s.sha1.clone());
    }
    let read_at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
    let sha1 = hash(path)?;
    seen.insert(rel.to_string(), Seen { size, modified, sha1: sha1.clone(), read_at });
    Ok(sha1)
}

/// The game's folder `dir` checked against `want` (paths compared ignoring case, as on
/// Windows), with `hash` for a file's SHA-1. The state is read from the folder, updated
/// and written back.
pub fn check(dir: &Path, want: &[Want], hash: &dyn Fn(&Path) -> std::io::Result<String>) -> std::io::Result<Check> {
    let mut out = Check { folder: dir.display().to_string(), ..Default::default() };
    if !dir.is_dir() {
        out.state = "not_there".into();
        out.missing = want.iter().map(|w| w.path.clone()).collect();
        return Ok(out);
    }
    let mut state = State::read(dir);
    let by: BTreeMap<String, &Want> = want.iter().map(|w| (w.path.to_lowercase(), w)).collect();
    let mut found: BTreeSet<String> = BTreeSet::new();
    let mut seen = std::mem::take(&mut state.seen);
    let mut still: BTreeMap<String, Seen> = BTreeMap::new();
    for (rel, m) in files_in(dir) {
        let lower = rel.to_lowercase();
        match by.get(&lower) {
            Some(w) => {
                found.insert(lower);
                let same = if m.len() != w.size {
                    false
                } else if w.sha1.is_empty() {
                    true
                } else {
                    sha1_seen(&mut seen, &rel, &dir.join(&rel), &m, hash)?.eq_ignore_ascii_case(&w.sha1)
                };
                if let Some(s) = seen.remove(&rel) {
                    still.insert(rel.clone(), s);
                }
                if !same && state.keeps(&rel) {
                    out.kept += 1;
                } else if !same {
                    out.changed.push(rel);
                }
            }
            None if made_by_playing(&rel) => out.played += 1,
            None if state.keeps(&rel) => out.kept += 1,
            None => out.added.push(rel),
        }
    }
    for (k, w) in &by {
        if found.contains(k) {
            continue;
        }
        if state.keeps(&w.path) {
            out.kept += 1;
        } else {
            out.missing.push(w.path.clone());
        }
    }
    state.seen = still;
    out.state = if out.is_changed() { "changed" } else { "as_installed" }.into();
    out.put_aside = put_aside(dir);
    out.mods = state.mods.clone();
    out.owners = out.changed.iter().filter_map(|r| state.owners.get(r).map(|m| (r.clone(), m.clone()))).collect();
    state.write(dir)?;
    Ok(out)
}

/// What put backs put aside in the game's folder, newest first.
pub fn put_aside(dir: &Path) -> Vec<Aside> {
    let top = dir.join(DIR).join(PUT_ASIDE);
    let mut v: Vec<Aside> = std::fs::read_dir(&top)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| {
            let files = files_in(&e.path());
            Aside { when: e.file_name().to_string_lossy().into_owned(), files: files.len(), size: files.iter().map(|(_, m)| m.len()).sum() }
        })
        .collect();
    v.sort_by(|a, b| b.when.cmp(&a.when));
    v
}

/// A file of the game's folder moved into `put-aside/<when>/`, its folders kept.
pub fn put_file_aside(dir: &Path, when: &str, rel: &str) -> std::io::Result<()> {
    let from = dir.join(rel);
    let to = dir.join(DIR).join(PUT_ASIDE).join(when).join(rel);
    if let Some(p) = to.parent() {
        std::fs::create_dir_all(p)?;
    }
    std::fs::rename(&from, &to)?;
    // the folders it leaves empty go too
    let mut at = from.parent().map(Path::to_path_buf);
    while let Some(d) = at {
        if d == dir || std::fs::remove_dir(&d).is_err() {
            break;
        }
        at = d.parent().map(Path::to_path_buf);
    }
    Ok(())
}

/// A put-aside folder's name for now (UTC).
pub fn when_now() -> String {
    let s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
    let (days, rem) = (s.div_euclid(86_400), s.rem_euclid(86_400));
    // civil from days (Howard Hinnant's)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}{m:02}{d:02}-{:02}{:02}{:02}", rem / 3600, rem % 3600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_hash(p: &Path) -> std::io::Result<String> {
        Ok(format!("{:x}", std::fs::read(p)?.iter().fold(7u64, |h, b| h.wrapping_mul(31).wrapping_add(*b as u64))))
    }

    #[test]
    fn a_check_finds_whats_changed_added_and_missing_but_not_what_playing_made() {
        let dir = std::env::temp_dir().join(format!("gamerepo-wire-gamefiles-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Data")).unwrap();
        std::fs::create_dir_all(dir.join("logs")).unwrap();
        for (p, b) in [("game.exe", "exe"), ("Data/level1.dat", "level-1"), ("Data/level2.dat", "level-2")] {
            std::fs::write(dir.join(p), b).unwrap();
        }
        let want: Vec<Want> = [("game.exe", "exe"), ("Data/Level1.dat", "level-1"), ("Data/level2.dat", "level-2"), ("readme.txt", "hi")]
            .iter()
            .map(|(p, b)| Want { path: p.to_string(), size: b.len() as u64, sha1: fake_hash_of(b) })
            .collect();
        // as installed but the readme, deleted
        let c = check(&dir, &want, &fake_hash).unwrap();
        assert_eq!((c.state.as_str(), c.missing.clone(), c.changed.len(), c.added.len()), ("changed", vec!["readme.txt".to_string()], 0, 0), "case doesn't matter: {c:?}");
        // a same-size change, a file added, a log made
        std::fs::write(dir.join("Data/level2.dat"), "LEVEL-2").unwrap();
        std::fs::write(dir.join("Data/mod.pak"), "mod").unwrap();
        std::fs::write(dir.join("logs/run.txt"), "x").unwrap();
        std::fs::write(dir.join("crash.dmp"), "x").unwrap();
        let c = check(&dir, &want, &fake_hash).unwrap();
        assert_eq!(c.changed, vec!["Data/level2.dat"]);
        assert_eq!(c.added, vec!["Data/mod.pak"]);
        assert_eq!(c.played, 2);
        // kept as the game's own, and put aside
        let mut s = State::read(&dir);
        assert!(s.seen.contains_key("game.exe"), "what was read is kept");
        // a file settled before it was read isn't read again while its size and time stay
        let mut old = s.clone();
        old.seen.get_mut("game.exe").unwrap().sha1 = "cached".into();
        old.seen.get_mut("game.exe").unwrap().read_at = u64::MAX;
        let m = std::fs::metadata(dir.join("game.exe")).unwrap();
        assert_eq!(sha1_seen(&mut old.seen, "game.exe", &dir.join("game.exe"), &m, &fake_hash).unwrap(), "cached");
        s.keep.push("data/mod.pak".into());
        s.write(&dir).unwrap();
        let c = check(&dir, &want, &fake_hash).unwrap();
        assert_eq!((c.added.len(), c.kept), (0, 1));
        put_file_aside(&dir, "20261101-120000", "Data/level2.dat").unwrap();
        let c = check(&dir, &want, &fake_hash).unwrap();
        assert!(c.missing.contains(&"Data/level2.dat".to_string()) && c.changed.is_empty());
        assert_eq!(c.put_aside, vec![Aside { when: "20261101-120000".into(), files: 1, size: 7 }]);
        assert!(!c.added.iter().any(|a| a.contains(".gamerepo")), "its own folder isn't the game's");
        assert_eq!(when_now().len(), 15);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn fake_hash_of(b: &str) -> String {
        format!("{:x}", b.bytes().fold(7u64, |h, b| h.wrapping_mul(31).wrapping_add(b as u64)))
    }
}
