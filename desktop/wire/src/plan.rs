//! A plan (D93): every file a device writes for some games, each with its place (one of
//! the layout's places, such as ES-DE's `roms`, `esde` and `media`, which the device finds
//! for itself), its path there, its size and what it is, and the entries to merge into the
//! frontend's lists. The server works it out with the export's layout engine; the client
//! writes what it says and never works out a path itself.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Plan {
    pub files: Vec<File>,
    /// Entries for the systems' gamelists, merged into the device's own (D35)
    pub lists: Vec<List>,
    /// Lines for custom collections, added to the device's own
    pub collections: Vec<Lines>,
    /// What can't go, and why
    pub left: Vec<Left>,
    pub notes: Vec<String>,
    /// Every file can be fetched now; else some are still being made, and asking for the
    /// plan again gives them as they're done
    pub ready: bool,
    /// Each media kind's size for these games, asked for or not (C13)
    pub kinds: Vec<crate::api::KindSize>,
    /// The PC games' programs, for their shortcuts (D112)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub programs: Vec<Program>,
    /// PC games whose files the server is still unpacking (D111), so the plan doesn't list
    /// them yet: asking for it again gives them once they're unpacked
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub waiting: Vec<String>,
}

/// A PC game's program (D112), for its Start menu or Steam shortcut.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Program {
    /// The catalogue's key of the game
    pub key: String,
    /// What the shortcut's called (the game's title)
    pub name: String,
    /// Its place (`pc`) and the game's folder there
    pub place: String,
    pub folder: String,
    /// The program in the game's folder, with `/`; empty: the game has none to start, so it
    /// gets no shortcut
    pub path: String,
    pub args: String,
    /// The folder it starts in, in the game's folder (empty: the program's own)
    pub dir: String,
    /// A Windows program (`.exe`, `.bat`, `.cmd`), which runs through Proton on Linux
    pub windows: bool,
    /// Its pictures for Steam's grid: (media kind, the plan file's place and path)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub art: Vec<(String, String, String)>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct File {
    /// The layout's place it goes in (`roms`, `esde`, `media`…)
    pub place: String,
    /// Its path there, with `/`
    pub path: String,
    /// Its size; for a file still to be made (`made`), what it's expected to be
    pub size: u64,
    /// Its SHA-1, when it's known
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha1: Option<String>,
    pub system: String,
    /// The game it's for (its DAT name), or the BIOS set or device it is
    pub game: String,
    /// The catalogue's key of the game it's for (empty for what a system needs, such as its
    /// BIOS files, and for a set games share: see `needed_by`)
    pub key: String,
    /// game | bios | device | playlist | manual | doc | bios-file | text, or a media kind (cover…)
    pub role: String,
    /// For a BIOS set or device: the games of the plan that need it (their catalogue keys),
    /// so it stays while one does
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub needed_by: Vec<String>,
    /// Made by the server rather than sent as the library has it (a set rebuilt as a ZIP,
    /// a ROM taken out of an archive, a cue sheet rewritten, a picture made smaller, a playlist)
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub made: bool,
    /// Where it's fetched from (`/v1/files/…`), with its SHA-1 at the same address with
    /// `?sha1` once it's known
    pub url: String,
}

/// A system's gamelist and the entries to merge into it.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct List {
    pub place: String,
    pub path: String,
    /// es-xml (EmulationStation's gamelist.xml)
    pub kind: String,
    pub system: String,
    pub entries: Vec<Entry>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Entry {
    /// From the gamelist's folder (`Chrono Trigger (USA).zip`, `Final Fantasy VII (USA).m3u`)
    pub path: String,
    /// The name it's listed by
    pub name: String,
    /// The game (its DAT name; a game on several discs, its first disc's)
    pub game: String,
    /// The catalogue's key of the game
    pub key: String,
    /// The details chosen, as the list's (tag, value)
    pub fields: Vec<(String, String)>,
}

/// A file of lines, such as an ES-DE custom collection, and the lines to add to it.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Lines {
    pub place: String,
    pub path: String,
    pub lines: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Left {
    pub what: String,
    pub why: String,
}
