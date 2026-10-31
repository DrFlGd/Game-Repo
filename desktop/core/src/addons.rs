//! Updates, DLC and patches tied to the game they belong to (docs/PLAN.md, "Phase 8
//! design", D66). An *add-on* is a DAT game that's an update, DLC or patch: its DAT says so
//! in its name ("(Updates and DLC)"), or its own name has an `(Update)`, `[UPD]`, `(DLC)` or
//! `(Patch)` tag, or a title ID that's one (D160: Switch's, 3DS's and Wii U's). Each belongs
//! to a base game in the system's audit DAT: the one its custom DAT entry names, the one with
//! its title ID, the one with the same title and region, else the only one with the same
//! title, else the one whose title its name starts with (D161). Links made or broken by hand
//! are kept in library.json `addons` (`{system: {add-on: base or null}}`).

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

/// A title ID in a name (D160: sixteen hex digits, as Switch, 3DS and Wii U files are often
/// named): the ID of the base game it belongs to, and what it is (None: the base game
/// itself). Switch's base games' IDs end in 000 (and an even digit before it), an update's
/// in 800, a DLC's are the base's plus 1001 on; 3DS's and Wii U's share their last eight
/// digits, the first eight saying what each is.
pub(crate) fn title_id(name: &str) -> Option<(u64, Option<&'static str>)> {
    groups(name).into_iter().find_map(|g| {
        if g.len() != 16 || !g.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let id = u64::from_str_radix(&g, 16).ok()?;
        let (high, low) = (id >> 32, id & 0xFFFF_FFFF);
        match high {
            // 3DS and Wii U
            0x0004_0000 | 0x0005_0000 => Some((id, None)),
            0x0004_000E | 0x0005_000E => Some(((high & 0xFFFF_0000) << 32 | low, Some("update"))),
            0x0004_008C | 0x0005_000C => Some(((high & 0xFFFF_0000) << 32 | low, Some("dlc"))),
            // Switch
            _ if g.starts_with("01") => {
                let base = id & !0x1FFF;
                Some((base, match id & 0x1FFF {
                    0 => None,
                    0x800 => Some("update"),
                    n if n & 0x1000 != 0 => Some("dlc"),
                    _ => return None,
                }))
            }
            _ => None,
        }
    })
}

/// What kind of add-on a DAT game is, if it's one: `update`, `dlc` or `patch`.
pub fn kind_of(name: &str, in_addon_dat: bool) -> Option<&'static str> {
    let g = groups(name);
    if g.iter().any(|x| x == "dlc" || x.starts_with("dlc ") || x.ends_with(" dlc") || x == "add-on" || x == "addon") {
        Some("dlc")
    } else if g.iter().any(|x| x.starts_with("patch")) {
        Some("patch")
    } else if g.iter().any(|x| x.contains("update") || x == "upd") {
        Some("update")
    } else if let Some((_, kind)) = title_id(name).filter(|t| t.1.is_some()) {
        kind
    } else if in_addon_dat {
        Some("update")
    } else {
        None
    }
}

/// An add-on's version from its name ("(v1.2)", "(Version 1.2)", "[v131072]", "(Update
/// 1.2)"), if it says.
pub fn version_of(name: &str) -> Option<String> {
    groups(name).into_iter().find_map(|g| {
        let v = g
            .strip_prefix("version ")
            .or_else(|| g.strip_prefix("update v"))
            .or_else(|| g.strip_prefix("update "))
            .or_else(|| g.strip_prefix('v'))?
            .trim();
        v.chars().next().filter(char::is_ascii_digit).map(|_| format!("v{v}"))
    })
}

