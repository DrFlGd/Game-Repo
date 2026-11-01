//! A PC game Game Repo Go installed, checked against how it was installed and put back
//! (docs/PLAN.md, "Mods on installed games", D207–D208). The record has each file the
//! install put in the game's folder with its SHA-1, so a check finds every change, however
//! it was made (a mod put in by hand, a patcher, a file deleted); putting the game back
//! fetches what changed or went from the library again (the game as the library has it,
//! with no mods), and moves what was added into the game's `.gamerepo-mods/put-aside/`,
//! never deleting it. Files the user unticks are kept as they are from then on.

use crate::client::{Server, Watch};
use crate::{add_planned, AddOptions, Device, WRITING};
use anyhow::{anyhow, bail, Result};
use gamerepo_wire::gamefiles::{self, Check, State, Want};
use gamerepo_wire::Plan;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;

/// A PC game on the device: its folder in the PC games folder (place `pc`) and its files
/// there as the install put them.
fn installed(d: &Device, key: &str) -> Result<(String, Vec<Want>)> {
    let rec = d.record();
    let h = rec.games.iter().find(|g| g.key == key).ok_or_else(|| anyhow!("This device doesn't hold {key} from the library."))?;
    let pc: Vec<_> = h.files.iter().filter(|f| f.place == "pc").collect();
    let folder = pc
        .iter()
        .find(|f| f.role == "game")
        .or(pc.first())
        .and_then(|f| f.path.split_once('/').map(|(t, _)| t.to_string()))
        .ok_or_else(|| anyhow!("{} isn't a PC game Game Repo Go installed.", h.title))?;
    let prefix = format!("{folder}/");
    let want = pc.iter().filter_map(|f| Some(Want { path: f.path.strip_prefix(&prefix)?.to_string(), size: f.size, sha1: f.sha1.clone() })).collect();
    Ok((folder, want))
}

fn dir_of(d: &Device, folder: &str) -> Result<PathBuf> {
    if !d.places.contains_key("pc") {
        bail!("This device has no PC games folder.");
    }
    d.at("pc", folder)
}

fn hash(p: &std::path::Path) -> std::io::Result<String> {
    crate::sha1_file(p).map_err(std::io::Error::other)
}

/// *Check the game's files* (D207).
pub fn check(d: &Device, key: &str) -> Result<Check> {
    let (folder, want) = installed(d, key)?;
    Ok(gamefiles::check(&dir_of(d, &folder)?, &want, &hash)?)
}

/// What *Put the game back…* is asked to leave as it is: the files unticked.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PutBack {
    pub leave: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PutBackDone {
    /// Files fetched again from the library
    pub fetched: usize,
    /// Files moved into `.gamerepo-mods/put-aside/<when>/`
    pub put_aside: usize,
    /// Files left as they are, kept as the game's own
    pub kept: usize,
    pub when: String,
    pub notes: Vec<String>,
}

/// *Put the game back as it was installed…* (D208): what changed or went fetched again,
/// what was added put aside, what's in `leave` kept as it is.
pub fn put_back(s: &Server, d: &Device, key: &str, o: &AddOptions, ask: &PutBack, progress: &mut dyn FnMut(&str)) -> Result<PutBackDone> {
    let (folder, _) = installed(d, key)?;
    let dir = dir_of(d, &folder)?;
    let c = check(d, key)?;
    if c.state == "not_there" {
        bail!("Its folder isn't there any more ({}): remove it from the device and download it again.", c.folder);
    }
    let listed: HashSet<String> = c.changed.iter().chain(&c.added).chain(&c.missing).map(|p| p.to_lowercase()).collect();
    let leave: HashSet<String> = ask.leave.iter().map(|p| p.to_lowercase()).filter(|p| listed.contains(p)).collect();
    // the game as the library has it, no mods (D208)
    let o = AddOptions { mods: false, media_only: false, adopt: vec![], ..o.clone() };
    let mut plan: Plan = s.call("POST", "/v1/plans", Some(&o.ask(&[key.to_string()])))?;
    if plan.waiting.iter().any(|k| k == key) {
        bail!("The library is still unpacking it: try again in a minute.");
    }
    let prefix = format!("{folder}/");
    let rel_of = |p: &str| p.strip_prefix(&prefix).map(str::to_lowercase);
    if !plan.files.iter().any(|f| f.place == "pc" && f.key == key) {
        let why = plan.left.first().map(|l| l.why.clone()).unwrap_or_else(|| "The library has none of its files now.".into());
        bail!("It can't be put back: {why}");
    }
    let mut out = PutBackDone { when: gamefiles::when_now(), ..Default::default() };
    // the files left as they are (now, or kept before) stay out of the plan, so they aren't fetched
    let kept: HashSet<String> = State::read(&dir).keep.iter().map(|k| k.to_lowercase()).collect();
    plan.files.retain(|f| !(f.place == "pc" && f.key == key && rel_of(&f.path).is_some_and(|r| leave.contains(&r) || kept.contains(&r))));
    let in_plan: HashSet<String> = plan.files.iter().filter(|f| f.place == "pc" && f.key == key).filter_map(|f| rel_of(&f.path)).collect();
    // what was added goes aside, and so does a file only a mod gave (not in the game now)
    let mut aside: Vec<String> = c.added.iter().filter(|p| !leave.contains(&p.to_lowercase())).cloned().collect();
    for w in installed(d, key)?.1 {
        let l = w.path.to_lowercase();
        if !in_plan.contains(&l) && !leave.contains(&l) && !c.missing.iter().any(|m| m.eq_ignore_ascii_case(&w.path)) && dir.join(&w.path).is_file() {
            aside.push(w.path);
        }
    }
    // a file changed is fetched again: the record forgets its SHA-1 so it isn't taken as there
    {
        let _w = WRITING.lock().unwrap_or_else(|e| e.into_inner());
        let mut rec = d.record();
        if let Some(h) = rec.games.iter_mut().find(|g| g.key == key) {
            for f in h.files.iter_mut().filter(|f| f.place == "pc") {
                if f.path.strip_prefix(&prefix).is_some_and(|r| c.changed.iter().any(|x| x.eq_ignore_ascii_case(r)) && !leave.contains(&r.to_lowercase())) {
                    f.sha1.clear();
                }
            }
        }
        d.save_record(&rec)?;
    }
    let (todo, _) = crate::to_fetch(d, &plan)?;
    out.fetched = todo.iter().filter(|&&i| plan.files[i].place == "pc").count();
    let added = add_planned(s, d, &plan, &[key.to_string()], &o, Watch::default(), progress)?;
    out.notes = added.notes;
    for rel in &aside {
        if dir.join(rel).is_file() {
            gamefiles::put_file_aside(&dir, &out.when, rel)?;
            out.put_aside += 1;
        }
    }
    let mut st = State::read(&dir);
    for p in &ask.leave {
        if leave.contains(&p.to_lowercase()) && !st.keeps(p) {
            st.keep.push(p.clone());
            out.kept += 1;
        }
    }
    st.write(&dir)?;
    Ok(out)
}

