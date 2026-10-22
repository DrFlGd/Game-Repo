//! The external tools some conversions use (D14): looked for in the app's `tools/`
//! folder, then on the PATH, then at the path set for each in *Settings › Tools*
//! (a path set there is tried first). Nothing is ever downloaded. Game Repo reads
//! and writes CHD, CSO and ZSO itself (its CHDs hold the same data, with the same
//! SHA1, as `chdman`'s), so the one tool it uses is `dolphin-tool`, for RVZ; besides it,
//! innoextract for GOG installers and Tesseract for magazines' text (D145).

use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::RwLock;

pub struct Tool {
    pub id: &'static str,
    /// Its program names (without .exe)
    pub names: &'static [&'static str],
    pub what: &'static str,
}

pub const TOOLS: &[Tool] = &[
    Tool {
        id: "dolphin-tool",
        names: &["dolphin-tool", "DolphinTool"],
        what: "Reads and writes RVZ (GameCube and Wii). It comes with Dolphin.",
    },
    Tool {
        id: "innoextract",
        names: &["innoextract"],
        what: "Finds a GOG installer's game id and unpacks GOG installers (PC games). From constexpr.org/innoextract.",
    },
    Tool {
        id: "tesseract",
        names: &["tesseract"],
        what: "Reads the text in magazines' scanned pages, in place of Windows' own text recognition (on Linux and the Steam Deck, the only way). From github.com/tesseract-ocr.",
    },
];

/// The paths set in Settings › Tools.
static PATHS: RwLock<BTreeMap<String, PathBuf>> = RwLock::new(BTreeMap::new());

/// Called when the app's config is loaded or changed.
pub fn configure(paths: &BTreeMap<String, PathBuf>) {
    *PATHS.write().unwrap_or_else(|e| e.into_inner()) = paths.clone();
}

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// The app's own `tools/` folder, beside its program.
pub fn app_dir() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(|d| d.join("tools"))
}

/// Where a tool is: the path set for it, the app's tools folder, then the PATH.
pub fn find(id: &str) -> Option<PathBuf> {
    let set = PATHS.read().ok().and_then(|s| s.get(id).cloned());
    if let Some(p) = set.filter(|p| p.is_file()) {
        return Some(p);
    }
    let tool = TOOLS.iter().find(|t| t.id == id)?;
    which(tool.names)
}

/// The first of these programs in the app's tools folder or on the PATH.
pub fn which(names: &[&str]) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = app_dir().into_iter().collect();
    dirs.extend(std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect::<Vec<_>>()).unwrap_or_default());
    for d in dirs {
        for n in names {
            let p = d.join(exe(n));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// A command for a tool, without a console window on Windows.
pub fn command(path: &Path) -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(path);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c
}

/// The first line a tool prints about itself that names a version (a number like 5.0 or 2412).
fn version(path: &Path) -> Option<String> {
    let out = command(path).arg("--version").output().ok()?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let numbered = |l: &str| {
        let b = l.as_bytes();
        b.windows(3).any(|w| w[0].is_ascii_digit() && w[1] == b'.' && w[2].is_ascii_digit())
            || l.split(|c: char| !c.is_ascii_digit()).any(|n| n.len() >= 4)
    };
    text.lines().map(str::trim).find(|l| numbered(l)).map(|l| l.chars().take(120).collect())
}

/// What Settings › Tools shows.
pub fn list() -> Value {
    let set = PATHS.read().map(|s| s.clone()).unwrap_or_default();
    let tools: Vec<Value> = TOOLS
        .iter()
        .map(|t| {
            let found = find(t.id);
            json!({
                "id": t.id, "what": t.what, "set": set.get(t.id).map(|p| p.display().to_string()),
                "path": found.as_ref().map(|p| p.display().to_string()),
                "version": found.as_ref().and_then(|p| version(p)),
            })
        })
        .collect();
    json!({ "tools": tools, "app_dir": app_dir().map(|d| d.display().to_string()) })
}

/// Tests that set the tools' paths take turns (the paths are the process's).
#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
