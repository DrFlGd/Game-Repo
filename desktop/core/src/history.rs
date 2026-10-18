//! A collection's history (docs/PLAN.md, "Phase 8 design", D67): every change to what's in
//! it is a step, kept newest first in its `_library/collections/<id>.json` `history` (the
//! newest 500). A step says when, how (`hand`, `rules`, `read` for *Read them again…*,
//! `undo`), the games it added and left out, and, for a change of rules or a reading again,
//! what it was before (its rules; the DATs it was read into, copied to
//! `_library/collections/<id>/history/<step>/`). *Undo* puts a step back: a step by hand
//! leaves out the games it added and puts back those it left out (any one can be undone);
//! a change of rules or a reading again goes back to the rules (and DATs) before it, and
//! only the newest of those can be. An undo is a step of its own, so it can be undone too.
//! With groups (D75), a change of rules is a change of any group's rules, names or the
//! groups themselves, and what it was before is all of them; a reading again is of one
//! group (`group`), whose DATs are kept.

use crate::collection::{self, Collection, Worked};
use crate::index::Index;
use crate::library::Library;
use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// How many steps are kept.
const KEEP: usize = 500;
/// How many games a change of rules or a reading again lists (it's undone by what was
/// before, not by its games).
const SHOWN: usize = 50;

fn keys(w: &Worked) -> BTreeSet<String> {
    w.games().map(|g| g.key()).collect()
}

/// Each game's group (by id) where it isn't the first.
fn groups_of(w: &Worked) -> BTreeMap<String, String> {
    w.systems.iter().filter(|s| s.group > 0).flat_map(|s| s.games.iter().map(move |g| (g.key(), w.groups[s.group].id.clone()))).collect()
}

/// What chooses a collection's games apart from the hands: each group's name, rules and
/// what it was read from.
fn ways(c: &Collection) -> Value {
    json!(c.all_groups().iter().map(|g| json!([g.id, g.name, g.rules, g.made_from])).collect::<Vec<_>>())
}

/// Its games added and left out by hand, each group's.
fn hands(c: &Collection) -> Value {
    json!(c.all_groups().iter().map(|g| json!([g.id, g.added, g.removed])).collect::<Vec<_>>())
}

fn file(lib: &Library, id: &str) -> Result<PathBuf> {
    crate::library::valid_id(id)?;
    Ok(lib.root().join(crate::library::APP_DIR).join("collections").join(format!("{id}.json")))
}

fn read(lib: &Library, id: &str) -> Result<Value> {
    let p = file(lib, id)?;
    let text = std::fs::read(&p).map_err(|_| anyhow!("There's no collection {id}."))?;
    serde_json::from_slice(&text).with_context(|| format!("{} can't be read", p.display()))
}

/// A collection's steps, newest first.
pub fn steps(lib: &Library, id: &str) -> Result<Vec<Value>> {
    Ok(read(lib, id)?["history"].as_array().cloned().unwrap_or_default())
}

fn write_steps(lib: &Library, id: &str, mut steps: Vec<Value>) -> Result<()> {
    let mut v = read(lib, id)?;
    // the oldest go, and the DATs kept for them
    for gone in steps.drain(KEEP.min(steps.len())..) {
        if let Some(d) = gone["dats_before"].as_str() {
            let _ = std::fs::remove_dir_all(dats_dir(lib, id)?.join(d));
        }
    }
    v["history"] = json!(steps);
    crate::config::write_json(&file(lib, id)?, &v)
}

/// Where the DATs a collection had before it was read again are kept.
fn dats_dir(lib: &Library, id: &str) -> Result<PathBuf> {
    crate::library::valid_id(id)?;
    Ok(lib.root().join(crate::library::APP_DIR).join("collections").join(id).join("history"))
}

