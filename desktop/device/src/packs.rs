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
//! nothing of a pack but its description. A newer file here stays here: nothing travels
//! back to the library (Jerred's decision of 2026-11-19, for now).
//!
//! A copy Go didn't deliver (the contract's saving, "Maps and Guides notes, part 6"): just
//! before Go would fetch a pack it doesn't have here, [`theirs_instead`] reads the
//! descriptions of the other `.atlaspack` files in the packs folder, as far as Atlas reads
//! it, and when one of the same `packId` is as new as the library's or newer, Go's isn't
//! fetched (Atlas would show that copy, not Go's); the record notes it ([`Theirs`]) so it
//! isn't read again while it's unchanged, and the game's page says so. That copy is never
//! replaced, moved or removed.
//!
//! Packs on their own (D225, "Maps and Guides notes, part 8"): Go's Maps and guides puts a
//! pack of the library's in the packs folder without any game, by its `packId`. The record
//! keeps those packs in [`Record::alone`], beside each game's ticks: such a pack is one file
//! however many games have it too (a pack is keyed by its `packId` everywhere here), it
//! stays when a game is removed or unticks it ([`take_off`]), and taking it off on its own
//! ([`remove_alone`]) keeps the file while a game this device has still has it ticked. The
//! same rules as a game's pack: only Go's own file is ever replaced or removed, and never
//! by an older one; [`library_newer`] says when the library's copy is newer than the file
//! here, so Go fetches it again after a read of the library.

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
    /// Copies of packs in the packs folder that Go didn't deliver, as new as the library's or
    /// newer, for which Go's own wasn't fetched
    pub theirs: Vec<Theirs>,
    /// The packs put in the packs folder on their own, from Maps and guides (D225), by
    /// their `packId`s: kept there whatever the games here do
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub alone: Vec<String>,
    /// The guidebooks unticked in a game's pack of guidebooks (D238): by the game's key, their
    /// ids (their files' names in the library)
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub guidebooks_left: BTreeMap<String, Vec<String>>,
}

impl Record {
    /// Whether a pack was put in the packs folder on its own (D225).
    pub fn is_alone(&self, pack: &str) -> bool {
        self.alone.iter().any(|p| p == pack)
    }
}

/// A copy of a pack in Atlas's packs folder that Go didn't deliver (copied in by hand, or put
/// there by Atlas or another app), found the same version as the library's or newer just
/// before Go would have fetched its own ([`theirs_instead`]): Atlas shows the newer of two
/// files of a pack, so Go's isn't fetched while this copy is there unchanged. The copy is
/// never replaced, moved or removed.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Theirs {
    pub pack_id: String,
    /// The library's pack that wasn't fetched for it, by its `pack.updated` and `exportedAt`
    /// as the plan gave them
    pub updated: String,
    pub exported_at: String,
    /// The copy, with its size and when it was last changed (nanoseconds since 1970) as they
    /// were when it was read: it's read again only once either changes
    pub path: PathBuf,
    pub size: u64,
    pub modified: u64,
    /// The copy is newer than the library's (else as new)
    pub newer: bool,
}

impl Theirs {
    /// Its copy is there as it was when it was read.
    pub fn unchanged(&self) -> bool {
        stamp(&self.path) == Some((self.size, self.modified))
    }
}

/// A file's size and when it was last changed (nanoseconds since 1970).
fn stamp(path: &Path) -> Option<(u64, u64)> {
    let m = std::fs::metadata(path).ok().filter(|m| m.is_file())?;
    let t = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos();
    Some((m.len(), u64::try_from(t).unwrap_or(u64::MAX)))
}

/// How far down Atlas reads its packs folder for pack files (the contract): 4 folders down.
pub const LOOK_DEPTH: usize = 4;
/// How many pack files Atlas reads in its packs folder at most (the contract).
pub const LOOK_MOST: usize = 1000;

/// Whether a file's name is a pack's as Atlas reads it: `<name>.atlaspack`, never one still
/// being written (`.part`, Atlas Importer's `.atlas-importer.part`) nor a hidden one.
pub fn is_pack_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(".atlaspack") && !lower.starts_with('.')
}

/// The pack files in a packs folder, as far as Atlas reads it ([`LOOK_DEPTH`] folders down,
/// [`LOOK_MOST`] files), shallower folders first and each by name. Links are not followed.
pub fn pack_files(folder: &Path) -> Vec<PathBuf> {
    let mut out = vec![];
    let mut dirs = std::collections::VecDeque::from([(folder.to_path_buf(), 0usize)]);
    while let Some((dir, depth)) = dirs.pop_front() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut entries: Vec<(String, std::fs::FileType)> = rd.flatten().filter_map(|e| Some((e.file_name().to_string_lossy().into_owned(), e.file_type().ok()?))).collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        for (name, kind) in entries {
            if kind.is_dir() {
                if depth < LOOK_DEPTH {
                    dirs.push_back((dir.join(&name), depth + 1));
                }
            } else if kind.is_file() && is_pack_name(&name) {
                out.push(dir.join(&name));
                if out.len() >= LOOK_MOST {
                    return out;
                }
            }
        }
    }
    out
}

/// Just before Go would fetch a pack it doesn't have here (the contract's saving, "Maps and
/// Guides notes, part 6"): the packs folder read for a copy of the same `packId` that Go
/// didn't deliver, and when one is the same version as the library's or newer (the plan's
/// `pack.updated`, then `exportedAt`, by `gamerepo_wire::packs::newer`), Go's isn't fetched,
/// as Atlas would show that copy and not Go's. Noted in the record ([`Theirs`]), so the folder
/// isn't read again while that copy stays as it is and the library's doesn't change. Only the
/// descriptions of `.atlaspack` files are read, as far as Atlas reads ([`pack_files`]); the
/// copy is never replaced, moved or removed. A plan that doesn't give the pack's dates (an
/// older server): Go fetches it as before. True when Go's is left out.
pub fn theirs_instead(d: &Device, f: &File) -> bool {
    let (Some(data), Some(folder)) = (&d.data, d.places.get(PLACE)) else { return false };
    let date = |s: &str| Some(s.to_string()).filter(|s| !s.trim().is_empty());
    let (updated, exported_at) = (date(&f.pack_updated), date(&f.pack_exported_at));
    if f.pack.is_empty() || (updated.is_none() && exported_at.is_none()) {
        return false;
    }
    let ours: Vec<PathBuf> = load(data).delivered.iter().map(Delivered::path).collect();
    // the newest copy that isn't Go's, if it's as new as the library's or newer
    let mut best: Option<(PathBuf, Option<String>, String)> = None;
    for p in pack_files(folder).into_iter().filter(|p| !ours.contains(p)) {
        let Some((id, u, e)) = dates(&p) else { continue };
        let newer = |than: &(PathBuf, Option<String>, String)| gamerepo_wire::packs::newer(u.as_deref(), Some(&e), than.1.as_deref(), Some(&than.2)).is_gt();
        if id == f.pack && best.as_ref().map_or(true, newer) {
            best = Some((p, u, e));
        }
    }
    let Some((path, u, e)) = best else { return false };
    let vs = gamerepo_wire::packs::newer(u.as_deref(), Some(&e), updated.as_deref(), exported_at.as_deref());
    let Some((size, modified)) = stamp(&path).filter(|_| vs.is_ge()) else { return false };
    let t = Theirs { pack_id: f.pack.clone(), updated: f.pack_updated.clone(), exported_at: f.pack_exported_at.clone(), path, size, modified, newer: vs.is_gt() };
    change(data, |rec| {
        rec.theirs.retain(|x| x.pack_id != t.pack_id);
        rec.theirs.push(t);
        Ok(())
    })
    .is_ok()
}

