//! Atlas started with a game through Steam's launch options, on Linux and the Steam Deck
//! (docs/PLAN.md, "Maps and Guides notes, part 11", D241–D244; the Atlas pack contract's
//! entries of 2026-11-20 and 2026-11-21, after trial T3). The contract's line, exactly, with
//! the AppImage's path where `<AppImage>` stands:
//!
//! ```sh
//! bash -c '"<AppImage>" --background & exec "$@"' -- %command%
//! ```
//!
//! The path is `linuxAppImage` in Atlas's shared settings file
//! (`$XDG_DATA_HOME/atlas/settings.json`), read as leniently as Go reads its `packsFolder`
//! (D230); when it's absent, `~/Applications/Atlas.AppImage` with the person's real home
//! folder, said to be the usual place. A path the line can't hold (`'`, `"`, `$`, a backtick,
//! `\`, a line break or another control character, or not a full path) never gets a line:
//! why is said instead ([`why_not`]).
//!
//! Go writes the line into the launch options of the Steam shortcuts it makes for its PC
//! games (`steam.rs`), and never over launch options that aren't its own: [`resolve`] reads
//! what the shortcut has now and gives what to write. Windows has none of this (the line is
//! bash's).

use crate::packs;
use serde::Serialize;
use serde_json::Value;
use std::path::Path;

/// The line before the AppImage's path.
pub const BEFORE: &str = "bash -c '\"";
/// The line after it.
pub const AFTER: &str = "\" --background & exec \"$@\"' -- %command%";

/// Where the AppImage usually is, under the person's home folder, when Atlas's settings don't
/// say (the trial's notes: `$HOME/Applications/Atlas.AppImage`).
pub const USUAL: &str = "Applications/Atlas.AppImage";

/// The contract's line for the AppImage at `appimage`: only for a path [`why_not`] passes.
pub fn line(appimage: &str) -> String {
    format!("{BEFORE}{appimage}{AFTER}")
}

/// Why the line can't hold a path, in a few words ("its path holds a backtick (`)"); None when
/// it can. The path stands in double quotes inside single quotes, so nothing in it may end
/// the quotes or be expanded by the shell (`'`, `"`, `$`, a backtick, `\`), the line is one
/// line (no line break, nor any other control character), and it's a full path.
pub fn why_not(path: &str) -> Option<String> {
    if path.trim().is_empty() {
        return Some("it's empty".into());
    }
    for c in path.chars() {
        let named = match c {
            '\'' => "a single quote (')",
            '"' => "a double quote (\")",
            '$' => "a dollar sign ($)",
            '`' => "a backtick (`)",
            '\\' => "a backslash (\\)",
            '\n' | '\r' => "a line break",
            c if c.is_control() => "a control character",
            _ => continue,
        };
        return Some(format!("its path holds {named}"));
    }
    if !path.starts_with('/') || !Path::new(path).is_absolute() {
        return Some("it isn't a full path".into());
    }
    None
}

/// The line Go would write, worked out from Atlas's settings and the home folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Launch {
    /// The AppImage's path (as Atlas's settings give it, or the usual one)
    pub path: String,
    /// atlas (Atlas's settings say it, `linuxAppImage`) or usual (they don't: the default)
    pub from: &'static str,
    /// Whether the AppImage is there
    pub exists: bool,
    /// The line; None when the path can't stand in it
    pub line: Option<String>,
    /// Why there's no line
    pub refused: Option<String>,
}

/// `linuxAppImage` of Atlas's settings, read as Atlas reads it (`lenient_optional_text`):
/// text, else nothing (absent, `null`, or a value that isn't text). `settings`: the file as
/// [`packs::atlas_settings`] reads it, None when there's no file Atlas reads.
pub fn appimage_said(settings: Option<&Value>) -> Option<String> {
    settings?.get("linuxAppImage")?.as_str().map(String::from)
}

/// The line for what Atlas's settings say (`said`, [`appimage_said`]), else the usual place
/// under `home`; `is_file` tells whether the AppImage is there.
pub fn launch_from(said: Option<&str>, home: &Path, is_file: impl Fn(&Path) -> bool) -> Launch {
    let (path, from) = match said {
        Some(p) => (p.to_string(), "atlas"),
        None => (home.join(USUAL).to_string_lossy().into_owned(), "usual"),
    };
    let refused = why_not(&path);
    Launch { exists: refused.is_none() && is_file(Path::new(&path)), line: refused.is_none().then(|| line(&path)), refused, path, from }
}