/// A title's words, lower-case, with no punctuation: "Alpha: Quest" and "Alpha - Quest" are
/// both "alpha quest".
fn words(title: &str) -> String {
    title.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

/// A name's title (before its first group) and region (as No-Intro writes them): what a base
/// game and its add-ons share.
fn title_region(name: &str) -> (String, String) {
    let cut = name.find(['(', '[']).unwrap_or(name.len());
    (words(&name[..cut]), crate::dat::parse_name(name).regions.join(", ").to_lowercase())
}

/// A system's base games, to find an add-on's by (D161).
#[derive(Default)]
pub struct Bases {
    names: Vec<String>,
    by_id: HashMap<u64, usize>,
    by_both: HashMap<(String, String), Vec<usize>>,
    by_title: HashMap<String, Vec<usize>>,
}

impl Bases {
    /// The games of the DAT `dat` that aren't add-ons themselves.
    pub fn of(conn: &Connection, dat: i64) -> Result<Bases> {
        let names: Vec<String> = conn.prepare("SELECT name FROM dat_games WHERE dat_id = ?1 ORDER BY name")?.query_map([dat], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        Ok(Bases::from_names(names))
    }

    fn from_names(all: Vec<String>) -> Bases {
        let mut b = Bases::default();
        for name in all.into_iter().filter(|n| kind_of(n, false).is_none()) {
            let i = b.names.len();
            if let Some((id, None)) = title_id(&name) {
                b.by_id.entry(id).or_insert(i);
            }
            let (t, r) = title_region(&name);
            b.by_both.entry((t.clone(), r)).or_default().push(i);
            b.by_title.entry(t).or_default().push(i);
            b.names.push(name);
        }
        b
    }

    pub fn has(&self, name: &str) -> bool {
        self.names.iter().any(|n| n == name)
    }

    /// The base game an add-on's name says it belongs to: the one with its title ID, the
    /// same title and region, the only one with the title; else the one whose title is the
    /// longest its title starts with ("Alpha - Booster Pack" is Alpha's), by region if
    /// several have it.
    pub fn find(&self, name: &str) -> Option<String> {
        if let Some(i) = title_id(name).and_then(|(id, _)| self.by_id.get(&id)) {
            return Some(self.names[*i].clone());
        }
        let (t, r) = title_region(name);
        let pick = |t: &str| -> Option<usize> {
            let v = self.by_title.get(t)?;
            self.by_both.get(&(t.to_string(), r.clone())).and_then(|v| v.first().copied()).or_else(|| (v.len() == 1).then(|| v[0]))
        };
        if let Some(i) = pick(&t) {
            return Some(self.names[i].clone());
        }
        let w: Vec<&str> = t.split(' ').collect();
        // the longest title it starts with decides, even when it can't say which of them
        (1..w.len()).rev().map(|n| w[..n].join(" ")).find(|p| self.by_title.contains_key(p)).and_then(|p| pick(&p)).map(|i| self.names[i].clone())
    }
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
    let mut q = conn.prepare("SELECT g.id, g.dat_id, g.name, d.name, g.cloneof FROM dat_games g JOIN dats d ON d.id = g.dat_id WHERE d.platform = ?1 AND d.current = 1 ORDER BY g.name")?;
    let rows: Vec<(i64, i64, String, String, Option<String>)> = q.query_map([platform], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?.collect::<rusqlite::Result<_>>()?;
    // the base games: the audit DAT's games that aren't add-ons themselves
    let bases = Bases::from_names(rows.iter().filter(|r| r.1 == primary).map(|r| r.2.clone()).collect());
    let hand = lib.meta()["addons"][platform].as_object().cloned().unwrap_or_default();
    let mut v = vec![];
    for (id, dat, name, dat_name, cloneof) in rows {
        let Some(kind) = kind_of(&name, is_addon_dat(&dat_name)) else { continue };
        let (base, by_hand) = match hand.get(&name) {
            Some(Value::String(b)) => (Some(b.clone()), true),
            Some(Value::Null) => (None, true),
            // the game a custom DAT's entry names (D162), else the one its name says
            _ => (cloneof.filter(|c| bases.has(c)).or_else(|| bases.find(&name)), false),
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
    have_of(conn, &links(conn, lib, platform)?)
}

/// How many of these add-ons you have (all their files), by base game.
pub fn have_of(conn: &Connection, links: &[Link]) -> Result<HashMap<String, i64>> {
    let status = statuses(conn, links.iter().map(|l| l.dat_id).collect::<std::collections::BTreeSet<_>>())?;
    let mut m = HashMap::new();
    for l in links {
        if let (Some(b), Some("have")) = (&l.base, status.get(&l.id).map(String::as_str)) {
            *m.entry(b.clone()).or_insert(0) += 1;
        }
    }
    Ok(m)
}

/// An add-on's name as the Library shows it: without its title ID, which says nothing to a
/// person ("Alpha [Bonus][0100ABCD12341001][v0]" is "Alpha [Bonus][v0]").
pub fn shown_name(name: &str) -> String {
    let mut out = String::new();
    let mut rest = name;
    while let Some(a) = rest.find(['(', '[']) {
        let close = if rest[a..].starts_with('(') { ')' } else { ']' };
        let Some(b) = rest[a..].find(close) else { break };
        let inner = &rest[a + 1..a + b];
        out.push_str(&rest[..a]);
        if !(inner.len() == 16 && inner.chars().all(|c| c.is_ascii_hexdigit())) {
            out.push_str(&rest[a..a + b + 1]);
        }
        rest = &rest[a + b + 1..];
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
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

    #[test]
    fn add_ons_are_told_by_the_tags_and_title_ids_files_are_named_with() {
        // D160: scene tags, in brackets too
        assert_eq!(kind_of("Alpha [UPD][v65536]", false), Some("update"));
        assert_eq!(kind_of("Alpha [DLC] Booster Pack", false), Some("dlc"));
        assert_eq!(kind_of("Alpha (Update 1.0.2)", false), Some("update"));
        assert_eq!(version_of("Alpha (Update 1.0.2)").as_deref(), Some("v1.0.2"));
        assert_eq!(version_of("Alpha [0100ABCD12340800][v131072]").as_deref(), Some("v131072"));
        // Switch: a base game's ID ends in 000, an update's in 800, a DLC's from 1001
        assert_eq!(kind_of("Alpha [0100ABCD12340000][v0]", false), None);
        assert_eq!(kind_of("Alpha [0100ABCD12340800][v131072]", false), Some("update"));
        assert_eq!(kind_of("Alpha [Bonus Track][0100ABCD12341001][v0]", false), Some("dlc"));
        assert_eq!(title_id("Alpha [0100ABCD12341003]"), Some((0x0100_ABCD_1234_0000, Some("dlc"))));
        // 3DS and Wii U: the first eight digits say which
        assert_eq!(kind_of("Alpha (USA) [0004000E00055D00]", false), Some("update"));
        assert_eq!(kind_of("Alpha (USA) [0004008C00055D00]", false), Some("dlc"));
        assert_eq!(kind_of("Alpha (USA) [0005000E10101C00]", false), Some("update"));
        assert_eq!(title_id("Alpha [0004008C00055D00]"), title_id("Alpha [0004000000055D00]").map(|(id, _)| (id, Some("dlc"))));
        // not a title ID: a CRC, or sixteen hex digits that aren't one
        assert_eq!(kind_of("Alpha (USA) [ABCD1234]", false), None);
        assert_eq!(kind_of("Alpha [FFFFFFFFFFFFFFFF]", false), None);
        // "DLC Quest" is a game, not a DLC
        assert_eq!(kind_of("DLC Quest (USA)", false), None);
        assert_eq!(shown_name("Alpha [Bonus Track][0100ABCD12341001][v0]"), "Alpha [Bonus Track][v0]");
        assert_eq!(shown_name("Alpha (USA) [0004000E00055D00] (v1.2)"), "Alpha (USA) (v1.2)");
    }

    #[test]
    fn a_base_game_is_found_by_title_id_then_title_and_region_then_the_title_a_name_starts_with() {
        let b = Bases::from_names(
            ["Alpha (USA)", "Alpha (Europe)", "Alpha Centauri (USA)", "Beta [0100ABCD12340000][v0]", "Beta [0100ABCD12340800][v65536]", "Gamma: The Quest (World)", "Delta (USA)", "Delta (Japan)"]
                .map(String::from)
                .to_vec(),
        );
        // an add-on in the list isn't a base game
        assert!(b.has("Beta [0100ABCD12340000][v0]") && !b.has("Beta [0100ABCD12340800][v65536]"));
        let f = |n: &str| b.find(n);
        assert_eq!(f("Alpha (Europe) (DLC)").as_deref(), Some("Alpha (Europe)"));
        // by title ID, whatever the name says
        assert_eq!(f("Something Else [0100ABCD12341001][v0]").as_deref(), Some("Beta [0100ABCD12340000][v0]"));
        // the longest title the name starts with, punctuation aside
        assert_eq!(f("Alpha Centauri - Map Pack (USA) (DLC)").as_deref(), Some("Alpha Centauri (USA)"));
        assert_eq!(f("Gamma - The Quest - Update 2 [UPD]").as_deref(), Some("Gamma: The Quest (World)"));
        assert_eq!(f("Alpha - Booster Pack (USA) (DLC)").as_deref(), Some("Alpha (USA)"));
        // several with the title and none with the region: none
        assert_eq!(f("Delta - Extra [DLC]"), None);
        assert_eq!(f("Omega (USA) (DLC)"), None);
    }
}
