//! What the app keeps on this computer (the OS's per-user config and data
//! folders), as opposed to the portable library folder: which library is open,
//! recently opened libraries and preferences (theme, view settings).

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// The user's home folder.
pub fn home_dir() -> PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// `~/Game Repo`, until the user opens another folder.
pub fn default_library_path() -> PathBuf {
    home_dir().join("Game Repo")
}

/// App config, in the OS's per-user config folder.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AppConfig {
    /// The open library folder.
    #[serde(default)]
    pub library: Option<PathBuf>,
    /// Libraries opened before, newest first.
    #[serde(default)]
    pub recent_libraries: Vec<PathBuf>,
    /// How scans read files on this computer.
    #[serde(default)]
    pub scan: ScanSettings,
}

/// How scans read files on this computer (Settings › Folders with games).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScanSettings {
    /// Unpack and hash every file in a ZIP, instead of trusting the CRC32s it records (D16: off)
    #[serde(default)]
    pub full: bool,
    /// Files read at once: more for SSDs and network drives, 1 for a single slow disk
    #[serde(default = "two")]
    pub readers: usize,
}

fn two() -> usize {
    2
}

impl Default for ScanSettings {
    fn default() -> Self {
        Self { full: false, readers: 2 }
    }
}

impl AppConfig {
    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        write_atomic(path, &serde_json::to_vec_pretty(self)?)
    }

    pub fn library_path(&self) -> PathBuf {
        self.library.clone().unwrap_or_else(default_library_path)
    }

    /// Remember `dir` as the open library (and at the top of the recent list).
    pub fn set_library(&mut self, dir: &Path) {
        self.library = Some(dir.to_path_buf());
        self.recent_libraries.retain(|p| p != dir);
        self.recent_libraries.insert(0, dir.to_path_buf());
        self.recent_libraries.truncate(8);
    }
}

/// Preferences the page stores (a JSON object), kept with the app.
pub struct Prefs {
    path: PathBuf,
}

impl Prefs {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn exists(&self) -> bool {
        self.path.is_file()
    }
    pub fn get(&self) -> Value {
        read_json_object(&self.path)
    }
    pub fn set(&self, prefs: &Value) -> Result<()> {
        if !prefs.is_object() {
            bail!("preferences must be an object");
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        write_atomic(&self.path, &serde_json::to_vec_pretty(prefs)?)
    }
}

/// A JSON object from a file, or {} if it's missing or unreadable.
pub fn read_json_object(path: &Path) -> Value {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| Value::Object(Default::default()))
}

/// Write via a temporary file and rename, so a crash never leaves half a file.
pub fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("couldn't create {}", parent.display()))?;
    }
    let tmp = path.with_extension(
        format!(
            "tmp-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        )
        .replace(['(', ')'], ""),
    );
    std::fs::write(&tmp, data).with_context(|| format!("couldn't write {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("couldn't write {}", path.display()))?;
    Ok(())
}

/// Pretty JSON with sorted keys and a final newline, so files in a library kept
/// in git or a sync tool change only where something changed.
pub fn write_json(path: &Path, v: &Value) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(&sorted(v))?;
    bytes.push(b'\n');
    write_atomic(path, &bytes)
}

/// A copy with object keys in sorted order (at every level).
pub fn sorted(v: &Value) -> Value {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            Value::Object(
                keys.into_iter()
                    .map(|k| (k.clone(), sorted(&m[k])))
                    .collect(),
            )
        }
        Value::Array(a) => Value::Array(a.iter().map(sorted).collect()),
        other => other.clone(),
    }
}
