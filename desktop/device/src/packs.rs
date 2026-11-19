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
//!
//! A rename into place, or over the old file of a pack, that a drive refuses because a file
//! is held open is tried again for about seven seconds ([`patiently`], as the contract
//! asks); Atlas opens a pack only for each read. Atlas Importer's unfinished files
//! (`.atlas-importer.part`) are never Go's: it touches only the files in its record.
//!
//! Never an older pack over a newer one (D222 revised, the contract's new rule of
//! 2026-11-19): Atlas can change a map made in it and write its pack file again where it
//! lies, so the file on a device can be newer than the library's. Before a pack coming
//! replaces the file Go delivered of it ([`put_in_place`]: renamed over it, or the old one
//! removed when its name or folder changed), both descriptions are read with Atlas's own
//! crate and compared ([`Verdict`]): `pack.updated`, then `exportedAt`
//! (`gamerepo_wire::packs::newer`). The file here newer or as new: it's kept, the one
//! coming dropped, and the record follows the file here (its size and SHA-1, and the
//! library's SHA-1 it was kept over, so it isn't fetched again; [`Delivered::kept_over`]).
//! The one coming newer: it replaces the file here, even one changed since Go delivered it
//! (a pack Atlas wrote again is expected now, and this one is older). Either can't be read
//! as this pack: the rules from before, Go replaces only a file it delivered that's
//! unchanged. Packs are always sent and put in place whole, `mine/` and all: Go reads
//! nothing of a pack but its description. Carrying a newer file here back to the library
//! is open in the contract, and not done.

use crate::Device;
use anyhow::{bail, Result};
use gamerepo_wire::File;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

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
    /// The library's SHA-1 of this pack that wasn't put over the file here, as the file here
    /// was newer or as new by its description (empty: none): that file is fetched no more
    pub kept_over: String,
    /// The file here was newer than the library's (by `pack.updated`, then `exportedAt`) and
    /// was kept: "Newer on this device: kept"
    pub newer_here: bool,
}

impl Delivered {
    pub fn path(&self) -> PathBuf {
        self.folder.join(&self.file)
    }

    /// Its file is there as Go put it (its size the same): Go's to replace or remove.
    pub fn there(&self) -> bool {
        std::fs::metadata(self.path()).is_ok_and(|m| m.is_file() && m.len() == self.size)
    }

    /// It's in the packs folder for Atlas: there as Go put it, or a file Go kept over the
    /// library's (newer, or as new) there still, whatever Atlas has written in it since.
    pub fn here(&self) -> bool {
        self.there() || (!self.kept_over.is_empty() && self.path().is_file())
    }
}

/// Which of a pack coming and the file of it on the device is the newer, by their
/// descriptions read with Atlas's crate: `pack.updated`, then `exportedAt`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The one coming is newer: it replaces the file here
    Coming,
    /// The file here is newer: it's kept
    Here,
    /// Both are as new: the file here is kept
    Same,
    /// One of them can't be read as this pack (a newer format, not a pack, another
    /// `packId`, gone): the rules from before
    Unknown,
}

/// A pack file's `packId`, `pack.updated` and `exportedAt`, read by Atlas's crate.
fn dates(path: &Path) -> Option<(String, Option<String>, String)> {
    let h = atlas_pack::read_header(path).ok()?;
    Some((h.pack_id, h.pack.updated, h.exported_at))
}

