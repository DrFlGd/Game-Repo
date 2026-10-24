//! The external tools some conversions use (D14): looked for in the app's `tools/`
//! folder, then on the PATH, then at the path set for each in *Settings › Tools*
//! (a path set there is tried first). *Get it…* downloads a pinned build of dolphin-tool
//! or innoextract into the `tools/` folder and checks it (D178); Tesseract has an installer,
//! so its button opens its download page. Game Repo reads
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
            let got = app_dir().and_then(|d| std::fs::read_to_string(got_path(&d, t.id)).ok()).and_then(|s| serde_json::from_str::<Value>(&s).ok());
            json!({
                "id": t.id, "what": t.what, "set": set.get(t.id).map(|p| p.display().to_string()),
                "path": found.as_ref().map(|p| p.display().to_string()),
                "version": found.as_ref().and_then(|p| version(p)),
                "get": get_info(t.id), "got": got,
            })
        })
        .collect();
    json!({ "tools": tools, "app_dir": app_dir().map(|d| d.display().to_string()) })
}

// ------------------------------------------------------------ Get it… (D178)

/// A build of a tool that *Get it…* downloads: one pinned release per tool and OS.
pub struct Download {
    pub tool: &'static str,
    /// `std::env::consts::OS`
    pub os: &'static str,
    /// `std::env::consts::ARCH`, or "" for any
    pub arch: &'static str,
    pub version: &'static str,
    pub url: &'static str,
    /// The archive's size in bytes, when known (0 when not)
    pub size: u64,
    /// The archive's SHA-256, once pinned ("" until then: the first download's is shown
    /// and recorded, to pin here)
    pub sha256: &'static str,
    /// The program inside the archive, by its file name
    pub member: &'static str,
}

pub const DOWNLOADS: &[Download] = &[
    Download {
        tool: "dolphin-tool", os: "windows", arch: "x86_64", version: "2609a",
        url: "https://dl.dolphin-emu.org/releases/2609a/dolphin-2609a-x64.7z", size: 0, sha256: "", member: "DolphinTool.exe",
    },
    Download {
        tool: "dolphin-tool", os: "windows", arch: "aarch64", version: "2609a",
        url: "https://dl.dolphin-emu.org/releases/2609a/dolphin-2609a-arm64.7z", size: 0, sha256: "", member: "DolphinTool.exe",
    },
    Download {
        tool: "innoextract", os: "windows", arch: "", version: "1.9",
        url: "https://github.com/dscharrer/innoextract/releases/download/1.9/innoextract-1.9-windows.zip", size: 520705, sha256: "", member: "innoextract.exe",
    },
];

/// Where each tool is downloaded from by hand: its page, for this OS.
pub fn page(id: &str) -> &'static str {
    match (id, cfg!(windows)) {
        ("dolphin-tool", _) => "https://dolphin-emu.org/download/",
        ("innoextract", _) => "https://constexpr.org/innoextract/",
        ("tesseract", true) => "https://github.com/UB-Mannheim/tesseract/wiki",
        ("tesseract", false) => "https://tesseract-ocr.github.io/tessdoc/Installation.html",
        _ => "",
    }
}

/// The build *Get it…* downloads on this computer, if there is one.
pub fn download_for(id: &str) -> Option<&'static Download> {
    DOWNLOADS.iter().find(|d| d.tool == id && d.os == std::env::consts::OS && (d.arch.is_empty() || d.arch == std::env::consts::ARCH))
}

/// What *Get it…* does for a tool: `{ kind: "download", version, url }`, or `{ kind: "page", url }`.
fn get_info(id: &str) -> Value {
    match download_for(id) {
        Some(d) => json!({ "kind": "download", "version": d.version, "url": d.url, "pinned": !d.sha256.is_empty() }),
        None => json!({ "kind": "page", "url": page(id) }),
    }
}

/// The note kept beside a downloaded tool: what was downloaded, and its SHA-256.
fn got_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.json"))
}

const DOWNLOAD_LIMIT: u64 = 400 << 20;

