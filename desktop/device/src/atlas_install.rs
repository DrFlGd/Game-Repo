//! *Install Atlas* in Game Repo Go (docs/PLAN.md, "Maps and Guides notes, part 13",
//! D250–D255), on Windows and Linux: Atlas installed once, on a press, from Atlas's latest
//! GitHub release. Never on its own and never again: Atlas updates itself after that, so Go
//! offers no update of it.
//!
//! The release's `latest.json` (what Atlas's own updater reads: `version`, `notes`,
//! `pub_date` and, for each system, the file's `url` and minisign `signature`) names this
//! system's file: `windows-x86_64`, the NSIS installer `Atlas-<version>-setup.exe`, or
//! `linux-x86_64`, `Atlas.AppImage`. Only a file of Atlas's own releases is fetched
//! ([`Source::downloads`]), following redirects only to GitHub's hosts for release files
//! ([`ASSET_HOSTS`]). It's downloaded to a `.part` file in Go's data folder
//! (`atlas-download/`) and checked as it comes against the signature and Atlas's updater key
//! ([`PUBKEY`], copied from Atlas's `tauri.conf.json` at v0.29.0); a file that doesn't
//! match is deleted and nothing is installed.
//!
//! Windows: the installer is run silently (`/S`) for this user, as Atlas's installer installs
//! (Tauri's `currentUser` mode: no administrator), with no window, waited for on the
//! install's own thread, then deleted. Linux and the Steam Deck: the AppImage is put at
//! `~/Applications/Atlas.AppImage`, made runnable (755), never over a file that's there.

use crate::update;
use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use serde_json::Value;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Atlas's repository on GitHub, whose releases are Atlas's.
pub const RELEASES: &str = "https://github.com/DrFlGd/Atlas";

/// Atlas's updater key: `plugins.updater.pubkey` in Atlas's
/// `apps/desktop/src-tauri/tauri.conf.json` at v0.29.0, exactly (minisign key D07A3EBAB3B43CC1).
/// If Atlas ever changes its key, this must change with it, or Go refuses every release.
pub const PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEQwN0EzRUJBQjNCNDNDQzEKUldUQlBMU3p1ajU2ME9zVTlKaHhwWXNJVUFweCtQRXVYeHB0czc5V3BFOE51a1ZxNGxoSks1dTQK";

/// The hosts GitHub sends a release's files from: the only places a redirect is followed to.
pub const ASSET_HOSTS: &[&str] = &["objects.githubusercontent.com", "release-assets.githubusercontent.com"];

/// `latest.json` is small; anything larger isn't it.
const LATEST_MOST: u64 = 1 << 20;
/// Atlas's installer is tens of MB and its AppImage a little over 100 MB.
const FILE_MOST: u64 = 2 << 30;
/// Redirects followed at most for one file.
const REDIRECTS: usize = 5;
/// The folder in Go's data where the file is downloaded.
pub const DOWNLOADS: &str = "atlas-download";
/// How long the Windows installer is waited for at most.
#[cfg(windows)]
const INSTALLER_WAIT: Duration = Duration::from_secs(15 * 60);

/// Where Atlas's releases are and the key they're signed with: Atlas's, or (the page test
/// only, through `gamerepo-cli client-ui`) a stand-in's.
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    /// The repository's address: its releases are `<releases>/releases/…`
    pub releases: String,
    /// The updater key, as Atlas's `tauri.conf.json` has it
    pub pubkey: String,
}

impl Default for Source {
    fn default() -> Self {
        Source { releases: RELEASES.into(), pubkey: PUBKEY.into() }
    }
}

impl Source {
    /// The latest release's `latest.json`.
    pub fn latest(&self) -> String {
        format!("{}/releases/latest/download/latest.json", self.releases.trim_end_matches('/'))
    }

    /// Where every release's files are: a file named in `latest.json` must be under it.
    pub fn downloads(&self) -> String {
        format!("{}/releases/download/", self.releases.trim_end_matches('/'))
    }
}

/// This system's entry in `latest.json`: windows-x86_64 or linux-x86_64. None elsewhere
/// (Android, another processor), where Go offers no *Install Atlas*.
pub fn platform() -> Option<&'static str> {
    if cfg!(all(windows, target_arch = "x86_64")) {
        Some("windows-x86_64")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("linux-x86_64")
    } else {
        None
    }
}

/// The file of Atlas's latest release for this system.
#[derive(Clone, Debug, PartialEq)]
pub struct Build {
    pub version: String,
    pub url: String,
    /// Its minisign signature, as `latest.json` carries it (the `.sig` file's text)
    pub signature: String,
    /// Its file name, the last part of `url`
    pub name: String,
}

