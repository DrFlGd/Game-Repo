//! Maps and guides for Atlas on Windows and Linux (docs/PLAN.md, "Maps and Guides: Atlas
//! packs", D222): a game's packs (`.atlaspack`, files of a game's maps, guides and wiki
//! pages that Atlas reads) put straight into Atlas's packs folder, the device's place
//! `atlas`, as the Atlas pack contract says: each downloaded and checked like a game's
//! file, written there as `<name>.part` and renamed to `<name>` once it's whole. Game Repo
//! Go's record of what it delivered (each pack's `packId`, file, size, SHA-1 and games) is
//! kept in its own data folder (`packs.json`), never in the packs folder, so a newer file
//! of a pack replaces the old one (renamed over it, or the old one removed once the new one
//! is in when its name changed), and only the files Go delivered are ever replaced or
//! removed: any other file in the folder (Atlas's own, or your data beside a pack) is
//! Atlas's or yours. Removing a game leaves its packs (taking them off is an open point of
//! the contract). The packs a game's page unticks are kept here too (`left_out`). Not on
//! Android until the contract's trial T2 is settled.

use crate::Device;
use anyhow::{bail, Result};
use gamerepo_wire::File;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The device's place for packs: Atlas's packs folder.
pub const PLACE: &str = "atlas";
/// A pack's role in a plan.
pub const ROLE: &str = "pack";
/// A pack still being written ends in this (the contract's rule for every app).
pub const PART: &str = ".part";
/// Go's record, in its data folder.
const RECORD: &str = "packs.json";
/// Where a pack's ETag is kept while it comes, in Go's data folder (not Atlas's).
const COMING: &str = "packs-coming";

/// A pack Game Repo Go put in Atlas's packs folder.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Delivered {
    pub pack_id: String,
    /// Its file's name, and the packs folder it's in
    pub file: String,
    pub folder: PathBuf,
    pub size: u64,
    pub sha1: String,
    /// The games it came with (their catalogue keys)
    pub games: Vec<String>,
    /// When it came (seconds since 1970)
    pub at: u64,
}

impl Delivered {
    pub fn path(&self) -> PathBuf {
        self.folder.join(&self.file)
    }

    /// Its file is there as Go put it (its size the same): Go's to replace or remove.
    pub fn there(&self) -> bool {
        std::fs::metadata(self.path()).is_ok_and(|m| m.is_file() && m.len() == self.size)
    }
}

/// `packs.json` in Go's data folder.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Record {
    pub delivered: Vec<Delivered>,
    /// The packs unticked on a game's page: by the game's key, their `packId`s
    pub left_out: BTreeMap<String, Vec<String>>,
}

static WRITING: Mutex<()> = Mutex::new(());

