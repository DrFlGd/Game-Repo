//! Saves in the library (docs/PLAN.md, "Save sync and the Devices page", D181, D184):
//! `Saves/<system>/<Game name>/<save>/` holds the newest of each save as ordinary files,
//! and `.versions/<when>-<device>/` under it every version with its `version.json`.
//! Versions are only ever added, each in a folder of its own, so Game Repo and Game Repo
//! Server can both write here without a lock; the newest is worked out from them (the
//! newest made that isn't behind a conflict), and its files copied up beside `.versions`
//! by whichever wrote last.

use crate::config;
use anyhow::{bail, Context, Result};
use gamerepo_wire::saves::{same_files, SaveFile, Unit, UnitState, Upload, Uploaded, Version};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The library's folder of saves.
pub const FOLDER: &str = "Saves";
/// A state's slot keeps this many versions (saves keep every one, D184).
pub const STATE_VERSIONS: usize = 10;

/// What's in a save's folder beside `.versions`: the save, and the newest version's files
/// copied there, so they can be taken out of the folder.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct UnitFile {
    unit: Unit,
    current: String,
    files: Vec<String>,
}

pub struct Store {
    dir: PathBuf,
}

/// A name that works as a file or folder name on Windows and elsewhere.
pub fn safe_name(s: &str) -> String {
    let mut out: String = s.chars().map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { '_' } else { c }).collect();
    while out.ends_with('.') || out.ends_with(' ') {
        out.pop();
    }
    if out.is_empty() || out == ".." {
        out = "_".into();
    }
    out
}

/// The game's own name, as its save files are named in the library (`{name}`).
pub fn game_name(key: &str) -> String {
    safe_name(key.split_once('/').map(|(_, n)| n).unwrap_or(key))
}

/// A save file's name in a folder: the game's name for `{name}`, refused if it would
/// leave the folder or couldn't be made on Windows (so one sent to a server on Linux works
/// everywhere).
pub fn file_name(name: &str, game: &str) -> Result<PathBuf> {
    let n = name.replace("{name}", game);
    let p = Path::new(&n);
    if n.is_empty() || n.contains(|c: char| c.is_control() || "<>:\"\\|?*".contains(c)) || p.is_absolute() || p.components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
        bail!("{name} isn't a save file's name");
    }
    Ok(p.to_path_buf())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// The newest version and what it differs from (D184).
pub fn state_of(unit: &Unit, versions: &[Version]) -> Option<UnitState> {
    let current = versions.iter().filter(|v| !v.behind).max_by_key(|v| (v.made, v.id.clone()))?.clone();
    let mut against = vec![];
    if !current.differs.is_empty() {
        against.push(current.differs.clone());
    }
    against.extend(versions.iter().filter(|v| v.behind && v.made > current.made).map(|v| v.id.clone()));
    Some(UnitState { unit: unit.clone(), conflict: !against.is_empty(), against, versions: versions.len() as u32, current })
}

impl Store {
    pub fn of(library: &Path) -> Self {
        Self { dir: library.join(FOLDER) }
    }

    fn game_dir(&self, key: &str) -> PathBuf {
        let (system, _) = key.split_once('/').unwrap_or(("other", key));
        self.dir.join(safe_name(system)).join(game_name(key))
    }

    /// A save's folder: `Save (retroarch)`, `State 1 (retroarch Snes9x)`.
    pub fn unit_dir(&self, u: &Unit) -> PathBuf {
        let format = safe_name(&u.format.replace(':', " "));
        let name = match u.slot.as_str() {
            "" => format!("Save ({format})"),
            "auto" => format!("State auto ({format})"),
            s => format!("State {} ({format})", safe_name(s)),
        };
        self.game_dir(&u.key).join(name)
    }

