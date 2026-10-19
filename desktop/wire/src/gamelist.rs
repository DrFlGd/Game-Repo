//! EmulationStation's `gamelist.xml` (ES-DE's, Knulli's, ROCKNIX's): an entry added for
//! each game that has none and the chosen details set in each game's entry, keeping every
//! other field and every other game's entry (D35). Moved here from the export (Phase 9a)
//! so a device client merges a plan's entries into the device's gamelist the same way.

use std::collections::{HashMap, HashSet};

pub fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

pub fn xml_unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// Where the next `<game>` entry of a gamelist starts: a plain `<game>` or one with
/// attributes (Knulli's and Batocera's `<game id=… source=…>`), whichever is first (review
/// H15), never `<gameList>`.
pub fn next_game(s: &str) -> Option<usize> {
    s.match_indices("<game").map(|(at, _)| at).find(|&at| s[at + 5..].chars().next().is_some_and(|c| c == '>' || c.is_whitespace()))
}

/// A gamelist with an entry for each game of `list` (its path from the gamelist's folder,
/// its game, the name it's listed by) that has none, and the details chosen (`details`, by
/// the entry's path in lower case) set in each one's entry (its other fields, and other
/// games' entries, are kept). None: nothing changes.
pub fn merge(old: Option<&str>, list: &[(String, String, String)], details: &HashMap<String, Vec<(String, String)>>) -> Option<String> {
    let norm = |p: &str| xml_unescape(p.trim()).replace('\\', "/").trim_start_matches("./").to_lowercase();
    let text = old.unwrap_or("").to_string();
    // the entries there already, with their details set
    let mut have: HashSet<String> = HashSet::new();
    let mut out = String::new();
    let mut rest = text.as_str();
    while let Some(at) = next_game(rest) {
        let Some(end) = rest[at..].find("</game>").map(|e| at + e + "</game>".len()) else { break };
        out.push_str(&rest[..at]);
        let mut block = rest[at..end].to_string();
        let path = block.split("<path>").nth(1).and_then(|s| s.split("</path>").next()).map(norm).unwrap_or_default();
        if !path.is_empty() {
            if let Some(fields) = details.get(&path) {
                for (tag, value) in fields {
                    block = set_tag(&block, tag, value);
                }
            }
            have.insert(path);
        }
        out.push_str(&block);
        rest = &rest[end..];
    }
    out.push_str(rest);
    let add: String = list
        .iter()
        .filter(|(entry, _, _)| !have.contains(&entry.to_lowercase()))
        .map(|(entry, _, title)| {
            let mut g = format!("\t<game>\n\t\t<path>./{}</path>\n\t\t<name>{}</name>\n", xml_escape(entry), xml_escape(title));
            for (tag, value) in details.get(&entry.to_lowercase()).into_iter().flatten() {
                g.push_str(&format!("\t\t<{tag}>{}</{tag}>\n", xml_escape(value)));
            }
            g + "\t</game>\n"
        })
        .collect();
    let merged = if add.is_empty() {
        out
    } else if out.contains("</gameList>") {
        let at = out.rfind("</gameList>").unwrap();
        format!("{}{add}{}", &out[..at], &out[at..])
    } else if out.contains("<gameList/>") {
        out.replacen("<gameList/>", &format!("<gameList>\n{add}</gameList>"), 1)
    } else {
        format!("<?xml version=\"1.0\"?>\n<gameList>\n{add}</gameList>\n")
    };
    (old != Some(merged.as_str())).then_some(merged)
}

/// A plan's entries for one gamelist merged into it, as [`merge`] does.
pub fn merge_entries(old: Option<&str>, entries: &[crate::Entry]) -> Option<String> {
    let list: Vec<(String, String, String)> = entries.iter().map(|e| (e.path.clone(), e.game.clone(), e.name.clone())).collect();
    let details: HashMap<String, Vec<(String, String)>> = entries.iter().filter(|e| !e.fields.is_empty()).map(|e| (e.path.to_lowercase(), e.fields.clone())).collect();
    merge(old, &list, &details)
}

/// A `<game>` block with `<tag>` set to `value` (replaced if it's there, else added before `</game>`).
pub fn set_tag(block: &str, tag: &str, value: &str) -> String {
    set_tag_in(block, "game", tag, value)
}

/// A `<element>` block with `<tag>` set to `value`.
pub fn set_tag_in(block: &str, element: &str, tag: &str, value: &str) -> String {
    let (open, close, empty) = (format!("<{tag}>"), format!("</{tag}>"), format!("<{tag}/>"));
    let new = format!("{open}{}{close}", xml_escape(value));
    if let Some(a) = block.find(&open) {
        if let Some(b) = block[a..].find(&close) {
            return format!("{}{new}{}", &block[..a], &block[a + b + close.len()..]);
        }
    }
    if let Some(a) = block.find(&empty) {
        return format!("{}{new}{}", &block[..a], &block[a + empty.len()..]);
    }
    let at = block.rfind(&format!("</{element}>")).unwrap_or(block.len());
    format!("{}\t{new}\n\t{}", block[..at].trim_end_matches(['\t', ' ']), &block[at..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Entry;

    #[test]
    fn a_plans_entries_are_added_and_the_devices_own_kept() {
        let old = "<?xml version=\"1.0\"?>\n<gameList>\n\t<game>\n\t\t<path>./Mine.zip</path>\n\t\t<name>Mine</name>\n\t\t<playcount>4</playcount>\n\t</game>\n</gameList>\n";
        let entries = vec![
            Entry { path: "Mine.zip".into(), name: "Mine".into(), game: "Mine (USA)".into(), fields: vec![("developer".into(), "Me & Co".into())] },
            Entry { path: "Final Fantasy VII (USA).m3u".into(), name: "Final Fantasy VII".into(), game: "Final Fantasy VII (USA) (Disc 1)".into(), fields: vec![] },
        ];
        let merged = merge_entries(Some(old), &entries).unwrap();
        assert!(merged.contains("<playcount>4</playcount>"), "the device's own fields stay: {merged}");
        assert!(merged.contains("<developer>Me &amp; Co</developer>"), "{merged}");
        assert_eq!(merged.matches("<path>./Mine.zip</path>").count(), 1, "no second entry for a game listed already");
        assert!(merged.contains("<path>./Final Fantasy VII (USA).m3u</path>\n\t\t<name>Final Fantasy VII</name>"), "{merged}");
        assert_eq!(merge_entries(Some(&merged), &entries), None, "merged again, nothing changes");
    }
}