/// The host of an `https://` address with no user, password or port in it.
fn https_host(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://")?;
    let host = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    (!host.is_empty() && host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')).then_some(host)
}

/// A plain part of a path: letters, digits and `.-_+`, not `.` or `..`.
fn plain(part: &str) -> bool {
    !part.is_empty() && part != "." && part != ".." && part.chars().all(|c| c.is_ascii_alphanumeric() || "._-+".contains(c))
}

/// Whether Go may fetch from `url` for Atlas: a file of Atlas's own releases (the latest's
/// address, or one under [`Source::downloads`]), or one on GitHub's hosts for release files,
/// over HTTPS.
pub fn may_fetch(url: &str, source: &Source) -> bool {
    if url.contains("..") || url.contains(['\\', ' ', '@']) {
        return false;
    }
    url.starts_with(&source.downloads()) || url == source.latest() || https_host(url).is_some_and(|h| ASSET_HOSTS.contains(&h.to_ascii_lowercase().as_str()))
}

/// This system's file in Atlas's `latest.json` (`file`): refused, in words for the page, when
/// the file isn't Atlas's update file, has no build for `platform`, or names a file that
/// isn't one of Atlas's releases (`<downloads><tag>/<name>`, an installer `.exe` on Windows,
/// `Atlas.AppImage` on Linux).
pub fn build_of(file: &str, platform: &str, source: &Source) -> Result<Build, String> {
    let v: Value = serde_json::from_str(file).map_err(|_| "Atlas's latest release has no update file Game Repo Go can read, so nothing was installed.".to_string())?;
    let version = v["version"].as_str().map(str::trim).filter(|s| plain(s)).ok_or("Atlas's latest release doesn't say its version, so nothing was installed.")?;
    let p = &v["platforms"][platform];
    if p.is_null() {
        return Err("Atlas's latest release has no build for this system yet.".into());
    }
    let (Some(url), Some(signature)) = (p["url"].as_str(), p["signature"].as_str().filter(|s| !s.trim().is_empty())) else {
        return Err("Atlas's latest release has no build for this system yet.".into());
    };
    let refused = || format!("Atlas's latest release names a file that isn't one of Atlas's releases ({url}), so nothing was downloaded.");
    let rest = url.strip_prefix(&source.downloads()).ok_or_else(refused)?;
    let (tag, name) = rest.split_once('/').ok_or_else(refused)?;
    let wanted = match platform {
        "windows-x86_64" => name.to_ascii_lowercase().ends_with(".exe"),
        _ => name == "Atlas.AppImage",
    };
    if !plain(tag) || !plain(name) || !wanted {
        return Err(refused());
    }
    Ok(Build { version: version.to_string(), url: url.to_string(), signature: signature.trim().to_string(), name: name.to_string() })
}

/// Go's agent for Atlas's files: redirects not followed by themselves (each is checked,
/// [`get`]), the proxy from the environment (`HTTPS_PROXY`, `NO_PROXY`), Go's own name.
fn agent() -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .max_redirects(0)
        .max_redirects_will_error(false)
        .http_status_as_error(false)
        .user_agent(concat!("GameRepoGo/", env!("CARGO_PKG_VERSION")))
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_recv_response(Some(Duration::from_secs(60)))
        .build();
    ureq::Agent::new_with_config(config)
}

/// `url` fetched, its redirects followed only where [`may_fetch`] allows.
fn get(agent: &ureq::Agent, url: &str, source: &Source) -> Result<ureq::http::Response<ureq::Body>, String> {
    let mut url = url.to_string();
    for _ in 0..=REDIRECTS {
        if !may_fetch(&url, source) {
            return Err(format!("Atlas's release sent Game Repo Go to {url}, which isn't GitHub's, so nothing was downloaded."));
        }
        // a connection that broke before any answer is tried once more
        let r = match agent.get(&url).call() {
            Err(ureq::Error::Io(_)) => {
                std::thread::sleep(Duration::from_secs(1));
                agent.get(&url).call()
            }
            r => r,
        };
        let r = r.map_err(|e| format!("Atlas's release couldn't be reached ({e}). Is this device online?"))?;
        let status = r.status().as_u16();
        if (300..400).contains(&status) {
            let to = r.headers().get("location").and_then(|v| v.to_str().ok()).unwrap_or_default().trim().to_string();
            if to.is_empty() {
                return Err(format!("Atlas's release answered {status} with nowhere to go."));
            }
            url = if to.starts_with('/') {
                // the same server's
                let origin_end = url.find("://").map(|i| i + 3).and_then(|i| url[i..].find('/').map(|j| i + j)).unwrap_or(url.len());
                format!("{}{to}", &url[..origin_end])
            } else {
                to
            };
            continue;
        }
        if status != 200 {
            return Err(if status == 404 { "Atlas has no release to install yet (GitHub answered 404).".to_string() } else { format!("Atlas's release couldn't be fetched (GitHub answered {status}).") });
        }
        return Ok(r);
    }
    Err("Atlas's release sent Game Repo Go round too many redirects, so nothing was downloaded.".into())
}

