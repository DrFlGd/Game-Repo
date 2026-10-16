//! Core of the Game Repo desktop app, kept free of any GUI so it builds and tests
//! on its own (and powers `gamerepo-cli`, including `serve`: the app's backend
//! over HTTP, used by the tests).
//!
//! - [`config`]: what the app keeps on this computer (open library, preferences)
//! - [`library`]: the portable library folder
//! - [`index`]: the SQLite index of the library, kept on this computer and rebuilt from the files
//! - [`api`]: the commands the page calls
//!
//! The layout, the phases and every design decision are in docs/PLAN.md.

pub mod api;
pub mod config;
pub mod index;
pub mod library;

pub use library::Library;

/// The app's version. The repository keeps `<major>.<minor>.0`; release builds
/// get their number from CI (tools/set_version.py), one higher each release.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