/// What a collection was before a change, for its step.
pub struct Before {
    pub collection: Collection,
    pub games: BTreeSet<String>,
    /// The games of its groups after the first, by key: their group's id
    pub groups: BTreeMap<String, String>,
    /// For a reading again: the step's id, under which its DATs were copied
    pub dats: Option<String>,
    /// For a reading again: the group read (its id; "" is the first)
    pub group: String,
}

/// Note what a collection is before it's changed (`how`: `read` copies the DATs of the
/// group read, by id: "" is the first).
pub fn before(ix: &Index, lib: &Library, id: &str, how: &str, group: &str) -> Result<Before> {
    let c = collection::load(lib, id)?;
    let w = collection::work_out(ix, lib, &c)?;
    let (games, groups) = (keys(&w), groups_of(&w));
    let dats = if how == "read" {
        let step = crate::organise::new_id();
        let to = dats_dir(lib, id)?.join(&step);
        std::fs::create_dir_all(&to)?;
        let files = c.group(group).map(|g| g.rules.from_files).unwrap_or_default();
        for f in &files {
            let Ok(p) = lib.resolve(f) else { continue };
            if p.is_file() {
                std::fs::copy(&p, to.join(p.file_name().unwrap_or_default())).with_context(|| format!("couldn't keep a copy of {}", p.display()))?;
            }
        }
        Some(step)
    } else {
        None
    };
    Ok(Before { collection: c, games, groups, dats, group: group.to_string() })
}

/// What a new collection was before it was made: nothing.
pub fn new(c: &Collection) -> Before {
    Before { collection: c.clone(), games: BTreeSet::new(), groups: BTreeMap::new(), dats: None, group: String::new() }
}

/// Keep the step from `b` to what the collection is now. `how`: `hand`, `rules` (worked
/// out from what changed when it's left empty), `read`, `made` (with [`new`]), or `undo`
/// (of `undoes`).
pub fn record(ix: &Index, lib: &Library, b: Before, how: &str, undoes: Option<&str>) -> Result<Option<Value>> {
    let id = b.collection.id.clone();
    let now = collection::load(lib, &id)?;
    let games = keys(&*collection::work_out(ix, lib, &now)?);
    let added: Vec<&String> = games.difference(&b.games).collect();
    let removed: Vec<&String> = b.games.difference(&games).collect();
    let rules_changed = ways(&b.collection) != ways(&now);
    // a new one: its games are what it was made with
    let rules_changed = rules_changed && how != "made";
    let how = match how {
        "" if rules_changed => "rules",
        "" => "hand",
        h => h,
    };
    if added.is_empty() && removed.is_empty() && !rules_changed && hands(&b.collection) == hands(&now) {
        if let Some(d) = &b.dats {
            let _ = std::fs::remove_dir_all(dats_dir(lib, &id)?.join(d));
        }
        return Ok(None);
    }
    let whole = how == "hand" || (how == "undo" && !rules_changed);
    let list = |v: &[&String]| -> Vec<String> { v.iter().take(if whole { usize::MAX } else { SHOWN }).map(|s| s.to_string()).collect() };
    let mut step = json!({
        "id": b.dats.clone().unwrap_or_else(crate::organise::new_id), "at": crate::library::now(), "how": how,
        "added": list(&added), "removed": list(&removed), "added_count": added.len(), "removed_count": removed.len(),
    });
    if rules_changed {
        // what it was before: undone by going back to it
        let c = &b.collection;
        step["before"] = json!({ "rules": c.rules, "made_from": c.made_from, "group_name": c.group_name, "groups": c.groups });
    }
    if let Some(d) = &b.dats {
        step["dats_before"] = json!(d);
        if !b.group.is_empty() {
            step["group"] = json!(b.group);
        }
    }
    // the groups of the games it left out, to put them back in
    let theirs: BTreeMap<&String, &String> = removed.iter().filter_map(|k| b.groups.get_key_value(*k)).collect();
    if whole && !theirs.is_empty() {
        step["groups"] = json!(theirs);
    }
    if let Some(u) = undoes {
        step["undoes"] = json!(u);
    }
    let mut all = steps(lib, &id)?;
    if let Some(u) = undoes {
        if let Some(s) = all.iter_mut().find(|s| s["id"] == u) {
            s["undone"] = json!(true);
        }
    }
    all.insert(0, step.clone());
    write_steps(lib, &id, all)?;
    Ok(Some(step))
}

