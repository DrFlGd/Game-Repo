//! PC games on the device (docs/PLAN.md, "Phase 9c design", D111 and D112): their files
//! go in the device's PC games folder (place `pc`: `C:\Games` on Windows, `~/Games` on
//! Linux, or where Settings says), each in `<Title> (<Year>)/`; their pictures for Steam in
//! the data folder (place `pcmedia`). A game with a program to start gets a shortcut: on
//! Windows in the Start menu's *Game Repo Go* folder (place `startmenu`, a file the record
//! holds, so removing the game takes it), on Linux in Steam (D109's waiting shortcuts),
//! through Proton for a Windows program.

use crate::steam::{self, Wanted};
use crate::Device;
use anyhow::{anyhow, bail, Result};
use gamerepo_wire::api::HeldFile;
use gamerepo_wire::Program;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The catalogue's system for PC games.
pub const SYSTEM: &str = "pcgames";

/// The Start menu folder Game Repo Go's shortcuts go in.
pub const START_MENU: &str = "Game Repo Go";

/// Whether a catalogue key is a PC game's.
pub fn is_pc(key: &str) -> bool {
    key.strip_prefix(SYSTEM).is_some_and(|r| r.starts_with('/'))
}

/// The PC games folder when Settings doesn't say: `C:\Games` on Windows, `~/Games` elsewhere.
pub fn default_folder(home: &Path) -> PathBuf {
    if cfg!(windows) {
        let drive = std::env::var("SystemDrive").ok().filter(|d| !d.is_empty()).unwrap_or_else(|| "C:".into());
        PathBuf::from(format!("{drive}\\Games"))
    } else {
        home.join("Games")
    }
}

/// The Start menu's programs folder for this user (Windows), with Game Repo Go's in it.
pub fn start_menu() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(PathBuf::from(appdata).join("Microsoft").join("Windows").join("Start Menu").join("Programs").join(START_MENU))
}

/// A name a file can have on Windows and Linux alike.
fn file_name(s: &str) -> String {
    let n: String = s.chars().map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { '_' } else { c }).collect();
    let n = n.trim().trim_end_matches('.').to_string();
    if n.is_empty() {
        "Game".into()
    } else {
        n
    }
}

/// Its shortcut made (D112): on Windows a `.lnk` in the Start menu, returned for the
/// record; on Linux a Steam shortcut, written now if Steam is closed or else waiting
/// (`data`: the app's data folder, where what's wanted in Steam is kept).
pub fn shortcut(d: &Device, data: Option<&Path>, p: &Program) -> Result<Option<HeldFile>> {
    if p.path.is_empty() {
        return Ok(None);
    }
    let folder = d.at(&p.place, &p.folder)?;
    let program = d.at(&p.place, &format!("{}/{}", p.folder, p.path))?;
    let start = if p.dir.is_empty() { program.parent().map(Path::to_path_buf).unwrap_or(folder.clone()) } else { d.at(&p.place, &format!("{}/{}", p.folder, p.dir))? };
    if cfg!(windows) && d.places.contains_key("startmenu") {
        let name = format!("{}.lnk", file_name(&p.name));
        let lnk = d.at("startmenu", &name)?;
        if let Some(dir) = lnk.parent() {
            std::fs::create_dir_all(dir)?;
        }
        windows_shortcut(&lnk, &program, &start, &p.args)?;
        let size = std::fs::metadata(&lnk).map(|m| m.len()).unwrap_or(0);
        return Ok(Some(HeldFile { place: "startmenu".into(), path: name, size, role: "shortcut".into(), ..Default::default() }));
    }
    if cfg!(target_os = "linux") {
        let data = data.ok_or_else(|| anyhow!("Game Repo Go's data folder isn't known here."))?;
        let mut art = BTreeMap::new();
        for (kind, place, path) in &p.art {
            if let Ok(f) = d.at(place, path) {
                if f.is_file() {
                    art.insert(kind.clone(), f.display().to_string());
                }
            }
        }
        if !p.windows {
            set_executable(&program);
        }
        let icon = art.get("icon").cloned().unwrap_or_default();
        steam::want(
            data,
            &p.key,
            Some(Wanted {
                key: p.key.clone(),
                name: p.name.clone(),
                program: program.display().to_string(),
                start_dir: start.display().to_string(),
                args: p.args.clone(),
                icon,
                proton: p.windows,
                art,
            }),
        )?;
    }
    Ok(None)
}

/// Its Steam shortcut taken out (Linux); a Windows shortcut goes with the game's files.
pub fn unshortcut(data: &Path, key: &str) -> Result<()> {
    if steam::state(data).wanted.iter().any(|w| w.key == key) {
        steam::want(data, key, None)?;
    }
    Ok(())
}

#[cfg(unix)]
fn set_executable(p: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(m) = std::fs::metadata(p) {
        let mut perm = m.permissions();
        perm.set_mode(perm.mode() | 0o755);
        let _ = std::fs::set_permissions(p, perm);
    }
}

#[cfg(not(unix))]
fn set_executable(_: &Path) {}

/// A Start menu shortcut, made by Windows' own Shell (WScript.Shell through PowerShell),
/// its paths passed in the environment so no quoting can go wrong.
fn windows_shortcut(lnk: &Path, program: &Path, start: &Path, args: &str) -> Result<()> {
    let script = "$s = (New-Object -ComObject WScript.Shell).CreateShortcut($env:GR_LNK); $s.TargetPath = $env:GR_PROGRAM; $s.WorkingDirectory = $env:GR_START; $s.Arguments = $env:GR_ARGS; $s.IconLocation = $env:GR_PROGRAM + ',0'; $s.Save()";
    let mut cmd = std::process::Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script])
        .env("GR_LNK", lnk)
        .env("GR_PROGRAM", program)
        .env("GR_START", start)
        .env("GR_ARGS", args)
        .stdin(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // no console window
    }
    let out = cmd.output().map_err(|e| anyhow!("couldn't run PowerShell to make the shortcut: {e}"))?;
    if !out.status.success() || !lnk.is_file() {
        bail!("Windows didn't make the shortcut: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

/// What's left in a removed PC game's folder: the files the game made itself (saves,
/// settings), kept with the folder (D112). (folder, files) for each game folder that's
/// still there.
pub fn left_in(d: &Device, folders: &[String]) -> Vec<(String, usize)> {
    let mut out = vec![];
    for f in folders {
        let Ok(dir) = d.at("pc", f) else { continue };
        if !dir.is_dir() {
            continue;
        }
        let mut n = 0;
        let mut stack = vec![dir];
        while let Some(x) = stack.pop() {
            for e in std::fs::read_dir(&x).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    n += 1;
                }
            }
        }
        out.push((f.clone(), n));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shortcuts_name_is_one_windows_takes() {
        assert_eq!(file_name("Zeta: The Quest?"), "Zeta_ The Quest_");
        assert_eq!(file_name("  ..."), "Game");
        assert!(is_pc("pcgames/zeta") && !is_pc("pcgamesx/zeta") && !is_pc("gb/Alpha (USA)"));
    }
}
