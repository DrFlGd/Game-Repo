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
//! Left out (D224): starting Atlas by itself beside a game, and `--background` (pending
//! Jerred's decision and the contract's trial T3).

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

/// The default value of a key of `HKEY_CLASSES_ROOT` (the user's own classes over the
/// computer's), as text, its `%NAME%`s not expanded.
#[cfg(windows)]
fn class_default(key: &str) -> Option<String> {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CLASSES_ROOT, RRF_NOEXPAND, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ};
    let wide: Vec<u16> = key.encode_utf16().chain([0]).collect();
    let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND;
    let mut size: u32 = 0;
    // SAFETY: a key name ending in 0, no value name (the default), and only its size asked
    let r = unsafe { RegGetValueW(HKEY_CLASSES_ROOT, wide.as_ptr(), std::ptr::null(), flags, std::ptr::null_mut(), std::ptr::null_mut(), &mut size) };
    if r != 0 || size == 0 || size > 64 * 1024 {
        return None;
    }
    let mut buf = vec![0u16; (size as usize).div_ceil(2) + 1];
    let mut size = (buf.len() * 2) as u32;
    // SAFETY: `buf` holds `size` bytes
    let r = unsafe { RegGetValueW(HKEY_CLASSES_ROOT, wide.as_ptr(), std::ptr::null(), flags, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut size) };
    if r != 0 {
        return None;
    }
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..n])).filter(|s| !s.trim().is_empty())
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

/// Atlas's program: the one set in Settings (`set`, empty for none), else the program
/// registered for `.atlaspack` files on Windows. Refused, saying where to set it, when it
/// isn't found.
pub fn find(set: &str) -> Result<PathBuf> {
    let set = set.trim();
    if !set.is_empty() {
        let p = PathBuf::from(set);
        if !p.is_file() {
            bail!("Atlas isn't at {set}: set where it is in {WHERE_TO_SET}.");
        }
        return Ok(p);
    }
    if let Some(p) = registered() {
        return Ok(p);
    }
    if cfg!(windows) {
        bail!("Atlas wasn't found on this device: install it, or set where it is in {WHERE_TO_SET}.");
    }
    // on Linux nothing registers .atlaspack files, so there's nothing else to ask
    bail!("Atlas wasn't found on this device: set where it is in {WHERE_TO_SET}.");
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
        assert_eq!(find(&atlas.display().to_string()).unwrap(), atlas);
        let gone = dir.join("Gone.exe").display().to_string();
        let e = find(&gone).unwrap_err().to_string();
        assert!(e.contains(&gone) && e.contains(WHERE_TO_SET), "{e}");
        if !cfg!(windows) {
            // off Windows nothing else is asked (never the PATH): it says where to set it
            let e = find("").unwrap_err().to_string();
            assert!(e.starts_with("Atlas wasn't found on this device") && e.contains(WHERE_TO_SET), "{e}");
        }
        let _ = std::fs::remove_dir_all(&dir);
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
