//! Saves and save states synced between devices (docs/PLAN.md, "Save sync and the Devices
//! page", D180–D186): what a save is (a [`Unit`]: a game's save, or one of its states, in an
//! emulator's format), its versions, what Game Repo Server and Game Repo Go say about them
//! under `/v1/saves`, and the save profiles (D182) that say where each emulator keeps them.
//! Kept here so the library's side (gamerepo-core) and the device's (gamerepo-device) can't
//! drift.

use serde::{Deserialize, Serialize};

/// A game's save, or one of its save states, in one format.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(default)]
pub struct Unit {
    /// The Library's key (`<system>/<DAT name>`)
    pub key: String,
    /// The format a save is in, so it goes only where it can be read (D180): `retroarch`
    /// for RetroArch's saves, `retroarch:<core>` for its states (a state loads only in the
    /// core that made it)
    pub format: String,
    /// "" for the save; a state's slot: `0`–`9` or `auto`
    pub slot: String,
}

impl Unit {
    pub fn save(key: &str, format: &str) -> Self {
        Self { key: key.into(), format: format.into(), slot: String::new() }
    }

    pub fn state(key: &str, format: &str, slot: &str) -> Self {
        Self { key: key.into(), format: format.into(), slot: slot.into() }
    }

    pub fn is_state(&self) -> bool {
        !self.slot.is_empty()
    }

    /// The unit as one string, `key|format|slot`, for maps and addresses.
    pub fn id(&self) -> String {
        format!("{}|{}|{}", self.key, self.format, self.slot)
    }

    pub fn from_id(id: &str) -> Option<Self> {
        let mut p = id.rsplitn(3, '|');
        let slot = p.next()?.to_string();
        let format = p.next()?.to_string();
        let key = p.next()?.to_string();
        (!key.is_empty() && !format.is_empty()).then_some(Self { key, format, slot })
    }

    /// What it's called on a page: "Save" or "State 3", with the emulator.
    pub fn label(&self) -> String {
        let (emu, core) = self.format.split_once(':').unwrap_or((&self.format, ""));
        let emu = if emu == "retroarch" { "RetroArch".to_string() } else { emu.to_string() };
        let by = if core.is_empty() { emu } else { format!("{emu}, {core}") };
        match self.slot.as_str() {
            "" => format!("Save ({by})"),
            "auto" => format!("Automatic state ({by})"),
            s => format!("State {s} ({by})"),
        }
    }
}

/// One of a save's files. `name` is its path in the save's folder with the game's file name
/// (without its extension) as `{name}`, so the same save is written under each device's
/// own name for the game (D183): `{name}.srm`, `{name}.state1.png`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SaveFile {
    pub name: String,
    pub sha1: String,
    pub size: u64,
    /// When it was last written on the device it came from (seconds since 1970)
    pub modified: i64,
}

/// The same files (by name and SHA-1), whatever their times.
pub fn same_files(a: &[SaveFile], b: &[SaveFile]) -> bool {
    let key = |v: &[SaveFile]| {
        let mut k: Vec<(String, String)> = v.iter().map(|f| (f.name.clone(), f.sha1.clone())).collect();
        k.sort();
        k
    };
    key(a) == key(b)
}

/// A version of a save (D181, D184), kept in the library's `Saves/` for good.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Version {
    /// Its folder's name under `.versions/`: when it was made and by which device, so no
    /// two writers make the same one
    pub id: String,
    pub unit: Unit,
    /// The device's id on the server (`game-repo` for Game Repo itself)
    pub device: String,
    pub device_name: String,
    /// The emulator and its version as the device found them, shown beside the save
    pub emulator: String,
    /// When it reached the library (milliseconds since 1970)
    pub made: i64,
    /// When it was last played: its newest file's time (seconds since 1970)
    pub played: i64,
    /// The version the device had before it (none: its first)
    pub base: String,
    pub files: Vec<SaveFile>,
    /// Made while another device's save had moved on from `base` (a conflict): that one
    #[serde(skip_serializing_if = "String::is_empty")]
    pub differs: String,
    /// A conflict's side that was played earlier, kept but not the newest (D184)
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub behind: bool,
    /// Made by *Use this one* or *Put this version back* from that version
    #[serde(skip_serializing_if = "String::is_empty")]
    pub chosen: String,
}

