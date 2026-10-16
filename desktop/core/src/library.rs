//! The library folder: one folder the app owns, plain files that make sense
//! without the app, so it can be moved, copied or synced and opened by any build
//! of the app (docs/PLAN.md, section 4, "On-disk layout").
//!
//! ```text
//! library/
//!   _library/library.json         format, id, name, library-wide settings
//!   _library/README.txt           what this folder is, for someone who finds it
//!   _library/games/…              one folder per game: game.json, media/, docs/ (Phase 5)
//!   _library/dats/…               imported DAT files (Phase 1)
//!   _library/journal/…            moves and renames, for Undo (Phase 2)
//!   Nintendo - SNES/…             platform folders, when ROMs are moved in (Phase 2)
//!   PC/…                          imported PC games (Phase 4)
//! ```
//!
//! Folders are made when something first goes in them, so opening a folder that
//! already holds games adds only `_library/`. No database lives here: the index
//! is on each computer and can be rebuilt from the files ([`crate::index`]).

use crate::config::{read_json_object, write_json};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::path::{Component, Path, PathBuf};

/// The library format this build writes. A library with a higher number opens read-only.
pub const FORMAT: u64 = 1;

/// The app's own folder inside a library.
pub const APP_DIR: &str = "_library";

const README: &str = "Game Repo\n\
\n\
This folder is a library for the Game Repo desktop app. The app keeps its own\n\
files in this _library folder; your games stay ordinary files, in the folders\n\
beside it or wherever you keep them, so the library can be moved, copied or\n\
synced and opened again by any version of the app.\n\
\n\
  _library/library.json   the library's name and settings\n\
  _library/games/         each game's details, scraped pictures and your documents\n\
  _library/dats/          the DAT files you imported, by platform and version\n\
  _library/journal/       the moves and renames the app made, so they can be undone\n\
\n\
The app's index of your files is kept on each computer, not here, and is made\n\
again from these files when it's missing.\n";

#[derive(Clone, Debug)]
pub struct Library {
    root: PathBuf,
    /// Made by a newer app (format above [`FORMAT`]): nothing is written.
    read_only: Option<String>,
}

pub fn valid_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 96
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
        || id.starts_with('.')
    {
        bail!("invalid id {id}");
    }
    Ok(())
}

/// A library-relative path ("PC/Quake (1996)/game.json") that stays inside the library.
pub fn rel_inside(rel: &str) -> Result<PathBuf> {
    let p = Path::new(rel.trim_start_matches('/'));
    if rel.is_empty() || p.components().any(|c| !matches!(c, Component::Normal(_))) {
        bail!("invalid library path {rel}");
    }
    Ok(p.to_path_buf())
}

/// "2026-10-05T09:48:26Z" from seconds since 1970.
pub fn iso_from_unix(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // civil_from_days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// The time now, as [`iso_from_unix`] writes it.
pub fn now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    iso_from_unix(secs as i64)
}

fn random_id(prefix: &str) -> String {
    use sha2::{Digest, Sha256};
    let seed = format!(
        "{:?}{}{:?}",
        std::time::SystemTime::now(),
        std::process::id(),
        std::thread::current().id()
    );
    format!(
        "{prefix}{}",
        &hex::encode(Sha256::digest(seed.as_bytes()))[..16]
    )
}

impl Library {
    /// Open a library folder, creating it (and `_library/`) if needed. A folder that
    /// already holds games is left as it is: only `_library/` is added.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root)
            .with_context(|| format!("couldn't create {}", root.display()))?;
        let meta_path = root.join(APP_DIR).join("library.json");
        if meta_path.is_file() {
            let format = read_json_object(&meta_path)["format"].as_u64().unwrap_or(0);
            if format > FORMAT {
                let msg = format!(
                    "This library was made by a newer version of Game Repo (library format {format}; this version reads up to {FORMAT}). \
                     It's open read-only: update the app to change it."
                );
                return Ok(Self {
                    root,
                    read_only: Some(msg),
                });
            }
        }
        let lib = Self {
            root,
            read_only: None,
        };
        let app_dir = lib.root.join(APP_DIR);
        std::fs::create_dir_all(&app_dir)
            .with_context(|| format!("couldn't create {}", app_dir.display()))?;
        if !meta_path.is_file() {
            let name = lib
                .root
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Library".into());
            write_json(
                &meta_path,
                &json!({ "format": FORMAT, "id": random_id("lib-"), "name": name, "created": now() }),
            )?;
        }
        let readme = app_dir.join("README.txt");
        if !readme.exists() {
            let _ = std::fs::write(&readme, README);
        }
        Ok(lib)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn read_only(&self) -> Option<&str> {
        self.read_only.as_deref()
    }

