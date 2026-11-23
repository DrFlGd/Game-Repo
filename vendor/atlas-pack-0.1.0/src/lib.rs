//! Atlas pack files (`.atlaspack`): what another app needs to show one
//! without unpacking it.
//!
//! A pack file is one game's pack folder in a ZIP archive. Its first entry,
//! `atlaspack.json`, is the description ([`Header`]): which game, its maps
//! and its pages, where its data came from and when, and which app made the
//! file. From version 7 of the format, a small picture of each map
//! (`preview/<name>.jpg`) follows directly after it, before everything else,
//! so a card for the file can be shown by reading the start of the file only.
//!
//! This crate holds the description's types, the format's constants, the
//! checks a reader makes ([`parse_header`]), and two bounded reads of a file
//! on disk or of any stream read in order: the description
//! ([`read_header`], [`read_header_from`]) and one small picture by its
//! entry name ([`read_preview`], [`read_preview_from`]). It has no app code
//! and makes no network request. A file that is not read is refused with a
//! [`Refusal`]: a sentence to show, and its kind, so an app can add its own
//! advice (a newer format needs a newer version of the app that reads it,
//! whichever app that is). The Atlas app writes and imports pack files
//! with these same types. See `docs/pack-file-format.md` in the Atlas
//! repository for the format.
//!
//! # Writing a pack of guidebooks
//!
//! A guidebook is a published guide a person has as one file, a PDF or a
//! CBZ (a ZIP of pictures of pages). From version 8 of the format, a pack
//! file can hold one game's guidebooks, in a pack of their own, so a large
//! pack of maps is never written again to add one. Another program writes
//! one with [`write_guidebook_pack`]: the pack id, the game's title, the
//! app that writes it, and each guidebook's file, title and (when known)
//! the text of each page. The file is written as `<target>.part` and
//! renamed to `target` when it is complete, so put it in Atlas's packs
//! folder under its `.atlaspack` name and Atlas reads it where it lies.
//! [`rewrite_guidebook_pack`] adds guidebooks to such a pack, or takes some
//! out; Atlas changes only the packs it wrote itself, which it tells by
//! `pack.writtenBy`.
//!
//! ```no_run
//! use std::path::{Path, PathBuf};
//!
//! use atlas_pack::{write_guidebook_pack, AppInfo, GuidebookPack, NewGuidebook};
//!
//! fn write(packs_folder: &Path) -> Result<(), String> {
//!     let pack = GuidebookPack {
//!         pack_id: "guidebooks-hollow-marsh".to_owned(),
//!         game_title: "Hollow Marsh".to_owned(),
//!         written_by: AppInfo {
//!             name: "Fen Shelf".to_owned(),
//!             version: "1.0.0".to_owned(),
//!             platform: "windows".to_owned(),
//!         },
//!         guidebooks: vec![NewGuidebook {
//!             file: PathBuf::from("Hollow Marsh Field Guide.pdf"),
//!             title: "Hollow Marsh Field Guide".to_owned(),
//!             // Counted from the file.
//!             pages: None,
//!             // The text of each page, for search, when the program has it.
//!             text: None,
//!         }],
//!     };
//!     let header = write_guidebook_pack(&pack, &packs_folder.join("Hollow Marsh guidebooks.atlaspack"))?;
//!     println!("{} guidebooks", header.guidebooks.len());
//!     Ok(())
//! }
//! ```

//! # Writing a pack of magazines
//!
//! From version 10, a pack file can hold the issues of one magazine, each a
//! PDF or a CBZ with the text of its pages, a small JPEG of its cover and
//! the games it covers with their pages. An app that keeps magazines
//! writes one per magazine title with [`write_magazine_pack`], under the
//! id `magazines-<name>`; Atlas only reads it. An app that reads version 9
//! at most refuses it as a file in a newer format.

mod guidebook;
mod magazine;
mod magazine_write;
mod pdf;
mod read;
mod write;

#[cfg(test)]
mod guidebook_tests;
#[cfg(test)]
mod magazine_tests;
#[cfg(test)]
mod tests;