/// The latest release's `latest.json`, as text.
pub fn fetch_latest(source: &Source) -> Result<String, String> {
    let mut r = get(&agent(), &source.latest(), source)?;
    r.body_mut().with_config().limit(LATEST_MOST).read_to_string().map_err(|e| format!("Atlas's update file couldn't be read ({e})."))
}

/// What an install is doing, for the page.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Progress {
    /// idle, reading (latest.json), downloading, installing, done or failed
    pub state: &'static str,
    /// The version being installed
    pub version: Option<String>,
    /// Bytes downloaded, and the file's size when the server says it
    pub done: u64,
    pub total: Option<u64>,
    /// What came of it (done or failed), in a sentence
    pub message: Option<String>,
}

/// Downloads `build` to `part`, checking it as it comes against its signature and the key:
/// `Err` (with the file deleted) when it doesn't match, can't be read or breaks off.
pub fn download(build: &Build, source: &Source, part: &Path, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<(), String> {
    let r = download_inner(build, source, part, progress);
    if r.is_err() {
        let _ = std::fs::remove_file(part);
    }
    r
}

fn download_inner(build: &Build, source: &Source, part: &Path, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<(), String> {
    let (key, sig) = update::decode(&build.signature, &source.pubkey).map_err(|e| match e {
        update::Unreadable::Key(e) => format!("Atlas's key built into Game Repo Go can't be read ({e}), so nothing was installed."),
        update::Unreadable::Signature(e) => format!("Atlas's latest release has a signature that can't be read ({e}), so nothing was installed."),
    })?;
    let mismatch = || "The Atlas downloaded doesn't match Atlas's signature, so it was deleted and nothing was installed.".to_string();
    // a signature by another key is refused before anything is downloaded
    let mut check = key.verify_stream(&sig).map_err(|_| mismatch())?;
    let mut r = get(&agent(), &build.url, source)?;
    let total: Option<u64> = r.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok());
    if total.is_some_and(|t| t > FILE_MOST) {
        return Err("Atlas's file is far larger than Atlas is, so it wasn't downloaded.".into());
    }
    if let Some(d) = part.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("Game Repo Go couldn't make {} ({e}).", d.display()))?;
    }
    let mut out = std::fs::File::create(part).map_err(|e| format!("Game Repo Go couldn't write {} ({e}).", part.display()))?;
    let mut body = r.body_mut().with_config().limit(FILE_MOST).reader();
    let mut buf = vec![0u8; 1 << 20];
    let mut n: u64 = 0;
    progress(0, total);
    loop {
        let got = match body.read(&mut buf) {
            Ok(0) => break,
            Ok(got) => got,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(format!("The download of Atlas broke off ({e}). Try again.")),
        };
        check.update(&buf[..got]);
        out.write_all(&buf[..got]).map_err(|e| format!("Game Repo Go couldn't write {} ({e}).", part.display()))?;
        n += got as u64;
        progress(n, total);
    }
    out.flush().and_then(|_| out.sync_all()).map_err(|e| format!("Game Repo Go couldn't write {} ({e}).", part.display()))?;
    drop(out);
    if total.is_some_and(|t| t != n) {
        return Err("The download of Atlas broke off. Try again.".into());
    }
    check.finalize().map_err(|_| mismatch())
}

/// Checks a file against a signature and the key, the same way as [`download`] does as it
/// comes: `Err` in a sentence for the page.
pub fn verify(data: &[u8], signature: &str, pubkey: &str) -> Result<(), String> {
    let (key, sig) = update::decode(signature, pubkey).map_err(|e| match e {
        update::Unreadable::Key(e) => format!("Atlas's key built into Game Repo Go can't be read ({e}), so nothing was installed."),
        update::Unreadable::Signature(e) => format!("Atlas's latest release has a signature that can't be read ({e}), so nothing was installed."),
    })?;
    let mut check = key.verify_stream(&sig).map_err(|_| "The Atlas downloaded doesn't match Atlas's signature, so it was deleted and nothing was installed.".to_string())?;
    check.update(data);
    check.finalize().map_err(|_| "The Atlas downloaded doesn't match Atlas's signature, so it was deleted and nothing was installed.".to_string())
}

/// Where the AppImage goes on Linux: `~/Applications/Atlas.AppImage` under `home`, where
/// Atlas's release notes say to put it and the Steam launch line looks (D241).
pub fn appimage_place(home: &Path) -> PathBuf {
    home.join(crate::launch::USUAL)
}

