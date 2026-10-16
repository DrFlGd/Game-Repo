//! Core of the Game Repo desktop app, kept free of any GUI so it builds and tests
//! on its own (and powers `gamerepo-cli`, including `serve`: the app's backend
//! over HTTP, used by the tests).
//!
//! - [`config`]: what the app keeps on this computer (open library, preferences)
//! - [`library`]: the portable library folder
//! - [`index`]: the SQLite index of the library, kept on this computer and rebuilt from the files
//! - [`roots`]: the folders with games, kept in the library
//! - [`platforms`]: the platforms the app knows, and how DATs and files find theirs
//! - [`hash`]: hashing a file in one read (and without its header)
//! - [`scan`]: scanning the folders with games into the index
//! - [`dat`]: reading and importing DAT files
//! - [`audit`]: matching files with DATs, and what an audit shows
//! - [`makedat`]: making a DAT from a scanned set
//! - [`starter`]: starter DATs downloaded from libretro's copy of No-Intro's
//! - [`api`]: the commands the page calls
//!
//! The layout, the phases and every design decision are in docs/PLAN.md.

pub mod api;
pub mod audit;
pub mod config;
pub mod dat;
pub mod hash;
pub mod index;
pub mod library;
pub mod makedat;
pub mod platforms;
pub mod roots;
pub mod scan;
pub mod starter;

pub use library::Library;

/// The app's version. The repository keeps `<major>.<minor>.0`; release builds
/// get their number from CI (tools/set_version.py), one higher each release.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