/// Download a tool's pinned build into `dir` (the app's tools folder), check it and
/// put its program there: the archive must come from the pinned URL, be the size
/// pinned (when one is), have the SHA-256 pinned (when one is), open, hold the program,
/// and the program must run (its version is taken when it says one). Nothing is kept
/// when any check fails.
pub fn get(id: &str, dir: &Path, cancel: &std::sync::atomic::AtomicBool, progress: &mut dyn FnMut(Value)) -> anyhow::Result<Value> {
    use anyhow::{bail, Context};
    let d = download_for(id).ok_or_else(|| anyhow::anyhow!("Game Repo doesn't download {id} on this computer: get it from {}.", page(id)))?;
    std::fs::create_dir_all(dir).with_context(|| format!("couldn't make the tools folder {}", dir.display()))?;
    let kind = crate::archive::kind_of(d.url).ok_or_else(|| anyhow::anyhow!("{} isn't an archive Game Repo opens.", d.url))?;
    let archive = dir.join(format!("{id}.download"));
    let fetched = fetch_to(d, &archive, cancel, progress);
    let r = fetched.and_then(|sha| install(d, &archive, kind, dir, cancel, progress).map(|p| (sha, p)));
    let _ = std::fs::remove_file(&archive);
    let (sha256, path) = r?;
    progress(json!({ "text": format!("Checking {id}") }));
    if command(&path).arg("--version").output().is_err() {
        let _ = std::fs::remove_file(&path);
        bail!("{} was downloaded but doesn't run on this computer, so it wasn't kept.", d.member);
    }
    let version = version(&path);
    let got = json!({ "url": d.url, "version": d.version, "sha256": sha256, "size": std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0), "at": crate::library::now(), "pinned": !d.sha256.is_empty() });
    let _ = crate::config::write_json(&got_path(dir, id), &got);
    Ok(json!({ "id": id, "path": path.display().to_string(), "version": version, "got": got }))
}

/// Download `d.url` to `to`, with progress by bytes; the archive's SHA-256, checked
/// against the pinned one (and the pinned size) when there is one.
fn fetch_to(d: &Download, to: &Path, cancel: &std::sync::atomic::AtomicBool, progress: &mut dyn FnMut(Value)) -> anyhow::Result<String> {
    use anyhow::{bail, Context};
    use sha2::Digest;
    use std::io::{Read, Write};
    let mut r = ureq::get(d.url)
        .header("User-Agent", concat!("GameRepo/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|e| anyhow::anyhow!("couldn't download {}: {e}", d.url))?;
    let total: Option<u64> = r.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok());
    if let (Some(t), true) = (total, d.size > 0) {
        if t != d.size {
            bail!("{} isn't the file expected: it's {t} bytes, not {}. Nothing was installed.", d.url, d.size);
        }
    }
    let mut body = r.body_mut().with_config().limit(DOWNLOAD_LIMIT).reader();
    let mut out = std::fs::File::create(to).with_context(|| format!("couldn't write {}", to.display()))?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut n: u64 = 0;
    let text = format!("Downloading {} {}", d.tool, d.version);
    progress(json!({ "text": text, "done": 0, "total": total.unwrap_or(d.size) }));
    loop {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            bail!("Stopped.");
        }
        let got = match body.read(&mut buf) {
            Ok(0) => break,
            Ok(got) => got,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => bail!("the download of {} broke off: {e}", d.url),
        };
        hasher.update(&buf[..got]);
        out.write_all(&buf[..got])?;
        n += got as u64;
        progress(json!({ "text": text, "done": n, "total": total.unwrap_or(d.size.max(n)) }));
    }
    out.flush()?;
    drop(out);
    if d.size > 0 && n != d.size {
        bail!("{} isn't the file expected: it's {n} bytes, not {}. Nothing was installed.", d.url, d.size);
    }
    let sha = format!("{:x}", hasher.finalize());
    if !d.sha256.is_empty() && sha != d.sha256 {
        bail!("{} isn't the file expected: its SHA-256 is {sha}, not {}. Nothing was installed.", d.url, d.sha256);
    }
    Ok(sha)
}