/// `file` (the checked download) put at `to` without ever replacing a file there: linked
/// in (which fails when `to` is there), else, where the folder can't take a link, renamed
/// after a last look.
fn put_new(file: &Path, to: &Path) -> std::io::Result<()> {
    match std::fs::hard_link(file, to) {
        Ok(()) => std::fs::remove_file(file),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(e),
        Err(_) if std::fs::symlink_metadata(to).is_ok() => Err(std::io::ErrorKind::AlreadyExists.into()),
        Err(_) => std::fs::rename(file, to),
    }
}

/// Linux: the checked AppImage (`file`) put at `~/Applications/Atlas.AppImage` under `home`,
/// the folder made when it isn't there, mode 755. Never over a file that's there: refused,
/// and `file` is deleted. The AppImage's path.
pub fn place_appimage(file: &Path, home: &Path) -> Result<PathBuf> {
    let to = appimage_place(home);
    let dir = to.parent().ok_or_else(|| anyhow!("no folder for {}", to.display()))?;
    let r = (|| -> Result<PathBuf> {
        if std::fs::symlink_metadata(&to).is_ok() {
            bail!("There's an Atlas.AppImage in {} already, so Game Repo Go left it as it is.", dir.display());
        }
        std::fs::create_dir_all(dir).with_context(|| format!("Game Repo Go couldn't make {}", dir.display()))?;
        // into the same folder first (Go's data may be on another drive), then in place at once
        let near = dir.join(format!(".Atlas.AppImage.{}.part", std::process::id()));
        let _ = std::fs::remove_file(&near);
        if std::fs::rename(file, &near).is_err() {
            std::fs::copy(file, &near).with_context(|| format!("Game Repo Go couldn't write {}", near.display()))?;
        }
        set_runnable(&near)?;
        let put = put_new(&near, &to);
        let _ = std::fs::remove_file(&near);
        match put {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => bail!("There's an Atlas.AppImage in {} already, so Game Repo Go left it as it is.", dir.display()),
            Err(e) => return Err(e).with_context(|| format!("Game Repo Go couldn't put Atlas at {}", to.display())),
        }
        set_runnable(&to)?;
        Ok(to.clone())
    })();
    let _ = std::fs::remove_file(file);
    r
}

/// Mode 755: its user may change it, everyone may run it.
fn set_runnable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).with_context(|| format!("Game Repo Go couldn't make {} runnable", path.display()))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Windows: Atlas's installer run silently (`/S`) for this user, with no window, waited for.
#[cfg(windows)]
fn run_installer(exe: &Path) -> Result<(), String> {
    let status = crate::run_alone(exe, &["/S"], INSTALLER_WAIT).map_err(|e| {
        if e.raw_os_error() == Some(740) {
            // ERROR_ELEVATION_REQUIRED: an installer for every user, which Go doesn't run
            "Atlas's installer asks for an administrator, so Game Repo Go didn't run it. Install Atlas from its releases page instead.".to_string()
        } else {
            format!("Atlas's installer didn't run ({e}).")
        }
    })?;
    match status {
        Some(s) if s.success() => Ok(()),
        Some(s) => Err(format!("Atlas's installer stopped without installing Atlas (it ended with {}).", s.code().map_or("no code".into(), |c| c.to_string()))),
        None => Err("Atlas's installer didn't finish within 15 minutes, so Game Repo Go stopped waiting for it.".into()),
    }
}

/// Install Atlas: its latest release's file for this system fetched from `source`, checked
/// against its signature, and installed (`data`: Go's data folder; `home`: the user's folder,
/// for `~/Applications` on Linux). `tell` hears each step. What came of it, in a sentence;
/// the downloaded file is gone either way.
pub fn install(source: &Source, data: &Path, home: &Path, tell: &mut dyn FnMut(Progress)) -> Result<String, String> {
    let Some(platform) = platform() else { return Err("Game Repo Go can't install Atlas on this system.".into()) };
    let dir = data.join(DOWNLOADS);
    // what an install cut short left, and then what this one downloaded, whatever came of it
    let _ = std::fs::remove_dir_all(&dir);
    let r = install_in(source, platform, &dir, home, tell);
    let _ = std::fs::remove_dir_all(&dir);
    r
}

fn install_in(source: &Source, platform: &str, dir: &Path, home: &Path, tell: &mut dyn FnMut(Progress)) -> Result<String, String> {
    if platform.starts_with("linux") && std::fs::symlink_metadata(appimage_place(home)).is_ok() {
        return Err(format!("There's an Atlas.AppImage in {} already, so Game Repo Go left it as it is.", appimage_place(home).parent().unwrap_or(home).display()));
    }
    tell(Progress { state: "reading", ..Default::default() });
    let build = build_of(&fetch_latest(source)?, platform, source)?;
    let part = dir.join(format!("{}.part", build.name));
    let version = Some(build.version.clone());
    download(&build, source, &part, &mut |done, total| tell(Progress { state: "downloading", version: version.clone(), done, total, message: None }))?;
    tell(Progress { state: "installing", version: version.clone(), ..Default::default() });
    if platform.starts_with("linux") {
        place_appimage(&part, home).map(|to| format!("Atlas {} is installed at {}.", build.version, to.display())).map_err(|e| format!("{e:#}"))
    } else {
        install_windows(&part, dir, &build)
    }
}