/// The pack coming (its `.part`) compared with the file of it here, both read as the pack
/// `pack_id`.
pub fn compare(pack_id: &str, coming: &Path, here: &Path) -> Verdict {
    let (Some(c), Some(h)) = (dates(coming), dates(here)) else { return Verdict::Unknown };
    if c.0 != pack_id || h.0 != pack_id {
        return Verdict::Unknown;
    }
    match gamerepo_wire::packs::newer(c.1.as_deref(), Some(&c.2), h.1.as_deref(), Some(&h.2)) {
        std::cmp::Ordering::Greater => Verdict::Coming,
        std::cmp::Ordering::Less => Verdict::Here,
        std::cmp::Ordering::Equal => Verdict::Same,
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

/// The user's data folder Atlas's packs folder is in: `%LOCALAPPDATA%` on Windows,
/// `$XDG_DATA_HOME` (`~/.local/share`) on Linux. `home` is the user's folder (the
/// environment's folders are taken only when they're that user's, so a stand-in home keeps
/// to itself).
fn data_home(home: &Path) -> PathBuf {
    let own = |var: &str| std::env::var_os(var).map(PathBuf::from).is_some_and(|p| p == home);
    let from = |var: &str| std::env::var_os(var).map(PathBuf::from).filter(|p| p.is_absolute());
    if cfg!(windows) {
        from("LOCALAPPDATA").filter(|_| own("USERPROFILE")).unwrap_or_else(|| home.join("AppData").join("Local"))
    } else {
        from("XDG_DATA_HOME").filter(|_| own("HOME")).unwrap_or_else(|| home.join(".local").join("share"))
    }
}

/// Atlas's packs folder where it is (the contract's answer 5, as Atlas's block A1 built
/// it): `%LOCALAPPDATA%\Atlas\Packs` on Windows; `$XDG_DATA_HOME/atlas/packs`
/// (`~/.local/share/atlas/packs`), all lower case, on Linux.
pub fn default_folder(home: &Path) -> PathBuf {
    let base = data_home(home);
    if cfg!(windows) {
        base.join("Atlas").join("Packs")
    } else {
        base.join("atlas").join("packs")
    }
}

/// Where 0.5.72 guessed Atlas's packs folder was on Linux, `$XDG_DATA_HOME/Atlas/Packs`: a
/// folder Atlas doesn't look in. None on Windows, where the guess was right. A saved
/// setting of this folder is taken as the usual one, and the packs Go delivered there are
/// moved ([`move_from`]).
pub fn old_default_folder(home: &Path) -> Option<PathBuf> {
    (!cfg!(windows)).then(|| data_home(home).join("Atlas").join("Packs"))
}

/// How long a rename a drive refuses while a file is held open is tried again: about seven
/// seconds, as the contract asks of every app that writes a pack.
pub const PATIENCE: Duration = Duration::from_secs(7);

/// Whether an error is one a drive gives while a file is held open (Windows' sharing and
/// lock violations, access denied; a busy file elsewhere): worth trying again. A file
/// that's gone, a name that's taken or another drive isn't.
pub fn held(e: &io::Error) -> bool {
    matches!(e.kind(), io::ErrorKind::PermissionDenied | io::ErrorKind::ResourceBusy) || (cfg!(windows) && matches!(e.raw_os_error(), Some(32 | 33)))
}

/// `f` (a rename, a removal) tried again while it fails as a held file does ([`held`]), for
/// about [`PATIENCE`], with short sleeps between; any other result at once.
pub fn patiently<T>(mut f: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    patiently_for(PATIENCE, &mut f)
}

fn patiently_for<T>(patience: Duration, f: &mut dyn FnMut() -> io::Result<T>) -> io::Result<T> {
    let start = Instant::now();
    let mut wait = Duration::from_millis(50);
    loop {
        match f() {
            Err(e) if held(&e) && start.elapsed() + wait <= patience => {
                std::thread::sleep(wait);
                wait = (wait * 2).min(Duration::from_secs(1));
            }
            r => return r,
        }
    }
}

/// `std::fs::rename`, tried again for a while when the drive refuses it as a held file's.
pub fn rename_patiently(from: &Path, to: &Path) -> io::Result<()> {
    patiently(|| std::fs::rename(from, to))
}

/// The packs Go delivered into `old` (0.5.72's wrong guess at Atlas's packs folder on
/// Linux), moved into `new`, the folder Go uses now: renamed, or, when that can't be done
/// (another drive), copied as `<name>.part` beside the new file and renamed, then the old
/// removed. Only the files in Go's record that are there as it put them; a name taken in
/// `new` by a file that isn't Go's gives `<name> (2).atlaspack`. The old folder is left
/// as it is, emptied or not (it may be yours). How many were moved.
pub fn move_from(data: &Path, old: &Path, new: &Path) -> Result<usize> {
    // the same folder, or one a case-insensitive drive takes as the same (`Atlas/Packs` and
    // `atlas/packs`): nothing to move
    let same = |a: &Path, b: &Path| matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y);
    if old == new || same(old, new) {
        return Ok(0);
    }
    change(data, |rec| {
        let mut moved = 0;
        for i in 0..rec.delivered.len() {
            let e = rec.delivered[i].clone();
            if e.folder != old || !e.there() {
                continue;
            }
            std::fs::create_dir_all(new)?;
            let name = free_name(new, &e.file, |_| false);
            let dest = new.join(&name);
            if let Err(err) = rename_patiently(&e.path(), &dest) {
                if held(&err) {
                    continue;
                }
                let part = new.join(format!("{name}{PART}"));
                let whole = |n: u64| if n == e.size { Ok(()) } else { Err(io::Error::other("the copy isn't whole")) };
                let copied = std::fs::copy(e.path(), &part).and_then(whole).and_then(|_| rename_patiently(&part, &dest));
                if copied.is_err() {
                    let _ = std::fs::remove_file(&part);
                    continue;
                }
                // the copy is in: the old file goes (if it can't now, it's left beside, and
                // the record has the new one)
                let _ = patiently(|| std::fs::remove_file(e.path()));
            }
            rec.delivered[i].folder = new.to_path_buf();
            rec.delivered[i].file = name;
            moved += 1;
        }
        Ok(moved)
    })
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
/// its file there), or the library's file is one Go kept the file here over (it was older),
/// so it's never fetched again and again.
pub fn there(d: &Device, f: &File) -> bool {
    let (Some(data), Some(folder), Some(sha1)) = (&d.data, d.places.get(PLACE), &f.sha1) else { return false };
    load(data).delivered.iter().any(|e| e.pack_id == f.pack && &e.folder == folder && ((&e.sha1 == sha1 && e.there()) || (&e.kept_over == sha1 && e.here())))
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
/// the new one is in. Never an older pack over a newer one: the two descriptions are
/// compared first ([`compare`]), and when the file here is newer or as new, it's kept, the
/// `.part` removed, and the record follows the file here. A file of that name that isn't
/// this pack's as Go delivered it (one copied in by hand, another pack's) is never written
/// over: the pack takes a name beside it. The record notes it with its games. The file it's
/// in now.
pub fn put_in_place(d: &Device, f: &File, part: &Path, sha1: &str) -> Result<PathBuf> {
    let Some(data) = &d.data else { bail!("Game Repo Go has no data folder to note the packs it delivers in.") };
    let folder = d.places.get(PLACE).cloned().ok_or_else(|| anyhow::anyhow!("This device has no packs folder for Atlas."))?;
    change(data, |rec| {
        let old = rec.delivered.iter().position(|e| e.pack_id == f.pack);
        let old_entry = old.map(|i| rec.delivered[i].clone());
        // the file Go delivered of this pack, wherever it is now and whatever was written in
        // it since, compared with the one coming
        let verdict = match &old_entry {
            Some(e) if e.path().is_file() => compare(&f.pack, part, &e.path()),
            _ => Verdict::Unknown,
        };
        if let (Some(i), Some(e), Verdict::Here | Verdict::Same) = (old, &old_entry, verdict) {
            if e.folder == folder {
                // the file here is newer (Atlas wrote it again), or as new: kept, the one
                // coming dropped, and the record follows the file here so it isn't taken as
                // changed by hand nor fetched again
                let _ = std::fs::remove_file(part);
                let _ = std::fs::remove_file(tag_of(d, part));
                let here = e.path();
                let k = &mut rec.delivered[i];
                k.size = std::fs::metadata(&here).map(|m| m.len()).unwrap_or(k.size);
                k.sha1 = crate::sha1_file(&here).unwrap_or_else(|_| k.sha1.clone());
                k.kept_over = sha1.to_string();
                k.newer_here = verdict == Verdict::Here;
                for g in games_of(f) {
                    if !k.games.contains(&g) {
                        k.games.push(g);
                    }
                }
                return Ok(here);
            }
        }
        // this pack's file as Go put it, in this folder: written over; or, older than the one
        // coming by its description, even if it was written again since
        let mine = |name: &str| old_entry.as_ref().is_some_and(|e| e.folder == folder && e.file == name && (e.there() || verdict == Verdict::Coming));
        let name = free_name(&folder, &f.path, mine);
        let dest = folder.join(&name);
        std::fs::create_dir_all(&folder)?;
        // over the old file Atlas may be reading a moment: tried again for a while
        rename_patiently(part, &dest)?;
        let _ = std::fs::remove_file(tag_of(d, part));
        let mut games = games_of(f);
        if let Some(e) = &old_entry {
            // the old file under another name (or in the folder used before), once the new one's in
            // (one that still can't be removed is left beside it: the new one is in and noted):
            // as Go put it and not newer than the one coming, or older than it by its
            // description; a newer or changed one in the folder used before is left there
            let older = verdict == Verdict::Coming || (e.there() && verdict != Verdict::Here);
            if (e.folder != folder || e.file != name) && older && e.path().is_file() {
                let _ = patiently(|| std::fs::remove_file(e.path()));
            }
            for g in &e.games {
                if !games.contains(g) {
                    games.push(g.clone());
                }
            }
        }
        rec.delivered.retain(|e| e.pack_id != f.pack);
        rec.delivered.push(Delivered { pack_id: f.pack.clone(), file: name, folder: folder.clone(), size: f.size, sha1: sha1.to_string(), games, at: now(), ..Default::default() });
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
/// as Go put it. A file Go kept over the library's (newer or as new here: Atlas wrote it,
/// maybe with the maker's own things) is never removed. True when the file was removed.
pub fn take_off(data: &Path, key: &str, pack: &str) -> Result<bool> {
    change(data, |rec| {
        let Some(i) = rec.delivered.iter().position(|e| e.pack_id == pack) else { return Ok(false) };
        let e = &mut rec.delivered[i];
        e.games.retain(|g| g != key);
        if !e.games.is_empty() || !e.kept_over.is_empty() {
            // a file kept over the library's stays, and stays noted, so ticked again it isn't
            // fetched (nor put beside it)
            return Ok(false);
        }
        let e = rec.delivered.remove(i);
        // unchanged: its size, and its SHA-1 (Atlas may have written it again at the same size)
        if e.there() && e.kept_over.is_empty() && crate::sha1_file(&e.path()).is_ok_and(|h| h.eq_ignore_ascii_case(&e.sha1)) {
            std::fs::remove_file(e.path())?;
            return Ok(true);
        }
        Ok(false)
    })
}

/// The pack Go delivered of a `packId`, with its file there (or kept over the library's).
pub fn here(data: &Path, pack: &str) -> Option<Delivered> {
    load(data).delivered.into_iter().find(|e| e.pack_id == pack && e.here())
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
        use sha1::Digest;
        let sha1 = hex::encode(sha1::Sha1::digest(bytes));
        File { place: PLACE.into(), path: name.into(), size: bytes.len() as u64, sha1: Some(sha1), key: key.into(), role: ROLE.into(), pack: id.into(), ..Default::default() }
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
        // a pack changed since by someone else (Atlas, or you) isn't Go's to remove, even one
        // written again at the same size (its SHA-1 tells)
        std::fs::write(folder.join("Beta - Example Maps (2).atlaspack"), b"BETA").unwrap();
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

    /// A made-up pack's bytes as Atlas writes one (docs/pack-file-format.md): its description
    /// first, with every field a pack always has, then a tile, then `more` (the maker's own
    /// things under `mine/`).
    fn atlas_pack(id: &str, updated: &str, exported_at: &str, more: &[(&str, &[u8])]) -> Vec<u8> {
        use std::io::Write;
        use zip::write::SimpleFileOptions;
        let description = serde_json::json!({
            "format": "atlas-pack-file", "version": 5, "packId": id, "title": "Island", "packVersion": "1.0.0", "detail": "full",
            "game": { "title": "Island" },
            "exportedAt": exported_at, "exportedBy": { "name": "Atlas", "version": "0.23.0", "platform": "windows" },
            "pack": { "id": id, "title": "Island", "packVersion": "1.0.0", "schemaVersion": 2, "attribution": [], "updated": updated, "writtenBy": { "name": "Atlas", "version": "0.23.0" } },
            "source": { "kind": "unknown" },
            "contents": { "maps": 1, "locations": 3 },
            "images": { "detail": "full", "files": 1, "bytes": 64, "layers": [] },
            "files": 1 + more.len(), "bytes": 64,
            "mine": { "locations": 3, "notes": 1 },
        });
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let deflated = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        z.start_file("atlaspack.json", deflated).unwrap();
        z.write_all(description.to_string().as_bytes()).unwrap();
        z.start_file("images/tile-0-0.png", SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored)).unwrap();
        z.write_all(&[7u8; 64]).unwrap();
        for (n, b) in more {
            z.start_file(*n, deflated).unwrap();
            z.write_all(b).unwrap();
        }
        z.finish().unwrap().into_inner()
    }

    /// The maker's own things, as a map made in Atlas carries them.
    const MINE: &[(&str, &[u8])] = &[("mine/my-locations.json", br#"{"locations":[{"id":"x1"}]}"#), ("mine/my-notes.json", br#"{"notes":[{"text":"by the well"}]}"#)];

    /// The contract's new rule (D222 revised): Atlas wrote a map made in it again on the
    /// device, so the file there is newer than the library's; the library's older one comes
    /// and isn't put over it: the file here is kept whole, the `.part` dropped, the record
    /// follows the file here and isn't fetched again; a newer one from the library later
    /// replaces it, though Atlas changed it since Go delivered it.
    #[test]
    fn an_older_pack_is_never_put_over_a_newer_one_atlas_wrote_on_the_device() {
        let (home, d) = device("older");
        let folder = d.places[PLACE].clone();
        let data = d.data.clone().unwrap();
        let key = "pc/island";
        // delivered whole, the maker's own things and all
        let v1 = atlas_pack("island", "2026-11-18T08:00:00Z", "2026-11-18T08:05:00Z", MINE);
        let f1 = pack("Island.atlaspack", "island", key, &v1);
        let at = deliver(&d, &f1, &v1);
        assert_eq!(std::fs::read(&at).unwrap(), v1, "byte for byte, mine/ and all");
        assert!(there(&d, &f1));
        // Atlas changes the map here and writes its pack again, newer (more of your own things)
        let v2 = atlas_pack("island", "2026-11-19T10:00:00Z", "2026-11-19T10:00:00Z", &[MINE[0], MINE[1], ("mine/my-pictures/well.jpg", &[0xFF, 0xD8, 0xFF, 0xD9])]);
        std::fs::write(&at, &v2).unwrap();
        assert!(!there(&d, &f1), "changed here: the library's is fetched to compare");
        // the library's comes, written again by the Importer later (a later exportedAt) but
        // of an older change (pack.updated first): it isn't put over the newer file
        let lib1 = atlas_pack("island", "2026-11-18T08:00:00Z", "2026-11-19T12:00:00Z", MINE);
        let g1 = pack("Island.atlaspack", "island", key, &lib1);
        assert_eq!(compare("island", &at, &at), Verdict::Same);
        let kept = deliver(&d, &g1, &lib1);
        assert_eq!(kept, at);
        assert_eq!(std::fs::read(&at).unwrap(), v2, "the newer file here is kept");
        assert_eq!(names(&folder), ["Island.atlaspack"], "the .part dropped, nothing beside it");
        let e = here(&data, "island").unwrap();
        assert_eq!((e.size, e.sha1.clone(), e.kept_over.clone(), e.newer_here), (v2.len() as u64, crate::sha1_file(&at).unwrap(), g1.sha1.clone().unwrap(), true));
        assert!(e.there(), "as it is now: not taken as changed by hand");
        // the next sync: not fetched again, even once Atlas has written it again since
        assert!(there(&d, &g1));
        let v3 = atlas_pack("island", "2026-11-19T11:00:00Z", "2026-11-19T11:00:00Z", &[MINE[0]]);
        std::fs::write(&at, &v3).unwrap();
        assert!(there(&d, &g1), "no download loop");
        // unticked: a file kept over the library's is never taken out, and stays noted
        assert!(!take_off(&data, key, "island").unwrap());
        assert_eq!(std::fs::read(&at).unwrap(), v3);
        assert!(there(&d, &g1), "ticked again, it isn't fetched");
        note_there(&d, &g1).unwrap();
        assert_eq!(here(&data, "island").unwrap().games, [key]);
        // a newer one in the library replaces the file here, though Atlas wrote it since Go
        // delivered it: it's older by its description
        let lib2 = atlas_pack("island", "2026-11-20T09:00:00Z", "2026-11-20T09:00:00Z", MINE);
        let g2 = pack("Island.atlaspack", "island", key, &lib2);
        assert!(!there(&d, &g2));
        deliver(&d, &g2, &lib2);
        assert_eq!(std::fs::read(&at).unwrap(), lib2);
        assert_eq!(names(&folder), ["Island.atlaspack"]);
        let e = here(&data, "island").unwrap();
        assert_eq!((e.size, e.sha1.clone(), e.kept_over.as_str(), e.newer_here, e.games.clone()), (lib2.len() as u64, g2.sha1.clone().unwrap(), "", false, vec![key.to_string()]));
        assert!(there(&d, &g2) && !there(&d, &g1));
        let _ = std::fs::remove_dir_all(&home);
    }

    /// The same `pack.updated` is decided by `exportedAt`, whatever the files are named; both
    /// as new keeps the file here; and a file that can't be read as the pack, here or coming,
    /// follows the rules from before (Go replaces only its own file, unchanged).
    #[test]
    fn the_same_pack_updated_is_decided_by_exported_at_and_a_file_that_cant_be_read_by_the_rules_before() {
        let (home, d) = device("tie");
        let folder = d.places[PLACE].clone();
        let data = d.data.clone().unwrap();
        let key = "pc/island";
        let t = "2026-11-18T08:00:00Z";
        let a = atlas_pack("island", t, "2026-11-18T08:05:00Z", &[]);
        let at = deliver(&d, &pack("Island - Example Maps.atlaspack", "island", key, &a), &a);
        // written again here later, the same change
        let b = atlas_pack("island", t, "2026-11-19T08:05:00Z", &[]);
        std::fs::write(&at, &b).unwrap();
        // the library's, renamed and exported before it: the file here is kept, under its name
        let c = atlas_pack("island", t, "2026-11-18T20:00:00+02:00", MINE);
        let kept = deliver(&d, &pack("Island - Better Maps.atlaspack", "island", key, &c), &c);
        assert_eq!((kept, names(&folder)), (at.clone(), vec!["Island - Example Maps.atlaspack".to_string()]));
        assert_eq!(std::fs::read(&at).unwrap(), b);
        assert!(here(&data, "island").unwrap().newer_here);
        // exported after it: it replaces the file here, and the old name goes
        let e4 = atlas_pack("island", t, "2026-11-19T09:00:00Z", MINE);
        let new_at = deliver(&d, &pack("Island - Better Maps.atlaspack", "island", key, &e4), &e4);
        assert_eq!(names(&folder), ["Island - Better Maps.atlaspack"]);
        assert_eq!(std::fs::read(&new_at).unwrap(), e4, "whole, mine/ and all");
        // both as new (the same dates, other bytes): the file here is kept, not said newer
        let same = atlas_pack("island", t, "2026-11-19T09:00:00Z", &[MINE[0]]);
        let g = pack("Island - Better Maps.atlaspack", "island", key, &same);
        deliver(&d, &g, &same);
        assert_eq!(std::fs::read(&new_at).unwrap(), e4);
        let e = here(&data, "island").unwrap();
        assert_eq!((e.newer_here, e.kept_over.clone()), (false, g.sha1.clone().unwrap()));
        assert!(there(&d, &g));

        // a file here that can't be read as a pack (changed by hand): not Go's to replace,
        // the one coming goes beside it, as before
        let p1 = atlas_pack("gamma", t, t, &[]);
        let gamma = deliver(&d, &pack("Gamma.atlaspack", "gamma", "pc/gamma", &p1), &p1);
        std::fs::write(&gamma, b"not a pack any more").unwrap();
        let p2 = atlas_pack("gamma", "2026-11-19T08:00:00Z", t, &[]);
        let beside = deliver(&d, &pack("Gamma.atlaspack", "gamma", "pc/gamma", &p2), &p2);
        assert_eq!(beside, folder.join("Gamma (2).atlaspack"));
        assert_eq!(std::fs::read(&gamma).unwrap(), b"not a pack any more");
        // one coming that Game Repo can't read (a newer format, say) over Go's own file,
        // unchanged: replaced, as before
        let newer_format = b"PK a pack in a format this Game Repo Go doesn't know".to_vec();
        deliver(&d, &pack("Gamma.atlaspack", "gamma", "pc/gamma", &newer_format), &newer_format);
        assert_eq!(std::fs::read(&beside).unwrap(), newer_format);
        // the file Go delivered in the folder used before, newer there: the one coming goes
        // into the folder used now, and the newer file there is left as it is
        let mut d = d;
        let old_folder = folder.clone();
        let new_folder = home.join("Other Packs");
        d.places.insert(PLACE.into(), new_folder.clone());
        let older = atlas_pack("island", "2026-11-17T08:00:00Z", t, &[]);
        deliver(&d, &pack("Island - Better Maps.atlaspack", "island", key, &older), &older);
        assert_eq!(std::fs::read(old_folder.join("Island - Better Maps.atlaspack")).unwrap(), e4);
        assert_eq!(std::fs::read(new_folder.join("Island - Better Maps.atlaspack")).unwrap(), older);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn atlass_packs_folder_is_where_the_contract_says() {
        let home = Path::new("/home/someone-else");
        let f = default_folder(home);
        if cfg!(windows) {
            assert_eq!(f, home.join("AppData").join("Local").join("Atlas").join("Packs"));
            assert_eq!(old_default_folder(home), None, "0.5.72's guess was right on Windows");
        } else {
            // all lower case, as Atlas's block A1 built it (the contract's answer 5)
            assert_eq!(f, home.join(".local").join("share").join("atlas").join("packs"));
            assert_eq!(old_default_folder(home), Some(home.join(".local").join("share").join("Atlas").join("Packs")));
        }
    }

    #[test]
    fn the_packs_go_put_in_the_old_folder_are_moved_into_atlass_own() {
        let (home, d) = device("move");
        let data = d.data.clone().unwrap();
        // (two folders apart, as on Linux; on a drive that ignores case, `Atlas/Packs` and
        // `atlas/packs` are one folder, which `move_from` leaves alone)
        let (old, new) = (home.join("old").join("Atlas").join("Packs"), home.join("share").join("atlas").join("packs"));
        let mut d = d;
        d.places.insert(PLACE.into(), old.clone());
        // 0.5.72 delivered two packs into the old folder; one was changed by hand since
        let delta = pack("Delta - Example Maps.atlaspack", "delta-example-maps", "gb/Delta (World)", b"delta");
        let beta = pack("Beta - Example Maps.atlaspack", "beta-example-maps", "gb/Beta (Japan)", b"beta");
        deliver(&d, &delta, b"delta");
        deliver(&d, &beta, b"beta");
        std::fs::write(old.join("Beta - Example Maps.atlaspack"), b"beta, changed by hand").unwrap();
        // and files that aren't Go's: yours, and an unfinished one of Atlas Importer's
        std::fs::write(old.join("Mine.atlaspack"), b"yours").unwrap();
        std::fs::write(old.join("Zeta - Example Maps.atlaspack.atlas-importer.part"), b"half").unwrap();
        // a file of Delta's name already in the new folder that Go didn't put there
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(new.join("Delta - Example Maps.atlaspack"), b"copied in by hand").unwrap();

        assert_eq!(move_from(&data, &old, &new).unwrap(), 1);
        assert_eq!(names(&new), ["Delta - Example Maps (2).atlaspack", "Delta - Example Maps.atlaspack"]);
        assert_eq!(std::fs::read(new.join("Delta - Example Maps (2).atlaspack")).unwrap(), b"delta");
        assert_eq!(std::fs::read(new.join("Delta - Example Maps.atlaspack")).unwrap(), b"copied in by hand");
        // only Go's own, as it put them: the rest stays where it is, the folder too
        assert_eq!(names(&old), ["Beta - Example Maps.atlaspack", "Mine.atlaspack", "Zeta - Example Maps.atlaspack.atlas-importer.part"]);
        assert_eq!(std::fs::read(old.join("Zeta - Example Maps.atlaspack.atlas-importer.part")).unwrap(), b"half");
        let rec = load(&data);
        let moved = rec.delivered.iter().find(|e| e.pack_id == "delta-example-maps").unwrap();
        assert_eq!((moved.folder.clone(), moved.file.as_str(), moved.games.clone()), (new.clone(), "Delta - Example Maps (2).atlaspack", vec!["gb/Delta (World)".to_string()]));
        assert!(moved.there());
        assert_eq!(rec.delivered.iter().find(|e| e.pack_id == "beta-example-maps").unwrap().folder, old);
        // the next run has nothing more to move, and the same folder twice is nothing
        assert_eq!(move_from(&data, &old, &new).unwrap(), 0);
        assert_eq!(move_from(&data, &new, &new).unwrap(), 0);
        // a newer file of the moved pack, delivered into the new folder, goes over it there
        d.places.insert(PLACE.into(), new.clone());
        let newer = pack("Delta - Example Maps.atlaspack", "delta-example-maps", "gb/Delta (World)", b"delta, newer");
        deliver(&d, &newer, b"delta, newer");
        assert_eq!(std::fs::read(new.join("Delta - Example Maps (2).atlaspack")).unwrap(), b"delta, newer");
        assert_eq!(names(&new), ["Delta - Example Maps (2).atlaspack", "Delta - Example Maps.atlaspack"]);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_rename_refused_while_a_file_is_held_is_tried_again_for_a_while() {
        let busy = || io::Error::new(io::ErrorKind::PermissionDenied, "held open");
        // refused twice, then done: two short sleeps
        let mut tries = 0;
        let started = Instant::now();
        let r = patiently(|| {
            tries += 1;
            if tries < 3 { Err(busy()) } else { Ok(tries) }
        });
        assert_eq!(r.unwrap(), 3);
        assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
        // a file that's gone, or a name that's taken, isn't tried again
        for kind in [io::ErrorKind::NotFound, io::ErrorKind::AlreadyExists] {
            let mut tries = 0;
            let r: io::Result<()> = patiently(|| {
                tries += 1;
                Err(io::Error::new(kind, "no"))
            });
            assert_eq!((r.unwrap_err().kind(), tries), (kind, 1));
        }
        // held all along: given up once the time's up
        let mut tries = 0;
        let started = Instant::now();
        let r: io::Result<()> = patiently_for(Duration::from_millis(300), &mut || {
            tries += 1;
            Err(busy())
        });
        assert!(r.is_err() && tries >= 3 && started.elapsed() < Duration::from_secs(1), "{tries} {:?}", started.elapsed());
        assert!(PATIENCE >= Duration::from_secs(5) && PATIENCE <= Duration::from_secs(10));
    }
}