/// The copy Go didn't deliver of a pack that's there instead of Go's ([`Theirs`]), while
/// it's in the packs folder `folder`.
pub fn theirs_here<'a>(rec: &'a Record, folder: &Path, pack: &str) -> Option<&'a Theirs> {
    rec.theirs.iter().find(|t| t.pack_id == pack && t.path.starts_with(folder) && t.path.is_file())
}

/// The pack *Show in Atlas* asks Atlas for (D224), of a game's packs (`ids`, in the library's
/// order): of those in Atlas's packs folder here (one Go delivered or kept there, or a copy Go
/// didn't deliver that it left in place of its own), the newest by the file's own description
/// (`pack.updated`, then `exportedAt`, as Atlas picks between two files of a pack), the first
/// in the library's order when they're as new or can't be read. None when none is here.
pub fn to_show(rec: &Record, folder: &Path, ids: &[&str]) -> Option<String> {
    // a pack's `pack.updated` and `exportedAt`, when its file can be read as it
    type When = Option<(Option<String>, String)>;
    let mut best: Option<(&str, When)> = None;
    for &id in ids {
        let file = rec.delivered.iter().find(|e| e.pack_id == id && e.here()).map(Delivered::path).or_else(|| theirs_here(rec, folder, id).map(|t| t.path.clone()));
        let Some(file) = file else { continue };
        let when = dates(&file).filter(|d| d.0 == id).map(|d| (d.1, d.2));
        let newer = |than: &When| match (&when, than) {
            (Some(a), Some(b)) => gamerepo_wire::packs::newer(a.0.as_deref(), Some(&a.1), b.0.as_deref(), Some(&b.1)).is_gt(),
            (Some(_), None) => true,
            _ => false,
        };
        if best.as_ref().is_none_or(|b| newer(&b.1)) {
            best = Some((id, when));
        }
    }
    best.map(|b| b.0.to_string())
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

/// The folder every Atlas desktop app on the computer shares (Atlas's `settings.rs`,
/// `shared_dir`): `%LOCALAPPDATA%\Atlas` on Windows, `$XDG_DATA_HOME/atlas` on Linux.
fn atlas_shared(home: &Path) -> PathBuf {
    data_home(home).join(if cfg!(windows) { "Atlas" } else { "atlas" })
}

/// Atlas's packs folder where it is until the person chooses another in Atlas (the
/// contract's answer 5, as Atlas's block A1 built it): `%LOCALAPPDATA%\Atlas\Packs` on
/// Windows; `$XDG_DATA_HOME/atlas/packs` (`~/.local/share/atlas/packs`), all lower case, on
/// Linux.
pub fn default_folder(home: &Path) -> PathBuf {
    atlas_shared(home).join(if cfg!(windows) { "Packs" } else { "packs" })
}

/// Atlas's shared settings file (D230), where Atlas keeps the packs folder the person chose
/// (`packsFolder`): `%LOCALAPPDATA%\Atlas\settings.json` on Windows,
/// `$XDG_DATA_HOME/atlas/settings.json` on Linux.
pub fn atlas_settings_file(home: &Path) -> PathBuf {
    atlas_shared(home).join("settings.json")
}

/// A settings file larger than this isn't Atlas's (its `MAX_SETTINGS_BYTES`).
const ATLAS_SETTINGS_MOST: u64 = 1 << 20;
/// The longest `packsFolder` Atlas takes, in characters (its `MAX_FOLDER_CHARS`).
const ATLAS_FOLDER_MOST: usize = 4096;

/// What Atlas's settings file says of its packs folder (D230), read as Atlas reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AtlasSays {
    /// Nothing: no file, a file Atlas wouldn't read (too large, not JSON, another format
    /// or version), or no `packsFolder` (or `null`): Atlas uses its usual folder
    Nothing,
    /// The folder the person chose in Atlas, a full path
    Folder(PathBuf),
    /// A `packsFolder` Atlas can't use, and why (in Go's words): Atlas uses its usual folder
    Unusable(&'static str),
}

/// Why a `packsFolder` value can't be used, by Atlas's own rules (`settings.rs`'s
/// `unusable_folder`): empty, longer than 4,096 characters, with a NUL, not a full path, or
/// going up a folder with `..`. None when it can be used.
fn unusable_folder(text: &str) -> Option<&'static str> {
    if text.trim().is_empty() {
        return Some("it's empty");
    }
    if text.chars().count() > ATLAS_FOLDER_MOST {
        return Some("it's too long");
    }
    if text.contains('\0') {
        return Some("it isn't a path");
    }
    let path = Path::new(text);
    if !path.is_absolute() {
        return Some("it isn't a full path");
    }
    if path.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Some("it goes up a folder with ..");
    }
    None
}

/// What the text of Atlas's settings file says of its packs folder, read leniently as Atlas
/// reads it: a file that isn't a JSON object of `format` "atlas-settings" and `version` 1
/// says nothing (Atlas takes its defaults); `packsFolder` absent or `null` says nothing; a
/// value that isn't text, or text Atlas can't use ([`unusable_folder`]), is unusable, and
/// Atlas then uses its usual folder.
pub fn atlas_says(text: &str) -> AtlasSays {
    let Some(v) = atlas_settings_of(text) else { return AtlasSays::Nothing };
    match v.get("packsFolder") {
        None | Some(serde_json::Value::Null) => AtlasSays::Nothing,
        Some(serde_json::Value::String(t)) => match unusable_folder(t) {
            None => AtlasSays::Folder(PathBuf::from(t)),
            Some(why) => AtlasSays::Unusable(why),
        },
        Some(_) => AtlasSays::Unusable("it isn't text"),
    }
}