#[cfg(windows)]
fn install_windows(part: &Path, dir: &Path, build: &Build) -> Result<String, String> {
    let exe = dir.join(&build.name);
    std::fs::rename(part, &exe).map_err(|e| format!("Game Repo Go couldn't write {} ({e}).", exe.display()))?;
    let r = run_installer(&exe);
    let _ = std::fs::remove_file(&exe);
    r.map(|_| format!("Atlas {} is installed.", build.version))
}

#[cfg(not(windows))]
fn install_windows(part: &Path, _dir: &Path, _build: &Build) -> Result<String, String> {
    let _ = std::fs::remove_file(part);
    Err("Game Repo Go runs Atlas's installer on Windows only.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use std::io::Cursor;

    fn b64(text: &str) -> String {
        base64::engine::general_purpose::STANDARD.encode(text)
    }

    /// A test key, and a file signed with it as the Tauri CLI signs Atlas's files (the file's
    /// name in the trusted comment, no version).
    fn key() -> (String, minisign::KeyPair) {
        let kp = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        (b64(&kp.pk.to_box().unwrap().to_string()), kp)
    }

    fn sign(kp: &minisign::KeyPair, data: &[u8]) -> String {
        let sig = minisign::sign(Some(&kp.pk), &kp.sk, Cursor::new(data), Some("timestamp:1763700000\tfile:Atlas.AppImage"), Some("signature from tauri secret key")).unwrap();
        b64(&sig.to_string())
    }

    fn latest(platforms: serde_json::Value) -> String {
        serde_json::json!({ "version": "0.29.0", "notes": "- **Magazines.**", "pub_date": "2026-11-21T10:00:00Z", "platforms": platforms }).to_string()
    }

    #[test]
    fn the_pinned_key_is_atlass_and_reads() {
        // Atlas's tauri.conf.json at v0.29.0: a minisign public key, its id D07A3EBAB3B43CC1
        let text = String::from_utf8(base64::engine::general_purpose::STANDARD.decode(PUBKEY).unwrap()).unwrap();
        assert!(text.starts_with("untrusted comment: minisign public key: D07A3EBAB3B43CC1\n"), "{text}");
        assert!(minisign_verify::PublicKey::decode(&text).is_ok());
        assert_eq!(Source::default(), Source { releases: "https://github.com/DrFlGd/Atlas".into(), pubkey: PUBKEY.into() });
        assert_eq!(Source::default().latest(), "https://github.com/DrFlGd/Atlas/releases/latest/download/latest.json");
    }

    #[test]
    fn latest_json_gives_this_systems_file_of_atlass_releases() {
        let s = Source::default();
        let file = latest(serde_json::json!({
            "windows-x86_64": { "url": "https://github.com/DrFlGd/Atlas/releases/download/v0.29.0/Atlas-0.29.0-setup.exe", "signature": "c2lnLXdpbg==" },
            "linux-x86_64": { "url": "https://github.com/DrFlGd/Atlas/releases/download/v0.29.0/Atlas.AppImage", "signature": "c2lnLWxpbnV4\n" }
        }));
        let w = build_of(&file, "windows-x86_64", &s).unwrap();
        assert_eq!((w.version.as_str(), w.name.as_str(), w.signature.as_str()), ("0.29.0", "Atlas-0.29.0-setup.exe", "c2lnLXdpbg=="));
        let l = build_of(&file, "linux-x86_64", &s).unwrap();
        assert_eq!((l.url.as_str(), l.name.as_str(), l.signature.as_str()), ("https://github.com/DrFlGd/Atlas/releases/download/v0.29.0/Atlas.AppImage", "Atlas.AppImage", "c2lnLWxpbnV4"));
        // a release with Windows only (the AppImage failed to build): said plainly
        let windows_only = latest(serde_json::json!({ "windows-x86_64": { "url": "https://github.com/DrFlGd/Atlas/releases/download/v0.29.0/Atlas-0.29.0-setup.exe", "signature": "c2ln" } }));
        assert_eq!(build_of(&windows_only, "linux-x86_64", &s).unwrap_err(), "Atlas's latest release has no build for this system yet.");
        assert_eq!(build_of(&latest(serde_json::json!({ "linux-x86_64": { "url": "https://github.com/DrFlGd/Atlas/releases/download/v0.29.0/Atlas.AppImage" } })), "linux-x86_64", &s).unwrap_err(), "Atlas's latest release has no build for this system yet.");
        assert!(build_of("<html>Not Found</html>", "linux-x86_64", &s).unwrap_err().contains("no update file"));
        assert!(build_of(r#"{"platforms": {}}"#, "linux-x86_64", &s).unwrap_err().contains("version"));
        // a file on another host, another repository, or not a release file: refused
        for bad in [
            "https://evil.example/DrFlGd/Atlas/releases/download/v0.29.0/Atlas.AppImage",
            "http://github.com/DrFlGd/Atlas/releases/download/v0.29.0/Atlas.AppImage",
            "https://github.com/Someone/Atlas/releases/download/v0.29.0/Atlas.AppImage",
            "https://github.com/DrFlGd/Atlas.evil/releases/download/v0.29.0/Atlas.AppImage",
            "https://github.com/DrFlGd/Atlas/releases/download/v0.29.0/../../../x/Atlas.AppImage",
            "https://github.com/DrFlGd/Atlas/releases/download/v0.29.0/sub/Atlas.AppImage",
            "https://github.com/DrFlGd/Atlas/releases/download/v0.29.0/atlas.sh",
            "https://github.com/DrFlGd/Atlas/releases/download/Atlas.AppImage",
            "https://github.com/DrFlGd/Atlas/archive/v0.29.0.zip",
            "https://objects.githubusercontent.com/Atlas.AppImage",
        ] {
            let f = latest(serde_json::json!({ "linux-x86_64": { "url": bad, "signature": "c2ln" } }));
            assert!(build_of(&f, "linux-x86_64", &s).unwrap_err().contains("isn't one of Atlas's releases"), "{bad}");
        }
        let exe = latest(serde_json::json!({ "windows-x86_64": { "url": "https://github.com/DrFlGd/Atlas/releases/download/v0.29.0/Atlas.AppImage", "signature": "c2ln" } }));
        assert!(build_of(&exe, "windows-x86_64", &s).is_err());
        // a stand-in's releases (the page test) are its own
        let stand_in = Source { releases: "http://127.0.0.1:9/DrFlGd/Atlas".into(), pubkey: "k".into() };
        let f = latest(serde_json::json!({ "linux-x86_64": { "url": "http://127.0.0.1:9/DrFlGd/Atlas/releases/download/v0.29.0/Atlas.AppImage", "signature": "c2ln" } }));
        assert_eq!(build_of(&f, "linux-x86_64", &stand_in).unwrap().name, "Atlas.AppImage");
        assert!(build_of(&f, "linux-x86_64", &s).is_err());
    }

    #[test]
    fn redirects_are_followed_only_to_githubs_hosts_for_release_files() {
        let s = Source::default();
        for ok in [
            "https://github.com/DrFlGd/Atlas/releases/latest/download/latest.json",
            "https://github.com/DrFlGd/Atlas/releases/download/v0.29.0/latest.json",
            "https://objects.githubusercontent.com/github-production-release-asset-2e65be/1/2?X-Amz-Signature=abc",
            "https://release-assets.githubusercontent.com/github-production-release-asset/1/2?sp=r&sig=x",
        ] {
            assert!(may_fetch(ok, &s), "{ok}");
        }
        for bad in [
            "http://objects.githubusercontent.com/x",
            "https://objects.githubusercontent.com.evil.example/x",
            "https://evil.example/?objects.githubusercontent.com",
            "https://user@objects.githubusercontent.com/x",
            "https://objects.githubusercontent.com:8443/x",
            "https://github.com/DrFlGd/Atlas/releases/latest/download/other.json",
            "https://github.com/DrFlGd/Atlas/releases/download/../../Other/releases/download/x",
            "https://github.com/Someone/Atlas/releases/download/v1/Atlas.AppImage",
            "file:///etc/passwd",
            "",
        ] {
            assert!(!may_fetch(bad, &s), "{bad}");
        }
    }

    #[test]
    fn a_file_is_checked_against_atlass_signature_good_bad_or_missing() {
        let (pubkey, kp) = key();
        let file = b"\x7fELF an AppImage".repeat(5000);
        let sig = sign(&kp, &file);
        assert_eq!(verify(&file, &sig, &pubkey), Ok(()));
        // changed after signing
        let mut changed = file.clone();
        changed[100] ^= 1;
        assert!(verify(&changed, &sig, &pubkey).unwrap_err().contains("doesn't match Atlas's signature"));
        // signed by another key (Atlas's real one here)
        assert!(verify(&file, &sig, PUBKEY).unwrap_err().contains("doesn't match Atlas's signature"));
        let (other, _) = key();
        assert!(verify(&file, &sig, &other).unwrap_err().contains("doesn't match"));
        // missing, or not a signature
        for bad in ["", "   ", "not a signature", &b64("untrusted comment: x\nnot base64\n")] {
            assert!(verify(&file, bad, &pubkey).unwrap_err().contains("signature that can't be read"), "{bad:?}");
        }
        // and build_of never takes a missing one
        let f = latest(serde_json::json!({ "linux-x86_64": { "url": "https://github.com/DrFlGd/Atlas/releases/download/v0.29.0/Atlas.AppImage", "signature": "" } }));
        assert!(build_of(&f, "linux-x86_64", &Source::default()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn the_appimage_goes_into_applications_runnable_and_never_over_one_there() {
        use std::os::unix::fs::PermissionsExt;
        let base = std::env::temp_dir().join(format!("gamerepo-atlas-install-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (home, data) = (base.join("home"), base.join("data"));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(data.join(DOWNLOADS)).unwrap();
        let part = data.join(DOWNLOADS).join("Atlas.AppImage.part");
        std::fs::write(&part, b"\x7fELF new").unwrap();
        // ~/Applications isn't there: it's made
        let to = place_appimage(&part, &home).unwrap();
        assert_eq!(to, home.join("Applications/Atlas.AppImage"));
        assert_eq!(std::fs::read(&to).unwrap(), b"\x7fELF new");
        assert_eq!(std::fs::metadata(&to).unwrap().permissions().mode() & 0o777, 0o755);
        assert!(!part.exists());
        // nothing left beside it
        let left: Vec<_> = std::fs::read_dir(home.join("Applications")).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left, ["Atlas.AppImage"]);
        // one there already (Atlas's own, updated by itself) is never written over
        std::fs::write(&part, b"\x7fELF newer").unwrap();
        let e = place_appimage(&part, &home).unwrap_err().to_string();
        assert!(e.contains("already"), "{e}");
        assert_eq!(std::fs::read(&to).unwrap(), b"\x7fELF new");
        assert!(!part.exists());
        // nor a link someone put there
        std::fs::remove_file(&to).unwrap();
        std::os::unix::fs::symlink(base.join("nowhere"), &to).unwrap();
        std::fs::write(&part, b"\x7fELF newer").unwrap();
        assert!(place_appimage(&part, &home).is_err());
        assert!(std::fs::symlink_metadata(&to).unwrap().file_type().is_symlink());
        let left: Vec<_> = std::fs::read_dir(home.join("Applications")).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left, ["Atlas.AppImage"]);
        let _ = std::fs::remove_dir_all(&base);
    }

    type Routes = Vec<(String, u16, String, Vec<u8>)>;

    /// A stand-in for GitHub's releases, its routes made knowing its own address: each path
    /// answered with (status, Location, body), else 404. Its address, and what was asked.
    fn stand_in(routes: impl FnOnce(&str) -> Routes) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let routes = routes(&base);
        let asked = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
        let seen = asked.clone();
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(mut s) = conn else { continue };
                let mut req = vec![];
                let mut buf = [0u8; 4096];
                while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                    match s.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => req.extend_from_slice(&buf[..n]),
                    }
                }
                let line = String::from_utf8_lossy(&req).lines().next().unwrap_or("").to_string();
                let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
                seen.lock().unwrap().push(path.clone());
                let (code, location, body) = routes.iter().find(|r| r.0 == path).map(|r| (r.1, r.2.clone(), r.3.clone())).unwrap_or((404, String::new(), b"Not Found".to_vec()));
                let loc = if location.is_empty() { String::new() } else { format!("Location: {location}\r\n") };
                let head = format!("HTTP/1.1 {code} X\r\n{loc}Content-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = s.write_all(head.as_bytes());
                let _ = s.write_all(&body);
            }
        });
        (base, asked)
    }

    fn route(path: &str, code: u16, to: &str, body: &[u8]) -> (String, u16, String, Vec<u8>) {
        (path.to_string(), code, to.to_string(), body.to_vec())
    }

    const LATEST: &str = "/DrFlGd/Atlas/releases/latest/download/latest.json";
    const TAGGED: &str = "/DrFlGd/Atlas/releases/download/v0.29.0/latest.json";
    const FILE: &str = "/DrFlGd/Atlas/releases/download/v0.29.0/Atlas.AppImage";

    fn linux_latest(base: &str, sig: &str) -> String {
        latest(serde_json::json!({ "linux-x86_64": { "url": format!("{base}{FILE}"), "signature": sig } }))
    }

    fn source_of(base: &str, pubkey: &str) -> Source {
        Source { releases: format!("{base}/DrFlGd/Atlas"), pubkey: pubkey.to_string() }
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    #[test]
    fn atlas_is_installed_from_its_latest_release_checked_and_a_bad_one_leaves_nothing() {
        use std::os::unix::fs::PermissionsExt;
        let (pubkey, kp) = key();
        let appimage = b"\x7fELF a stand-in for Atlas.AppImage".repeat(300);
        let sig = sign(&kp, &appimage);
        let tmp = std::env::temp_dir().join(format!("gamerepo-atlas-get-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let (home, data) = (tmp.join("home"), tmp.join("data"));
        std::fs::create_dir_all(&home).unwrap();
        let to = home.join("Applications/Atlas.AppImage");
        let nothing_left = |data: &Path| !data.join(DOWNLOADS).exists();

        // a file on another server than the release's: refused as not Atlas's
        let (other, _) = stand_in(|_| vec![route(FILE, 200, "", &appimage)]);
        let (base, asked) = stand_in(|_| vec![route(LATEST, 200, "", linux_latest(&other, &sig).as_bytes())]);
        let e = install(&source_of(&base, &pubkey), &data, &home, &mut |_| {}).unwrap_err();
        assert!(e.contains("isn't one of Atlas's releases"), "{e}");
        assert_eq!(asked.lock().unwrap().len(), 1);

        // a file changed after it was signed: deleted, nothing installed, nothing left
        let mut changed = appimage.clone();
        changed[7] ^= 1;
        let (base, _) = stand_in(|base| vec![route(LATEST, 200, "", linux_latest(base, &sig).as_bytes()), route(FILE, 200, "", &changed)]);
        let e = install(&source_of(&base, &pubkey), &data, &home, &mut |_| {}).unwrap_err();
        assert!(e.contains("doesn't match Atlas's signature") && e.contains("deleted"), "{e}");
        assert!(!home.join("Applications").exists() && nothing_left(&data));
        // signed with another key (Atlas's real one here): refused before anything's downloaded
        let (base, asked) = stand_in(|base| vec![route(LATEST, 200, "", linux_latest(base, &sig).as_bytes()), route(FILE, 200, "", &appimage)]);
        assert!(install(&source_of(&base, PUBKEY), &data, &home, &mut |_| {}).unwrap_err().contains("doesn't match"));
        assert_eq!(asked.lock().unwrap().as_slice(), [LATEST]);
        assert!(!home.join("Applications").exists() && nothing_left(&data));
        // a redirect elsewhere than GitHub's: not followed
        let (base, asked) = stand_in(|_| vec![route(LATEST, 302, "https://evil.example/latest.json", b"")]);
        let e = install(&source_of(&base, &pubkey), &data, &home, &mut |_| {}).unwrap_err();
        assert!(e.contains("evil.example") && e.contains("isn't GitHub's"), "{e}");
        assert_eq!(asked.lock().unwrap().len(), 1);
        // no release yet
        let (base, _) = stand_in(|_| vec![]);
        assert!(install(&source_of(&base, &pubkey), &data, &home, &mut |_| {}).unwrap_err().contains("404"));
        // a release with no build for Linux
        let windows_only = latest(serde_json::json!({ "windows-x86_64": { "url": "x", "signature": "c2ln" } }));
        let (base, _) = stand_in(|_| vec![route(LATEST, 200, "", windows_only.as_bytes())]);
        assert_eq!(install(&source_of(&base, &pubkey), &data, &home, &mut |_| {}).unwrap_err(), "Atlas's latest release has no build for this system yet.");

        // a good release, its latest.json reached through redirects as GitHub's is
        let (base, asked) = stand_in(|base| {
            vec![route(LATEST, 302, &format!("{base}{TAGGED}"), b""), route(TAGGED, 200, "", linux_latest(base, &sig).as_bytes()), route(FILE, 200, "", &appimage)]
        });
        let source = source_of(&base, &pubkey);
        let mut steps = vec![];
        let said = install(&source, &data, &home, &mut |p| steps.push(p.state)).unwrap();
        assert_eq!(said, format!("Atlas 0.29.0 is installed at {}.", to.display()));
        assert_eq!(std::fs::read(&to).unwrap(), appimage);
        assert_eq!(std::fs::metadata(&to).unwrap().permissions().mode() & 0o777, 0o755);
        assert!(nothing_left(&data));
        assert_eq!(steps.first(), Some(&"reading"));
        assert!(steps.contains(&"downloading") && steps.last() == Some(&"installing"), "{steps:?}");
        assert_eq!(asked.lock().unwrap().as_slice(), [LATEST, TAGGED, FILE]);
        // pressed again with one there: left as it is, never downloaded again
        let e = install(&source, &data, &home, &mut |_| {}).unwrap_err();
        assert!(e.contains("already"), "{e}");
        assert_eq!(asked.lock().unwrap().len(), 3);
        assert_eq!(std::fs::read(&to).unwrap(), appimage);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn this_systems_entry() {
        let p = platform();
        if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            assert_eq!(p, Some("linux-x86_64"));
        } else if cfg!(all(windows, target_arch = "x86_64")) {
            assert_eq!(p, Some("windows-x86_64"));
        }
    }
}