/// Unpack the tool's program (and the DLLs beside it, for Dolphin's) from the archive
/// into `dir`: the program's path there.
fn install(d: &Download, archive: &Path, kind: &str, dir: &Path, cancel: &std::sync::atomic::AtomicBool, progress: &mut dyn FnMut(Value)) -> anyhow::Result<PathBuf> {
    use anyhow::{bail, Context};
    progress(json!({ "text": format!("Unpacking {}", d.tool) }));
    let base = |name: &str| name.rsplit('/').next().unwrap_or(name).to_string();
    let want = |name: &str| {
        let b = base(name);
        b.eq_ignore_ascii_case(d.member) || (b.to_ascii_lowercase().ends_with(".dll") && name.matches('/').count() <= 1)
    };
    let mut written: Vec<(PathBuf, PathBuf)> = vec![];
    let mut found = None;
    let r = crate::archive::each_wanted_as(archive, kind, &want, cancel, &mut |name, _size, data| {
        let b = base(name);
        let to = dir.join(&b);
        let part = dir.join(format!("{b}.part"));
        let mut f = std::fs::File::create(&part)?;
        crate::archive::copy_stoppable(data, &mut f, cancel)?;
        if b.eq_ignore_ascii_case(d.member) {
            found = Some(to.clone());
        }
        written.push((part, to));
        Ok(())
    });
    let clean = |written: &[(PathBuf, PathBuf)]| {
        for (part, _) in written {
            let _ = std::fs::remove_file(part);
        }
    };
    if let Err(e) = r {
        clean(&written);
        bail!("couldn't open {}: {e}", archive.display());
    }
    let Some(path) = found else {
        clean(&written);
        bail!("The download holds no {}, so nothing was installed.", d.member);
    };
    for (part, to) in &written {
        std::fs::rename(part, to).with_context(|| format!("couldn't write {}", to.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(to, std::fs::Permissions::from_mode(0o755));
        }
    }
    Ok(path)
}

/// Tests that set the tools' paths take turns (the paths are the process's).
#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    /// Every pinned build is an https archive Game Repo opens, named for its version.
    #[test]
    fn downloads_are_pinned() {
        for d in DOWNLOADS {
            assert!(d.url.starts_with("https://"), "{}", d.url);
            assert!(crate::archive::kind_of(d.url).is_some(), "{}", d.url);
            assert!(d.url.contains(d.version), "{} should name {}", d.url, d.version);
            assert!(!d.member.is_empty() && !d.tool.is_empty());
            assert!(TOOLS.iter().any(|t| t.id == d.tool));
        }
        for t in TOOLS {
            let g = get_info(t.id);
            assert!(g["url"].as_str().is_some_and(|u| u.starts_with("https://")), "{g}");
        }
        // where there's no build to download, Get it… says where to get it
        if download_for("innoextract").is_none() {
            let dir = std::env::temp_dir().join(format!("gamerepo-test-tools-none-{}", std::process::id()));
            let e = get("innoextract", &dir, &AtomicBool::new(false), &mut |_| {}).unwrap_err().to_string();
            assert!(e.contains(page("innoextract")), "{e}");
            let _ = std::fs::remove_dir_all(&dir);
        }
        assert!(page("tesseract").starts_with("https://"));
    }

    /// The program (and DLLs beside it) come out of the archive into the tools folder,
    /// through a .part file; an archive without it leaves nothing behind.
    #[test]
    fn install_unpacks_the_program() {
        let dir = std::env::temp_dir().join(format!("gamerepo-test-tools-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let script = if cfg!(windows) { "@echo off\r\necho fake 1.9\r\n" } else { "#!/bin/sh\necho fake 1.9\n" };
        let member = if cfg!(windows) { "fake.bat" } else { "fake" };
        let zip = dir.join("fake.zip");
        {
            let mut w = zip::ZipWriter::new(std::fs::File::create(&zip).unwrap());
            let o = zip::write::SimpleFileOptions::default();
            for (name, body) in [(format!("fake-1.9/{member}"), script), ("fake-1.9/helper.dll".to_string(), "dll"), ("fake-1.9/docs/other.dll".to_string(), "not this one")] {
                use std::io::Write;
                w.start_file(name, o).unwrap();
                w.write_all(body.as_bytes()).unwrap();
            }
            w.finish().unwrap();
        }
        let d = Download { tool: "fake", os: std::env::consts::OS, arch: "", version: "1.9", url: "https://example.org/fake-1.9.zip", size: 0, sha256: "", member };
        let none = AtomicBool::new(false);
        let path = install(&d, &zip, "zip", &dir, &none, &mut |_| {}).unwrap();
        assert_eq!(path, dir.join(member));
        assert!(dir.join("helper.dll").is_file() && !dir.join("other.dll").exists());
        assert!(!dir.join(format!("{member}.part")).exists());
        assert_eq!(version(&path).as_deref(), Some("fake 1.9"));
        // an archive without the program
        let d2 = Download { member: "missing.exe", ..d };
        let e = install(&d2, &zip, "zip", &dir, &none, &mut |_| {}).unwrap_err().to_string();
        assert!(e.contains("holds no missing.exe"), "{e}");
        assert!(!dir.join("missing.exe").exists() && !dir.join("helper.dll.part").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
