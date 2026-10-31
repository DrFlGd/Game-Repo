//! Saves synced with Game Repo Go closed (docs/PLAN.md, D186, "Save sync notes, part 5"): the
//! program started on its own every few minutes with `--sync-saves <data folder>`, which
//! syncs and ends. On Windows that's a task of Task Scheduler's for the user (each five
//! minutes while they're signed in); on Linux and the Steam Deck a systemd user timer, which
//! runs in Game Mode too. On Android it's a job of WorkManager's instead, a [`Scheduler`]
//! the Kotlin plugin keeps (every fifteen minutes or so, when Android lets it), which calls
//! back into the Rust library (desktop/client/src/android.rs). Turned on and off by Settings ›
//! Saves › *Sync saves in the background*; set up again each time Go starts, so it follows
//! the program if it moves.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

/// How often it runs.
pub const EVERY_MINUTES: u32 = 5;

/// The task's and the units' name.
pub const NAME: &str = "Game Repo Go saves";
const UNIT: &str = "game-repo-go-saves";

/// What else sets the sync in the background up: Android's WorkManager (D186), through Go's
/// Kotlin plugin.
pub trait Scheduler: Send + Sync {
    /// On, syncing the saves of the Go whose data is in `data`, or off.
    fn set(&self, on: bool, data: &Path) -> Result<()>;
    /// About how often it runs, in minutes.
    fn every(&self) -> u32;
}

/// Who sets it up: Task Scheduler or systemd, starting this program (with whether that's
/// really so, not the stand-in's units only), or a [`Scheduler`].
pub enum By {
    Program(PathBuf, bool),
    Scheduler(Box<dyn Scheduler>),
}

/// The command line the background sync runs: the program, `--sync-saves`, the data folder.
pub fn command(program: &Path, data: &Path) -> Vec<String> {
    vec![program.display().to_string(), "--sync-saves".into(), data.display().to_string()]
}

/// The systemd user units (`.service` and `.timer`) for the command.
pub fn units(cmd: &[String]) -> (String, String) {
    let quote = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
    let line = cmd.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ");
    let service = format!("[Unit]\nDescription=Game Repo Go: saves synced with the library\n\n[Service]\nType=oneshot\nExecStart={line}\nNice=10\n");
    let timer = format!("[Unit]\nDescription=Game Repo Go: saves synced with the library every {EVERY_MINUTES} minutes\n\n[Timer]\nOnBootSec=2min\nOnUnitActiveSec={EVERY_MINUTES}min\nPersistent=false\n\n[Install]\nWantedBy=timers.target\n");
    (service, timer)
}

/// Where the user's systemd units go (`$XDG_CONFIG_HOME/systemd/user`, else `~/.config/…`).
pub fn units_dir(home: &Path) -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config")).join("systemd").join("user")
}

fn quiet(program: &str, args: &[&str]) -> std::io::Result<std::process::Output> {
    let mut c = std::process::Command::new(program);
    c.args(args).stdin(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    c.output()
}

/// Sets the background sync up for this command (`on`), or takes it away. `dir` is where
/// systemd's user units go ([`units_dir`]); `real` false only writes them, on any system,
/// and asks neither systemctl nor Task Scheduler (the stand-in and the tests).
pub fn set(on: bool, cmd: &[String], dir: &Path, real: bool) -> Result<()> {
    if cfg!(target_os = "android") && real {
        bail!("On Android, Android's own background jobs sync saves.");
    }
    let systemctl = real;
    if cfg!(windows) && real {
        if on {
            let tr = cmd.iter().map(|a| format!("\"{a}\"")).collect::<Vec<_>>().join(" ");
            let every = EVERY_MINUTES.to_string();
            let o = quiet("schtasks", &["/Create", "/F", "/SC", "MINUTE", "/MO", &every, "/TN", NAME, "/TR", &tr]).context("Task Scheduler couldn't be asked")?;
            if !o.status.success() {
                bail!("Task Scheduler didn't take the task: {}", String::from_utf8_lossy(&o.stderr).trim());
            }
        } else {
            let _ = quiet("schtasks", &["/Delete", "/F", "/TN", NAME]);
        }
        return Ok(());
    }
    let (service, timer) = (dir.join(format!("{UNIT}.service")), dir.join(format!("{UNIT}.timer")));
    if on {
        let (s, t) = units(cmd);
        let same = std::fs::read_to_string(&service).ok().as_deref() == Some(s.as_str()) && std::fs::read_to_string(&timer).ok().as_deref() == Some(t.as_str());
        if !same {
            std::fs::create_dir_all(dir).with_context(|| format!("couldn't make {}", dir.display()))?;
            std::fs::write(&service, s)?;
            std::fs::write(&timer, t)?;
        }
        if systemctl {
            if !same {
                let _ = quiet("systemctl", &["--user", "daemon-reload"]);
            }
            let o = quiet("systemctl", &["--user", "enable", "--now", &format!("{UNIT}.timer")]).context("systemctl couldn't be started")?;
            if !o.status.success() {
                bail!("systemd didn't start the timer: {}", String::from_utf8_lossy(&o.stderr).trim());
            }
        }
    } else if service.exists() || timer.exists() {
        if systemctl {
            let _ = quiet("systemctl", &["--user", "disable", "--now", &format!("{UNIT}.timer")]);
        }
        let _ = std::fs::remove_file(&timer);
        let _ = std::fs::remove_file(&service);
        if systemctl {
            let _ = quiet("systemctl", &["--user", "daemon-reload"]);
        }
    }
    Ok(())
}

/// Whether it's set up (its timer's in `dir`; on Windows, Task Scheduler has it).
pub fn is_set(dir: &Path, real: bool) -> bool {
    if cfg!(windows) && real {
        return quiet("schtasks", &["/Query", "/TN", NAME]).is_ok_and(|o| o.status.success());
    }
    dir.join(format!("{UNIT}.timer")).is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_timer_is_written_for_the_program_and_taken_away() {
        let home = std::env::temp_dir().join(format!("gamerepo-test-background-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let cmd = command(Path::new("/home/deck/Applications/Game Repo Go.AppImage"), Path::new("/home/deck/.local/share/go"));
        let dir = home.join(".config/systemd/user");
        set(true, &cmd, &dir, false).unwrap();
        let service = std::fs::read_to_string(dir.join("game-repo-go-saves.service")).unwrap();
        assert!(service.contains("ExecStart=\"/home/deck/Applications/Game Repo Go.AppImage\" \"--sync-saves\" \"/home/deck/.local/share/go\""), "{service}");
        let timer = std::fs::read_to_string(dir.join("game-repo-go-saves.timer")).unwrap();
        assert!(timer.contains("OnUnitActiveSec=5min") && timer.contains("WantedBy=timers.target"), "{timer}");
        assert!(is_set(&dir, false));
        set(false, &cmd, &dir, false).unwrap();
        assert!(!is_set(&dir, false) && !dir.join("game-repo-go-saves.service").exists());
        let _ = std::fs::remove_dir_all(&home);
    }
}
