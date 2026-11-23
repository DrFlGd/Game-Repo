//! *Show in Atlas* on a game's page in Game Repo Go (docs/PLAN.md, "Maps and Guides notes,
//! part 7", D224), on Windows and Linux: Atlas started with `--show-pack <packId>` (the Atlas
//! pack contract, Atlas 0.24.0's start options), the pack id as the argument after the
//! option and never joined to it with `=`, and without `--background`, so Atlas comes to the
//! front with the game. A second start hands its options to the Atlas running and ends; an
//! Atlas of 0.23.0 or older leaves the option out and simply starts in front. When Atlas has
//! nothing for the id it shows nothing and says nothing. Go never waits for Atlas.
//!
//! Where Atlas is: the program set in Settings (`atlas_program`), else, on Windows, the
//! program registered for `.atlaspack` files (Atlas's installer registers the type: the
//! default value of `HKCR\.atlaspack` names its class, whose `shell\open\command` is the
//! command line the program is read from, [`program_of_command`]). On Linux nothing says
//! where Atlas is: Atlas's Linux bundles install no MIME type for `.atlaspack` and the
//! contract names none, so there's nothing for `xdg-mime` to be asked about, and Go asks
//! for Atlas to be set. A program called `atlas` on the PATH is never looked for.
//!
//! Starting Atlas by itself beside a game, with `--background`, is `launch.rs`'s (D241–D243):
//! on Linux, the contract's line in the game's Steam launch options, as Jerred decided after
//! the contract's trial T3 (D224 had left it out).
//!
//! Whether Atlas is installed, for *Install Atlas* ("Maps and Guides notes, part 13",
//! D250–D251), is [`installed`]: the program set in Settings; on Windows Atlas's NSIS
//! uninstall key (Tauri's template, `Software\Microsoft\Windows\CurrentVersion\Uninstall\Atlas`,
//! the user's own first, as Atlas's `currentUser` installer writes it, then the computer's),
//! then the program registered for `.atlaspack` files; on Linux the AppImage Atlas's settings
//! name (`linuxAppImage`), else the usual `~/Applications/Atlas.AppImage`. [`find`] uses the
//! same, so an Atlas Go installed is the one *Show in Atlas* starts.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

/// Atlas's start option that shows the game of a pack, by the pack's id.
pub const SHOW_PACK: &str = "--show-pack";

/// What Settings is called where Atlas can be set, for the sentences that send people there.
const WHERE_TO_SET: &str = "Settings › Maps and guides for Atlas";

/// The arguments Atlas is started with to show a pack's game: `--show-pack`, then the pack's
/// id as an argument of its own (never `--show-pack=<id>`), checked as Atlas checks it. No
/// `--background`: Atlas comes to the front.
pub fn show_args(pack_id: &str) -> Result<Vec<String>> {
    if !atlas_pack::is_pack_id(pack_id) {
        bail!("{pack_id:?} isn't a pack's id Atlas can be asked to show.");
    }
    Ok(vec![SHOW_PACK.to_string(), pack_id.to_string()])
}

/// A command line's `%NAME%`s replaced by `var(NAME)` (a value of type `REG_EXPAND_SZ`, such
/// as `%LOCALAPPDATA%\Atlas\Atlas.exe`); one `var` doesn't know is left as it is.
pub fn expand(text: &str, var: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) if end > 0 && after[..end].chars().all(|c| c.is_ascii_alphanumeric() || "_()".contains(c)) => {
                let name = &after[..end];
                match var(name) {
                    Some(v) => out.push_str(&v),
                    None => out.push_str(&rest[start..start + end + 2]),
                }
                rest = &after[end + 1..];
            }
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The program a file type's open command starts (`shell\open\command`): the path in quotes
/// at its start (`"C:\Program Files\Atlas\Atlas.exe" "%1"`), else, unquoted, up to the first
/// `.exe` followed by a space or the end (`C:\Program Files\Atlas\Atlas.exe %1`), else its
/// first word. Its `%NAME%`s are expanded by `var` first. None for a command with no program
/// in it, or one still naming a variable no one knows.
pub fn program_of_command(command: &str, var: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let text = expand(command.trim(), var);
    let text = text.trim();
    let program = if let Some(quoted) = text.strip_prefix('"') {
        &quoted[..quoted.find('"')?]
    } else {
        let lower = text.to_ascii_lowercase();
        let mut at = None;
        let mut from = 0;
        while let Some(i) = lower[from..].find(".exe") {
            let end = from + i + 4;
            if lower[end..].chars().next().is_none_or(char::is_whitespace) {
                at = Some(end);
                break;
            }
            from = end;
        }
        match at {
            Some(end) => &text[..end],
            None => text.split_whitespace().next()?,
        }
    };
    let program = program.trim();
    (!program.is_empty() && !program.contains('%') && !program.contains('"')).then(|| PathBuf::from(program))
}