/// The line on this computer now, from Atlas's settings file read afresh.
pub fn launch(home: &Path) -> Launch {
    let settings = packs::atlas_settings(home);
    launch_from(appimage_said(settings.as_ref()).as_deref(), home, Path::is_file)
}

/// Go's line at the front of launch options (for any AppImage path the line can hold), and
/// what follows it after one space: (the path, the rest). None when they don't start with it.
pub fn split_line(options: &str) -> Option<(&str, &str)> {
    let rest = options.strip_prefix(BEFORE)?;
    let end = rest.find(AFTER)?;
    let path = &rest[..end];
    if why_not(path).is_some() {
        return None;
    }
    let after = &rest[end + AFTER.len()..];
    if after.is_empty() {
        Some((path, ""))
    } else {
        after.strip_prefix(' ').map(|r| (path, r))
    }
}

/// Whether launch options can follow the line with a result that's certain: none, or plain
/// words (letters, digits and `-_=.,/:+@`, spaces between them) that mean the same to a
/// shell as to Steam, and no `%command%` of their own.
pub fn joinable(options: &str) -> bool {
    options.chars().all(|c| c.is_ascii_alphanumeric() || " -_=.,/:+@".contains(c))
}

fn join(line: &str, options: &str) -> String {
    if options.is_empty() {
        line.to_string()
    } else {
        format!("{line} {options}")
    }
}

/// What a shortcut's launch options are after [`resolve`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// No line: Go's own options (the game's arguments), or the person's left as they are
    Off,
    /// Go's line, with the game's own arguments after it
    On,
    /// Go's line in front of the person's own options, which are plain words
    Joined,
    /// The person's own options, which can't be joined for certain: left as they are, and
    /// the line is shown to copy
    Theirs,
    /// The game's own arguments can't follow the line for certain: Go's options without it,
    /// and the line shown to copy
    CannotJoin,
}

/// What to write in a shortcut's launch options, and how it came out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub value: String,
    pub outcome: Outcome,
}

