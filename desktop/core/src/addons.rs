//! Updates, DLC and patches tied to the game they belong to (docs/PLAN.md, "Phase 8
//! design", D66). An *add-on* is a DAT game that's an update, DLC or patch: its DAT says so
//! in its name ("(Updates and DLC)"), or its own name has an `(Update)`, `(DLC)` or
//! `(Patch)` tag. Each belongs to a base game in the system's audit DAT: the one with the
//! same title and region, else the only one with the same title. Links made or broken by
//! hand are kept in library.json `addons` (`{system: {add-on: base or null}}`).

use crate::audit::{self, GAMES};
use crate::library::Library;
use anyhow::{bail, Result};
use rusqlite::Connection;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap};

/// The parenthesised groups of a name, lower-case: "Alpha (USA) (Update)" is ["usa", "update"].
fn groups(name: &str) -> Vec<String> {
    let mut v = vec![];
    let mut rest = name;
    while let Some(a) = rest.find(['(', '[']) {
        let close = if rest[a..].starts_with('(') { ')' } else { ']' };
        let Some(b) = rest[a..].find(close) else { break };
        v.push(rest[a + 1..a + b].trim().to_lowercase());
        rest = &rest[a + b + 1..];
    }
    v
}

/// Whether a DAT holds add-ons (No-Intro's "… (Digital) (Updates and DLC)", "… (Patches)").
pub fn is_addon_dat(name: &str) -> bool {
    groups(name).iter().any(|g| g.contains("update") || g.contains("dlc") || g.starts_with("patch") || g.contains("add-on"))
}

/// What kind of add-on a DAT game is, if it's one: `update`, `dlc` or `patch`.
pub fn kind_of(name: &str, in_addon_dat: bool) -> Option<&'static str> {
    let g = groups(name);
    if g.iter().any(|x| x == "dlc" || x.starts_with("dlc ") || x.ends_with(" dlc")) {
        Some("dlc")
    } else if g.iter().any(|x| x.starts_with("patch")) {
        Some("patch")
    } else if g.iter().any(|x| x.contains("update")) {
        Some("update")
    } else if in_addon_dat {
        Some("update")
    } else {
        None
    }
}

/// An add-on's version from its name ("(v1.2)", "(Version 1.2)"), if it says.
pub fn version_of(name: &str) -> Option<String> {
    groups(name).into_iter().find_map(|g| {
        let v = g.strip_prefix("version ").or_else(|| g.strip_prefix('v'))?.trim();
        v.chars().next().filter(char::is_ascii_digit).map(|_| format!("v{v}"))
    })
}

/// A name's title and first group (its region, as No-Intro writes them): what a base
/// game and its add-ons share.
fn title_region(name: &str) -> (String, String) {
    let title = name.split(" (").next().unwrap_or(name).trim().to_lowercase();
    (title, groups(name).into_iter().next().unwrap_or_default())
}

/// An add-on and the game it belongs to.
#[derive(Clone, Debug)]
pub struct Link {
    pub id: i64,
    pub dat_id: i64,
    pub name: String,
    pub kind: &'static str,
    pub base: Option<String>,
    /// Linked (or unlinked) by hand
    pub by_hand: bool,
}

/// Every add-on of a system and the base game each belongs to.
pub fn links(conn: &Connection, lib: &Library, platform: &str) -> Result<Vec<Link>> {
    let Some(primary) = audit::primary_dat(conn, lib, platform)? else { return Ok(vec![]) };
    let mut q = conn.prepare("SELECT g.id, g.dat_id, g.name, d.name FROM dat_games g JOIN dats d ON d.id = g.dat_id WHERE d.platform = ?1 AND d.current = 1 ORDER BY g.name")?;
    let rows: Vec<(i64, i64, String, String)> = q.query_map([platform], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<rusqlite::Result<_>>()?;
    // the base games: the audit DAT's games that aren't add-ons themselves
    let mut by_both: HashMap<(String, String), Vec<String>> = HashMap::new();
    let mut by_title: HashMap<String, Vec<String>> = HashMap::new();
    for (_, dat, name, _) in &rows {
        if *dat == primary && kind_of(name, false).is_none() {
            let (t, r) = title_region(name);
            by_both.entry((t.clone(), r)).or_default().push(name.clone());
            by_title.entry(t).or_default().push(name.clone());
        }
    }
    let hand = lib.meta()["addons"][platform].as_object().cloned().unwrap_or_default();
    let mut v = vec![];
    for (id, dat, name, dat_name) in rows {
        let Some(kind) = kind_of(&name, is_addon_dat(&dat_name)) else { continue };
        let (base, by_hand) = match hand.get(&name) {
            Some(Value::String(b)) => (Some(b.clone()), true),
            Some(Value::Null) => (None, true),
            _ => {
                let (t, r) = title_region(&name);
                let one = |v: Option<&Vec<String>>| v.filter(|v| v.len() == 1).map(|v| v[0].clone());
                // the same title and region, else the only game with the title
                let base = by_both.get(&(t.clone(), r)).and_then(|v| v.first().cloned()).or_else(|| one(by_title.get(&t)));
                (base, false)
            }
        };
        v.push(Link { id, dat_id: dat, name, kind, base, by_hand });
    }
    Ok(v)
}

/// The add-ons that belong to a system's games, by base game.
pub fn by_base(conn: &Connection, lib: &Library, platform: &str) -> Result<BTreeMap<String, Vec<Link>>> {
    let mut m: BTreeMap<String, Vec<Link>> = BTreeMap::new();
    for l in links(conn, lib, platform)? {
        if let Some(b) = l.base.clone() {
            m.entry(b).or_default().push(l);
        }
    }
    Ok(m)
}

/// How many add-ons you have (all their files) of each of a system's base games, by name.
pub fn have_by_base(conn: &Connection, lib: &Library, platform: &str) -> Result<HashMap<String, i64>> {
    let all = links(conn, lib, platform)?;
    let status = statuses(conn, all.iter().map(|l| l.dat_id).collect::<std::collections::BTreeSet<_>>())?;
    let mut m = HashMap::new();
    for l in all {
        if let (Some(b), Some("have")) = (l.base, status.get(&l.id).map(String::as_str)) {
            *m.entry(b).or_insert(0) += 1;
        }
    }
    Ok(m)
}

/// Each game's audit status (have, partial, missing) in the DATs `dats`.
pub fn statuses(conn: &Connection, dats: impl IntoIterator<Item = i64>) -> Result<HashMap<i64, String>> {
    let mut out = HashMap::new();
    for d in dats {
        let mut q = conn.prepare(&format!("WITH {GAMES} SELECT gs.id, gs.status FROM gs"))?;
        for r in q.query_map([d], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))? {
            let (id, s) = r?;
            out.insert(id, s);
        }
    }
    Ok(out)
}