pub fn load(data: &Path) -> Record {
    std::fs::read(data.join(RECORD)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save(data: &Path, r: &Record) -> Result<()> {
    crate::write_whole(&data.join(RECORD), &serde_json::to_vec_pretty(r)?)
}

/// The record changed and saved, one change at a time.
fn change<T>(data: &Path, f: impl FnOnce(&mut Record) -> Result<T>) -> Result<T> {
    let _w = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = load(data);
    let out = f(&mut r)?;
    save(data, &r)?;
    Ok(out)
}

/// Atlas's packs folder where it usually is (the contract): `%LOCALAPPDATA%\Atlas\Packs` on
/// Windows, `$XDG_DATA_HOME/Atlas/Packs` (`~/.local/share/Atlas/Packs`) on Linux until
/// Atlas's block A9 fixes it. `home` is the user's folder (the environment's folders are
/// taken only when they're that user's, so a stand-in home keeps to itself).
pub fn default_folder(home: &Path) -> PathBuf {
    let own = |var: &str| std::env::var_os(var).map(PathBuf::from).is_some_and(|p| p == home);
    let from = |var: &str| std::env::var_os(var).map(PathBuf::from).filter(|p| p.is_absolute());
    let base = if cfg!(windows) {
        from("LOCALAPPDATA").filter(|_| own("USERPROFILE")).unwrap_or_else(|| home.join("AppData").join("Local"))
    } else {
        from("XDG_DATA_HOME").filter(|_| own("HOME")).unwrap_or_else(|| home.join(".local").join("share"))
    };
    base.join("Atlas").join("Packs")
}

/// A plan's file that's a pack for Atlas.
pub fn is_pack(f: &File) -> bool {
    f.role == ROLE && f.place == PLACE
}

/// Where a pack is written while it comes: `<name>.part` in the packs folder.
pub fn part_of(d: &Device, f: &File) -> Result<PathBuf> {
    if f.path.contains(['/', '\\']) || !f.path.to_lowercase().ends_with(".atlaspack") {
        bail!("{} isn't a pack's name.", f.path);
    }
    let dest = d.at(PLACE, &f.path)?;
    Ok(dest.with_file_name(format!("{}{PART}", f.path)))
}

/// Where a pack's ETag is kept while it comes: Go's data folder, so nothing but the pack
/// itself is written in Atlas's folder.
pub fn tag_of(d: &Device, part: &Path) -> PathBuf {
    let name = part.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    match &d.data {
        Some(data) => data.join(COMING).join(format!("{name}.etag")),
        None => part.with_file_name(format!("{name}.etag")),
    }
}

/// Whether a plan's pack is here already, as Go delivered it (the same `packId` and SHA-1,
/// its file there).
pub fn there(d: &Device, f: &File) -> bool {
    let (Some(data), Some(folder), Some(sha1)) = (&d.data, d.places.get(PLACE), &f.sha1) else { return false };
    load(data).delivered.iter().any(|e| e.pack_id == f.pack && &e.sha1 == sha1 && &e.folder == folder && e.there())
}

/// The games a plan's pack came with.
fn games_of(f: &File) -> Vec<String> {
    let mut v = vec![f.key.clone()];
    v.extend(f.needed_by.iter().filter(|k| **k != f.key).cloned());
    v.retain(|k| !k.is_empty());
    v
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// A name in the folder that isn't taken: `name`, else `<stem> (2).atlaspack` and on.
fn free_name(folder: &Path, name: &str, mine: impl Fn(&str) -> bool) -> String {
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, "atlaspack"));
    let mut n = 1;
    loop {
        let candidate = if n == 1 { name.to_string() } else { format!("{stem} ({n}).{ext}") };
        let p = folder.join(&candidate);
        if mine(&candidate) || std::fs::symlink_metadata(&p).is_err() {
            return candidate;
        }
        n += 1;
    }
}

/// A pack fetched and checked, put in place: its `.part` renamed to its name, over the file
/// of the same pack Go delivered before; when that one had another name, it's removed once
/// the new one is in. A file of that name that isn't this pack's as Go delivered it (one
/// copied in by hand, another pack's) is never written over: the pack takes a name beside
/// it. The record notes it with its games.
pub fn put_in_place(d: &Device, f: &File, part: &Path, sha1: &str) -> Result<PathBuf> {
    let Some(data) = &d.data else { bail!("Game Repo Go has no data folder to note the packs it delivers in.") };
    let folder = d.places.get(PLACE).cloned().ok_or_else(|| anyhow::anyhow!("This device has no packs folder for Atlas."))?;
    change(data, |rec| {
        let old = rec.delivered.iter().position(|e| e.pack_id == f.pack);
        let old_entry = old.map(|i| rec.delivered[i].clone());
        // this pack's file as Go put it, in this folder: written over
        let mine = |name: &str| old_entry.as_ref().is_some_and(|e| e.folder == folder && e.file == name && e.there());
        let name = free_name(&folder, &f.path, mine);
        let dest = folder.join(&name);
        std::fs::create_dir_all(&folder)?;
        std::fs::rename(part, &dest)?;
        let _ = std::fs::remove_file(tag_of(d, part));
        let mut games = games_of(f);
        if let Some(e) = &old_entry {
            // the old file under another name (or in the folder used before), once the new one's in
            if (e.folder != folder || e.file != name) && e.there() {
                std::fs::remove_file(e.path())?;
            }
            for g in &e.games {
                if !games.contains(g) {
                    games.push(g.clone());
                }
            }
        }
        rec.delivered.retain(|e| e.pack_id != f.pack);
        rec.delivered.push(Delivered { pack_id: f.pack.clone(), file: name, folder: folder.clone(), size: f.size, sha1: sha1.to_string(), games, at: now() });
        Ok(dest)
    })
}

/// A plan's pack that was here already: the plan's games noted as its too.
pub fn note_there(d: &Device, f: &File) -> Result<()> {
    let Some(data) = &d.data else { return Ok(()) };
    change(data, |rec| {
        if let Some(e) = rec.delivered.iter_mut().find(|e| e.pack_id == f.pack) {
            for g in games_of(f) {
                if !e.games.contains(&g) {
                    e.games.push(g);
                }
            }
        }
        Ok(())
    })
}

/// The packs unticked on each game's page, for a plan (`PlanAsk::packs_left`).
pub fn left_out(data: &Path) -> Vec<(String, Vec<String>)> {
    load(data).left_out.into_iter().filter(|(_, v)| !v.is_empty()).collect()
}

/// A pack ticked or unticked on a game's page.
pub fn set_ticked(data: &Path, key: &str, pack: &str, on: bool) -> Result<()> {
    change(data, |rec| {
        let v = rec.left_out.entry(key.to_string()).or_default();
        v.retain(|p| p != pack);
        if !on {
            v.push(pack.to_string());
        }
        rec.left_out.retain(|_, v| !v.is_empty());
        Ok(())
    })
}

/// A pack unticked on the page of a game the device has: the game taken off what Go
/// delivered it for, and its file removed once no game here wants it, only when it's there
/// as Go put it. True when the file was removed.
pub fn take_off(data: &Path, key: &str, pack: &str) -> Result<bool> {
    change(data, |rec| {
        let Some(i) = rec.delivered.iter().position(|e| e.pack_id == pack) else { return Ok(false) };
        let e = &mut rec.delivered[i];
        e.games.retain(|g| g != key);
        if !e.games.is_empty() {
            return Ok(false);
        }
        let e = rec.delivered.remove(i);
        if e.there() {
            std::fs::remove_file(e.path())?;
            return Ok(true);
        }
        Ok(false)
    })
}

/// The pack Go delivered of a `packId`, with its file there.
pub fn here(data: &Path, pack: &str) -> Option<Delivered> {
    load(data).delivered.into_iter().find(|e| e.pack_id == pack && e.there())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(name: &str) -> (PathBuf, Device) {
        let home = std::env::temp_dir().join(format!("gamerepo-device-packs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("ES-DE/settings")).unwrap();
        std::fs::write(home.join("ES-DE/settings/es_settings.xml"), "<?xml version=\"1.0\"?>\n").unwrap();
        let mut d = Device::open(&home).unwrap();
        d.places.insert(PLACE.into(), home.join("Atlas").join("Packs"));
        d.data = Some(home.join("go"));
        (home, d)
    }

    fn pack(name: &str, id: &str, key: &str, bytes: &[u8]) -> File {
        File { place: PLACE.into(), path: name.into(), size: bytes.len() as u64, sha1: Some(format!("sha-{}", bytes.len())), key: key.into(), role: ROLE.into(), pack: id.into(), ..Default::default() }
    }

    /// The pack as it comes: written whole as `<name>.part`, then put in place.
    fn deliver(d: &Device, f: &File, bytes: &[u8]) -> PathBuf {
        let part = part_of(d, f).unwrap();
        assert_eq!(part.file_name().unwrap().to_string_lossy(), format!("{}.part", f.path));
        std::fs::create_dir_all(part.parent().unwrap()).unwrap();
        std::fs::write(&part, bytes).unwrap();
        let at = put_in_place(d, f, &part, f.sha1.as_deref().unwrap()).unwrap();
        assert!(!part.exists(), "the .part is renamed");
        at
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    }

    #[test]
    fn packs_go_into_atlass_folder_whole_and_a_newer_file_replaces_only_gos_own() {
        let (home, d) = device("deliver");
        let folder = d.places[PLACE].clone();
        let data = d.data.clone().unwrap();
        // Atlas's own file and your data beside a pack, there before Go
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("atlas-index.db"), b"Atlas's own").unwrap();
        std::fs::write(folder.join("Delta - Example Maps.atlasdata"), b"your notes").unwrap();

        let first = pack("Delta - Example Maps.atlaspack", "delta-example-maps", "gb/Delta (World)", b"pack one");
        assert!(!there(&d, &first));
        let at = deliver(&d, &first, b"pack one");
        assert_eq!(at, folder.join("Delta - Example Maps.atlaspack"));
        assert!(there(&d, &first), "the same pack again isn't fetched");
        assert!(!data.join(COMING).exists() || names(&data.join(COMING)).is_empty());

        // a newer file of the same pack under the same name: renamed over it
        let newer = pack("Delta - Example Maps.atlaspack", "delta-example-maps", "gb/Delta (World)", b"pack one, newer");
        assert!(!there(&d, &newer));
        deliver(&d, &newer, b"pack one, newer");
        assert_eq!(std::fs::read(&at).unwrap(), b"pack one, newer");
        // …and under another name: the old file removed once the new one is in
        let renamed = pack("Delta - Better Maps.atlaspack", "delta-example-maps", "gb/Delta (World)", b"pack one, renamed");
        deliver(&d, &renamed, b"pack one, renamed");
        assert_eq!(names(&folder), ["Delta - Better Maps.atlaspack", "Delta - Example Maps.atlasdata", "atlas-index.db"]);
        let rec = load(&data);
        assert_eq!(rec.delivered.len(), 1);
        assert_eq!((rec.delivered[0].file.as_str(), rec.delivered[0].games.clone()), ("Delta - Better Maps.atlaspack", vec!["gb/Delta (World)".to_string()]));

        // a file of a pack's name that Go didn't put there is never written over
        std::fs::write(folder.join("Beta - Example Maps.atlaspack"), b"copied in by hand").unwrap();
        let beta = pack("Beta - Example Maps.atlaspack", "beta-example-maps", "gb/Beta (Japan)", b"beta");
        let at = deliver(&d, &beta, b"beta");
        assert_eq!(at, folder.join("Beta - Example Maps (2).atlaspack"));
        assert_eq!(std::fs::read(folder.join("Beta - Example Maps.atlaspack")).unwrap(), b"copied in by hand");

        // unticked on a game it came with: removed once no game here wants it, Go's own only
        note_there(&d, &pack("Delta - Better Maps.atlaspack", "delta-example-maps", "gb/Delta (USA)", b"pack one, renamed")).unwrap();
        assert!(!take_off(&data, "gb/Delta (World)", "delta-example-maps").unwrap(), "Delta (USA) still wants it");
        assert!(folder.join("Delta - Better Maps.atlaspack").is_file());
        assert!(take_off(&data, "gb/Delta (USA)", "delta-example-maps").unwrap());
        assert!(!folder.join("Delta - Better Maps.atlaspack").exists());
        // a pack changed since by someone else (Atlas, or you) isn't Go's to remove
        std::fs::write(folder.join("Beta - Example Maps (2).atlaspack"), b"beta, changed by hand").unwrap();
        assert!(!take_off(&data, "gb/Beta (Japan)", "beta-example-maps").unwrap());
        assert_eq!(names(&folder), ["Beta - Example Maps (2).atlaspack", "Beta - Example Maps.atlaspack", "Delta - Example Maps.atlasdata", "atlas-index.db"]);
        assert_eq!(std::fs::read(folder.join("atlas-index.db")).unwrap(), b"Atlas's own");
        assert!(load(&data).delivered.is_empty());

        // ticks kept by game
        set_ticked(&data, "gb/Beta (Japan)", "beta-example-maps", false).unwrap();
        assert_eq!(left_out(&data), [("gb/Beta (Japan)".to_string(), vec!["beta-example-maps".to_string()])]);
        set_ticked(&data, "gb/Beta (Japan)", "beta-example-maps", true).unwrap();
        assert!(left_out(&data).is_empty());
        // only a pack's own name, in the folder itself
        assert!(part_of(&d, &pack("../x.atlaspack", "x", "k", b"x")).is_err());
        assert!(part_of(&d, &pack("sub/x.atlaspack", "x", "k", b"x")).is_err());
        assert!(part_of(&d, &pack("x.exe", "x", "k", b"x")).is_err());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn atlass_packs_folder_is_where_the_contract_says() {
        let home = Path::new("/home/someone-else");
        let f = default_folder(home);
        assert!(f.ends_with(Path::new("Atlas").join("Packs")), "{f:?}");
        if cfg!(windows) {
            assert_eq!(f, home.join("AppData").join("Local").join("Atlas").join("Packs"));
        } else {
            assert_eq!(f, home.join(".local").join("share").join("Atlas").join("Packs"));
        }
    }
}