pub use guidebook::{
    guidebook_name_of, is_guidebook_name, parse_guidebook_text, text_name_of, GuidebookFacts,
    GuidebookList, GuidebookText, GuidebookType, PageText, GUIDEBOOK_DIR, GUIDEBOOK_LIST_FILE,
    GUIDEBOOK_LIST_FORMAT, GUIDEBOOK_TEXT_FORMAT, GUIDEBOOK_TEXT_VERSION, MAX_GUIDEBOOKS,
    MAX_GUIDEBOOK_BYTES, MAX_GUIDEBOOK_NAME, MAX_GUIDEBOOK_PAGES, MAX_GUIDEBOOK_TEXT_BYTES,
    MAX_GUIDEBOOK_TITLE,
};
pub use magazine::{
    is_issue_date, is_issue_name, issue_cover_name, issue_name_of, issue_text_name, IssueFacts,
    IssueGame, MagazineFacts, MagazineList, COVER_SIDE, MAGAZINE_DIR, MAGAZINE_LIST_FILE,
    MAGAZINE_LIST_FORMAT, MAGAZINE_PACK_PREFIX, MAX_COVER_BYTES, MAX_GAME_PAGE_RUNS, MAX_ISSUES,
    MAX_ISSUE_GAMES, MAX_MAGAZINE_NAME, MAX_MAGAZINE_TITLE,
};
pub use magazine_write::{write_magazine_pack, MagazinePack, NewIssue};
pub use read::{read_header, read_header_from, read_preview, read_preview_from};
pub use write::{
    guidebook_pages, rewrite_guidebook_pack, write_guidebook_pack, GuidebookPack, NewGuidebook,
    WriteError, MAX_CBZ_PICTURES,
};

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `format` of the description.
pub const FORMAT: &str = "atlas-pack-file";
/// The newest version of the format this crate reads. A file is written at
/// the lowest version that can hold what it carries, so an older app reads
/// every file it can, and refuses a newer one with a message.
pub const VERSION: u32 = 10;
/// A file with the issues of a magazine (`magazines/`, see
/// [`IssueFacts`]). Only a pack of magazines is written in it: every other
/// file stays at the version of what else it carries, so an app made
/// before it reads it, and refuses a pack of magazines as a file in a newer
/// format.
pub const VERSION_WITH_MAGAZINES: u32 = 10;
/// A file with routes (`routes.json`, or the person's own under `mine/`)
/// or with pictures of its types of location. Only a file that carries one
/// of them is written in it: every other file stays at the version of what
/// else it carries, so an app made before it reads it.
pub const VERSION_WITH_ROUTES: u32 = 9;
/// A file with guidebooks (`guidebooks/`, see [`GuidebookFacts`]). Only a
/// file that carries guidebooks is written in it: every other file stays at
/// the version of what else it carries, so an app made before it reads it.
pub const VERSION_WITH_GUIDEBOOKS: u32 = 8;
/// A file with small pictures of its maps (`preview/`).
pub const VERSION_WITH_PREVIEWS: u32 = 7;
/// A file with the title the game is shown under with other games
/// (`mine/game-group.json`) and no small pictures.
pub const VERSION_WITH_TITLE: u32 = 6;
/// A file with a person's notes, or a guide page linked to one location
/// alone, but no title.
pub const VERSION_WITH_NOTES: u32 = 5;
/// A file with a person's own pages but no notes.
pub const VERSION_WITH_PAGES: u32 = 4;
/// A file with a person's photos or guide links but no own pages.
pub const VERSION_WITH_PHOTOS: u32 = 3;
/// A file with a person's own lists, locations and types only.
pub const VERSION_WITH_MINE: u32 = 2;
/// A file without anything of a person's own, which every app with pack
/// files reads.
pub const VERSION_WITHOUT_MINE: u32 = 1;
/// The description, the file's first entry.
pub const HEADER_FILE: &str = "atlaspack.json";
/// The file name extension, without the dot.
pub const EXTENSION: &str = "atlaspack";
/// A description larger than this is not read.
pub const MAX_HEADER_BYTES: u64 = 4 << 20;

/// Where the small pictures of the maps are: `preview/<name>.jpg`.
pub const PREVIEW_DIR: &str = "preview/";
/// The longest side of a small picture, in pixels.
pub const PREVIEW_SIDE: u32 = 512;
/// A small picture larger than this is not one.
pub const MAX_PREVIEW_BYTES: u64 = 512 << 10;
/// The most small pictures one file holds.
pub const MAX_PREVIEWS: usize = 1024;
/// The longest name of a small picture, without `preview/` and `.jpg`.
pub const MAX_PREVIEW_NAME: usize = 80;