    /// Every version of a save, oldest first; a folder without its `version.json` (one
    /// being written) left out.
    pub fn versions(&self, u: &Unit) -> Vec<Version> {
        let mut out: Vec<Version> = std::fs::read_dir(self.unit_dir(u).join(".versions"))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| std::fs::read(e.path().join("version.json")).ok())
            .filter_map(|b| serde_json::from_slice::<Version>(&b).ok())
            .filter(|v| v.unit == *u)
            .collect();
        out.sort_by_key(|v| (v.made, v.id.clone()));
        out
    }

    pub fn state(&self, u: &Unit) -> Option<UnitState> {
        state_of(u, &self.versions(u))
    }

    /// The saves a game has.
    pub fn units_of(&self, key: &str) -> Vec<Unit> {
        let mut out: Vec<Unit> = std::fs::read_dir(self.game_dir(key))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| std::fs::read(e.path().join("save.json")).ok())
            .filter_map(|b| serde_json::from_slice::<UnitFile>(&b).ok())
            .map(|f| f.unit)
            .filter(|u| u.key == key)
            .collect();
        out.sort();
        out
    }

    /// The saves of these games, as the library has them now.
    pub fn list(&self, keys: &[String]) -> Vec<UnitState> {
        keys.iter().flat_map(|k| self.units_of(k)).filter_map(|u| self.state(&u)).collect()
    }

    /// A version's file on disk.
    pub fn file(&self, u: &Unit, version: &str, name: &str) -> Result<PathBuf> {
        let v = self.versions(u).into_iter().find(|v| v.id == version).with_context(|| format!("That version of {} isn't in the library.", u.label()))?;
        if !v.files.iter().any(|f| f.name == name) {
            bail!("That save has no file {name}.");
        }
        Ok(self.unit_dir(u).join(".versions").join(&v.id).join(file_name(name, &game_name(&u.key))?))
    }

    /// A device's save added as a version (D184), its files read by `blob` (by SHA-1).
    /// The same files as the newest make no version.
    pub fn add(&self, up: &Upload, device: &str, device_name: &str, blob: &dyn Fn(&SaveFile) -> Option<PathBuf>) -> Result<Uploaded> {
        if up.files.is_empty() {
            bail!("A save with no files can't be kept.");
        }
        let game = game_name(&up.unit.key);
        for f in &up.files {
            file_name(&f.name, &game)?;
        }
        if let Some(st) = self.state(&up.unit) {
            if same_files(&st.current.files, &up.files) {
                return Ok(Uploaded { version: st.current.clone(), state: st });
            }
        }
        let mut v = Version {
            unit: up.unit.clone(),
            device: device.into(),
            device_name: device_name.into(),
            emulator: up.emulator.clone(),
            played: up.files.iter().map(|f| f.modified).max().unwrap_or(0),
            base: up.base.clone(),
            files: up.files.clone(),
            ..Default::default()
        };
        if let Some(st) = self.state(&up.unit) {
            if st.current.id != up.base {
                // both moved on since the device's last: a conflict, the later played newest
                v.differs = st.current.id.clone();
                v.behind = v.played < st.current.played;
            }
        }
        self.write(v, |f| blob(f).with_context(|| format!("{} wasn't sent first.", f.name)))
    }

    /// *Use this one* or *Put this version back* (D184, D191): that version made again as
    /// the newest, so it goes everywhere at the next sync (and can itself be undone).
    pub fn choose(&self, u: &Unit, version: &str, device: &str, device_name: &str) -> Result<Uploaded> {
        let versions = self.versions(u);
        let from = versions.iter().find(|v| v.id == version).with_context(|| format!("That version of {} isn't in the library.", u.label()))?.clone();
        let st = state_of(u, &versions);
        let dir = self.unit_dir(u).join(".versions").join(&from.id);
        let game = game_name(&u.key);
        let v = Version {
            unit: u.clone(),
            device: device.into(),
            device_name: device_name.into(),
            emulator: from.emulator.clone(),
            played: from.played,
            base: st.map(|s| s.current.id).unwrap_or_default(),
            files: from.files.clone(),
            chosen: from.id.clone(),
            ..Default::default()
        };
        self.write(v, |f| Ok(dir.join(file_name(&f.name, &game)?)))
    }

    /// A version written: its files, then its `version.json`, then the newest copied up.
    fn write(&self, mut v: Version, source: impl Fn(&SaveFile) -> Result<PathBuf>) -> Result<Uploaded> {
        let udir = self.unit_dir(&v.unit);
        let vdir = udir.join(".versions");
        std::fs::create_dir_all(&vdir).with_context(|| format!("couldn't make {}", vdir.display()))?;
        // after every version there, so the newest is always the last made
        let floor = self.versions(&v.unit).iter().map(|x| x.made + 1).max().unwrap_or(0);
        let mut made = now_ms().max(floor);
        let dir = loop {
            let id = format!("{made:013}-{}", safe_name(&v.device));
            let d = vdir.join(&id);
            match std::fs::create_dir(&d) {
                Ok(()) => {
                    v.id = id;
                    v.made = made;
                    break d;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => made += 1,
                Err(e) => return Err(e).with_context(|| format!("couldn't make {}", d.display())),
            }
        };
        let game = game_name(&v.unit.key);
        let written = (|| -> Result<()> {
            for f in &v.files {
                let to = dir.join(file_name(&f.name, &game)?);
                if let Some(p) = to.parent() {
                    std::fs::create_dir_all(p)?;
                }
                let from = source(f)?;
                std::fs::copy(&from, &to).with_context(|| format!("couldn't copy {}", from.display()))?;
                set_modified(&to, f.modified);
            }
            config::write_json(&dir.join("version.json"), &serde_json::to_value(&v)?)
        })();
        if let Err(e) = written {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(e);
        }
        let versions = self.versions(&v.unit);
        let state = state_of(&v.unit, &versions).context("the save has no newest version")?;
        self.copy_up(&udir, &state.current)?;
        if v.unit.is_state() {
            self.prune(&vdir, &versions, &state);
        }
        Ok(Uploaded { version: v, state })
    }

    /// The newest version's files beside `.versions`, and `save.json` saying which.
    fn copy_up(&self, udir: &Path, current: &Version) -> Result<()> {
        let path = udir.join("save.json");
        let old: UnitFile = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        if old.current == current.id && old.unit == current.unit {
            return Ok(());
        }
        let game = game_name(&current.unit.key);
        for f in &old.files {
            if let Ok(p) = file_name(f, &game) {
                let _ = std::fs::remove_file(udir.join(p));
            }
        }
        let from = udir.join(".versions").join(&current.id);
        let mut names = vec![];
        for f in &current.files {
            let rel = file_name(&f.name, &game)?;
            let to = udir.join(&rel);
            if let Some(p) = to.parent() {
                std::fs::create_dir_all(p)?;
            }
            std::fs::copy(from.join(&rel), &to)?;
            set_modified(&to, f.modified);
            names.push(f.name.clone());
        }
        config::write_json(&path, &serde_json::to_value(UnitFile { unit: current.unit.clone(), current: current.id.clone(), files: names })?)
    }

    /// A state's oldest versions past [`STATE_VERSIONS`] taken away, never the newest nor
    /// one a conflict is about.
    fn prune(&self, vdir: &Path, versions: &[Version], state: &UnitState) {
        let keep = |v: &Version| v.id == state.current.id || state.against.contains(&v.id);
        let extra = versions.len().saturating_sub(STATE_VERSIONS);
        for v in versions.iter().filter(|v| !keep(v)).take(extra) {
            let _ = std::fs::remove_dir_all(vdir.join(&v.id));
        }
    }
}