/// The mods Game Repo Go put in an installed PC game, as its folder's state says (none: not
/// known), without reading its files.
pub fn mods_in(d: &Device, key: &str) -> Result<Option<Vec<String>>> {
    let (folder, _) = installed(d, key)?;
    Ok(State::read(&dir_of(d, &folder)?).mods)
}

/// What mods a plan put in a PC game, kept in its folder's state (D209): each file's mod,
/// and the mods in order. Called as a game's files are put in place.
pub fn note_mods(d: &Device, key: &str, plan: &Plan) -> Result<()> {
    let (folder, _) = installed(d, key)?;
    let dir = dir_of(d, &folder)?;
    let prefix = format!("{folder}/");
    let mut st = State::read(&dir);
    st.owners.clear();
    let mut mods: Vec<String> = vec![];
    for f in plan.files.iter().filter(|f| f.place == "pc" && f.key == key && !f.from.is_empty()) {
        let Some(rel) = f.path.strip_prefix(&prefix) else { continue };
        st.owners.insert(rel.to_string(), f.from.clone());
        if !mods.contains(&f.from) {
            mods.push(f.from.clone());
        }
    }
    st.mods = Some(mods);
    Ok(st.write(&dir)?)
}

/// *Apply* on an installed PC game's mods (D209): what each file decided on when it was
/// changed by hand since (replace it, or keep yours).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Apply {
    /// The mods wanted, by name (any order: they go in the library's)
    pub mods: Vec<String>,
    /// Files changed by hand to write over anyway
    pub replace: Vec<String>,
    /// Files changed by hand to keep as they are
    pub keep: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Applied {
    /// Files changed by hand that the mods would change: nothing's done until each is
    /// replaced or kept
    pub ask: Vec<String>,
    pub fetched: usize,
    pub removed: usize,
    pub mods: Vec<String>,
    pub notes: Vec<String>,
}

