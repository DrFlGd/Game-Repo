//! Game Repo Go in Steam (docs/PLAN.md, "Phase 9c design", D107, D109, D112): the
//! non-Steam games it wants in Steam (itself, so Game Mode can start it, and the PC games
//! it puts on the device), kept in `steam.json` in its data folder, and written into
//! Steam's own files only while Steam is closed. While Steam runs they wait; *Restart
//! Steam* starts a helper that outlives Steam (and Game Repo Go, which Steam closes with it
//! in Game Mode), closes Steam, writes them and starts Steam again.
//!
//! On Linux a PC game's shortcut can start Atlas with the game (D242, "Maps and Guides
//! notes, part 11"): the contract's launch line ([`crate::launch`]) goes in its launch
//! options when Go's line is wanted (`atlas_line`, from Settings' *Start Atlas with my
//! games*) and the game's switch isn't off (`atlas_off`). Launch options the person changed
//! in Steam are never written over: each shortcut's are read back from shortcuts.vdf just
//! before it's written ([`crate::launch::resolve`]).

use crate::app::Settings;
use crate::launch::{self, Outcome};
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
    /// Go's own launch options of each shortcut when they were written (the game's
    /// arguments), so a change of them in the library is still taken as Go's
    pub launch: BTreeMap<u32, String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct State {
    pub wanted: Vec<Wanted>,
    pub written: Written,
    /// The launch line that starts Atlas with a game (D242), when Go puts it in its games'
    /// shortcuts: Settings' *Start Atlas with my games* on, and a line for the AppImage
    pub atlas_line: Option<String>,
    /// The games whose own switch for Atlas is off, by their keys
    pub atlas_off: BTreeSet<String>,
}