/// Whether `id` can be a pack's id (`packId`): 1 to 64 characters of lower
/// case letters, digits, `-` and `_`, starting with a letter or a digit.
/// It names the pack in addresses and file names, and a pack is known by
/// it, so a description with another id, or none, is refused.
pub fn is_pack_id(id: &str) -> bool {
    let mut chars = id.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_'))
        && id.len() <= 64
}

/// Whether `name` is an entry name a small picture may have:
/// `preview/<name>.jpg`, the name 1 to [`MAX_PREVIEW_NAME`] characters of
/// lower case letters, digits, `-` and `_`, starting with a letter or a
/// digit. Lower case only, so no two names differ only by case.
pub fn is_preview_name(name: &str) -> bool {
    let Some(stem) = name
        .strip_prefix(PREVIEW_DIR)
        .and_then(|rest| rest.strip_suffix(".jpg"))
    else {
        return false;
    };
    (1..=MAX_PREVIEW_NAME).contains(&stem.len())
        && stem.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && stem
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// How much of a map's pyramid of images a file holds.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Detail {
    /// Two levels short of the top: a quick download, blurry close up.
    Overview,
    /// One level short of the top: about a quarter of the full download.
    Standard,
    /// Every level.
    Full,
}

impl Detail {
    pub const ALL: [Detail; 3] = [Detail::Overview, Detail::Standard, Detail::Full];

    /// Levels left out at the top of each layer.
    pub fn levels_short(self) -> u8 {
        match self {
            Detail::Overview => 2,
            Detail::Standard => 1,
            Detail::Full => 0,
        }
    }
}

/// The app that wrote a file.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    /// `windows`, `linux`, `macos`, `android`, …
    #[serde(default)]
    pub platform: String,
}

// ---------------------------------------------------------------------------
// The description

/// `atlaspack.json`: what a pack file holds, readable without unpacking
/// anything else. Every date is ISO 8601 in UTC. Fields added after
/// version 1 are optional: an older file has none of them, and an older
/// reader ignores them.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Header {
    pub format: String,
    pub version: u32,
    pub pack_id: String,
    pub title: String,
    /// The game, as it is shown with other games.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game: Option<GameFacts>,
    pub pack_version: String,
    pub detail: Detail,
    pub exported_at: String,
    pub exported_by: AppInfo,
    pub pack: PackFacts,
    /// The pack's manifest says it is kept for the person's own use (a page
    /// of it was read under a licence the person filled in, or with none
    /// recorded). Written only when true.
    #[serde(default, skip_serializing_if = "is_false")]
    pub personal_use: bool,
    pub source: SourceFacts,
    pub contents: Counts,
    /// Each map of the pack, in the order of its `maps.json`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub maps: Vec<MapFacts>,
    /// The pages of the pack, as a guide or a wiki (at most one entry: a
    /// pack's pages are one source of pages).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub guides: Vec<GuideFacts>,
    /// The entry name of the small picture to show for the file: that of
    /// the first map that has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<String>,
    pub images: ImagesFacts,
    /// Entries after the description, and their size unpacked.
    pub files: u64,
    pub bytes: u64,
    /// How many of a person's own records the file carries, when it carries
    /// them (version 2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mine: Option<MineFacts>,
    /// The guidebooks the file carries, each a PDF or CBZ file with the
    /// text of its pages (version 8). [`parse_header`] checks them and
    /// leaves them out of a file of an older version.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub guidebooks: Vec<GuidebookFacts>,
    /// The magazine whose issues the file carries (version 10).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub magazine: Option<MagazineFacts>,
    /// The issues the file carries, each a PDF or CBZ file with the text
    /// of its pages, its cover and the games it covers (version 10).
    /// [`parse_header`] checks them and leaves them out of a file of an
    /// older version.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub issues: Vec<IssueFacts>,
}

fn is_false(value: &bool) -> bool {
    !value
}

impl Header {
    /// The entry names of the small pictures the description lists: each
    /// map's and the cover, each once.
    pub fn previews(&self) -> Vec<&str> {
        let mut names: Vec<&str> = Vec::new();
        for name in self
            .maps
            .iter()
            .filter_map(|map| map.preview.as_deref())
            .chain(self.cover.as_deref())
        {
            if !names.contains(&name) {
                names.push(name);
            }
        }
        names
    }