/// The mods on a game already installed (D209): the game's folder made what its files with
/// these mods over them are, changing only what differs. A file changed by hand is never
/// written over or taken away without asking (`ask`).
pub fn apply_mods(s: &Server, d: &Device, key: &str, o: &AddOptions, ask: &Apply, progress: &mut dyn FnMut(&str)) -> Result<Applied> {
    let (folder, had) = installed(d, key)?;
    let dir = dir_of(d, &folder)?;
    let c = check(d, key)?;
    if c.state == "not_there" {
        bail!("Its folder isn't there any more ({}): remove it from the device and download it again.", c.folder);
    }
    let o = AddOptions { mods: false, media_only: false, adopt: vec![], ..o.clone() };
    let mut want = o.ask(&[key.to_string()]);
    want.mod_picks = vec![(key.to_string(), ask.mods.clone())];
    let mut plan: Plan = s.call("POST", "/v1/plans", Some(&want))?;
    if plan.waiting.iter().any(|k| k == key) {
        bail!("The library is still unpacking it or its mods: try again in a minute.");
    }
    if !plan.files.iter().any(|f| f.place == "pc" && f.key == key) {
        let why = plan.left.first().map(|l| l.why.clone()).unwrap_or_else(|| "The library has none of its files now.".into());
        bail!("Its mods can't be applied: {why}");
    }
    let prefix = format!("{folder}/");
    let lower = |p: &str| p.to_lowercase();
    let new: std::collections::HashMap<String, Option<String>> = plan.files.iter().filter(|f| f.place == "pc" && f.key == key).filter_map(|f| Some((lower(f.path.strip_prefix(&prefix)?), f.sha1.clone()))).collect();
    let old: std::collections::HashMap<String, String> = had.iter().map(|w| (lower(&w.path), w.sha1.clone())).collect();
    // what the mods change: a file new, different, or no longer there
    let touched = |rel: &str| -> bool {
        let l = lower(rel);
        match (new.get(&l), old.get(&l)) {
            (Some(Some(n)), Some(o)) => !n.eq_ignore_ascii_case(o),
            (Some(_), _) => true,
            (None, Some(_)) => true,
            (None, None) => false,
        }
    };
    let decided = |rel: &String| ask.replace.iter().chain(&ask.keep).any(|x| x.eq_ignore_ascii_case(rel));
    let by_hand: Vec<&String> = c.changed.iter().chain(&c.missing).collect();
    let asks: Vec<String> = by_hand.iter().filter(|r| touched(r) && !decided(r)).map(|r| r.to_string()).collect();
    if !asks.is_empty() {
        return Ok(Applied { ask: asks, ..Default::default() });
    }
    // a file kept as the game's own (now, or before) is left as it is
    let keep: HashSet<String> = ask.keep.iter().chain(&State::read(&dir).keep).map(|x| lower(x)).collect();
    let replace: HashSet<String> = ask.replace.iter().map(|x| lower(x)).collect();
    plan.files.retain(|f| !(f.place == "pc" && f.key == key && f.path.strip_prefix(&prefix).is_some_and(|r| keep.contains(&lower(r)))));
    // what no longer belongs: taken away after (one changed by hand and replaced, put aside)
    let gone: Vec<(String, bool)> = had.iter().filter(|w| !new.contains_key(&lower(&w.path)) && !keep.contains(&lower(&w.path))).map(|w| (w.path.clone(), replace.contains(&lower(&w.path)))).collect();
    {
        let _w = WRITING.lock().unwrap_or_else(|e| e.into_inner());
        let mut rec = d.record();
        if let Some(h) = rec.games.iter_mut().find(|g| g.key == key) {
            for f in h.files.iter_mut().filter(|f| f.place == "pc") {
                if f.path.strip_prefix(&prefix).is_some_and(|r| replace.contains(&lower(r))) {
                    f.sha1.clear();
                }
            }
        }
        d.save_record(&rec)?;
    }
    let (todo, _) = crate::to_fetch(d, &plan)?;
    let mut out = Applied { fetched: todo.iter().filter(|&&i| plan.files[i].place == "pc").count(), ..Default::default() };
    let added = add_planned(s, d, &plan, &[key.to_string()], &o, Watch::default(), progress)?;
    out.notes = added.notes;
    let when = gamefiles::when_now();
    for (rel, aside) in gone {
        if !dir.join(&rel).is_file() {
            continue;
        }
        if aside {
            gamefiles::put_file_aside(&dir, &when, &rel)?;
        } else {
            std::fs::remove_file(dir.join(&rel))?;
            let mut at = dir.join(&rel).parent().map(PathBuf::from);
            while let Some(p) = at {
                if p == dir || std::fs::remove_dir(&p).is_err() {
                    break;
                }
                at = p.parent().map(PathBuf::from);
            }
        }
        out.removed += 1;
    }
    let mut st = State::read(&dir);
    for k in &ask.keep {
        if !st.keeps(k) {
            st.keep.push(k.clone());
        }
    }
    out.mods = st.mods.clone().unwrap_or_default();
    st.write(&dir)?;
    Ok(out)
}

/// *Empty*: what put backs put aside deleted (one time's, or all with `when` empty).
pub fn empty_aside(d: &Device, key: &str, when: &str) -> Result<usize> {
    let (folder, _) = installed(d, key)?;
    let top = dir_of(d, &folder)?.join(gamefiles::DIR).join(gamefiles::PUT_ASIDE);
    let gone: Vec<PathBuf> = if when.is_empty() {
        std::fs::read_dir(&top).into_iter().flatten().flatten().map(|e| e.path()).collect()
    } else {
        if when.contains(['/', '\\']) || when.contains("..") {
            bail!("That isn't one of its put-aside folders.");
        }
        vec![top.join(when)]
    };
    let mut n = 0;
    for p in gone.into_iter().filter(|p| p.is_dir()) {
        n += gamefiles::files_in(&p).len();
        std::fs::remove_dir_all(&p)?;
    }
    Ok(n)
}