/// A text value of a registry key (None: its default value), as text, its `%NAME%`s not
/// expanded.
#[cfg(windows)]
fn reg_text(root: windows_sys::Win32::System::Registry::HKEY, key: &str, value: Option<&str>) -> Option<String> {
    use windows_sys::Win32::System::Registry::{RegGetValueW, RRF_NOEXPAND, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ};
    let wide: Vec<u16> = key.encode_utf16().chain([0]).collect();
    let name: Option<Vec<u16>> = value.map(|v| v.encode_utf16().chain([0]).collect());
    let name_ptr = name.as_ref().map_or(std::ptr::null(), |n| n.as_ptr());
    let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND;
    let mut size: u32 = 0;
    // SAFETY: a key name and a value name (or none: the default) ending in 0, and only its size asked
    let r = unsafe { RegGetValueW(root, wide.as_ptr(), name_ptr, flags, std::ptr::null_mut(), std::ptr::null_mut(), &mut size) };
    if r != 0 || size == 0 || size > 64 * 1024 {
        return None;
    }
    let mut buf = vec![0u16; (size as usize).div_ceil(2) + 1];
    let mut size = (buf.len() * 2) as u32;
    // SAFETY: `buf` holds `size` bytes
    let r = unsafe { RegGetValueW(root, wide.as_ptr(), name_ptr, flags, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut size) };
    if r != 0 {
        return None;
    }
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..n])).filter(|s| !s.trim().is_empty())
}

/// The default value of a key of `HKEY_CLASSES_ROOT` (the user's own classes over the
/// computer's), as text, its `%NAME%`s not expanded.
#[cfg(windows)]
fn class_default(key: &str) -> Option<String> {
    reg_text(windows_sys::Win32::System::Registry::HKEY_CLASSES_ROOT, key, None)
}

/// The program Windows opens `.atlaspack` files with: `HKCR\.atlaspack`'s default value names
/// the file type's class, and that class's `shell\open\command` the program. Only a program
/// that's there.
#[cfg(windows)]
fn registered() -> Option<PathBuf> {
    let class = class_default(".atlaspack")?;
    let class = class.trim();
    if class.is_empty() || class.contains(['\\', '/', '"']) {
        return None;
    }
    let command = class_default(&format!(r"{class}\shell\open\command"))?;
    program_of_command(&command, |n| std::env::var(n).ok()).filter(|p| p.is_absolute() && p.is_file())
}

#[cfg(not(windows))]
fn registered() -> Option<PathBuf> {
    None
}

/// Atlas's uninstall key as Tauri's NSIS installer writes it (`installer.nsi`: the key is
/// named after the product, `Atlas`), under `HKEY_CURRENT_USER` for Atlas's installer, whose
/// install mode is Tauri's default, `currentUser` (D251).
pub const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Atlas";

/// The values of Atlas's uninstall key that say where it is, as the registry has them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Uninstall {
    /// `DisplayName`: the product's name, `Atlas`
    pub name: Option<String>,
    /// `DisplayVersion`: the version installed
    pub version: Option<String>,
    /// `InstallLocation`: the folder, in quotes (`"C:\Users\me\AppData\Local\Atlas"`)
    pub location: Option<String>,
    /// `DisplayIcon`: the program, in quotes
    pub icon: Option<String>,
    /// `MainBinaryName`: the program's file name, with `.exe`
    pub binary: Option<String>,
}