/// A game's saves for its page (D191): each with the newest, whether two differ, and every
/// version, newest first.
pub fn of_game(lib: &crate::library::Library, key: &str) -> serde_json::Value {
    let store = Store::of(lib.root());
    let units: Vec<serde_json::Value> = store
        .units_of(key)
        .into_iter()
        .filter_map(|u| {
            let mut versions = store.versions(&u);
            let st = state_of(&u, &versions)?;
            versions.reverse();
            Some(serde_json::json!({
                "id": u.id(),
                "label": u.label(),
                "state": u.is_state(),
                "conflict": st.conflict,
                "against": st.against,
                "current": st.current,
                "folder": store.unit_dir(&u),
                "versions": versions,
            }))
        })
        .collect();
    serde_json::json!({ "units": units, "folder": store.game_dir(key) })
}

/// Settings › Saves (D180, D182): whether states are synced, the folder and the room it
/// takes, and the save profiles built in.
pub fn settings(lib: &crate::library::Library) -> serde_json::Value {
    fn size(d: &Path) -> u64 {
        std::fs::read_dir(d).into_iter().flatten().flatten().map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => size(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        }).sum()
    }
    let dir = lib.root().join(FOLDER);
    let profiles: Vec<serde_json::Value> = gamerepo_wire::saves::built_in().into_iter().map(|p| serde_json::json!({ "id": p.id, "name": p.name, "about": p.about })).collect();
    serde_json::json!({
        "states": lib.meta().pointer("/saves/states").and_then(|v| v.as_bool()).unwrap_or(true),
        "folder": dir,
        "size": size(&dir),
        "profiles": profiles,
    })
}

