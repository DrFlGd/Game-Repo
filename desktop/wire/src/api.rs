//! Game Repo Server's API (docs/PLAN.md, "Phase 9a design", D91–D95): what each request
//! sends and gets back, as JSON over HTTPS under `/v1/`, and the record a device keeps of
//! what it holds. The server and the clients both use these types, so they can't drift.

use serde::{Deserialize, Serialize};

/// The API's version: a client refuses a server with a newer one ("update the client").
pub const API: u32 = 1;

/// The DNS-SD service a server announces on the home network (D91).
pub const SERVICE: &str = "_gamerepo._tcp.local.";

/// The port a server listens on unless it's set otherwise.
pub const PORT: u16 = 47210;

/// Files sent to one device at once, at most (D95); a client fetches three at a time.
pub const DOWNLOADS: usize = 4;

/// `GET /v1/info`, the one request needing no token.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Info {
    /// The server's own id, kept from its first start
    pub server: String,
    /// The library's name
    pub library: String,
    /// Game Repo Server's version
    pub version: String,
    pub api: u32,
}

impl Info {
    /// The TXT record a server announces with DNS-SD (D91), as key and value.
    pub fn txt(&self) -> Vec<(String, String)> {
        vec![
            ("id".into(), self.server.clone()),
            ("library".into(), self.library.clone()),
            ("version".into(), self.version.clone()),
            ("api".into(), self.api.to_string()),
        ]
    }

    /// An announcement's TXT record read back. None: not a Game Repo Server's.
    pub fn from_txt(txt: &[(String, String)]) -> Option<Self> {
        let get = |k: &str| txt.iter().find(|(key, _)| key.eq_ignore_ascii_case(k)).map(|(_, v)| v.clone());
        Some(Self { server: get("id")?, library: get("library").unwrap_or_default(), version: get("version").unwrap_or_default(), api: get("api")?.parse().ok()? })
    }
}

/// `POST /v1/pair`: the six-digit code the server shows, with the device's name and kind.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PairAsk {
    pub code: String,
    pub name: String,
    /// android | windows | linux | steamdeck | test
    pub kind: String,
}

/// The answer to a pairing: the token every later request carries.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Paired {
    pub token: String,
    /// The device's id on the server
    pub device: String,
    pub server: String,
}

/// An error, as every request that fails answers.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Problem {
    pub error: String,
}

/// `GET /v1/systems`: a system with games.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct System {
    pub id: String,
    pub name: String,
    /// Its folder in ES-DE's ROMs folder
    pub folder: String,
    pub games: u64,
    pub size: u64,
    /// The extensions of the files ES-DE lists as its games (`GET /v1/systems?all=1`), so
    /// a device tells its games from what's beside them (D100)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<String>,
}

/// `GET /v1/collections`: a collection, with its groups (D75).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Collection {
    pub id: String,
    pub name: String,
    pub games: u64,
    pub size: u64,
    pub groups: Vec<Group>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Group {
    /// "" for the first, the collection's own
    pub id: String,
    pub name: String,
    pub games: u64,
    pub size: u64,
}

/// A game as the list gives it.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Game {
    /// The Library's key (`<system>/<DAT name>`; a game on several discs, its first disc's)
    pub key: String,
    pub system: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub year: Option<i64>,
    /// Its files' size (every disc's, for a game on several)
    pub size: u64,
    /// The media kinds it has (cover, screenshot, manual…)
    pub media: Vec<String>,
    /// Its picture as `GET /v1/pictures/<key>/<kind>`'s kind; none: no picture
    #[serde(skip_serializing_if = "Option::is_none")]
    pub picture: Option<String>,
    /// Its discs, for a game on several (D94)
    #[serde(skip_serializing_if = "is_zero")]
    pub discs: u32,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// `GET /v1/games`: a page of games.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Page {
    pub items: Vec<Game>,
    pub total: u64,
    pub offset: u64,
    /// The catalogue's change marker now (for `GET /v1/changes?since=`)
    pub rev: i64,
}

/// `GET /v1/changes?since=`: the games changed since a marker and the keys gone, a page at
/// a time (`more`: ask again with `offset` moved on).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Changes {
    pub items: Vec<Game>,
    pub gone: Vec<String>,
    pub rev: i64,
    pub more: bool,
}

/// `POST /v1/plans`: what a device asks for (D93).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PlanAsk {
    /// The games' keys
    pub games: Vec<String>,
    /// The media kinds wanted (cover, screenshot…)
    pub media: Vec<String>,
    pub manuals: bool,
    /// The details put in the gamelist (description, developer…; empty: the layout's)
    pub fields: Vec<String>,
    /// The device's longest screen side: pictures larger are made smaller (0: as they are)
    pub size: u32,
    /// The layout (empty: the built-in `esde`)
    pub layout: String,
    /// An ES-DE custom collection they're put in too (empty: none)
    pub collection: String,
    /// The BIOS files their systems need, those the library has (D40)
    pub bios: bool,
    /// Only the games' media and manuals, for games the device has already (D102)
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub media_only: bool,
    /// Games the device has under names of its own (a game from elsewhere the library knows,
    /// D102): each game's key and its path in the device's ROMs folder, so its media and
    /// entry are named for the device's file
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub named: Vec<(String, String)>,
}

/// A media kind's size for the games asked for, asked for or not (C13, D88), so a device
/// can show them before the user picks.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct KindSize {
    pub kind: String,
    /// The games with one
    pub games: u64,
    pub size: u64,
    /// Pictures not asked for are reckoned at the device's size, not made
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub estimated: bool,
}

/// `PUT /v1/device`: what a device holds, said when it connects (S3).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Holding {
    pub games: u64,
    pub size: u64,
    /// Free space where its games go
    pub free: u64,
}

/// `POST /v1/identify`: files on a device (C18), each by name and size with its CRC32s (a
/// ZIP's members' from its directory) or a CHD's SHA-1 from its header.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Found {
    pub name: String,
    pub size: u64,
    /// Each ROM's CRC32 and size (a loose file's own, a ZIP's members')
    pub crcs: Vec<(String, u64)>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chd_sha1: Option<String>,
}

/// The library's game for a file found, if one.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Identified {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game: Option<String>,
}

/// What a device holds from the server, kept at `ES-DE/gamerepo/device.json` (D93): each
/// game with its files, so it can be removed with them, and the sets games share kept
/// while one needs them.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Record {
    pub format: u32,
    pub server: String,
    pub games: Vec<Held>,
}

/// A game a device holds.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Held {
    pub key: String,
    pub title: String,
    pub system: String,
    /// When it was added (seconds since 1970)
    pub added: u64,
    pub files: Vec<HeldFile>,
    /// Its entries in the gamelists (the list's place and path, and the entry's path)
    pub entries: Vec<(String, String, String)>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct HeldFile {
    pub place: String,
    pub path: String,
    pub size: u64,
    pub sha1: String,
    pub role: String,
    /// For a BIOS set or device another game may need too: the games that need it
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub needed_by: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_servers_announcement_reads_back() {
        let info = Info { server: "srv-1".into(), library: "Retro & More".into(), version: "0.6.0".into(), api: API };
        assert_eq!(Info::from_txt(&info.txt()), Some(info));
        assert_eq!(Info::from_txt(&[("id".into(), "x".into())]), None, "no API: not ours");
    }
}