    /// Where the pack came from, in one word: `source.origin` when the
    /// file says, else what older files say of it: a file that carries
    /// guidebooks is [`Origin::Guidebooks`], one that carries issues of a
    /// magazine [`Origin::Magazines`], one read from web pages
    /// (`source.kind` `web-pages`) [`Origin::Site`], one whose pack was
    /// written by Atlas Map Maker (`pack.writtenBy.name`)
    /// [`Origin::MapMaker`], and another one an Atlas app made
    /// (`exportedBy.name` is `Atlas` or starts with `Atlas `)
    /// [`Origin::MadeInAtlas`]. None when nothing says.
    pub fn origin(&self) -> Option<Origin> {
        if let Some(origin) = self.source.origin.filter(|origin| *origin != Origin::Other) {
            return Some(origin);
        }
        if !self.guidebooks.is_empty() {
            return Some(Origin::Guidebooks);
        }
        if !self.issues.is_empty() {
            return Some(Origin::Magazines);
        }
        if self.source.kind == SourceKind::WebPages {
            return Some(Origin::Site);
        }
        let written_by = self
            .pack
            .written_by
            .as_ref()
            .and_then(|by| by.get("name"))
            .and_then(Value::as_str);
        if written_by == Some(MAP_MAKER) {
            return Some(Origin::MapMaker);
        }
        let by = self.exported_by.name.as_str();
        (by == "Atlas" || by.starts_with("Atlas ")).then_some(Origin::MadeInAtlas)
    }

    /// Whether the description lists `name` as a small picture.
    pub fn lists_preview(&self, name: &str) -> bool {
        self.maps
            .iter()
            .any(|map| map.preview.as_deref() == Some(name))
            || self.cover.as_deref() == Some(name)
    }
}

/// The game a file is of, as a list of games shows it.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GameFacts {
    /// The title the game is shown under with other games when it is in a
    /// group, whether or not the file carries that group
    /// (`mine/game-group.json`); otherwise the pack's own title. Two packs
    /// of one game, read from two sites and grouped, give the same title.
    pub title: String,
    /// The system the game is for, as the pack's manifest names it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
}

/// The pack itself, from its manifest.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PackFacts {
    pub id: String,
    pub title: String,
    pub pack_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_patch: Option<String>,
    pub schema_version: u32,
    #[serde(default)]
    pub attribution: Vec<Value>,
    /// When the pack was first written and last changed, if its manifest
    /// says (version 2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
    /// The tool or app that wrote the pack's files, as its manifest says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub written_by: Option<Value>,
}

/// Where the game's data came from.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SourceKind {
    /// Made from web pages with an importer ("Add a map").
    WebPages,
    /// The pack comes with the app that wrote the file.
    App,
    /// Nothing is recorded.
    #[default]
    Unknown,
}

/// The name Atlas Map Maker writes in a pack's manifest (`writtenBy.name`).
pub const MAP_MAKER: &str = "Atlas Map Maker";

/// Where a pack came from, in one word (`source.origin`), so another app
/// reads one value. Written by every Atlas app from version 0.25.0, in
/// every version of the format: a reader of an older version ignores it.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum Origin {
    /// Read from a website by the Importer (`site`).
    Site,
    /// Made in an Atlas app from a picture (`made-in-atlas`).
    MadeInAtlas,
    /// Made by Atlas Map Maker, whose manifest's `writtenBy.name` is
    /// exactly `Atlas Map Maker` (`map-maker`).
    MapMaker,
    /// A pack of guidebooks (`guidebooks`).
    Guidebooks,
    /// A pack of the issues of a magazine (`magazines`, version 10).
    Magazines,
    /// A word this crate does not know (from a newer file); see
    /// [`Header::origin`].
    #[serde(other)]
    Other,
}

impl Origin {
    /// The word as the description writes it.
    pub fn as_str(self) -> &'static str {
        match self {
            Origin::Site => "site",
            Origin::MadeInAtlas => "made-in-atlas",
            Origin::MapMaker => "map-maker",
            Origin::Guidebooks => "guidebooks",
            Origin::Magazines => "magazines",
            Origin::Other => "other",
        }
    }
}