    pub fn writable(&self) -> Result<()> {
        match &self.read_only {
            Some(msg) => bail!("{msg}"),
            None => Ok(()),
        }
    }

    fn meta_path(&self) -> PathBuf {
        self.root.join(APP_DIR).join("library.json")
    }

    pub fn meta(&self) -> Value {
        read_json_object(&self.meta_path())
    }

    /// The library's id ("lib-…"), which its index on each computer is kept under.
    pub fn id(&self) -> Option<String> {
        self.meta()["id"]
            .as_str()
            .filter(|id| valid_id(id).is_ok())
            .map(String::from)
    }

    /// Merge `patch` into library.json (top-level keys; null removes one).
    pub fn update_meta(&self, patch: Value) -> Result<Value> {
        self.writable()?;
        let mut meta = self.meta();
        for (k, v) in patch.as_object().into_iter().flatten() {
            if k == "format" || k == "id" {
                continue;
            }
            if v.is_null() {
                meta.as_object_mut().unwrap().shift_remove(k);
            } else {
                meta[k] = v.clone();
            }
        }
        write_json(&self.meta_path(), &meta)?;
        Ok(meta)
    }

    pub fn info(&self) -> Value {
        let meta = self.meta();
        json!({
            "path": self.root.display().to_string(),
            "id": meta["id"], "name": meta["name"], "format": meta["format"], "created": meta["created"],
            "read_only": self.read_only,
        })
    }

    // ------------------------------------------------------------ paths

    /// The library-relative form of a path inside the library (None if outside).
    pub fn relative(&self, p: &Path) -> Option<String> {
        let rel = p.strip_prefix(&self.root).ok()?;
        Some(
            rel.components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/"),
        )
    }

    pub fn resolve(&self, rel: &str) -> Result<PathBuf> {
        Ok(self.root.join(rel_inside(rel)?))
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("gamerepo-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn dates() {
        assert_eq!(iso_from_unix(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_from_unix(1_791_150_506), "2026-10-04T21:48:26Z");
        assert_eq!(iso_from_unix(951_782_400), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn creates_a_library_and_keeps_its_id() {
        let dir = temp_dir("create").join("My Games");
        let lib = Library::open(&dir).unwrap();
        assert!(dir.join("_library/library.json").is_file());
        assert!(dir.join("_library/README.txt").is_file());
        let meta = lib.meta();
        assert_eq!(meta["format"], json!(FORMAT));
        assert_eq!(meta["name"], "My Games");
        assert!(lib.id().unwrap().starts_with("lib-"));
        lib.update_meta(json!({ "name": "Retro", "id": "changed" }))
            .unwrap();
        let again = Library::open(&dir).unwrap();
        assert_eq!(again.meta()["id"], meta["id"]);
        assert_eq!(again.info()["name"], "Retro");
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn opening_a_folder_of_games_adds_only_the_apps_folder() {
        let dir = temp_dir("existing");
        std::fs::create_dir_all(dir.join("SNES")).unwrap();
        std::fs::write(dir.join("SNES/Chrono Trigger (USA).sfc"), "rom").unwrap();
        Library::open(&dir).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("SNES/Chrono Trigger (USA).sfc")).unwrap(),
            "rom"
        );
        let mut top: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        top.sort();
        assert_eq!(top, ["SNES", "_library"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn newer_format_is_read_only() {
        let dir = temp_dir("newer");
        std::fs::create_dir_all(dir.join("_library")).unwrap();
        std::fs::write(
            dir.join("_library/library.json"),
            r#"{"format": 99, "id": "x", "name": "Future"}"#,
        )
        .unwrap();
        let lib = Library::open(&dir).unwrap();
        assert!(lib.read_only().is_some());
        assert!(lib.update_meta(json!({ "name": "y" })).is_err());
        assert!(!dir.join("_library/README.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn paths_stay_inside() {
        assert!(rel_inside("PC/Quake (1996)/game.json").is_ok());
        assert!(rel_inside("../x").is_err());
        assert!(rel_inside("/etc/passwd").is_ok()); // leading slash is dropped: still relative
        assert!(rel_inside("a/../../b").is_err());
        assert!(valid_id("../x").is_err());
    }
}
