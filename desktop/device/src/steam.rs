//! Game Repo Go in Steam (docs/PLAN.md, "Phase 9c design", D107, D109, D112): the
//! non-Steam games it wants in Steam (itself, so Game Mode can start it, and the PC games
//! it puts on the device), kept in `steam.json` in its data folder, and written into
//! Steam's own files only while Steam is closed. While Steam runs they wait; *Restart
//! Steam* starts a helper that outlives Steam (and Game Repo Go, which Steam closes with it
//! in Game Mode), closes Steam, writes them and starts Steam again.

use crate::app::Settings;
use anyhow::{anyhow, bail, Result};
use gamerepo_steam::Shortcut;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The tag on Game Repo Go's shortcuts (Game Repo's own export uses "Game Repo").
pub const TAG: &str = "Game Repo Go";
/// The key of Game Repo Go's own shortcut.
pub const OWN: &str = "go";
/// Backups of Steam's files are named `<file>.gamerepogo-<time>`.
const BACKUP: &str = "gamerepogo";
/// Steam's grid pictures by the media kinds (as D28 names them).
const GRID: &[(&str, &str)] = &[("cover", "p"), ("banner", ""), ("hero", "_hero"), ("logo", "_logo"), ("icon", "_icon")];

/// A shortcut Game Repo Go wants in Steam.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Wanted {
    /// `go` for the app itself, else the catalogue's key of a PC game
    pub key: String,
    pub name: String,
    pub program: String,
    pub start_dir: String,
    pub args: String,
    pub icon: String,
    /// Run through Proton (a Windows program): the Proton chosen in Settings
    pub proton: bool,
    /// Pictures for Steam's grid: media kind → file
    pub art: BTreeMap<String, String>,
}

impl Wanted {
    fn shortcut(&self) -> Shortcut {
        Shortcut::new(&self.name, Path::new(&self.program), Path::new(&self.start_dir), &self.args, &self.icon)
    }
}