/// A shortcut's launch options worked out from what Steam's file has now (`current`, read
/// back from shortcuts.vdf just before writing; None: the shortcut isn't there yet), Go's own
/// (`base`: the game's arguments, empty for most), Go's line when Atlas starts with this game
/// (`line`), and Go's own options when it last wrote them (`last`: the game's arguments then,
/// so a change of them in the library is still taken as Go's).
///
/// Options that are Go's (empty, the game's arguments, or what Go wrote last), with or
/// without a line of Go's in front (for any path, so a moved AppImage is followed), are
/// replaced. Anything else is the person's (changed in Steam): never written over. With the
/// line wanted, plain words of theirs ([`joinable`]) go after it, said so; any other options
/// of theirs are left whole. With the line not wanted, exactly Go's line is taken out of the
/// front and theirs stay.
pub fn resolve(current: Option<&str>, base: &str, line: Option<&str>, last: Option<&str>) -> Resolved {
    let gos = |x: &str| x.is_empty() || x == base || last == Some(x);
    // the person's own options (None: Go's), and whether they're kept whole as they are
    let (own, whole): (Option<&str>, bool) = match current {
        None => (None, false),
        Some(c) => match split_line(c) {
            Some((_, rest)) if joinable(rest) => (Some(rest).filter(|r| !gos(r)), false),
            // Go's line followed by something Go would never put after it: theirs, whole
            Some(_) => (Some(c), true),
            None => (Some(c).filter(|c| !gos(c)), !joinable(c)),
        },
    };
    match (own, line) {
        (None, None) => Resolved { value: base.to_string(), outcome: Outcome::Off },
        (None, Some(l)) if joinable(base) => Resolved { value: join(l, base), outcome: Outcome::On },
        (None, Some(_)) => Resolved { value: base.to_string(), outcome: Outcome::CannotJoin },
        (Some(x), None) => Resolved { value: x.to_string(), outcome: Outcome::Off },
        (Some(x), Some(l)) if !whole => Resolved { value: join(l, x), outcome: Outcome::Joined },
        (Some(_), Some(_)) => Resolved { value: current.unwrap_or_default().to_string(), outcome: Outcome::Theirs },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const DECK: &str = "/home/deck/Applications/Atlas.AppImage";

    #[test]
    fn the_line_is_the_contracts_exactly() {
        assert_eq!(line(DECK), r#"bash -c '"/home/deck/Applications/Atlas.AppImage" --background & exec "$@"' -- %command%"#);
        assert_eq!(line("/home/deck/My Games/Atlas Viewer.AppImage"), r#"bash -c '"/home/deck/My Games/Atlas Viewer.AppImage" --background & exec "$@"' -- %command%"#);
        assert_eq!(split_line(&line(DECK)), Some((DECK, "")));
    }

    #[test]
    fn a_path_with_a_character_the_line_cant_hold_gets_no_line_and_says_why() {
        for (path, why) in [
            ("/home/deck/it's/Atlas.AppImage", "its path holds a single quote (')"),
            ("/home/deck/\"Atlas\".AppImage", "its path holds a double quote (\")"),
            ("/home/$USER/Atlas.AppImage", "its path holds a dollar sign ($)"),
            ("/home/deck/`id`/Atlas.AppImage", "its path holds a backtick (`)"),
            ("/home/deck/a\\b/Atlas.AppImage", "its path holds a backslash (\\)"),
            ("/home/deck/a\nb/Atlas.AppImage", "its path holds a line break"),
            ("/home/deck/a\rb/Atlas.AppImage", "its path holds a line break"),
            ("/home/deck/a\tb/Atlas.AppImage", "its path holds a control character"),
            ("Applications/Atlas.AppImage", "it isn't a full path"),
            ("", "it's empty"),
            ("   ", "it's empty"),
        ] {
            assert_eq!(why_not(path).as_deref(), Some(why), "{path:?}");
            let l = launch_from(Some(path), Path::new("/home/deck"), |_| true);
            assert_eq!((l.line, l.refused.as_deref(), l.exists, l.from), (None, Some(why), false, "atlas"), "{path:?}");
            // and a line that holds one is never taken as Go's
            assert_eq!(split_line(&format!("{BEFORE}{path}{AFTER}")), None, "{path:?}");
        }
        assert_eq!(why_not(DECK), None);
        assert_eq!(why_not("/home/deck/Spiele & Co (1)/Atlas Viewer.AppImage"), None);
    }

    #[test]
    fn the_appimage_is_read_from_atlass_settings_leniently_else_its_the_usual_one() {
        let home = Path::new("/home/deck");
        let said = |text: &str| appimage_said(packs::atlas_settings_of(text).as_ref());
        // absent, null, not text, a file Atlas wouldn't read: nothing said, the usual place
        for text in [
            r#"{"format":"atlas-settings","version":1}"#,
            r#"{"format":"atlas-settings","version":1,"linuxAppImage":null}"#,
            r#"{"format":"atlas-settings","version":1,"linuxAppImage":7}"#,
            r#"{"format":"atlas-settings","version":1,"linuxAppImage":["/a"]}"#,
            r#"{"format":"other","version":1,"linuxAppImage":"/opt/Atlas.AppImage"}"#,
            r#"{"format":"atlas-settings","version":2,"linuxAppImage":"/opt/Atlas.AppImage"}"#,
            "{ not json",
        ] {
            assert_eq!(said(text), None, "{text}");
        }
        let usual = launch_from(None, home, |_| false);
        assert_eq!(usual, Launch { path: DECK.into(), from: "usual", exists: false, line: Some(line(DECK)), refused: None });
        assert!(launch_from(None, home, |p| p == Path::new(DECK)).exists);
        // a good path, with a byte order mark and other fields of any type around it
        let text = "\u{feff}{\"format\":\"atlas-settings\",\"version\":1,\"packsFolder\":5,\"linuxAppImage\":\"/run/media/deck/SD/Atlas.AppImage\"}";
        assert_eq!(said(text).as_deref(), Some("/run/media/deck/SD/Atlas.AppImage"));
        let l = launch_from(said(text).as_deref(), home, |_| true);
        assert_eq!((l.from, l.exists, l.line.as_deref(), l.refused), ("atlas", true, Some(r#"bash -c '"/run/media/deck/SD/Atlas.AppImage" --background & exec "$@"' -- %command%"#), None));
        // a bad one: no line, and why
        let bad = json!({ "format": "atlas-settings", "version": 1, "linuxAppImage": "/home/deck/$HOME/Atlas.AppImage" }).to_string();
        let l = launch_from(said(&bad).as_deref(), home, |_| true);
        assert_eq!((l.line, l.refused.as_deref(), l.path.as_str()), (None, Some("its path holds a dollar sign ($)"), "/home/deck/$HOME/Atlas.AppImage"));
        // a home folder the line can't hold: the usual place gets no line either
        let l = launch_from(None, Path::new("/home/o'brien"), |_| true);
        assert_eq!((l.line, l.refused.as_deref(), l.from), (None, Some("its path holds a single quote (')"), "usual"));
    }

    #[test]
    fn go_writes_its_line_only_over_its_own_launch_options() {
        let l = line(DECK);
        let moved = line("/home/deck/Games/Atlas.AppImage");
        let r = |current: Option<&str>, base: &str, want: Option<&str>, last: Option<&str>| {
            let x = resolve(current, base, want, last);
            (x.value, x.outcome)
        };
        // a new shortcut, or empty options: the line written
        assert_eq!(r(None, "", Some(&l), None), (l.clone(), Outcome::On));
        assert_eq!(r(Some(""), "", Some(&l), None), (l.clone(), Outcome::On));
        // the game's own arguments follow it, certain as they're plain words
        assert_eq!(r(Some("-windowed"), "-windowed", Some(&l), None), (format!("{l} -windowed"), Outcome::On));
        // Go's own line again: the same; for a moved AppImage, the new path
        assert_eq!(r(Some(&l), "", Some(&l), Some("")), (l.clone(), Outcome::On));
        assert_eq!(r(Some(&l), "", Some(&moved), Some("")), (moved.clone(), Outcome::On));
        assert_eq!(r(Some(&format!("{l} -windowed")), "-windowed", Some(&moved), None), (format!("{moved} -windowed"), Outcome::On));
        // turned off: exactly Go's line taken out
        assert_eq!(r(Some(&l), "", None, Some("")), (String::new(), Outcome::Off));
        assert_eq!(r(Some(&format!("{l} -windowed")), "-windowed", None, None), ("-windowed".into(), Outcome::Off));
        // the person's own options (changed in Steam): never written over
        for theirs in ["gamemoderun %command%", "PROTON_LOG=1 %command%", "MANGOHUD=1 %command% -novid", "-lang \"en\""] {
            assert_eq!(r(Some(theirs), "", Some(&l), None), (theirs.into(), Outcome::Theirs), "{theirs}");
            assert_eq!(r(Some(theirs), "", None, Some("")), (theirs.into(), Outcome::Off), "{theirs}");
        }
        // plain words of theirs: the line goes in front, said so, and comes out again alone
        assert_eq!(r(Some("-novid -high"), "", Some(&l), None), (format!("{l} -novid -high"), Outcome::Joined));
        assert_eq!(r(Some(&format!("{l} -novid -high")), "", Some(&moved), Some("")), (format!("{moved} -novid -high"), Outcome::Joined));
        assert_eq!(r(Some(&format!("{l} -novid -high")), "", None, None), ("-novid -high".into(), Outcome::Off));
        // a line of Go's followed by something it would never put there: theirs, whole
        let odd = format!("{l} gamemoderun %command%");
        assert_eq!(r(Some(&odd), "", Some(&l), None), (odd.clone(), Outcome::Theirs));
        assert_eq!(r(Some(&odd), "", None, None), (odd.clone(), Outcome::Off));
        // a line changed by hand (not exactly Go's): theirs
        let edited = l.replace("--background", "--background --show-game Delta");
        assert_eq!(r(Some(&edited), "", Some(&l), Some("")), (edited.clone(), Outcome::Theirs));
        assert_eq!(r(Some(&edited), "", None, Some("")), (edited.clone(), Outcome::Off));
        // the game's arguments changed in the library: what Go wrote last is still Go's
        assert_eq!(r(Some(&format!("{l} -old")), "-new", Some(&l), Some("-old")), (format!("{l} -new"), Outcome::On));
        assert_eq!(r(Some("-old"), "-new", Some(&l), Some("-old")), (format!("{l} -new"), Outcome::On));
        // game arguments that can't follow the line for certain: Go's own without it
        assert_eq!(r(Some("--name \"A B\""), "--name \"A B\"", Some(&l), None), ("--name \"A B\"".into(), Outcome::CannotJoin));
        // no line wanted, nothing of Go's changed
        assert_eq!(r(Some("-windowed"), "-windowed", None, None), ("-windowed".into(), Outcome::Off));
        assert_eq!(r(None, "", None, None), (String::new(), Outcome::Off));
    }
}