/// Reads `source.origin` loosely: anything but a text is none, and a text
/// this crate does not know is [`Origin::Other`], so a value from a newer
/// file never makes the description unreadable.
fn loose_origin<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Origin>, D::Error> {
    let value = Option::<Value>::deserialize(deserializer)?;
    Ok(value.as_ref().and_then(Value::as_str).map(|text| {
        serde_json::from_value(Value::String(text.to_owned())).unwrap_or(Origin::Other)
    }))
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SourceFacts {
    pub kind: SourceKind,
    /// Where the pack came from, in one word (see [`Origin`] and
    /// [`Header::origin`], which also reads a file without it).
    #[serde(
        default,
        deserialize_with = "loose_origin",
        skip_serializing_if = "Option::is_none"
    )]
    pub origin: Option<Origin>,
    /// The name of the site the pages were read from: the name the pack's
    /// manifest records (`source.site`), else the site its guide pages name
    /// most, else, for a pack read from web pages, the host of the first
    /// page it records without a leading `www.`. Never the importer's
    /// title. Absent when there is none of these.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub importer: Option<ImporterFacts>,
    /// Each page the pack was made from, with when it was read.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pages: Vec<PageFacts>,
    /// The latest and earliest time a page was read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub newest_page_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oldest_page_at: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImporterFacts {
    pub id: String,
    pub title: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PageFacts {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub map_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imported_at: Option<String>,
}

/// How much is in the pack.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct Counts {
    pub maps: u64,
    pub layers: u64,
    pub locations: u64,
    pub categories: u64,
    pub regions: u64,
    pub checklists: u64,
    pub guides: u64,
    /// Types of location drawn with a picture of their own that the file
    /// carries (the icons of the map the pack was read from, kept for the
    /// person's own use). Optional: absent when there are none, and in a
    /// file made before it.
    #[serde(skip_serializing_if = "is_zero")]
    pub type_pictures: u64,
}

/// One map of the pack.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MapFacts {
    pub id: String,
    pub name: String,
    /// The size of its most detailed image, in pixels.
    pub width: u32,
    pub height: u32,
    /// Its locations in the pack.
    pub locations: u64,
    /// The entry name of its small picture (`preview/<name>.jpg`), when the
    /// file holds one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
}

/// How a pack's pages are read.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum GuideType {
    /// Read in order: each page leads on to the next.
    Guide,
    /// Read as a collection of pages.
    Wiki,
    /// A type this crate does not know (from a newer file).
    #[serde(other)]
    Other,
}

/// The pages of a pack, as one guide or one wiki.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GuideFacts {
    pub title: String,
    #[serde(rename = "type")]
    pub guide_type: GuideType,
    /// Its pages.
    pub pages: u64,
}

/// How many of a person's own records a file carries.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct MineFacts {
    pub lists: u64,
    pub locations: u64,
    pub types: u64,
    pub areas: u64,
    /// Their own photos.
    pub photos: u64,
    /// Whether their links between guide pages and the map are in it.
    pub guide_links: bool,
    /// Their own guide pages, and the pack's pages they changed or hid
    /// (version 4).
    pub pages: u64,
    /// Their own sections of the guide pages.
    pub sections: u64,
    /// Their notes on the game's locations (version 5).
    pub notes: u64,
    /// Whether the title the game is shown under with other games is in it
    /// (version 6).
    pub group: bool,
    /// Their own routes (version 9).
    #[serde(skip_serializing_if = "is_zero")]
    pub routes: u64,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

/// The map images in the file.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ImagesFacts {
    pub detail: Detail,
    pub files: u64,
    pub bytes: u64,
    pub layers: Vec<LayerFacts>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LayerFacts {
    pub id: String,
    pub name: String,
    pub map_name: String,
    pub min_zoom: u8,
    pub max_zoom: u8,
    /// The pyramid levels included.
    pub levels: Vec<u8>,
    pub files: u64,
    pub bytes: u64,
    /// Every level up to the detail's top is included.
    pub complete: bool,
    /// Every level of the layer is included (nothing left out at the top).
    pub full: bool,
    /// Its images can be downloaded from a remote source, so the detail
    /// limits it. A layer whose images all came with the pack is included
    /// whole at every detail.
    pub downloadable: bool,
    /// When its images were last downloaded (its status file's time).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downloaded_at: Option<String>,
}

// ---------------------------------------------------------------------------
// Reading a description

const NOT_A_PACK_FILE: &str = "This file is not an Atlas pack file.";

/// What kind of refusal a [`Refusal`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RefusalKind {
    /// The file is not a pack file at all.
    NotAPackFile,
    /// The file was made in a newer version of the format than this crate
    /// reads: the app that uses the crate needs a newer version of itself.
    NewerFormat,
    /// The file is a pack file, but its description or its small pictures
    /// are damaged or cannot be used.
    Damaged,
    /// The file could not be opened.
    CannotOpen,
    /// The file has no small picture of the name asked for.
    NoSuchPicture,
}