fn set_modified(path: &Path, secs: i64) {
    if secs <= 0 {
        return;
    }
    let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs as u64);
    if let Ok(f) = std::fs::File::options().write(true).open(path) {
        let _ = f.set_modified(t);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha1::Digest;

    fn blob(dir: &Path, text: &str, modified: i64) -> (SaveFile, PathBuf) {
        let sha1 = hex::encode(sha1::Sha1::digest(text.as_bytes()));
        let p = dir.join(&sha1);
        std::fs::write(&p, text).unwrap();
        (SaveFile { name: "{name}.srm".into(), sha1, size: text.len() as u64, modified }, p)
    }

    fn upload(store: &Store, dir: &Path, unit: &Unit, base: &str, text: &str, modified: i64, device: &str) -> Uploaded {
        let (f, p) = blob(dir, text, modified);
        let up = Upload { unit: unit.clone(), base: base.into(), files: vec![f], emulator: "RetroArch 1.19".into() };
        store.add(&up, device, device, &|_| Some(p.clone())).unwrap()
    }

    #[test]
    fn versions_follow_each_device_and_a_conflict_keeps_both() {
        let t = crate::library::tests::temp_dir("saves-versions");
        let blobs = t.join("blobs");
        std::fs::create_dir_all(&blobs).unwrap();
        let store = Store::of(&t.join("lib"));
        let unit = Unit::save("snes/Super Mario World (USA)", "retroarch");
        let a1 = upload(&store, &blobs, &unit, "", "one", 100, "deck");
        assert!(!a1.state.conflict && a1.state.current.id == a1.version.id);
        let srm = t.join("lib/Saves/snes/Super Mario World (USA)/Save (retroarch)/Super Mario World (USA).srm");
        assert_eq!(std::fs::read_to_string(&srm).unwrap(), "one", "the newest is an ordinary file");
        // the same again makes nothing
        let again = upload(&store, &blobs, &unit, &a1.version.id, "one", 100, "deck");
        assert_eq!(again.version.id, a1.version.id);
        // the phone played on from the Deck's
        let b = upload(&store, &blobs, &unit, &a1.version.id, "two", 200, "phone");
        assert!(!b.state.conflict && b.state.current.id == b.version.id);
        assert_eq!(std::fs::read_to_string(&srm).unwrap(), "two");
        // the Deck played on from its own first, offline, earlier than the phone: kept, behind
        let c = upload(&store, &blobs, &unit, &a1.version.id, "three", 150, "deck");
        assert!(c.version.behind && c.state.conflict && c.state.current.id == b.version.id);
        assert_eq!(c.state.against, vec![c.version.id.clone()]);
        assert_eq!(store.versions(&unit).len(), 3);
        // choosing the Deck's ends it, and it's the newest everywhere
        let d = store.choose(&unit, &c.version.id, "game-repo", "Game Repo").unwrap();
        assert!(!d.state.conflict && d.state.current.chosen == c.version.id);
        assert_eq!(std::fs::read_to_string(&srm).unwrap(), "three");
        assert_eq!(store.list(&[unit.key.clone()]).len(), 1);
        assert_eq!(std::fs::read_to_string(store.file(&unit, &a1.version.id, "{name}.srm").unwrap()).unwrap(), "one", "every version stays");
    }

    #[test]
    fn a_later_conflict_is_the_newest_and_states_keep_ten() {
        let t = crate::library::tests::temp_dir("saves-conflict");
        std::fs::create_dir_all(&t).unwrap();
        let store = Store::of(&t.join("lib"));
        let unit = Unit::save("gba/Game (Europe)", "retroarch");
        let a = upload(&store, &t, &unit, "", "a", 100, "one");
        let b = upload(&store, &t, &unit, &a.version.id, "b", 200, "one");
        let c = upload(&store, &t, &unit, &a.version.id, "c", 300, "two");
        assert!(!c.version.behind && c.state.conflict && c.state.current.id == c.version.id && c.state.against == vec![b.version.id.clone()]);
        let d = upload(&store, &t, &unit, &c.version.id, "d", 400, "two");
        assert!(!d.state.conflict, "a save played on from the newest ends it");
        let state = Unit::state("gba/Game (Europe)", "retroarch:mGBA", "1");
        let mut base = String::new();
        for n in 0..14 {
            base = upload(&store, &t, &state, &base, &format!("s{n}"), 100 + n, "one").version.id;
        }
        assert_eq!(store.versions(&state).len(), STATE_VERSIONS);
        assert_eq!(store.state(&state).unwrap().current.id, base);
        assert!(file_name("../x", "g").is_err() && file_name("{name}.srm", &game_name("snes/A: B")).is_ok() && file_name("C:x.srm", "g").is_err());
        assert_eq!(game_name("snes/A: B?"), "A_ B_");
    }
}