/// The text of Atlas's settings file as Atlas reads it (D230): a JSON object (a byte order
/// mark allowed) of `format` "atlas-settings" and `version` 1; None for anything else, which
/// says nothing (Atlas takes its defaults). Each field is then read leniently on its own.
pub fn atlas_settings_of(text: &str) -> Option<serde_json::Value> {
    let v = serde_json::from_str::<serde_json::Value>(text.trim_start_matches('\u{feff}')).ok()?;
    (v.get("format").and_then(serde_json::Value::as_str) == Some("atlas-settings") && v.get("version").and_then(serde_json::Value::as_u64) == Some(1)).then_some(v)
}

/// Atlas's settings file on this computer now ([`atlas_settings_of`]): None when there's no
/// file, it's larger than Atlas reads, or it isn't one Atlas reads. Read afresh each time, as
/// the person can change it in Atlas while Go runs.
pub fn atlas_settings(home: &Path) -> Option<serde_json::Value> {
    let file = atlas_settings_file(home);
    match std::fs::metadata(&file) {
        Ok(m) if m.is_file() && m.len() <= ATLAS_SETTINGS_MOST => std::fs::read_to_string(&file).ok().and_then(|t| atlas_settings_of(&t)),
        _ => None,
    }
}

/// What Atlas's settings file on this computer says of its packs folder now ([`atlas_says`]),
/// read afresh each time, as the person can change it in Atlas while Go runs.
pub fn atlas_packs_folder(home: &Path) -> AtlasSays {
    let file = atlas_settings_file(home);
    match std::fs::metadata(&file) {
        Ok(m) if m.is_file() && m.len() <= ATLAS_SETTINGS_MOST => std::fs::read_to_string(&file).map(|t| atlas_says(&t)).unwrap_or(AtlasSays::Nothing),
        _ => AtlasSays::Nothing,
    }
}

/// Where the packs folder Go uses comes from (D230).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FolderFrom {
    /// Set in Go's own Settings (`atlas_folder`)
    Go,
    /// Set in Atlas (`packsFolder` in its settings file)
    Atlas,
    /// Atlas's usual folder ([`default_folder`])
    Usual,
}

impl FolderFrom {
    /// Its word for the page: go, atlas or usual.
    pub fn word(self) -> &'static str {
        match self {
            FolderFrom::Go => "go",
            FolderFrom::Atlas => "atlas",
            FolderFrom::Usual => "usual",
        }
    }
}

/// The packs folder Go puts packs in (D230): the one set in Go's Settings (`go_setting`) when
/// one is (0.5.72's guess on Linux, [`old_default_folder`], counts as none), else the one set
/// in Atlas ([`atlas_packs_folder`]), else Atlas's usual one. Worked out afresh each time.
pub fn choose_folder(go_setting: &str, home: &Path) -> (PathBuf, FolderFrom) {
    let set = go_setting.trim();
    if !set.is_empty() && old_default_folder(home).is_none_or(|old| Path::new(set) != old) {
        return (PathBuf::from(set), FolderFrom::Go);
    }
    match atlas_packs_folder(home) {
        AtlasSays::Folder(f) => (f, FolderFrom::Atlas),
        _ => (default_folder(home), FolderFrom::Usual),
    }
}