/// A game's add-ons for its page: each with its kind, version and whether you have it;
/// and the system's add-ons that belong to no game, for linking by hand.
pub fn of_game(conn: &Connection, lib: &Library, platform: &str, base: &str) -> Result<Value> {
    let all = links(conn, lib, platform)?;
    let mine: Vec<&Link> = all.iter().filter(|l| l.base.as_deref() == Some(base)).collect();
    let status = statuses(conn, mine.iter().map(|l| l.dat_id).collect::<std::collections::BTreeSet<_>>())?;
    let items: Vec<Value> = mine
        .iter()
        .map(|l| json!({ "name": l.name, "kind": l.kind, "version": version_of(&l.name), "status": status.get(&l.id).cloned().unwrap_or_else(|| "missing".into()), "by_hand": l.by_hand }))
        .collect();
    let have = items.iter().filter(|i| i["status"] == "have").count();
    let loose: Vec<&str> = all.iter().filter(|l| l.base.is_none()).map(|l| l.name.as_str()).collect();
    Ok(json!({ "items": items, "have": have, "loose": loose }))
}

/// Link an add-on to `base` by hand (None: it belongs to no game), or (`clear`) go back
/// to the link its name gives it.
pub fn set(conn: &Connection, lib: &Library, platform: &str, addon: &str, base: Option<&str>, clear: bool) -> Result<()> {
    lib.writable()?;
    if !links(conn, lib, platform)?.iter().any(|l| l.name == addon) {
        bail!("{addon} isn't an update, DLC or patch of {}.", crate::platforms::name_of(platform));
    }
    let mut all: Map<String, Value> = lib.meta()["addons"].as_object().cloned().unwrap_or_default();
    let mut sys: Map<String, Value> = all.get(platform).and_then(Value::as_object).cloned().unwrap_or_default();
    if clear {
        sys.remove(addon);
    } else {
        sys.insert(addon.to_string(), base.map(|b| json!(b)).unwrap_or(Value::Null));
    }
    all.insert(platform.to_string(), Value::Object(sys));
    lib.update_meta(json!({ "addons": all }))?;
    audit::changed(conn)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_ons_are_told_by_their_dat_or_their_tags_and_their_versions_read() {
        assert!(is_addon_dat("Nintendo - Nintendo 3DS (Digital) (Updates and DLC)"));
        assert!(!is_addon_dat("Nintendo - Nintendo 3DS (Digital)"));
        assert_eq!(kind_of("Alpha (USA) (DLC)", false), Some("dlc"));
        assert_eq!(kind_of("Alpha (USA) (Update) (v1.2)", false), Some("update"));
        assert_eq!(kind_of("Alpha (USA) (v1.2)", true), Some("update"));
        assert_eq!(kind_of("Alpha (USA)", false), None);
        assert_eq!(version_of("Alpha (USA) (Update) (v1.2)").as_deref(), Some("v1.2"));
        assert_eq!(version_of("Alpha (USA) (Version 2.0)").as_deref(), Some("v2.0"));
        assert_eq!(version_of("Alpha (USA) (Rev 1)"), None);
        assert_eq!(title_region("Alpha Quest (Europe) (Update)"), ("alpha quest".into(), "europe".into()));
    }
}
