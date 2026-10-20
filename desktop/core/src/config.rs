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

/// Game Repo's own folders on this computer, config and data (as Tauri names them for its
/// window): for the programs beside it, which find its settings and indexes there.
pub fn app_dirs() -> (PathBuf, PathBuf) {
    const ID: &str = "io.github.drflgd.gamerepo";
    let home = home_dir();
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    if cfg!(windows) {
        let roaming = env("APPDATA").unwrap_or_else(|| home.join("AppData/Roaming"));
        let local = env("LOCALAPPDATA").unwrap_or_else(|| home.join("AppData/Local"));
        (roaming.join(ID), local.join(ID))
    } else if cfg!(target_os = "macos") {
        let d = home.join("Library/Application Support").join(ID);
        (d.clone(), d)
    } else {
        let config = env("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"));
        let data = env("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local/share"));
        (config.join(ID), data.join(ID))
    }
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
    /// Where starter DATs are downloaded from, when not libretro's copy (crate::starter::SOURCE).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dat_source: Option<String>,
    /// Where each external tool is, when set in Settings › Tools (crate::tools)
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub tools: std::collections::BTreeMap<String, PathBuf>,
    /// The sources Identify asks, in order, and which are on (Settings › Sources; crate::identify)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<SourceSetting>,
    /// A source's address, when not its own (crate::identify::URLS)
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub source_urls: std::collections::BTreeMap<String, String>,
    /// The accounts and keys of the sources that need them (Settings › Sources; D33):
    /// source → { field → value }. Kept on this computer, never in the library.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub source_keys: std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>>,
    /// The PC games inbox (Import… reads it; Downloads when not set)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pc_inbox: Option<PathBuf>,
    /// Steam's folder, when the app shouldn't look for it
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steam_dir: Option<PathBuf>,
    /// Game Repo Server, as Settings › Serve to devices sets it (D89; crate::serving)
    #[serde(default)]
    pub serve: ServeSettings,
}

/// Settings › Serve to devices: whether Game Repo serves the library (started again when
/// it opens), on which port, and whether the server stays when Game Repo is closed.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServeSettings {
    #[serde(default)]
    pub on: bool,
    #[serde(default = "serve_port")]
    pub port: u16,
    #[serde(default)]
    pub keep: bool,
}

fn serve_port() -> u16 {
    gamerepo_wire::api::PORT
}

impl Default for ServeSettings {
    fn default() -> Self {
        Self { on: false, port: serve_port(), keep: false }
    }
}

/// A source in Settings › Sources: on or off.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SourceSetting {
    pub id: String,
    #[serde(default = "yes")]
    pub on: bool,
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
    /// Watch the folders with games on local drives while the app is open, and scan what changes
    #[serde(default = "yes")]
    pub watch: bool,
}

fn two() -> usize {
    2
}

fn yes() -> bool {
    true
}

impl Default for ScanSettings {
    fn default() -> Self {
        Self { full: false, readers: 2, watch: true }
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
        if path.is_file() && try_read_json_object(path).is_err() {
            // a hand edit gone wrong is kept beside it, not written over
            let keep = path.with_extension("unreadable.json");
            std::fs::rename(path, &keep).with_context(|| format!("{} can't be read, and couldn't be kept as {}", path.display(), keep.display()))?;
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

/// A JSON object from a file: {} if it's missing, an error naming what's wrong (with
/// its line) if it can't be read. Anything that writes the file back reads it this way,
/// so a typo in a hand edit is never written over with only the new settings.
pub fn try_read_json_object(path: &Path) -> Result<Value> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Value::Object(Default::default())),
        Err(e) => bail!("{} can't be read: {e}", path.display()),
    };
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(v) if v.is_object() => Ok(v),
        Ok(_) => bail!("{} isn't a JSON object", path.display()),
        Err(e) => bail!("{} isn't valid JSON ({e})", path.display()),
    }
}

/// A JSON object from a file, or {} if it's missing or unreadable (for reading only:
/// see `try_read_json_object` before writing it back).
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