/// The place, beside [`PLACE`], a device has when its packs folder is the one chosen in Atlas
/// (D230): such a folder that isn't there (a drive that isn't connected) is never made, as
/// Atlas never makes it, since its path would then be made on another drive.
pub const SET_IN_ATLAS: &str = "atlas-set-in-atlas";

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
///
/// D230: also when the packs folder changes (set in Go, or in Atlas): `make` false leaves a
/// folder that isn't there unmade (one chosen in Atlas), so nothing moves until it's there.
/// Never beside a newer copy: a pack whose `packId` has a copy in `new` Go didn't put there
/// that's as new as Go's file or newer (by its description, `pack.updated` then
/// `exportedAt`) stays where it is (Atlas shows that copy, and Go's would only be beside it).
pub fn move_from(data: &Path, old: &Path, new: &Path, make: bool) -> Result<usize> {
    // the same folder, or one a case-insensitive drive takes as the same (`Atlas/Packs` and
    // `atlas/packs`): nothing to move
    let same = |a: &Path, b: &Path| matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y);
    if old == new || same(old, new) || (!make && !new.is_dir()) {
        return Ok(0);
    }
    change(data, |rec| {
        let mut moved = 0;
        // the copies in the new folder Go didn't put there, read once: (packId, updated, exportedAt)
        let ours: Vec<PathBuf> = rec.delivered.iter().map(Delivered::path).collect();
        let mut theirs: Option<Vec<(String, Option<String>, String)>> = None;
        for i in 0..rec.delivered.len() {
            let e = rec.delivered[i].clone();
            if e.folder != old || !e.there() {
                continue;
            }
            let theirs = theirs.get_or_insert_with(|| pack_files(new).into_iter().filter(|p| !ours.contains(p)).filter_map(|p| dates(&p)).collect());
            if let Some((_, mine_u, mine_e)) = dates(&e.path()).filter(|d| d.0 == e.pack_id) {
                if theirs.iter().any(|t| t.0 == e.pack_id && gamerepo_wire::packs::newer(t.1.as_deref(), Some(&t.2), mine_u.as_deref(), Some(&mine_e)).is_ge()) {
                    continue;
                }
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
    // a packs folder chosen in Atlas that isn't there is never made (D230)
    if let Some(folder) = d.places.get(SET_IN_ATLAS).filter(|f| !f.is_dir()) {
        bail!("Atlas's packs folder, {}, chosen in Atlas, isn't there (is its drive connected?). Game Repo Go doesn't make it, as Atlas doesn't.", folder.display());
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
    let (Some(data), Some(folder)) = (&d.data, d.places.get(PLACE)) else { return false };
    let rec = load(data);
    let delivered = |sha1: &String| rec.delivered.iter().any(|e| e.pack_id == f.pack && &e.folder == folder && ((&e.sha1 == sha1 && e.there()) || (&e.kept_over == sha1 && e.here())));
    // or a copy Go didn't deliver was as new as this one of the library's or newer, and is
    // there as it was then
    let theirs = || !(f.pack_updated.is_empty() && f.pack_exported_at.is_empty()) && rec.theirs.iter().any(|t| t.pack_id == f.pack && t.path.starts_with(folder) && t.updated == f.pack_updated && t.exported_at == f.pack_exported_at && t.unchanged());
    f.sha1.as_ref().is_some_and(delivered) || theirs()
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
        rec.theirs.retain(|t| t.pack_id != f.pack);
        // its size as it came (a pack fetched by its packId may be a newer file than the plan's)
        let size = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(f.size);
        rec.delivered.push(Delivered { pack_id: f.pack.clone(), file: name, folder: folder.clone(), size, sha1: sha1.to_string(), games, at: now(), ..Default::default() });
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

/// The guidebooks unticked in each game's pack of guidebooks, for a plan
/// (`PlanAsk::guidebooks_left`, D238).
pub fn guidebooks_left(data: &Path) -> Vec<(String, Vec<String>)> {
    load(data).guidebooks_left.into_iter().filter(|(_, v)| !v.is_empty()).collect()
}

/// A guidebook ticked or unticked in a game's pack of guidebooks (D238).
pub fn set_guidebook_ticked(data: &Path, key: &str, guidebook: &str, on: bool) -> Result<()> {
    change(data, |rec| {
        let v = rec.guidebooks_left.entry(key.to_string()).or_default();
        v.retain(|g| g != guidebook);
        if !on {
            v.push(guidebook.to_string());
        }
        rec.guidebooks_left.retain(|_, v| !v.is_empty());
        Ok(())
    })
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
/// maybe with the maker's own things) is never removed, nor one put there on its own too
/// (D225). True when the file was removed.
pub fn take_off(data: &Path, key: &str, pack: &str) -> Result<bool> {
    change(data, |rec| {
        let alone = rec.is_alone(pack);
        let Some(i) = rec.delivered.iter().position(|e| e.pack_id == pack) else { return Ok(false) };
        let e = &mut rec.delivered[i];
        e.games.retain(|g| g != key);
        if !e.games.is_empty() || !e.kept_over.is_empty() || alone {
            // a file kept over the library's stays, and stays noted, so ticked again it isn't
            // fetched (nor put beside it); one put there on its own stays for itself
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

/// The file of a pack in the packs folder `folder` that Atlas reads as it, if one is: the one
/// Go delivered (or kept over the library's), else a copy Go didn't deliver that it left in
/// place of its own ([`theirs_here`]).
pub fn file_here(rec: &Record, folder: &Path, pack: &str) -> Option<PathBuf> {
    rec.delivered.iter().find(|e| e.pack_id == pack && &e.folder == folder && e.here()).map(Delivered::path).or_else(|| theirs_here(rec, folder, pack).map(|t| t.path.clone()))
}

/// Packs put in the packs folder on their own (D225), noted once they're there (delivered,
/// or there already as Go's or as a copy it left in place of its own).
pub fn set_alone(data: &Path, packs: &[String]) -> Result<()> {
    change(data, |rec| {
        for p in packs {
            if !rec.is_alone(p) {
                rec.alone.push(p.clone());
            }
        }
        rec.alone.sort();
        Ok(())
    })
}

/// Whether the library's copy of a pack, by the dates its list gives (`pack.updated`, then
/// `exportedAt`), is newer than the file of it in the packs folder (D225): Go then fetches
/// it, and puts it in place under the usual rules. False when no file of it is here, or the
/// file here can't be read as the pack (it's left as it is), or the library gives no dates.
pub fn library_newer(rec: &Record, folder: &Path, pack: &str, updated: &str, exported_at: &str) -> bool {
    let date = |s: &str| Some(s.to_string()).filter(|s| !s.trim().is_empty());
    let (u, e) = (date(updated), date(exported_at));
    if u.is_none() && e.is_none() {
        return false;
    }
    let Some(file) = file_here(rec, folder, pack) else { return false };
    let Some((id, hu, he)) = dates(&file) else { return false };
    id == pack && gamerepo_wire::packs::newer(u.as_deref(), e.as_deref(), hu.as_deref(), Some(&he)).is_gt()
}

/// What became of a pack taken off on its own (D225, [`remove_alone`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "result", content = "games")]
pub enum Off {
    /// Its file was taken out of the packs folder
    Removed,
    /// Its file stays: these games on this device have it ticked (their keys)
    KeptFor(Vec<String>),
    /// Its file stays: Atlas wrote it again since Go put it there (newer, or changed), so it's
    /// not Go's to remove
    Changed,
    /// Its file stays: it's a copy Go didn't put there
    NotGos,
    /// No file of it was here
    Gone,
}

/// A pack put in the packs folder on its own, taken off (D225): it's no longer kept on its
/// own, and its file is removed only when no game this device has still has it ticked
/// (`wanted`, asked with each game's key Go delivered it for), it's Go's own, and it's there
/// as Go put it (its size and SHA-1). A file Atlas wrote again since, or a copy Go didn't
/// deliver, stays as it is.
pub fn remove_alone(data: &Path, pack: &str, wanted: impl Fn(&str) -> bool) -> Result<Off> {
    change(data, |rec| {
        rec.alone.retain(|p| p != pack);
        let Some(i) = rec.delivered.iter().position(|e| e.pack_id == pack) else {
            return Ok(if rec.theirs.iter().any(|t| t.pack_id == pack && t.path.is_file()) { Off::NotGos } else { Off::Gone });
        };
        let e = &rec.delivered[i];
        let games: Vec<String> = e.games.iter().filter(|g| wanted(g)).cloned().collect();
        if !games.is_empty() {
            return Ok(Off::KeptFor(games));
        }
        if !e.kept_over.is_empty() {
            return Ok(Off::Changed);
        }
        let e = rec.delivered.remove(i);
        if !e.path().is_file() {
            return Ok(Off::Gone);
        }
        // unchanged: its size, and its SHA-1 (Atlas may have written it again at the same size)
        if e.there() && crate::sha1_file(&e.path()).is_ok_and(|h| h.eq_ignore_ascii_case(&e.sha1)) {
            patiently(|| std::fs::remove_file(e.path()))?;
            return Ok(Off::Removed);
        }
        Ok(Off::Changed)
    })
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

    /// Packs on their own (D225): a pack installed from Maps and guides (no game's key) is one
    /// file with the same pack a game brings, stays when a game unticks it, is taken off on
    /// its own only once no game this device has still has it ticked, and never when Atlas
    /// wrote it again here or it's a copy Go didn't put there.
    #[test]
    fn a_pack_on_its_own_is_one_file_with_a_games_and_stays_while_a_game_here_wants_it() {
        let (home, d) = device("alone");
        let folder = d.places[PLACE].clone();
        let data = d.data.clone().unwrap();
        let game = "gb/Island (World)";
        // installed on its own: no game's key
        let v1 = atlas_pack("island", "2026-11-18T08:00:00Z", "2026-11-18T08:05:00Z", &[]);
        let alone = pack("Island.atlaspack", "island", "", &v1);
        let at = deliver(&d, &alone, &v1);
        set_alone(&data, &["island".to_string()]).unwrap();
        let rec = load(&data);
        assert!(rec.is_alone("island"));
        assert_eq!(rec.delivered[0].games, Vec::<String>::new(), "no game brought it");
        // the game installed later brings the same pack: one file, noted as the game's too
        let with_game = pack("Island.atlaspack", "island", game, &v1);
        assert!(there(&d, &with_game), "the same packId and file: not fetched again");
        note_there(&d, &with_game).unwrap();
        assert_eq!(names(&folder), ["Island.atlaspack"], "never two");
        assert_eq!(here(&data, "island").unwrap().games, [game]);
        // unticked on the game's page: it stays, as it's on its own too
        assert!(!take_off(&data, game, "island").unwrap());
        assert!(at.is_file());
        note_there(&d, &with_game).unwrap();
        // taken off on its own while the game here still has it ticked: the file stays and only
        // the flag goes
        let off = remove_alone(&data, "island", |g| g == game).unwrap();
        assert_eq!(off, Off::KeptFor(vec![game.to_string()]));
        assert!(at.is_file() && !load(&data).is_alone("island"));
        // on its own again, the game gone from the device: taken out
        set_alone(&data, &["island".to_string()]).unwrap();
        assert_eq!(remove_alone(&data, "island", |_| false).unwrap(), Off::Removed);
        assert!(!at.exists() && load(&data).delivered.is_empty());
        assert_eq!(remove_alone(&data, "island", |_| false).unwrap(), Off::Gone);

        // Atlas writes it again here, newer: the library's older copy isn't put over it, and
        // taking it off on its own leaves it
        let at = deliver(&d, &alone, &v1);
        set_alone(&data, &["island".to_string()]).unwrap();
        let v2 = atlas_pack("island", "2026-11-19T10:00:00Z", "2026-11-19T10:00:00Z", MINE);
        std::fs::write(&at, &v2).unwrap();
        deliver(&d, &alone, &v1);
        assert_eq!(std::fs::read(&at).unwrap(), v2, "kept_over: never older over newer");
        assert!(!library_newer(&load(&data), &folder, "island", "2026-11-18T08:00:00Z", "2026-11-18T08:05:00Z"));
        assert_eq!(remove_alone(&data, "island", |_| false).unwrap(), Off::Changed);
        assert_eq!(std::fs::read(&at).unwrap(), v2);
        // changed by hand at another size: not Go's to remove either
        std::fs::remove_file(&at).unwrap();
        save(&data, &Record::default()).unwrap();
        let at = deliver(&d, &alone, &v1);
        set_alone(&data, &["island".to_string()]).unwrap();
        std::fs::write(&at, b"PK changed by hand").unwrap();
        assert_eq!(remove_alone(&data, "island", |_| false).unwrap(), Off::Changed);
        assert!(at.is_file());
        let _ = std::fs::remove_dir_all(&home);
    }

    /// After a read of the library (D225): a pack kept on its own comes again only when the
    /// library's copy is newer than the file here by its dates (`pack.updated`, then
    /// `exportedAt`), whether that file is Go's or a copy Go left in place of its own.
    #[test]
    fn the_librarys_copy_is_fetched_again_only_when_its_newer_than_the_file_here() {
        let (home, d) = device("alone-newer");
        let folder = d.places[PLACE].clone();
        let data = d.data.clone().unwrap();
        let v1 = atlas_pack("island", "2026-11-18T08:00:00Z", "2026-11-18T08:05:00Z", &[]);
        let rec = load(&data);
        assert!(!library_newer(&rec, &folder, "island", "2026-11-20T00:00:00Z", ""), "nothing here: nothing to update");
        deliver(&d, &pack("Island.atlaspack", "island", "", &v1), &v1);
        let rec = load(&data);
        assert!(!library_newer(&rec, &folder, "island", "2026-11-18T08:00:00Z", "2026-11-18T08:05:00Z"), "the same");
        assert!(!library_newer(&rec, &folder, "island", "2026-11-17T08:00:00Z", "2026-11-19T08:05:00Z"), "older by pack.updated");
        assert!(library_newer(&rec, &folder, "island", "2026-11-18T08:00:00Z", "2026-11-18T09:00:00Z"), "newer by exportedAt");
        assert!(library_newer(&rec, &folder, "island", "2026-11-19T08:00:00Z", ""), "newer by pack.updated");
        assert!(!library_newer(&rec, &folder, "island", "", ""), "no dates: not fetched");
        // a copy Go didn't deliver, newer, in place of Go's: compared with that one
        let theirs = atlas_pack("isle", "2026-11-21T08:00:00Z", "2026-11-21T08:00:00Z", &[]);
        std::fs::write(folder.join("Isle (mine).atlaspack"), &theirs).unwrap();
        let f = File { pack_updated: "2026-11-20T08:00:00Z".into(), ..pack("Isle.atlaspack", "isle", "", b"library's") };
        assert!(theirs_instead(&d, &f));
        let rec = load(&data);
        assert!(!library_newer(&rec, &folder, "isle", "2026-11-20T08:00:00Z", ""));
        assert!(library_newer(&rec, &folder, "isle", "2026-11-22T08:00:00Z", ""));
        // taken off on its own: a copy Go didn't put there stays
        set_alone(&data, &["isle".to_string()]).unwrap();
        assert_eq!(remove_alone(&data, "isle", |_| false).unwrap(), Off::NotGos);
        assert!(folder.join("Isle (mine).atlaspack").is_file());
        let _ = std::fs::remove_dir_all(&home);
    }

    /// Show in Atlas (D224): of a game's packs, the one in Atlas's packs folder here (Go's,
    /// kept, or a copy Go left in place of its own) that's newest by its own description; the
    /// first in the library's order when they're as new; none when none is here.
    #[test]
    fn show_in_atlas_asks_for_the_newest_of_a_games_packs_that_are_here() {
        let (home, d) = device("show");
        let folder = d.places[PLACE].clone();
        let data = d.data.clone().unwrap();
        let key = "pc/island";
        assert_eq!(to_show(&load(&data), &folder, &["alpha", "beta"]), None, "none here");
        // one that isn't a pack Go can read still counts while it's alone
        let broken = b"PK not a pack".to_vec();
        deliver(&d, &pack("Broken.atlaspack", "broken", key, &broken), &broken);
        assert_eq!(to_show(&load(&data), &folder, &["alpha", "broken"]).as_deref(), Some("broken"));
        let a = atlas_pack("alpha", "2026-11-18T08:00:00Z", "2026-11-18T08:05:00Z", &[]);
        deliver(&d, &pack("Alpha.atlaspack", "alpha", key, &a), &a);
        let b = atlas_pack("beta", "2026-11-19T08:00:00Z", "2026-11-19T08:05:00Z", &[]);
        deliver(&d, &pack("Beta.atlaspack", "beta", key, &b), &b);
        let rec = load(&data);
        assert_eq!(to_show(&rec, &folder, &["broken", "gamma", "alpha", "beta"]).as_deref(), Some("beta"), "the newest");
        assert_eq!(to_show(&rec, &folder, &["broken", "alpha"]).as_deref(), Some("alpha"), "one that can be read over one that can't");
        // as new: the first in the library's order
        let a2 = atlas_pack("alpha", "2026-11-19T08:00:00Z", "2026-11-19T08:05:00Z", &[]);
        let f = pack("Alpha.atlaspack", "alpha", key, &a2);
        deliver(&d, &f, &a2);
        assert_eq!(to_show(&load(&data), &folder, &["alpha", "beta"]).as_deref(), Some("alpha"));
        assert_eq!(to_show(&load(&data), &folder, &["beta", "alpha"]).as_deref(), Some("beta"));
        // taken out: it's not asked for
        assert!(take_off(&data, key, "beta").unwrap() && take_off(&data, key, "alpha").unwrap());
        assert_eq!(to_show(&load(&data), &folder, &["alpha", "beta"]), None);
        // a copy Go didn't deliver that Go left in place of its own is Atlas's to show too
        let c = atlas_pack("gamma", "2026-11-20T08:00:00Z", "2026-11-20T08:00:00Z", &[]);
        std::fs::write(folder.join("Gamma.atlaspack"), &c).unwrap();
        let g = File { pack_updated: "2026-11-20T08:00:00Z".into(), pack_exported_at: "2026-11-20T08:00:00Z".into(), ..pack("Gamma.atlaspack", "gamma", key, &c) };
        assert!(theirs_instead(&d, &g));
        assert_eq!(to_show(&load(&data), &folder, &["alpha", "gamma"]).as_deref(), Some("gamma"));
        let _ = std::fs::remove_dir_all(&home);
    }

    /// The contract's saving ("Maps and Guides notes, part 6"): just before Go would fetch a
    /// pack, a copy of it in the packs folder that Go didn't deliver and that's as new as the
    /// library's or newer means Go's isn't fetched (Atlas would show that copy); it's noted so
    /// the folder isn't read again while the copy is unchanged, and the copy is never
    /// replaced, moved or removed. Only `.atlaspack` files are read, as far down as Atlas reads.
    #[test]
    fn a_copy_go_didnt_deliver_as_new_or_newer_isnt_fetched_over_and_is_never_touched() {
        let (home, d) = device("theirs");
        let folder = d.places[PLACE].clone();
        let data = d.data.clone().unwrap();
        let key = "pc/island";
        let coming = |bytes: &[u8], updated: &str, exported_at: &str| File { pack_updated: updated.into(), pack_exported_at: exported_at.into(), ..pack("Island.atlaspack", "island", key, bytes) };
        let lib1 = atlas_pack("island", "2026-11-18T08:00:00Z", "2026-11-18T08:05:00Z", MINE);
        let f1 = coming(&lib1, "2026-11-18T08:00:00Z", "2026-11-18T08:05:00Z");
        // nothing in the folder: it's fetched
        assert!(!there(&d, &f1) && !theirs_instead(&d, &f1));
        // what doesn't count: a newer copy still being written (Go's or the Importer's), one
        // five folders down (deeper than Atlas reads), another pack, a file that isn't a pack
        let newer = atlas_pack("island", "2026-11-19T08:00:00Z", "2026-11-19T08:00:00Z", MINE);
        std::fs::create_dir_all(folder.join("a/b/c/d/e")).unwrap();
        std::fs::write(folder.join("Island.atlaspack.part"), &newer).unwrap();
        std::fs::write(folder.join("Island.atlaspack.atlas-importer.part"), &newer).unwrap();
        std::fs::write(folder.join("a/b/c/d/e/Island.atlaspack"), &newer).unwrap();
        std::fs::write(folder.join("Other.atlaspack"), atlas_pack("other", "2026-11-19T08:00:00Z", "2026-11-19T08:00:00Z", &[])).unwrap();
        std::fs::write(folder.join("Broken.atlaspack"), b"not a pack").unwrap();
        // an older copy put there by hand doesn't count either
        std::fs::write(folder.join("Island (old).atlaspack"), atlas_pack("island", "2026-11-17T08:00:00Z", "2026-11-17T08:00:00Z", &[])).unwrap();
        assert!(!theirs_instead(&d, &f1));
        assert!(load(&data).theirs.is_empty());
        // the same version, four folders down (as deep as Atlas reads): Go's isn't fetched
        let copy = folder.join("a/b/c/d/My Island.atlaspack");
        std::fs::write(&copy, &lib1).unwrap();
        assert!(theirs_instead(&d, &f1));
        let t = load(&data).theirs[0].clone();
        assert_eq!((t.pack_id.as_str(), t.path.clone(), t.updated.as_str(), t.newer), ("island", copy.clone(), "2026-11-18T08:00:00Z", false));
        // the library's SHA-1 isn't needed (a plan may not have it yet): its dates say which it is
        assert!(there(&d, &File { sha1: None, ..f1.clone() }));
        // noted: the next sync takes it as there without reading the folder again
        assert!(there(&d, &f1));
        let rec = load(&data);
        assert_eq!(theirs_here(&rec, &folder, "island").map(|t| t.path.clone()), Some(copy.clone()));
        assert!(rec.delivered.is_empty(), "nothing delivered");
        // a plan that doesn't give the pack's dates (an older server): fetched as before
        let undated = File { pack_updated: String::new(), pack_exported_at: String::new(), ..f1.clone() };
        assert!(!theirs_instead(&d, &undated));
        // the copy changed (a different size): read again; still as new or newer, still left out
        let v2 = atlas_pack("island", "2026-11-18T09:00:00Z", "2026-11-18T09:00:00Z", &[MINE[0]]);
        std::fs::write(&copy, &v2).unwrap();
        assert!(!there(&d, &f1), "changed: looked at again");
        assert!(theirs_instead(&d, &f1) && load(&data).theirs[0].newer, "newer now");
        assert!(there(&d, &f1));
        // the library's newer than the copy: Go's comes, beside it, and the copy is untouched
        let lib2 = atlas_pack("island", "2026-11-20T08:00:00Z", "2026-11-20T08:00:00Z", MINE);
        let f2 = coming(&lib2, "2026-11-20T08:00:00Z", "2026-11-20T08:00:00Z");
        assert!(!there(&d, &f2) && !theirs_instead(&d, &f2));
        let at = deliver(&d, &f2, &lib2);
        assert_eq!(at, folder.join("Island.atlaspack"));
        assert_eq!(std::fs::read(&copy).unwrap(), v2, "never replaced");
        assert!(load(&data).theirs.is_empty(), "Go's own is the one noted now");
        // Atlas writes Go's own file again, newer than the library's next: that's Go's, never
        // taken as a copy it didn't deliver (the rule of part 5 keeps it instead)
        let lib3 = atlas_pack("island", "2026-11-21T08:00:00Z", "2026-11-21T08:00:00Z", MINE);
        let f3 = coming(&lib3, "2026-11-21T08:00:00Z", "2026-11-21T08:00:00Z");
        std::fs::write(&at, atlas_pack("island", "2026-11-22T08:00:00Z", "2026-11-22T08:00:00Z", &[])).unwrap();
        assert!(!there(&d, &f3) && !theirs_instead(&d, &f3));
        // a copy by hand in the folder itself, newer than the library's, with the name Go
        // would give: Go's own is left there; and once that copy's gone, Go's is fetched
        std::fs::remove_file(&at).unwrap();
        let by_hand = atlas_pack("island", "2026-11-23T08:00:00Z", "2026-11-23T08:00:00Z", &[]);
        std::fs::write(&at, &by_hand).unwrap();
        let rec = load(&data);
        assert_eq!(rec.delivered[0].path(), at, "Go's record still names it");
        assert!(!theirs_instead(&d, &f3), "Go's file's name: Go's own (part 5 compares it)");
        std::fs::remove_file(&copy).unwrap();
        std::fs::remove_file(&at).unwrap();
        change(&data, |r| {
            r.delivered.clear();
            Ok(())
        })
        .unwrap();
        std::fs::write(&at, &by_hand).unwrap();
        assert!(theirs_instead(&d, &f3) && there(&d, &f3));
        assert!(!take_off(&data, key, "island").unwrap());
        assert_eq!(std::fs::read(&at).unwrap(), by_hand, "never removed by an untick");
        std::fs::remove_file(&at).unwrap();
        assert!(!there(&d, &f3) && !theirs_instead(&d, &f3), "gone: Go's is fetched");
        assert!(theirs_here(&load(&data), &folder, "island").is_none());
        let _ = std::fs::remove_dir_all(&home);
    }

    /// Atlas reads its packs folder 4 folders down and 1,000 pack files at most; so does Go
    /// when it looks for copies it didn't deliver.
    #[test]
    fn go_looks_for_copies_as_far_as_atlas_reads() {
        let dir = std::env::temp_dir().join(format!("gamerepo-device-packs-look-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("1/2/3/4/5")).unwrap();
        for (n, p) in ["A.atlaspack", "1/B.ATLASPACK", "1/2/3/4/C.atlaspack", "1/2/3/4/5/D.atlaspack", "E.atlaspack.part", "F.atlaspack.atlas-importer.part", ".G.atlaspack", "H.zip"].iter().enumerate() {
            std::fs::write(dir.join(p), [n as u8]).unwrap();
        }
        let found: Vec<String> = pack_files(&dir).iter().map(|p| p.strip_prefix(&dir).unwrap().to_string_lossy().replace('\\', "/")).collect();
        assert_eq!(found, ["A.atlaspack", "1/B.ATLASPACK", "1/2/3/4/C.atlaspack"]);
        let many = dir.join("many");
        std::fs::create_dir_all(&many).unwrap();
        for i in 0..LOOK_MOST + 5 {
            std::fs::write(many.join(format!("{i:04}.atlaspack")), b"").unwrap();
        }
        assert_eq!(pack_files(&many).len(), LOOK_MOST);
        let _ = std::fs::remove_dir_all(&dir);
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

        assert_eq!(move_from(&data, &old, &new, true).unwrap(), 1);
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
        assert_eq!(move_from(&data, &old, &new, true).unwrap(), 0);
        assert_eq!(move_from(&data, &new, &new, true).unwrap(), 0);
        // a newer file of the moved pack, delivered into the new folder, goes over it there
        d.places.insert(PLACE.into(), new.clone());
        let newer = pack("Delta - Example Maps.atlaspack", "delta-example-maps", "gb/Delta (World)", b"delta, newer");
        deliver(&d, &newer, b"delta, newer");
        assert_eq!(std::fs::read(new.join("Delta - Example Maps (2).atlaspack")).unwrap(), b"delta, newer");
        assert_eq!(names(&new), ["Delta - Example Maps (2).atlaspack", "Delta - Example Maps.atlaspack"]);
        let _ = std::fs::remove_dir_all(&home);
    }

    /// D230: Atlas's settings file read as Atlas reads it (its `settings.rs`): `packsFolder`
    /// absent, `null`, in a file Atlas wouldn't read, or one it can't use is the usual folder.
    #[test]
    fn atlass_settings_file_is_read_as_atlas_reads_it() {
        let home = std::env::temp_dir().join(format!("gamerepo-device-atlas-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let file = atlas_settings_file(&home);
        if cfg!(windows) {
            assert_eq!(file, home.join("AppData").join("Local").join("Atlas").join("settings.json"));
        } else {
            assert_eq!(file, home.join(".local").join("share").join("atlas").join("settings.json"));
        }
        // no file: nothing said
        assert_eq!(atlas_packs_folder(&home), AtlasSays::Nothing);
        let chosen = home.join("Elsewhere").join("Packs");
        let good = serde_json::json!({ "format": "atlas-settings", "version": 1, "mapsFolder": null, "packsFolder": chosen }).to_string();
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, &good).unwrap();
        assert_eq!(atlas_packs_folder(&home), AtlasSays::Folder(chosen.clone()), "a full path is the folder");
        // with a byte order mark too, as Atlas reads it
        assert_eq!(atlas_says(&format!("\u{feff}{good}")), AtlasSays::Folder(chosen.clone()));
        // a file Atlas wouldn't read at all says nothing: not JSON, not an object, another
        // format or version, no format; and one larger than 1 MB
        for text in ["", "not json", "[]", r#"{"format":"atlas-settings","version":2,"packsFolder":"/p"}"#, r#"{"format":"other","version":1,"packsFolder":"/p"}"#, r#"{"version":1,"packsFolder":"/p"}"#, r#"{"format":"atlas-settings","version":"1","packsFolder":"/p"}"#] {
            assert_eq!(atlas_says(text), AtlasSays::Nothing, "{text}");
        }
        std::fs::write(&file, format!("{good}{}", " ".repeat(1 << 20))).unwrap();
        assert_eq!(atlas_packs_folder(&home), AtlasSays::Nothing, "too large to be Atlas's");
        // absent or null: nothing said
        assert_eq!(atlas_says(r#"{"format":"atlas-settings","version":1,"mapsFolder":"/maps"}"#), AtlasSays::Nothing);
        assert_eq!(atlas_says(r#"{"format":"atlas-settings","version":1,"packsFolder":null}"#), AtlasSays::Nothing);
        // values Atlas can't use: the usual folder, as Atlas does
        let long = format!("/{}", "a".repeat(5000));
        let cases: Vec<(String, &str)> = vec![
            (r#""""#.into(), "it's empty"),
            (r#""   ""#.into(), "it's empty"),
            (r#""Packs""#.into(), "it isn't a full path"),
            (r#""packs/inside""#.into(), "it isn't a full path"),
            (serde_json::to_string(&home.join("packs").join("..").join("elsewhere")).unwrap(), "it goes up a folder with .."),
            (serde_json::to_string(&format!("{}\0x", home.join("packs").display())).unwrap(), "it isn't a path"),
            (format!("\"{long}\""), "it's too long"),
            ("42".into(), "it isn't text"),
            ("true".into(), "it isn't text"),
            (r#"["/packs"]"#.into(), "it isn't text"),
            (r#"{"path": "/packs"}"#.into(), "it isn't text"),
        ];
        for (bad, why) in cases {
            let text = format!(r#"{{"format":"atlas-settings","version":1,"mapsFolder":"/maps","packsFolder":{bad}}}"#);
            assert_eq!(atlas_says(&text), AtlasSays::Unusable(why), "{bad}");
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    /// D230: the packs folder Go uses: its own setting, else the one chosen in Atlas, else
    /// Atlas's usual one; read afresh, so a change in Atlas is followed at once.
    #[test]
    fn go_uses_its_own_folder_then_atlass_choice_then_the_usual_one() {
        let home = std::env::temp_dir().join(format!("gamerepo-device-atlas-choose-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let usual = default_folder(&home);
        let in_atlas = home.join("Atlas Packs");
        let mine = home.join("Go Packs");
        // nothing set anywhere: the usual folder
        assert_eq!(choose_folder("", &home), (usual.clone(), FolderFrom::Usual));
        // chosen in Atlas
        let file = atlas_settings_file(&home);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, serde_json::json!({ "format": "atlas-settings", "version": 1, "packsFolder": in_atlas }).to_string()).unwrap();
        assert_eq!(choose_folder("", &home), (in_atlas.clone(), FolderFrom::Atlas));
        assert_eq!(choose_folder("  ", &home), (in_atlas.clone(), FolderFrom::Atlas), "a blank setting is none");
        // set in Go: Go's own wins
        assert_eq!(choose_folder(mine.to_str().unwrap(), &home), (mine.clone(), FolderFrom::Go));
        // 0.5.72's guess on Linux counts as none set
        if let Some(old) = old_default_folder(&home) {
            assert_eq!(choose_folder(old.to_str().unwrap(), &home), (in_atlas.clone(), FolderFrom::Atlas));
        }
        // Atlas's choice taken back, or one it can't use: the usual folder again
        std::fs::write(&file, r#"{"format":"atlas-settings","version":1}"#).unwrap();
        assert_eq!(choose_folder("", &home), (usual.clone(), FolderFrom::Usual));
        std::fs::write(&file, r#"{"format":"atlas-settings","version":1,"packsFolder":"Packs"}"#).unwrap();
        assert_eq!(choose_folder("", &home), (usual.clone(), FolderFrom::Usual));
        let _ = std::fs::remove_dir_all(&home);
    }

    /// D230: when the packs folder changes, Go's own packs follow it, never beside a copy of
    /// the same pack there that's as new or newer; a folder chosen in Atlas that isn't there
    /// isn't made, so nothing moves into it and nothing is downloaded into it.
    #[test]
    fn gos_packs_follow_a_new_folder_but_never_beside_a_newer_copy() {
        let (home, d) = device("follow");
        let data = d.data.clone().unwrap();
        let old = d.places[PLACE].clone();
        let new = home.join("Chosen in Atlas");
        let island = atlas_pack("island-maps", "2026-11-01T00:00:00Z", "2026-11-01T00:00:00Z", &[]);
        let delta = atlas_pack("delta-maps", "2026-11-01T00:00:00Z", "2026-11-01T00:00:00Z", &[]);
        let island_f = pack("Island - Example Maps.atlaspack", "island-maps", "gb/Island (World)", &island);
        let delta_f = pack("Delta - Example Maps.atlaspack", "delta-maps", "gb/Delta (World)", &delta);
        deliver(&d, &island_f, &island);
        deliver(&d, &delta_f, &delta);
        // a newer copy of Island's pack in the new folder, put there by Atlas (not Go)
        let newer = atlas_pack("island-maps", "2026-11-10T00:00:00Z", "2026-11-10T00:00:00Z", &[]);
        let missing = home.join("Not connected").join("Packs");
        assert_eq!(move_from(&data, &old, &missing, false).unwrap(), 0, "a folder chosen in Atlas that isn't there isn't made");
        assert!(!missing.exists());
        // and nothing comes into it
        let mut away = Device { home: d.home.clone(), places: d.places.clone(), data: d.data.clone() };
        away.places.insert(PLACE.into(), missing.clone());
        away.places.insert(SET_IN_ATLAS.into(), missing.clone());
        let e = part_of(&away, &island_f).unwrap_err().to_string();
        assert!(e.contains("isn't there") && !missing.exists(), "{e}");
        std::fs::create_dir_all(new.join("by Atlas")).unwrap();
        std::fs::write(new.join("by Atlas").join("Island.atlaspack"), &newer).unwrap();
        assert_eq!(move_from(&data, &old, &new, false).unwrap(), 1, "only Delta's moves");
        assert_eq!(names(&new), ["Delta - Example Maps.atlaspack", "by Atlas"]);
        assert_eq!(names(&old), ["Island - Example Maps.atlaspack"], "Island's stays where it was: the copy there is newer");
        let rec = load(&data);
        assert_eq!(rec.delivered.iter().find(|e| e.pack_id == "delta-maps").unwrap().folder, new);
        assert_eq!(rec.delivered.iter().find(|e| e.pack_id == "island-maps").unwrap().folder, old);
        // the newer copy gone, Island's follows too (an older copy there is only beside it)
        std::fs::write(new.join("by Atlas").join("Island.atlaspack"), atlas_pack("island-maps", "2026-10-01T00:00:00Z", "2026-10-01T00:00:00Z", &[])).unwrap();
        assert_eq!(move_from(&data, &old, &new, false).unwrap(), 1);
        assert_eq!(std::fs::read(new.join("Island - Example Maps.atlaspack")).unwrap(), island);
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