impl State {
    /// The line wanted in a shortcut's launch options: none for Game Repo Go's own, nor for
    /// a game whose switch is off.
    fn line_for(&self, key: &str) -> Option<&str> {
        (key != OWN && !self.atlas_off.contains(key)).then_some(self.atlas_line.as_deref()).flatten()
    }
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

/// Each wanted shortcut's launch options worked out against what Steam's file has now
/// (`now`, by id): its id, and what to write ([`launch::resolve`]).
fn launches(s: &State, shortcuts: &[Shortcut], now: &BTreeMap<u32, String>) -> Vec<(u32, launch::Resolved)> {
    s.wanted
        .iter()
        .zip(shortcuts)
        .map(|(w, sc)| (sc.appid, launch::resolve(now.get(&sc.appid).map(String::as_str), &w.args, s.line_for(&w.key), s.written.launch.get(&sc.appid).map(String::as_str))))
        .collect()
}

/// Whether what's wanted differs from what was last written (for Steam's folder and user now).
pub fn waiting(data: &Path) -> bool {
    let s = state(data);
    let Ok((steam, user)) = place(data) else { return false };
    let (shortcuts, tools, grid) = target(data, &s);
    // a shortcut's launch options not yet as they should be (Atlas's line, D242)
    let now = gamerepo_steam::launch_options(&steam, &user, TAG);
    if launches(&s, &shortcuts, &now).iter().any(|(id, r)| now.get(id) != Some(&r.value)) {
        return true;
    }
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
    write_if_closed(data)
}

/// What's waiting written now if Steam's closed; gives whether it's still waiting.
fn write_if_closed(data: &Path) -> Result<bool> {
    if place(data).is_ok() && waiting(data) && !gamerepo_steam::running() {
        apply(data)?;
    }
    Ok(waiting(data))
}

/// The launch line Go puts in its games' shortcuts to start Atlas with them (D242), or none;
/// written at once if Steam's closed and it changed. Gives whether something waits.
pub fn set_atlas_line(data: &Path, line: Option<String>) -> Result<bool> {
    let mut s = state(data);
    if s.atlas_line != line {
        s.atlas_line = line;
        save(data, &s)?;
    }
    write_if_closed(data)
}

/// A game's own switch for starting Atlas with it (D242); written at once if Steam's closed.
pub fn set_atlas_game(data: &Path, key: &str, on: bool) -> Result<bool> {
    let mut s = state(data);
    let changed = if on { s.atlas_off.remove(key) } else { s.atlas_off.insert(key.to_string()) };
    if changed {
        save(data, &s)?;
    }
    write_if_closed(data)
}

/// Starting Atlas with each of Go's games in Steam (D242), for the page: its key and name,
/// its switch, how its launch options come out ([`Outcome`]), and whether that's still to be
/// written. `only`: one game's.
pub fn atlas_games(data: &Path, only: Option<&str>) -> Vec<Value> {
    let s = state(data);
    let (shortcuts, _, _) = target(data, &s);
    let now = place(data).map(|(steam, user)| gamerepo_steam::launch_options(&steam, &user, TAG)).unwrap_or_default();
    s.wanted
        .iter()
        .zip(launches(&s, &shortcuts, &now))
        .filter(|(w, _)| w.key != OWN && only.is_none_or(|k| k == w.key))
        .map(|(w, (id, r))| {
            json!({ "key": w.key, "name": w.name, "switch": !s.atlas_off.contains(&w.key), "outcome": r.outcome,
                "waiting": now.get(&id) != Some(&r.value), "options": now.get(&id) })
        })
        .collect()
}

/// Write what's wanted into Steam: only while it's closed.
pub fn apply(data: &Path) -> Result<Value> {
    let (steam, user) = place(data)?;
    if gamerepo_steam::running() {
        bail!("Steam is running: its files are written once it's closed (Restart Steam does it).");
    }
    let mut s = state(data);
    let (mut shortcuts, tools, grid) = target(data, &s);
    // each shortcut's launch options read back from Steam's file first: the person's own are
    // never written over, Go's line goes in or out of Go's own (D242)
    let launches = launches(&s, &shortcuts, &gamerepo_steam::launch_options(&steam, &user, TAG));
    for (sc, (_, r)) in shortcuts.iter_mut().zip(&launches) {
        sc.args = r.value.clone();
    }
    let launch: BTreeMap<u32, String> = s.wanted.iter().zip(&shortcuts).map(|(w, sc)| (sc.appid, w.args.clone())).collect();
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
    s.written = Written { steam: steam.display().to_string(), user: user.clone(), appids: shortcuts.iter().map(|x| x.appid).collect(), tools, grid: written, at: now(), launch };
    save(data, &s)?;
    let theirs = launches.iter().filter(|(_, r)| r.outcome == Outcome::Theirs).count();
    Ok(json!({ "shortcuts": shortcuts.len(), "others": others, "theirs": theirs }))
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

    /// A shortcut's launch options in Steam's file, as Steam would leave them after the person
    /// changed them in a game's Properties.
    fn set_options_in_steam(steam: &Path, appid: u32, options: &str) {
        let file = steam.join("userdata/22202/config/shortcuts.vdf");
        let top = gamerepo_steam::parse(&std::fs::read(&file).unwrap()).unwrap();
        let list = gamerepo_steam::shortcuts_of(&top)
            .into_iter()
            .map(|v| match v {
                gamerepo_steam::Vdf::Map(m) if gamerepo_steam::Vdf::Map(m.clone()).int("appid") == Some(appid) => gamerepo_steam::Vdf::Map(
                    m.into_iter().map(|(k, x)| if k.eq_ignore_ascii_case(b"LaunchOptions") { (k, gamerepo_steam::Vdf::Str(options.as_bytes().to_vec())) } else { (k, x) }).collect(),
                ),
                other => other,
            })
            .collect();
        std::fs::write(&file, gamerepo_steam::write(&gamerepo_steam::with_shortcuts(top, list))).unwrap();
    }

    #[test]
    fn atlass_line_goes_only_into_gos_own_launch_options_and_follows_the_appimage() {
        let dir = std::env::temp_dir().join(format!("gamerepo-go-steam-atlas-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (data, steam) = (dir.join("data"), dir.join("Steam"));
        std::fs::create_dir_all(steam.join("userdata/22202/config")).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        let s = Settings { steam: steam.display().to_string(), ..Default::default() };
        std::fs::write(data.join("settings.json"), serde_json::to_vec(&s).unwrap()).unwrap();
        if gamerepo_steam::running() {
            return;
        }
        let game = |name: &str, args: &str| Wanted { name: name.into(), program: format!("/home/deck/Games/{name}/game.sh"), start_dir: format!("/home/deck/Games/{name}"), args: args.into(), ..Default::default() };
        want(&data, "pcgames/zeta", Some(game("Zeta Quest", ""))).unwrap();
        want(&data, "pcgames/eta", Some(game("Eta Racer", "-windowed"))).unwrap();
        want(&data, OWN, Some(game("Game Repo Go", ""))).unwrap();
        let id = |name: &str| game(name, "").shortcut().appid;
        let (zeta, eta, own) = (id("Zeta Quest"), id("Eta Racer"), id("Game Repo Go"));
        let options = || gamerepo_steam::launch_options(&steam, "22202", TAG);
        assert_eq!(options(), BTreeMap::from([(zeta, String::new()), (eta, "-windowed".into()), (own, String::new())]));
        // the line wanted: written into the games' (empty, or the game's own arguments after
        // it), never Game Repo Go's own
        let line = launch::line("/home/deck/Applications/Atlas.AppImage");
        assert!(!set_atlas_line(&data, Some(line.clone())).unwrap());
        assert_eq!(options(), BTreeMap::from([(zeta, line.clone()), (eta, format!("{line} -windowed")), (own, String::new())]));
        assert!(atlas_games(&data, None).iter().all(|g| g["outcome"] == "on" && g["waiting"] == false), "{:?}", atlas_games(&data, None));
        // the person changes Zeta's in Steam: never written over, said so
        set_options_in_steam(&steam, zeta, "gamemoderun %command%");
        assert!(!waiting(&data), "theirs is as it should be");
        apply(&data).unwrap();
        assert_eq!(options()[&zeta], "gamemoderun %command%");
        assert_eq!(atlas_games(&data, Some("pcgames/zeta"))[0]["outcome"], "theirs");
        // the AppImage moved (linuxAppImage changed): Go's own lines follow it, theirs stays
        let moved = launch::line("/run/media/deck/SD/Atlas.AppImage");
        set_atlas_line(&data, Some(moved.clone())).unwrap();
        assert_eq!(options(), BTreeMap::from([(zeta, "gamemoderun %command%".into()), (eta, format!("{moved} -windowed")), (own, String::new())]));
        // Eta's switch off: exactly Go's line comes out, its own arguments stay
        set_atlas_game(&data, "pcgames/eta", false).unwrap();
        assert_eq!(options()[&eta], "-windowed");
        assert_eq!(atlas_games(&data, Some("pcgames/eta"))[0]["switch"], false);
        set_atlas_game(&data, "pcgames/eta", true).unwrap();
        assert_eq!(options()[&eta], format!("{moved} -windowed"));
        // the setting off: every line of Go's out, the person's left as they are
        set_atlas_line(&data, None).unwrap();
        assert_eq!(options(), BTreeMap::from([(zeta, "gamemoderun %command%".into()), (eta, "-windowed".into()), (own, String::new())]));
        // while Steam's files can't be written, it waits (a change made with Steam running)
        set_options_in_steam(&steam, eta, &format!("{moved} -windowed"));
        assert!(waiting(&data), "Go's line out of Eta's is still to be written");
        apply(&data).unwrap();
        assert!(!waiting(&data));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