/// What was last written into Steam.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Written {
    pub steam: String,
    pub user: String,
    pub appids: Vec<u32>,
    /// The shortcuts given a Proton, and which
    pub tools: BTreeMap<u32, String>,
    /// The grid files written
    pub grid: Vec<String>,
    pub at: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct State {
    pub wanted: Vec<Wanted>,
    pub written: Written,
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn path(data: &Path) -> PathBuf {
    data.join("steam.json")
}

pub fn state(data: &Path) -> State {
    std::fs::read(path(data)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save(data: &Path, s: &State) -> Result<()> {
    crate::write_whole(&path(data), &serde_json::to_vec_pretty(s)?)
}

fn settings(data: &Path) -> Settings {
    std::fs::read(data.join("settings.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Steam's folder (Settings', else found) and the user to write for (Settings', else the
/// one signed in most recently).
pub fn place(data: &Path) -> Result<(PathBuf, String)> {
    let s = settings(data);
    let set = Some(PathBuf::from(&s.steam)).filter(|_| !s.steam.is_empty());
    let steam = gamerepo_steam::find(set.as_deref()).ok_or_else(|| anyhow!("Steam isn't on this device (or choose its folder in Settings)."))?;
    let users = gamerepo_steam::users(&steam);
    let user = if !s.steam_user.is_empty() && users.iter().any(|u| u["id"] == s.steam_user.as_str()) {
        s.steam_user.clone()
    } else {
        users.first().and_then(|u| u["id"].as_str()).map(str::to_string).ok_or_else(|| anyhow!("Nobody has signed in to Steam on this device yet."))?
    };
    Ok((steam, user))
}

/// What would be written now: the shortcuts, the Proton for each that has one, the grid.
fn target(data: &Path, s: &State) -> (Vec<Shortcut>, BTreeMap<u32, String>, Vec<(String, PathBuf)>) {
    let proton = settings(data).proton;
    let mut shortcuts = vec![];
    let mut tools = BTreeMap::new();
    let mut grid = vec![];
    for w in &s.wanted {
        let sc = w.shortcut();
        if w.proton {
            tools.insert(sc.appid, proton.clone());
        }
        for (kind, suffix) in GRID {
            if let Some(src) = w.art.get(*kind).map(PathBuf::from).filter(|p| p.is_file()) {
                let ext = src.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_else(|| "png".into());
                grid.push((format!("{}{suffix}.{ext}", sc.appid), src));
            }
        }
        shortcuts.push(sc);
    }
    (shortcuts, tools, grid)
}

/// Whether what's wanted differs from what was last written (for Steam's folder and user now).
pub fn waiting(data: &Path) -> bool {
    let s = state(data);
    let Ok((steam, user)) = place(data) else { return false };
    let (shortcuts, tools, grid) = target(data, &s);
    let w = &s.written;
    let ids: BTreeSet<u32> = shortcuts.iter().map(|x| x.appid).collect();
    let grid: BTreeSet<&String> = grid.iter().map(|(n, _)| n).collect();
    if ids.is_empty() && tools.is_empty() && grid.is_empty() && w.appids.is_empty() && w.tools.is_empty() && w.grid.is_empty() {
        return false;
    }
    w.steam != steam.display().to_string()
        || w.user != user
        || w.appids.iter().copied().collect::<BTreeSet<_>>() != ids
        || w.tools != tools
        || w.grid.iter().collect::<BTreeSet<_>>() != grid
}

/// Want a shortcut (put in place of one with its key), or none for `key`; then written at
/// once if Steam's closed. Gives whether it's still waiting.
pub fn want(data: &Path, key: &str, w: Option<Wanted>) -> Result<bool> {
    let mut s = state(data);
    s.wanted.retain(|x| x.key != key);
    if let Some(mut w) = w {
        w.key = key.to_string();
        s.wanted.push(w);
    }
    save(data, &s)?;
    if place(data).is_ok() && waiting(data) && !gamerepo_steam::running() {
        apply(data)?;
    }
    Ok(waiting(data))
}

/// Write what's wanted into Steam: only while it's closed.
pub fn apply(data: &Path) -> Result<Value> {
    let (steam, user) = place(data)?;
    if gamerepo_steam::running() {
        bail!("Steam is running: its files are written once it's closed (Restart Steam does it).");
    }
    let mut s = state(data);
    let (shortcuts, tools, grid) = target(data, &s);
    let stamp = now().to_string();
    let old = s.written.clone();
    // another user or Steam folder before: Game Repo Go's shortcuts come out of that one
    if (!old.user.is_empty() && old.user != user) || (!old.steam.is_empty() && old.steam != steam.display().to_string()) {
        let old_steam = PathBuf::from(&old.steam);
        if gamerepo_steam::config_dir(&old_steam, &old.user).is_ok() {
            let _ = gamerepo_steam::put_shortcuts(&old_steam, &old.user, TAG, &[], BACKUP, &stamp);
        }
    }
    let (others, _) = gamerepo_steam::put_shortcuts(&steam, &user, TAG, &shortcuts, BACKUP, &stamp)?;
    // Proton: as wanted, and out again for the shortcuts given one before that aren't now
    let mut set: Vec<(u32, Option<String>)> = old.tools.keys().filter(|id| !tools.contains_key(id)).map(|id| (*id, None)).collect();
    set.extend(tools.iter().filter(|(id, t)| old.tools.get(id) != Some(t)).map(|(id, t)| (*id, Some(t.clone()))));
    if !set.is_empty() {
        gamerepo_steam::put_compat_tools(&steam, &set, BACKUP, &stamp)?;
    }
    // the grid: what was written before goes, then this time's
    let dir = gamerepo_steam::config_dir(&steam, &user)?.join("grid");
    for f in &old.grid {
        let _ = std::fs::remove_file(dir.join(f));
    }
    let mut written = vec![];
    for (name, src) in &grid {
        std::fs::create_dir_all(&dir)?;
        if std::fs::copy(src, dir.join(name)).is_ok() {
            written.push(name.clone());
        }
    }
    s.written = Written { steam: steam.display().to_string(), user: user.clone(), appids: shortcuts.iter().map(|x| x.appid).collect(), tools, grid: written, at: now() };
    save(data, &s)?;
    Ok(json!({ "shortcuts": shortcuts.len(), "others": others }))
}

/// For the page: Steam found or not, running, its users, what's wanted and whether it waits.
pub fn status(data: &Path) -> Value {
    let s = settings(data);
    let st = state(data);
    let set = Some(PathBuf::from(&s.steam)).filter(|_| !s.steam.is_empty());
    let Some(steam) = gamerepo_steam::find(set.as_deref()) else {
        return json!({ "found": false, "wanted": st.wanted.len(), "own": st.wanted.iter().any(|w| w.key == OWN) });
    };
    let users = gamerepo_steam::users(&steam);
    let user = place(data).map(|(_, u)| u).unwrap_or_default();
    json!({
        "found": true, "folder": steam.display().to_string(), "running": gamerepo_steam::running(), "users": users, "user": user,
        "wanted": st.wanted.iter().map(|w| json!({ "key": w.key, "name": w.name, "proton": w.proton })).collect::<Vec<_>>(),
        "own": st.wanted.iter().any(|w| w.key == OWN), "waiting": waiting(data), "written": st.written.at,
        "game_mode": game_mode(),
    })
}

/// Whether this is the Steam Deck's Game Mode (or Steam's Big Picture on another Linux PC
/// with gamescope): Steam says so to what it starts, and gamescope does to what runs in it.
pub fn game_mode() -> bool {
    let env = |k: &str| std::env::var(k).unwrap_or_default();
    env("SteamGamepadUI") == "1" || env("XDG_CURRENT_DESKTOP").eq_ignore_ascii_case("gamescope") || !env("GAMESCOPE_WAYLAND_DISPLAY").is_empty()
}

/// Start the helper (`helper`, the command line that runs [`helper`]) apart from Game Repo
/// Go and from Steam: as a unit of the user's own systemd, which Steam's closing doesn't
/// reach, else in a session of its own.
pub fn restart(helper: &[String]) -> Result<()> {
    let Some((program, args)) = helper.split_first() else { bail!("Steam can't be restarted from here.") };
    let unit = format!("gamerepo-go-steam-{}", now());
    // the unit gets the user manager's surroundings, not ours: what finding and starting Steam needs
    let mut run = vec!["--user".into(), "--collect".into(), "--quiet".into(), "-p".into(), "KillMode=process".into(), format!("--unit={unit}")];
    for k in ["PATH", "HOME", "DISPLAY", "WAYLAND_DISPLAY", "XAUTHORITY", "XDG_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS", "XDG_CURRENT_DESKTOP", "XDG_SESSION_TYPE"] {
        if let Ok(v) = std::env::var(k) {
            run.push(format!("--setenv={k}={v}"));
        }
    }
    run.push(program.clone());
    let tries: [(&str, Vec<String>); 2] = [
        ("systemd-run", [run, args.to_vec()].concat()),
        ("setsid", [vec!["-f".into(), program.clone()], args.to_vec()].concat()),
    ];
    for (cmd, a) in tries {
        if std::process::Command::new(cmd).args(&a).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success()) {
            return Ok(());
        }
    }
    std::process::Command::new(program).args(args).spawn()?;
    Ok(())
}

/// The helper: close Steam, write what's waiting, start Steam again (outside Game Mode;
/// Game Mode brings it back by itself). What it did goes in `steam-restart.log`.
pub fn helper(data: &Path, in_game_mode: bool) -> Result<()> {
    let log_path = data.join("steam-restart.log");
    let log = |line: &str| {
        let text = format!("{} {line}\n", now());
        let _ = std::fs::OpenOptions::new().create(true).append(true).open(&log_path).and_then(|mut f| std::io::Write::write_all(&mut f, text.as_bytes()));
    };
    log(&format!("restarting Steam (Game Mode: {in_game_mode})"));
    if gamerepo_steam::running() {
        let _ = std::process::Command::new("steam").arg("-shutdown").stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
        let end = Instant::now() + Duration::from_secs(90);
        while gamerepo_steam::running() {
            if Instant::now() > end {
                log("Steam didn't close within 90 s: nothing written");
                bail!("Steam didn't close.");
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }
    // a moment for Steam's last writes
    std::thread::sleep(Duration::from_secs(2));
    let r = apply(data);
    match &r {
        Ok(v) => log(&format!("written: {v}")),
        Err(e) => log(&format!("not written: {e:#}")),
    }
    if !in_game_mode {
        let started = std::process::Command::new("setsid").args(["-f", "steam"]).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status();
        if !started.is_ok_and(|s| s.success()) {
            let _ = std::process::Command::new("steam").spawn();
        }
        log("Steam started again");
    }
    r.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_waits_is_written_once_and_taken_out_again() {
        let dir = std::env::temp_dir().join(format!("gamerepo-go-steam-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (data, steam) = (dir.join("data"), dir.join("Steam"));
        std::fs::create_dir_all(steam.join("userdata/22202/config")).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        let s = Settings { steam: steam.display().to_string(), ..Default::default() };
        std::fs::write(data.join("settings.json"), serde_json::to_vec(&s).unwrap()).unwrap();
        std::fs::write(dir.join("cover.png"), b"png").unwrap();
        assert_eq!(place(&data).unwrap(), (steam.clone(), "22202".to_string()));
        assert!(!waiting(&data), "nothing wanted, nothing written");
        let game = Wanted {
            name: "Zeta Quest".into(),
            program: "/home/deck/Games/Zeta Quest (1999)/zeta.exe".into(),
            start_dir: "/home/deck/Games/Zeta Quest (1999)".into(),
            proton: true,
            art: BTreeMap::from([("cover".into(), dir.join("cover.png").display().to_string())]),
            ..Default::default()
        };
        let mut st = state(&data);
        st.wanted.push(Wanted { key: "pcgames/zeta".into(), ..game.clone() });
        save(&data, &st).unwrap();
        assert!(waiting(&data));
        // (no Steam runs in the tests, or this test is told so)
        if gamerepo_steam::running() {
            return;
        }
        apply(&data).unwrap();
        assert!(!waiting(&data));
        let id = game.shortcut().appid;
        assert_eq!(gamerepo_steam::tagged_ids(&steam, "22202", TAG), [id]);
        assert_eq!(gamerepo_steam::compat_tool(&steam, id).as_deref(), Some("proton_experimental"));
        assert!(steam.join(format!("userdata/22202/config/grid/{id}p.png")).is_file());
        // not wanted any more: its shortcut, Proton and picture go
        assert!(!want(&data, "pcgames/zeta", None).unwrap());
        assert!(gamerepo_steam::tagged_ids(&steam, "22202", TAG).is_empty());
        assert_eq!(gamerepo_steam::compat_tool(&steam, id), None);
        assert!(!steam.join(format!("userdata/22202/config/grid/{id}p.png")).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