/// Why a pack file, or a small picture of one, is not read. Shown as it is
/// (`Display`, or `String::from`), it is one or two plain sentences. An app
/// that wants to give its own advice shows [`Refusal::what`] and adds to it
/// by [`Refusal::kind`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    kind: RefusalKind,
    what: String,
    advice: Option<&'static str>,
}

/// The advice this crate gives for a file in a newer format. It names no
/// app, as the app that reads the file is the one to renew.
const NEWER_FORMAT_ADVICE: &str = "A newer version of this app is needed to open it.";

impl Refusal {
    fn new(kind: RefusalKind, what: impl Into<String>) -> Self {
        Self {
            kind,
            what: what.into(),
            advice: None,
        }
    }

    fn not_a_pack_file() -> Self {
        Self::new(RefusalKind::NotAPackFile, NOT_A_PACK_FILE)
    }

    fn damaged(what: impl Into<String>) -> Self {
        Self::new(RefusalKind::Damaged, what)
    }

    /// What kind of refusal this is.
    pub fn kind(&self) -> RefusalKind {
        self.kind
    }

    /// What is wrong, in one sentence, without this crate's advice.
    pub fn what(&self) -> &str {
        &self.what
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.what)?;
        if let Some(advice) = self.advice {
            write!(f, " {advice}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Refusal {}

impl From<Refusal> for String {
    fn from(refusal: Refusal) -> Self {
        refusal.to_string()
    }
}

/// Reads a description from its JSON text: refuses another format, a
/// newer version with a message naming the app that wrote it, and a
/// description without the fields every pack file has, or with a pack id
/// that cannot be one (see [`is_pack_id`]). The names of small pictures
/// that are not safe entry names (see [`is_preview_name`]) are left out,
/// so a reader never meets one.
pub fn parse_header(text: &str) -> Result<Header, Refusal> {
    let not_ours = Refusal::not_a_pack_file;
    let value: Value =
        serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|_| not_ours())?;
    if value.get("format").and_then(Value::as_str) != Some(FORMAT) {
        return Err(not_ours());
    }
    let version = value.get("version").and_then(Value::as_u64).unwrap_or(0);
    if version > u64::from(VERSION) {
        let by = value.get("exportedBy");
        let name = by
            .and_then(|by| by.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("Atlas");
        let app_version = by
            .and_then(|by| by.get("version"))
            .and_then(Value::as_str)
            .unwrap_or("a newer version");
        return Err(Refusal {
            kind: RefusalKind::NewerFormat,
            what: format!(
                "This pack file was made by {} {} in a newer format (version {version}).",
                clean(name),
                clean(app_version)
            ),
            advice: Some(NEWER_FORMAT_ADVICE),
        });
    }
    if version < u64::from(VERSION_WITHOUT_MINE) {
        return Err(not_ours());
    }
    let mut header: Header = serde_json::from_value(value).map_err(|error| {
        Refusal::damaged(format!("This pack file's description is damaged: {error}."))
    })?;
    if !is_pack_id(&header.pack_id) {
        return Err(Refusal::damaged(
            "This pack file names a game id that cannot be used.",
        ));
    }
    for map in &mut header.maps {
        if map
            .preview
            .as_deref()
            .is_some_and(|name| !is_preview_name(name))
        {
            map.preview = None;
        }
    }
    if header
        .cover
        .as_deref()
        .is_some_and(|name| !is_preview_name(name))
    {
        header.cover = None;
    }
    // Guidebooks only from the version that has them.
    if header.version < VERSION_WITH_GUIDEBOOKS {
        header.guidebooks.clear();
    } else {
        guidebook::check_guidebooks(&header.guidebooks).map_err(Refusal::damaged)?;
    }
    // Magazines only from the version that has them.
    if header.version < VERSION_WITH_MAGAZINES {
        header.magazine = None;
        header.issues.clear();
    } else {
        magazine::check_issues(header.magazine.as_ref(), &header.issues)
            .map_err(Refusal::damaged)?;
    }
    Ok(header)
}

/// A text from a file made safe to show in one line: line breaks become
/// spaces, other control characters and those that change the direction of
/// text are left out.
fn clean(text: &str) -> String {
    text.chars()
        .filter_map(|c| match c {
            '\n' | '\r' | '\t' => Some(' '),
            c if c.is_control() => None,
            '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' => None,
            c => Some(c),
        })
        .collect()
}