impl Version {
    pub fn size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

/// A save as the library has it now.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UnitState {
    pub unit: Unit,
    /// The newest version (none: only versions behind a conflict, which can't be)
    pub current: Version,
    /// Two saves differ (D184) until one is chosen or a newer save follows the newest
    pub conflict: bool,
    /// The versions the newest differs from, while they do: a device whose last version is
    /// one of these keeps its own save and isn't given the newest
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub against: Vec<String>,
    pub versions: u32,
}

/// `POST /v1/saves`: the library's saves for these games.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SavesAsk {
    pub keys: Vec<String>,
}

/// The answer: each save the library has for them, and whether states are synced (D180,
/// Settings › Saves in Game Repo).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SavesList {
    pub units: Vec<UnitState>,
    pub states: bool,
}

/// `POST /v1/saves/versions`: a save the device has, its files already sent with
/// `PUT /v1/saves/blob/<sha1>`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Upload {
    pub unit: Unit,
    /// The version the device last had of it ("" for none)
    pub base: String,
    pub files: Vec<SaveFile>,
    pub emulator: String,
}

/// The answer: the version it became (or the newest, when that has the same files), and
/// the save as the library has it now.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Uploaded {
    pub version: Version,
    pub state: UnitState,
}

/// The largest save file a device sends (a PlayStation 3 save or a big state fits).
pub const MAX_FILE: u64 = 512 << 20;

/// A save profile (D182): where an emulator keeps its saves and states on each system Go
/// runs on, read from JSON so a new one is a file, not a release.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Profile {
    pub format: u32,
    pub id: String,
    pub name: String,
    pub about: String,
    /// How it's read: `retroarch` (its `retroarch.cfg` says where its folders are)
    pub kind: String,
    /// The format its saves are in (a state's adds `:<core>`)
    pub save_format: String,
    /// Its programs' names, so nothing is written while one runs (D185)
    pub processes: Vec<String>,
    /// Its settings file, for each of `windows`, `linux` and `android`, first found first:
    /// `{home}` the user's folder (on Android each storage volume), `{appdata}` Windows'
    /// roaming folder, `{config}` `~/.config`
    pub configs: std::collections::BTreeMap<String, Vec<String>>,
    /// Its folders when no settings file says, the same way
    pub saves_folders: std::collections::BTreeMap<String, Vec<String>>,
    pub states_folders: std::collections::BTreeMap<String, Vec<String>>,
    /// A save's files after the game's name (`.srm`)
    pub saves: Vec<String>,
    /// A state's file after the game's name, `{slot}` its number (`.state{slot}`; slot 0
    /// has none, the automatic one is `.state.auto`), and what goes with it (`.png`)
    pub state: String,
    pub state_extras: Vec<String>,
    /// The core each system starts in when the folders don't say (ES-DE's defaults), by
    /// ES-DE's system folder
    pub cores: std::collections::BTreeMap<String, String>,
}

/// The built-in save profiles.
pub const PROFILES: &[&str] = &[include_str!("../saves/retroarch.json")];

pub fn built_in() -> Vec<Profile> {
    PROFILES.iter().filter_map(|t| serde_json::from_str(t).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_unit_reads_back_from_its_id() {
        for u in [Unit::save("snes/Super Mario World (USA)", "retroarch"), Unit::state("gba/A|B (Europe)", "retroarch:mGBA", "auto")] {
            assert_eq!(Unit::from_id(&u.id()), Some(u.clone()));
        }
        assert_eq!(Unit::state("x/y", "retroarch:Snes9x", "3").label(), "State 3 (RetroArch, Snes9x)");
        assert_eq!(Unit::from_id("nothing"), None);
    }

    #[test]
    fn the_built_in_profiles_read() {
        let p = built_in();
        assert_eq!(p.len(), PROFILES.len(), "each built-in profile is good JSON");
        let ra = p.iter().find(|p| p.id == "retroarch").unwrap();
        assert!(ra.saves.contains(&".srm".to_string()) && ra.cores.contains_key("snes"));
        for os in ["windows", "linux", "android"] {
            assert!(!ra.configs[os].is_empty(), "{os}");
        }
    }

    #[test]
    fn files_are_the_same_by_name_and_hash() {
        let f = |n: &str, s: &str, m: i64| SaveFile { name: n.into(), sha1: s.into(), size: 1, modified: m };
        assert!(same_files(&[f("a", "1", 1), f("b", "2", 1)], &[f("b", "2", 9), f("a", "1", 5)]));
        assert!(!same_files(&[f("a", "1", 1)], &[f("a", "2", 1)]));
    }
}