/// Atlas's program and version from its uninstall key's values: `DisplayIcon`'s program, else
/// `MainBinaryName` in `InstallLocation`. None for a key that isn't Atlas's (its
/// `DisplayName` isn't `Atlas`) or names no program. Its `%NAME%`s are expanded by `var`.
pub fn from_uninstall(u: &Uninstall, var: impl Fn(&str) -> Option<String>) -> Option<(PathBuf, Option<String>)> {
    if u.name.as_deref().map(str::trim) != Some("Atlas") {
        return None;
    }
    let version = u.version.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(String::from);
    let by_icon = u.icon.as_deref().and_then(|i| program_of_command(i, &var)).filter(|p| p.to_string_lossy().to_ascii_lowercase().ends_with(".exe"));
    let program = by_icon.or_else(|| {
        let folder = expand(u.location.as_deref()?.trim(), &var);
        let folder = folder.trim().trim_matches('"').trim_end_matches(['\\', '/']);
        let binary = u.binary.as_deref()?.trim();
        if folder.is_empty() || folder.contains(['"', '%']) || binary.is_empty() || binary.contains(['\\', '/', '"', '%']) {
            return None;
        }
        let binary = if binary.to_ascii_lowercase().ends_with(".exe") { binary.to_string() } else { format!("{binary}.exe") };
        Some(PathBuf::from(format!("{folder}\\{binary}")))
    })?;
    Some((program, version))
}

/// Atlas as its uninstall key says, the user's own first, then the computer's: only a program
/// that's there.
#[cfg(windows)]
fn uninstall_key() -> Option<(PathBuf, Option<String>)> {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE].into_iter().find_map(|root| {
        let read = |v: &str| reg_text(root, UNINSTALL_KEY, Some(v));
        let u = Uninstall { name: read("DisplayName"), version: read("DisplayVersion"), location: read("InstallLocation"), icon: read("DisplayIcon"), binary: read("MainBinaryName") };
        from_uninstall(&u, |n| std::env::var(n).ok()).filter(|(p, _)| p.is_absolute() && p.is_file())
    })
}

#[cfg(not(windows))]
fn uninstall_key() -> Option<(PathBuf, Option<String>)> {
    None
}

/// Atlas found on this device.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Found {
    /// Its program (on Linux, its AppImage)
    pub program: PathBuf,
    /// How it was found: set (in Settings), installed (Atlas's uninstall key), registered
    /// (the program that opens `.atlaspack` files) or appimage
    pub how: &'static str,
    /// The version installed, when Windows says (its uninstall key)
    pub version: Option<String>,
    /// Whether it can be run (on Linux: a file with a mode that lets it run)
    pub runnable: bool,
}

/// Whether a file may be run: on Linux its mode lets someone run it.
pub fn runnable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// Atlas's AppImage on Linux: the one Atlas's settings name (`linuxAppImage`, what it last
/// started from), else `~/Applications/Atlas.AppImage` under `home`: a file that can be run,
/// or (the usual one) any file there, which Go never writes over.
pub fn appimage(home: &Path) -> Option<Found> {
    let said = crate::launch::appimage_said(crate::packs::atlas_settings(home).as_ref()).map(PathBuf::from).filter(|p| p.is_absolute() && runnable(p));
    let found = |program: PathBuf| Found { runnable: runnable(&program), program, how: "appimage", version: None };
    if let Some(p) = said {
        return Some(found(p));
    }
    let usual = home.join(crate::launch::USUAL);
    usual.is_file().then(|| found(usual))
}

/// Whether Atlas is installed on this device, and where: the program set in Settings (`set`,
/// empty for none); else on Windows Atlas's uninstall key, then the program registered for
/// `.atlaspack` files; on Linux its AppImage ([`appimage`], under `home`). Never on Android.
pub fn installed(set: &str, home: &Path) -> Option<Found> {
    let set = set.trim();
    if !set.is_empty() && Path::new(set).is_file() {
        let p = PathBuf::from(set);
        // as D224 takes it: the program set is started as it is
        return Some(Found { runnable: true, program: p, how: "set", version: None });
    }
    if cfg!(target_os = "android") {
        return None;
    }
    if let Some((program, version)) = uninstall_key() {
        return Some(Found { program, how: "installed", version, runnable: true });
    }
    if let Some(program) = registered() {
        return Some(Found { program, how: "registered", version: None, runnable: true });
    }
    if cfg!(windows) {
        return None;
    }
    appimage(home)
}

