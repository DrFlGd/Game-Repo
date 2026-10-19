//! What Game Repo Server and a device client say to each other (docs/PLAN.md, "Phase 9a
//! design", D93): a plan of the files a device writes for some games, and the merges both
//! sides use to add games to a frontend's lists while keeping what's there. The export
//! (gamerepo-core's export.rs) writes its gamelists with the same merge. Kept small, with
//! no archive or disc code, so the Android client can link it without the core.

pub mod api;
pub mod gamelist;
pub mod plan;

pub use plan::{Entry, File, Left, Lines, List, Plan};

/// A text file of lines (an ES-DE custom collection, say) with `lines` added after those
/// there already; lines there stay as they are and none is added twice. None: nothing
/// changes.
pub fn merge_lines(old: Option<&str>, lines: &[String]) -> Option<String> {
    let text = old.unwrap_or("");
    let norm = |l: &str| l.trim().replace('\\', "/");
    let have: std::collections::HashSet<String> = text.lines().map(norm).filter(|l| !l.is_empty()).collect();
    let mut out = text.to_string();
    let mut added: std::collections::HashSet<String> = std::collections::HashSet::new();
    for l in lines {
        let n = norm(l);
        if n.is_empty() || have.contains(&n) || !added.insert(n) {
            continue;
        }
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(l.trim());
        out.push('\n');
    }
    (!added.is_empty()).then_some(out)
}

/// A text file of lines without these (compared as [`merge_lines`] does). None: nothing changes.
pub fn remove_lines(old: &str, lines: &[String]) -> Option<String> {
    let norm = |l: &str| l.trim().replace('\\', "/");
    let gone: std::collections::HashSet<String> = lines.iter().map(|l| norm(l)).collect();
    let kept: Vec<&str> = old.lines().filter(|l| !gone.contains(&norm(l))).collect();
    if kept.len() == old.lines().count() {
        return None;
    }
    Some(kept.iter().map(|l| format!("{l}\n")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_added_once_after_those_there() {
        let lines = vec!["%ROMPATH%/snes/A.zip".to_string(), "%ROMPATH%/snes/B.zip".to_string(), "%ROMPATH%/snes/B.zip".to_string()];
        assert_eq!(merge_lines(None, &lines).as_deref(), Some("%ROMPATH%/snes/A.zip\n%ROMPATH%/snes/B.zip\n"));
        assert_eq!(merge_lines(Some("%ROMPATH%/nes/C.zip"), &lines).as_deref(), Some("%ROMPATH%/nes/C.zip\n%ROMPATH%/snes/A.zip\n%ROMPATH%/snes/B.zip\n"));
        assert_eq!(merge_lines(Some("%ROMPATH%\\snes\\A.zip\r\n%ROMPATH%/snes/B.zip\r\n"), &lines), None, "a line there in another form is the same line");
        assert_eq!(merge_lines(Some("x\n"), &[]), None);
        assert_eq!(remove_lines("a\n%ROMPATH%\\snes\\B.zip\nc\n", &["%ROMPATH%/snes/B.zip".to_string()]).as_deref(), Some("a\nc\n"));
        assert_eq!(remove_lines("a\n", &["b".to_string()]), None);
    }
}
