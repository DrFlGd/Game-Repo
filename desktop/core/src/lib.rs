//! Core of the Game Repo desktop app, kept free of any GUI so it builds and tests
//! on its own (and powers `gamerepo-cli`, including `serve`: the app's backend
//! over HTTP, used by the tests).
//!
//! - [`config`]: what the app keeps on this computer (open library, preferences)
//! - [`library`]: the portable library folder
//! - [`index`]: the SQLite index of the library, kept on this computer and rebuilt from the files
//! - [`roots`]: the folders with games, kept in the library
//! - [`platforms`]: the platforms the app knows, and how DATs and files find theirs
//! - [`systems`]: systems added by hand, kept in the library
//! - [`hash`]: hashing a file in one read (and without its header)
//! - [`archive`]: the archives a scan looks inside (ZIP, 7z, RAR)
//! - [`image`]: disc images read as what they hold (CHD, CSO/ZSO, RVZ), and written
//! - [`cue`]: cue sheets and GDI files
//! - [`tools`]: the external tools some conversions use (dolphin-tool)
//! - [`scan`]: scanning the folders with games into the index
//! - [`dat`]: reading and importing DAT files
//! - [`audit`]: matching files with DATs, and what an audit shows
//! - [`arcade`]: arcade sets (MAME, FBNeo): set types, what a machine needs, playable
//! - [`bios`]: each system's BIOS files, audited from a BIOS DAT, gathered and exported
//! - [`makedat`]: making a DAT from a scanned set
//! - [`organise`]: organising a set by its DAT (copy, rename or move), the journal and Undo
//! - [`convert`]: converting games to CHD, RVZ, CSO/ZSO or ZIP and extracting them, verified
//! - [`collection`]: collections of games from rules, 1G1R, saved as DATs
//! - [`layout`]: export layouts (ES-DE, the devices, a plain folder, the user's own) as JSON files
//! - [`export`]: exporting a collection in a layout, re-sync and the check
//! - [`starter`]: starter DATs downloaded from libretro's copy of No-Intro's
//! - [`torrentzip`]: writing ZIPs that are the same bytes every time
//! - [`watch`]: noticing changes in the folders with games while the app is open
//! - [`sources`]: each source's answer kept apart, the pictures' variants, deleting metadata
//! - [`pcmeta`]: a PC game's details, pictures and documents in the layout every game's has
//! - [`variants`]: the custom DAT of each system's translations, hacks and prototypes, and their tags
//! - [`api`]: the commands the page calls
//!
//! The layout, the phases and every design decision are in docs/PLAN.md.

pub mod addgame;
pub mod addons;
pub mod api;
pub mod arcade;
pub mod archive;
pub mod audit;
pub mod bios;
pub mod browse;
pub mod collection;
pub mod config;
pub mod convert;
pub mod cue;
pub mod dat;
pub mod docs;
pub mod export;
pub mod hash;
pub mod history;
pub mod identify;
pub mod image;
pub mod index;
pub mod launchbox;
pub mod layout;
pub mod library;
pub mod magazines;
pub mod makedat;
pub mod meta;
pub mod organise;
pub mod pcgames;
pub mod pcmeta;
pub mod platforms;
pub mod roots;
pub mod scan;
pub mod scrape;
pub mod scrapers;
pub mod serving;
pub mod sources;
pub mod starter;
pub mod steam;
pub mod subset;
pub mod systems;
pub mod tools;
pub mod torrentzip;
pub mod variants;
pub mod watch;

pub use library::Library;

/// The app's version. The repository keeps `<major>.<minor>.0`; release builds
/// get their number from CI (tools/set_version.py), one higher each release.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