/// Atlas's program: as [`installed`] finds it (`home`: the user's folder, for the AppImage on
/// Linux). Refused, saying where to set it, when it isn't found or can't be run.
pub fn find(set: &str, home: &Path) -> Result<PathBuf> {
    let set = set.trim();
    if !set.is_empty() && !Path::new(set).is_file() {
        bail!("Atlas isn't at {set}: set where it is in {WHERE_TO_SET}.");
    }
    match installed(set, home) {
        Some(f) if f.runnable => Ok(f.program),
        Some(f) => bail!("Atlas's AppImage at {} can't be run: tick Is executable in its Properties › Permissions, or set where Atlas is in {WHERE_TO_SET}.", f.program.display()),
        None => bail!("Atlas wasn't found on this device: install it, or set where it is in {WHERE_TO_SET}."),
    }
}

/// Atlas started showing the game of the pack `pack_id`, brought to the front (no
/// `--background`); Go doesn't wait for it.
pub fn show(program: &Path, pack_id: &str) -> Result<()> {
    let args = show_args(pack_id)?;
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    crate::start_alone(program, &args).with_context(|| format!("Atlas ({}) didn't start", program.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn atlas_is_asked_to_show_a_pack_by_its_id_as_an_argument_of_its_own() {
        assert_eq!(show_args("beta-example-maps").unwrap(), vec!["--show-pack".to_string(), "beta-example-maps".to_string()]);
        assert_eq!(show_args("7th_saga").unwrap(), ["--show-pack", "7th_saga"]);
        // never joined, never in the background
        let a = show_args("alpha").unwrap();
        assert!(a.iter().all(|x| !x.contains('=') && x != "--background"), "{a:?}");
        // an id Atlas wouldn't take is never handed on (it could be read as another option)
        for bad in ["", "Not An Id", "../valley", "--background", "-x", "a=b", "UPPER", "a b", &"a".repeat(65)] {
            assert!(show_args(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_program_is_read_from_a_file_types_open_command() {
        let p = |c: &str| program_of_command(c, none).map(|p| p.display().to_string());
        // as Tauri's installer writes it: the path quoted, then "%1"
        assert_eq!(p(r#""C:\Program Files\Atlas\Atlas.exe" "%1""#).as_deref(), Some(r"C:\Program Files\Atlas\Atlas.exe"));
        assert_eq!(p(r#"  "C:\Users\Zoë\AppData\Local\Atlas\Atlas.exe" --open "%1" "#).as_deref(), Some(r"C:\Users\Zoë\AppData\Local\Atlas\Atlas.exe"));
        // unquoted, with spaces in the path
        assert_eq!(p(r"C:\Program Files\Atlas\Atlas.exe %1").as_deref(), Some(r"C:\Program Files\Atlas\Atlas.exe"));
        assert_eq!(p(r"C:\Program Files\Atlas\ATLAS.EXE").as_deref(), Some(r"C:\Program Files\Atlas\ATLAS.EXE"));
        // ".exe" inside a folder's name isn't the end of the program
        assert_eq!(p(r"D:\Tools.exeter\Atlas.exe %1").as_deref(), Some(r"D:\Tools.exeter\Atlas.exe"));
        // no .exe: the first word
        assert_eq!(p(r"C:\Atlas\atlas %1").as_deref(), Some(r"C:\Atlas\atlas"));
        // a REG_EXPAND_SZ value's variables expanded
        let var = |n: &str| (n == "LOCALAPPDATA").then(|| r"C:\Users\me\AppData\Local".to_string());
        assert_eq!(program_of_command(r#""%LOCALAPPDATA%\Atlas\Atlas.exe" "%1""#, var).unwrap(), PathBuf::from(r"C:\Users\me\AppData\Local\Atlas\Atlas.exe"));
        assert_eq!(program_of_command(r"%LOCALAPPDATA%\Programs\Atlas\Atlas.exe %1", var).unwrap(), PathBuf::from(r"C:\Users\me\AppData\Local\Programs\Atlas\Atlas.exe"));
        // nothing usable
        for bad in ["", "   ", r#""C:\Atlas\Atlas.exe"#, r#""" "%1""#, r#""%UNKNOWN%\Atlas.exe" "%1""#] {
            assert_eq!(p(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn variables_are_expanded_as_windows_does() {
        let var = |n: &str| match n {
            "A" => Some("x".to_string()),
            "ProgramFiles(x86)" => Some(r"C:\Program Files (x86)".to_string()),
            _ => None,
        };
        assert_eq!(expand("%A%\\%A%", var), "x\\x");
        assert_eq!(expand("%ProgramFiles(x86)%\\Atlas", var), r"C:\Program Files (x86)\Atlas");
        assert_eq!(expand("\"%1\" 100% %A%", var), "\"%1\" 100% x");
        assert_eq!(expand("%NOPE% %%", var), "%NOPE% %%");
    }

    #[test]
    fn atlas_is_the_program_set_else_its_said_where_to_set_it() {
        let dir = std::env::temp_dir().join(format!("gamerepo-atlas-find-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let atlas = dir.join("Atlas.exe");
        std::fs::write(&atlas, b"").unwrap();
        assert_eq!(find(&atlas.display().to_string(), &dir).unwrap(), atlas);
        assert_eq!(installed(&atlas.display().to_string(), &dir).map(|f| (f.program, f.how)), Some((atlas.clone(), "set")));
        let gone = dir.join("Gone.exe").display().to_string();
        let e = find(&gone, &dir).unwrap_err().to_string();
        assert!(e.contains(&gone) && e.contains(WHERE_TO_SET), "{e}");
        if !cfg!(windows) {
            // off Windows nothing else is asked (never the PATH): it says to install it, or where to set it
            let e = find("", &dir).unwrap_err().to_string();
            assert!(e.starts_with("Atlas wasn't found on this device: install it") && e.contains(WHERE_TO_SET), "{e}");
            assert_eq!(installed("", &dir), None);
            assert_eq!(installed(&gone, &dir), None);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn on_linux_atlas_is_its_appimage_where_atlas_says_else_in_applications() {
        use std::os::unix::fs::PermissionsExt;
        let home = std::env::temp_dir().join(format!("gamerepo-atlas-appimage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("Applications")).unwrap();
        let usual = home.join("Applications/Atlas.AppImage");
        // not there: not installed
        assert_eq!(installed("", &home), None);
        // there but not executable: installed (Go never writes over it), and said it can't be run
        std::fs::write(&usual, b"\x7fELF").unwrap();
        std::fs::set_permissions(&usual, std::fs::Permissions::from_mode(0o644)).unwrap();
        let f = installed("", &home).unwrap();
        assert_eq!((f.program.as_path(), f.how, f.runnable), (usual.as_path(), "appimage", false));
        let e = find("", &home).unwrap_err().to_string();
        assert!(e.contains("can't be run") && e.contains("Is executable"), "{e}");
        std::fs::set_permissions(&usual, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(find("", &home).unwrap(), usual);
        // the one Atlas's settings name (linuxAppImage) comes first, when it can be run
        let elsewhere = home.join("apps/Atlas Viewer.AppImage");
        std::fs::create_dir_all(elsewhere.parent().unwrap()).unwrap();
        std::fs::write(&elsewhere, b"\x7fELF").unwrap();
        std::fs::set_permissions(&elsewhere, std::fs::Permissions::from_mode(0o755)).unwrap();
        let settings = crate::packs::atlas_settings_file(&home);
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(&settings, serde_json::json!({ "format": "atlas-settings", "version": 1, "linuxAppImage": elsewhere }).to_string()).unwrap();
        assert_eq!(find("", &home).unwrap(), elsewhere);
        // one it names that isn't there: the usual one
        std::fs::remove_file(&elsewhere).unwrap();
        assert_eq!(find("", &home).unwrap(), usual);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn atlas_is_read_from_its_uninstall_key_as_tauris_installer_writes_it() {
        let var = |n: &str| (n == "LOCALAPPDATA").then(|| r"C:\Users\Zoë\AppData\Local".to_string());
        // as installer.nsi writes it for a currentUser install
        let u = Uninstall {
            name: Some("Atlas".into()),
            version: Some("0.29.0".into()),
            location: Some(r#""C:\Users\Zoë\AppData\Local\Atlas""#.into()),
            icon: Some(r#""C:\Users\Zoë\AppData\Local\Atlas\atlas-desktop.exe""#.into()),
            binary: Some("atlas-desktop.exe".into()),
        };
        let at = PathBuf::from(r"C:\Users\Zoë\AppData\Local\Atlas\atlas-desktop.exe");
        assert_eq!(from_uninstall(&u, var), Some((at.clone(), Some("0.29.0".into()))));
        // no DisplayIcon: the program in InstallLocation; one named without .exe gets it
        let no_icon = Uninstall { icon: None, ..u.clone() };
        assert_eq!(from_uninstall(&no_icon, var), Some((at.clone(), Some("0.29.0".into()))));
        let bare = Uninstall { icon: None, binary: Some("atlas-desktop".into()), location: Some(r"C:\Users\Zoë\AppData\Local\Atlas\".into()), ..u.clone() };
        assert_eq!(from_uninstall(&bare, var).map(|x| x.0), Some(at.clone()));
        // an icon with its index, and a folder with a variable in it
        let indexed = Uninstall { icon: Some(r#""%LOCALAPPDATA%\Atlas\Atlas.exe",0"#.into()), ..u.clone() };
        assert_eq!(from_uninstall(&indexed, var).map(|x| x.0), Some(PathBuf::from(r"C:\Users\Zoë\AppData\Local\Atlas\Atlas.exe")));
        let in_var = Uninstall { icon: None, location: Some(r"%LOCALAPPDATA%\Atlas".into()), ..u.clone() };
        assert_eq!(from_uninstall(&in_var, var).map(|x| x.0), Some(at.clone()));
        // no version said: still Atlas
        assert_eq!(from_uninstall(&Uninstall { version: Some(" ".into()), ..u.clone() }, var), Some((at.clone(), None)));
        // not Atlas's key, or nothing that names a program
        assert_eq!(from_uninstall(&Uninstall { name: Some("Atlas Importer".into()), ..u.clone() }, var), None);
        assert_eq!(from_uninstall(&Uninstall { name: None, ..u.clone() }, var), None);
        assert_eq!(from_uninstall(&Uninstall { icon: None, binary: None, ..u.clone() }, var), None);
        assert_eq!(from_uninstall(&Uninstall { icon: Some("not a program".into()), binary: None, ..u.clone() }, var), None);
        assert_eq!(from_uninstall(&Uninstall { icon: None, location: Some(r#""%NOPE%\Atlas""#.into()), ..u.clone() }, var), None);
        assert_eq!(from_uninstall(&Uninstall { icon: None, binary: Some(r"..\evil.exe".into()), ..u.clone() }, var), None);
        assert_eq!(from_uninstall(&Uninstall::default(), var), None);
    }

    #[cfg(unix)]
    #[test]
    fn atlas_is_started_with_the_pack_and_not_waited_for() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("gamerepo-atlas-show-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let said = dir.join("args.txt");
        let atlas = dir.join("atlas-stand-in");
        // writes its arguments a line each, then stays a while as Atlas would
        std::fs::write(&atlas, format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}.part'\nmv '{0}.part' '{0}'\nsleep 5\n", said.display())).unwrap();
        std::fs::set_permissions(&atlas, std::fs::Permissions::from_mode(0o755)).unwrap();
        let t = std::time::Instant::now();
        show(&atlas, "beta-example-maps").unwrap();
        assert!(t.elapsed() < std::time::Duration::from_secs(3), "waited for Atlas");
        let mut got = String::new();
        for _ in 0..100 {
            if let Ok(s) = std::fs::read_to_string(&said) {
                got = s;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert_eq!(got, "--show-pack\nbeta-example-maps\n");
        assert!(show(&atlas, "Not An Id").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