/// Whether each step can be undone now: one by hand that isn't undone yet; a change of
/// rules or a reading again, the newest of those, not undone.
pub fn view(lib: &Library, id: &str) -> Result<Value> {
    let all = steps(lib, id)?;
    let newest = all.iter().position(|s| s.get("before").is_some());
    let v: Vec<Value> = all
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let mut s = s.clone();
            let undone = s["undone"] == true;
            s["can_undo"] = json!(!undone && s["how"] != "made" && if s.get("before").is_some() { Some(i) == newest } else { true });
            s
        })
        .collect();
    Ok(json!({ "steps": v }))
}

/// Undo a step (a step of its own).
pub fn undo(ix: &Index, lib: &Library, id: &str, step_id: &str) -> Result<Value> {
    lib.writable()?;
    let all = steps(lib, id)?;
    let Some(i) = all.iter().position(|s| s["id"] == step_id) else { bail!("That change isn't in the collection's history any more.") };
    let s = &all[i];
    if s["undone"] == true {
        bail!("That change has been undone already.");
    }
    if s["how"] == "made" {
        bail!("Making the collection can't be undone: delete it instead.");
    }
    let b = before(ix, lib, id, "", "")?;
    let mut c = b.collection.clone();
    if let Some(was) = s.get("before") {
        if all.iter().position(|x| x.get("before").is_some()) != Some(i) {
            bail!("Only the newest change of its rules can be undone: undo the ones after it first.");
        }
        // (the games added and left out by hand stay: they're steps of their own; a group
        // put back that's gone has its own from then)
        c.rules = serde_json::from_value(was["rules"].clone())?;
        c.made_from = was.get("made_from").cloned().filter(|v| !v.is_null());
        c.group_name = was["group_name"].as_str().unwrap_or_default().to_string();
        let mut groups: Vec<collection::Group> = serde_json::from_value(was.get("groups").cloned().filter(|v| !v.is_null()).unwrap_or_else(|| json!([])))?;
        for g in &mut groups {
            if let Some(now) = c.groups.iter().find(|x| x.id == g.id) {
                g.added = now.added.clone();
                g.removed = now.removed.clone();
            }
        }
        c.groups = groups;
        // the DATs it was read into before
        if let Some(d) = s["dats_before"].as_str() {
            let kept = dats_dir(lib, id)?.join(d);
            let own = collection::own_dir(lib, id, s["group"].as_str().unwrap_or_default())?;
            std::fs::create_dir_all(&own)?;
            for e in std::fs::read_dir(&own).into_iter().flatten().flatten() {
                if e.path().extension().is_some_and(|x| x == "dat") {
                    std::fs::remove_file(e.path())?;
                }
            }
            for e in std::fs::read_dir(&kept).into_iter().flatten().flatten() {
                std::fs::copy(e.path(), own.join(e.file_name()))?;
            }
        }
    } else {
        // the games it added are left out (taken off those added by hand, else left out by
        // hand), those it left out put back (no longer left out, and added by hand if the
        // rules don't bring them in)
        let str_list = |k: &str| -> Vec<String> { s[k].as_array().map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default() };
        let w = collection::work_out(ix, lib, &c)?;
        let inside = keys(&w);
        let came: Vec<String> = str_list("added").into_iter().filter(|k| inside.contains(k)).collect();
        c.leave_out(&w, &came)?;
        let back = str_list("removed");
        c.put_back(&back);
        let inside = keys(&*collection::work_out(ix, lib, &c)?);
        for k in back {
            if !inside.contains(&k) {
                // into the group it was in, if that's still there
                let g = s["groups"][&k].as_str().filter(|g| c.groups.iter().any(|x| x.id == *g)).unwrap_or_default().to_string();
                c.add_games(&g, &[k])?;
            }
        }
    }
    collection::save(lib, c)?;
    record(ix, lib, b, "undo", Some(step_id))?;
    view(lib, id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collection::tests::fixture;
    use crate::collection::Rules;

    fn games(ix: &Index, lib: &Library, id: &str) -> Vec<String> {
        keys(&collection::work_out(ix, lib, &collection::load(lib, id).unwrap()).unwrap()).into_iter().collect()
    }

    fn change(ix: &Index, lib: &Library, id: &str, how: &str, f: impl FnOnce(&mut Collection)) {
        let b = before(ix, lib, id, how, "").unwrap();
        let mut c = b.collection.clone();
        f(&mut c);
        collection::save(lib, c).unwrap();
        record(ix, lib, b, how, None).unwrap();
    }

    #[test]
    fn a_collections_changes_are_kept_and_undone_by_hand_and_by_rules() {
        let _turn = crate::platforms::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (dir, lib, ix) = fixture("history");
        let mut c = Collection { id: String::new(), name: "Kept".into(), rules: Rules::default(), added: vec![], removed: vec![], size_limit: None, made_from: None, created: String::new(), modified: String::new(), ..Default::default() };
        c.rules.platforms = vec!["gb".into()];
        let c = collection::save(&lib, c).unwrap();
        record(&ix, &lib, new(&c), "made", None).unwrap();
        let id = c.id.clone();
        let first = games(&ix, &lib, &id);
        assert!(first.contains(&"gb/Delta (World)".to_string()));
        // by hand: Delta left out; nothing changed: no step
        change(&ix, &lib, &id, "", |c| c.removed.push("gb/Delta (World)".into()));
        change(&ix, &lib, &id, "", |_| {});
        // the rules: Japan first
        change(&ix, &lib, &id, "", |c| c.rules.regions = ["Japan", "USA", "World", "Europe"].map(String::from).to_vec());
        let v = view(&lib, &id).unwrap();
        let s = v["steps"].as_array().unwrap();
        let hows: Vec<&str> = s.iter().map(|x| x["how"].as_str().unwrap()).collect();
        assert_eq!(hows, ["rules", "hand", "made"]);
        assert_eq!(s[1]["removed"], json!(["gb/Delta (World)"]));
        assert!(s[0]["added"].as_array().unwrap().iter().any(|g| g == "gb/Alpha (Japan)"), "{}", s[0]);
        assert_eq!(s.iter().map(|x| x["can_undo"] == true).collect::<Vec<_>>(), [true, true, false]);
        let (rules, hand) = (s[0]["id"].as_str().unwrap().to_string(), s[1]["id"].as_str().unwrap().to_string());
        // the older step by hand undone: Delta's back, the rules as they are
        undo(&ix, &lib, &id, &hand).unwrap();
        let now = games(&ix, &lib, &id);
        assert!(now.contains(&"gb/Delta (World)".to_string()) && now.contains(&"gb/Alpha (Japan)".to_string()), "{now:?}");
        assert!(undo(&ix, &lib, &id, &hand).is_err(), "only once");
        // the rules undone: as it was first
        undo(&ix, &lib, &id, &rules).unwrap();
        assert_eq!(games(&ix, &lib, &id), first);
        let s = steps(&lib, &id).unwrap();
        assert_eq!(s.iter().map(|x| x["how"].as_str().unwrap()).collect::<Vec<_>>(), ["undo", "undo", "rules", "hand", "made"]);
        // and the undo of the rules undone in its turn
        undo(&ix, &lib, &id, s[0]["id"].as_str().unwrap()).unwrap();
        assert!(games(&ix, &lib, &id).contains(&"gb/Alpha (Japan)".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_group_added_and_its_games_left_out_are_steps_undone_as_the_others() {
        let _turn = crate::platforms::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (dir, lib, ix) = fixture("history-groups");
        let mut c = Collection { name: "Grouped".into(), ..Default::default() };
        c.rules.platforms = vec!["gb".into()];
        let c = collection::save(&lib, c).unwrap();
        record(&ix, &lib, new(&c), "made", None).unwrap();
        let id = c.id.clone();
        let first = games(&ix, &lib, &id);
        // a group of the Favourites DAT's games: a change of its rules
        change(&ix, &lib, &id, "", |c| {
            c.add_group("Favs", Rules { platforms: vec!["gb".into()], include: "all".into(), dats: vec!["Favourites".into()], ..Rules::default() });
        });
        assert!(games(&ix, &lib, &id).contains(&"gb/My Delta".to_string()));
        let s = steps(&lib, &id).unwrap();
        assert_eq!(s[0]["how"], "rules");
        let added = s[0]["id"].as_str().unwrap().to_string();
        // its game left out by hand goes on its list, and the undo puts it back there
        let b = before(&ix, &lib, &id, "", "").unwrap();
        let mut c = b.collection.clone();
        let w = collection::work_out(&ix, &lib, &c).unwrap();
        c.leave_out(&w, &["gb/My Delta".into()]).unwrap();
        collection::save(&lib, c).unwrap();
        let step = record(&ix, &lib, b, "", None).unwrap().unwrap();
        assert_eq!((step["how"].as_str(), step["removed"][0].as_str()), (Some("hand"), Some("gb/My Delta")));
        let c = collection::load(&lib, &id).unwrap();
        assert_eq!(c.groups[0].removed, ["gb/My Delta"]);
        undo(&ix, &lib, &id, step["id"].as_str().unwrap()).unwrap();
        assert!(collection::load(&lib, &id).unwrap().groups[0].removed.is_empty());
        assert!(games(&ix, &lib, &id).contains(&"gb/My Delta".to_string()));
        // the group's adding undone: as it was first
        undo(&ix, &lib, &id, &added).unwrap();
        assert!(collection::load(&lib, &id).unwrap().groups.is_empty());
        assert_eq!(games(&ix, &lib, &id), first);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reading_a_collection_again_keeps_its_dats_to_go_back_to() {
        let _turn = crate::platforms::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (dir, lib, ix) = fixture("history-read");
        let id_of = |n: &str| -> i64 { ix.conn().query_row("SELECT g.id FROM dat_games g JOIN dats d ON d.id = g.dat_id WHERE d.name = 'Nintendo - Game Boy' AND g.name = ?1", [n], |r| r.get(0)).unwrap() };
        let c = crate::subset::save(&ix, &lib, crate::subset::Target::New("Read"), &[id_of("Alpha (Europe)"), id_of("Delta (World)")], json!({ "folders": [] })).unwrap();
        record(&ix, &lib, new(&c), "made", None).unwrap();
        let b = before(&ix, &lib, &c.id, "read", "").unwrap();
        crate::subset::save(&ix, &lib, crate::subset::Target::Again(&c.id, ""), &[id_of("Delta (World)")], json!({ "folders": ["again"] })).unwrap();
        let step = record(&ix, &lib, b, "read", None).unwrap().unwrap();
        assert_eq!(step["removed"], json!(["gb/Alpha (Europe)"]));
        assert!(dats_dir(&lib, &c.id).unwrap().join(step["id"].as_str().unwrap()).read_dir().unwrap().count() > 0, "its DATs kept");
        assert_eq!(games(&ix, &lib, &c.id), ["gb/Delta (World)"]);
        undo(&ix, &lib, &c.id, step["id"].as_str().unwrap()).unwrap();
        assert_eq!(games(&ix, &lib, &c.id), ["gb/Alpha (Europe)", "gb/Delta (World)"]);
        assert_eq!(collection::load(&lib, &c.id).unwrap().made_from, Some(json!({ "folders": [] })));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
