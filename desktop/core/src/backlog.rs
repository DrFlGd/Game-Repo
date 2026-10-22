//! The backlog (docs/PLAN.md, D157–D159): the games you mean to play, in the order you'll
//! play them, beside those you've finished. One to a library, kept with the collections'
//! files as `_library/backlog.json`: `games` (to play, the first is next) and `done`
//! (completed, the newest first), each game by its key in the Library with its title and
//! system (so one gone from the Library is still named), when it was added, since when it's
//! been in progress and when it was completed. It takes any game the Library shows, PC
//! games too (a collection holds the DATs' games only). Every change returns the backlog
//! as it was before it (`before`), which `restore` puts back: the page's Undo.

use crate::browse;
use crate::index::Index;
use crate::library::{now, Library, APP_DIR};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::path::PathBuf;

pub const FILE: &str = "backlog.json";

/// A game in the backlog.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Entry {
    /// Its key in the Library: `<system>/<DAT name>`, `pc/<id>`
    pub key: String,
    pub title: String,
    pub platform: String,
    pub added: String,
    /// In progress since (empty: it isn't)
    #[serde(skip_serializing_if = "String::is_empty")]
    pub started: String,
    /// When it was completed (those in `done`)
    #[serde(skip_serializing_if = "String::is_empty")]
    pub completed: String,
    /// Keys a newer app added
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Backlog {
    /// To play, in order: the first is the one to play next
    pub games: Vec<Entry>,
    /// Completed, the newest first
    pub done: Vec<Entry>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub modified: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A game to add: its key, and its title and system as the Library shows them.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Game {
    pub key: String,
    pub title: String,
    pub platform: String,
}

impl Backlog {
    /// Games added to the end of the list to play (to its top with `top`), in the order
    /// given. One completed is played again: it leaves Completed. Returns how many were
    /// new to the list.
    pub fn add(&mut self, games: &[Game], top: bool) -> usize {
        let mut new = vec![];
        for g in games {
            if g.key.is_empty() || self.games.iter().any(|e| e.key == g.key) || new.iter().any(|e: &Entry| e.key == g.key) {
                continue;
            }
            self.done.retain(|e| e.key != g.key);
            new.push(Entry { key: g.key.clone(), title: g.title.clone(), platform: g.platform.clone(), added: now(), ..Default::default() });
        }
        let n = new.len();
        if top {
            self.games.splice(0..0, new);
        } else {
            self.games.extend(new);
        }
        n
    }

    /// Games moved to place `to` (0 is the top) of the list to play, in their order there
    /// (those completed after them, back from Completed).
    pub fn move_to(&mut self, keys: &[String], to: usize) {
        let want: HashSet<&str> = keys.iter().map(String::as_str).collect();
        let (mut moved, rest): (Vec<Entry>, Vec<Entry>) = std::mem::take(&mut self.games).into_iter().partition(|e| want.contains(e.key.as_str()));
        self.games = rest;
        let (back, done): (Vec<Entry>, Vec<Entry>) = std::mem::take(&mut self.done).into_iter().partition(|e| want.contains(e.key.as_str()));
        self.done = done;
        moved.extend(back.into_iter().map(|e| Entry { completed: String::new(), started: String::new(), ..e }));
        let at = to.min(self.games.len());
        self.games.splice(at..at, moved);
    }

    /// In progress (`on`), or not: games in the list to play.
    pub fn set_playing(&mut self, keys: &[String], on: bool) {
        let when = now();
        for e in self.games.iter_mut().filter(|e| keys.contains(&e.key)) {
            if !on {
                e.started.clear();
            } else if e.started.is_empty() {
                e.started = when.clone();
            }
        }
    }

    /// Completed: out of the list to play, to the top of Completed (in their order),
    /// with today's date. When it was started is kept.
    pub fn complete(&mut self, keys: &[String]) {
        let when = now();
        let want: HashSet<&str> = keys.iter().map(String::as_str).collect();
        let (done, rest): (Vec<Entry>, Vec<Entry>) = std::mem::take(&mut self.games).into_iter().partition(|e| want.contains(e.key.as_str()));
        self.games = rest;
        let done: Vec<Entry> = done.into_iter().map(|e| Entry { completed: when.clone(), ..e }).collect();
        self.done.splice(0..0, done);
    }

    /// Games taken out of the backlog, from either list.
    pub fn remove(&mut self, keys: &[String]) {
        self.games.retain(|e| !keys.contains(&e.key));
        self.done.retain(|e| !keys.contains(&e.key));
    }
}

fn path(lib: &Library) -> PathBuf {
    lib.root().join(APP_DIR).join(FILE)
}

/// The library's backlog (an empty one before it has any).
pub fn load(lib: &Library) -> Result<Backlog> {
    let p = path(lib);
    match std::fs::read(&p) {
        Ok(bytes) => serde_json::from_slice(&bytes).with_context(|| format!("{} can't be read", p.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Backlog::default()),
        Err(e) => Err(e).with_context(|| format!("{} can't be read", p.display())),
    }
}

fn save(lib: &Library, b: &mut Backlog) -> Result<()> {
    lib.writable()?;
    // each game once, in one list or the other
    let mut seen = HashSet::new();
    b.games.retain(|e| !e.key.is_empty() && seen.insert(e.key.clone()));
    b.done.retain(|e| !e.key.is_empty() && seen.insert(e.key.clone()));
    for e in &mut b.done {
        if e.completed.is_empty() {
            e.completed = now();
        }
    }
    for e in &mut b.games {
        e.completed.clear();
    }
    b.modified = now();
    let p = path(lib);
    std::fs::create_dir_all(p.parent().unwrap_or(&p))?;
    crate::config::write_json(&p, &serde_json::to_value(&*b)?)
}

/// The backlog as the page shows it: each game with its picture, title, system and year
/// as the Library has them now (`missing` when it isn't in the Library any more).
pub fn view(ix: &Index, lib: &Library) -> Result<Value> {
    let b = load(lib)?;
    let keys: Vec<String> = b.games.iter().chain(&b.done).map(|e| e.key.clone()).collect();
    let rows = browse::rows_of(ix.conn(), &keys)?;
    let shown = |e: &Entry| {
        let mut v = serde_json::to_value(e).unwrap_or_default();
        match rows.get(&e.key) {
            Some(r) => {
                for (k, x) in r.as_object().into_iter().flatten() {
                    if k != "key" {
                        v[k] = x.clone();
                    }
                }
            }
            None => {
                v["missing"] = json!(true);
                v["platform_name"] = json!(browse::system_name(&e.platform));
            }
        }
        v
    };
    Ok(json!({
        "games": b.games.iter().map(shown).collect::<Vec<_>>(),
        "done": b.done.iter().map(shown).collect::<Vec<_>>(),
        "playing": b.games.iter().filter(|e| !e.started.is_empty()).count(),
    }))
}

/// A change to the backlog, saved; returns it as the page shows it, with what it was
/// before (`before`, for Undo).
pub fn change(ix: &Index, lib: &Library, f: impl FnOnce(&mut Backlog) -> Result<Value>) -> Result<Value> {
    let before = load(lib)?;
    let mut b = before.clone();
    let said = f(&mut b)?;
    save(lib, &mut b)?;
    let mut v = view(ix, lib)?;
    v["before"] = serde_json::to_value(&before)?;
    if let Some(o) = said.as_object() {
        for (k, x) in o {
            v[k] = x.clone();
        }
    }
    Ok(v)
}

/// Games added (`Backlog::add`), with their titles and systems from the Library where the
/// page didn't give them (*Add to the backlog* on every game a Library search found).
pub fn add(ix: &Index, lib: &Library, mut games: Vec<Game>, top: bool) -> Result<Value> {
    let keys: Vec<String> = games.iter().map(|g| g.key.clone()).collect();
    let rows = browse::rows_of(ix.conn(), &keys)?;
    for g in &mut games {
        let row = rows.get(&g.key);
        let of = |k: &str| row.and_then(|r| r[k].as_str()).unwrap_or_default().to_string();
        if g.title.is_empty() {
            g.title = of("title");
        }
        if g.platform.is_empty() {
            g.platform = of("platform");
        }
        if g.title.is_empty() {
            g.title = g.key.split_once('/').map_or(g.key.as_str(), |x| x.1).to_string();
        }
    }
    change(ix, lib, |b| Ok(json!({ "added": b.add(&games, top) })))
}

/// Put back the backlog as it was (Undo): `was` is a change's `before`.
pub fn restore(ix: &Index, lib: &Library, was: Value) -> Result<Value> {
    let was: Backlog = serde_json::from_value(was).context("That isn't a backlog")?;
    change(ix, lib, |b| {
        *b = was;
        Ok(json!({}))
    })
}

/// Where a game is in the backlog (for its page): `todo` with its place (1 is next) and
/// since when it's been in progress, or `done` with when; None: not in it.
pub fn of_game(lib: &Library, key: &str) -> Option<Value> {
    let b = load(lib).ok()?;
    if let Some(i) = b.games.iter().position(|e| e.key == key) {
        let e = &b.games[i];
        return Some(json!({ "list": "todo", "place": i + 1, "of": b.games.len(), "started": e.started, "added": e.added }));
    }
    b.done.iter().find(|e| e.key == key).map(|e| json!({ "list": "done", "completed": e.completed, "started": e.started }))
}

/// Keys from the page's arguments, at least one.
pub fn keys_of(args: &Value) -> Result<Vec<String>> {
    let keys: Vec<String> = args["keys"].as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from).collect();
    if keys.is_empty() {
        bail!("Choose a game first.");
    }
    Ok(keys)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(key: &str) -> Game {
        Game { key: key.into(), title: key.split('/').next_back().unwrap_or(key).into(), platform: key.split('/').next().unwrap_or("").into() }
    }

    fn keys(v: &[Entry]) -> Vec<&str> {
        v.iter().map(|e| e.key.as_str()).collect()
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn ranks_progress_and_completion() {
        let mut b = Backlog::default();
        assert_eq!(b.add(&[g("gb/A"), g("gb/B"), g("pc/c1"), g("gb/A")], false), 3);
        assert_eq!(keys(&b.games), ["gb/A", "gb/B", "pc/c1"]);
        // added again: nothing new; to the top
        assert_eq!(b.add(&[g("gb/B")], true), 0);
        assert_eq!(b.add(&[g("snes/D")], true), 1);
        assert_eq!(keys(&b.games), ["snes/D", "gb/A", "gb/B", "pc/c1"]);
        // moved: the place counts in the list without them
        b.move_to(&s(&["snes/D"]), 2);
        assert_eq!(keys(&b.games), ["gb/A", "gb/B", "snes/D", "pc/c1"]);
        b.move_to(&s(&["pc/c1", "gb/A"]), 0);
        assert_eq!(keys(&b.games), ["gb/A", "pc/c1", "gb/B", "snes/D"]);
        b.move_to(&s(&["gb/A"]), 99);
        assert_eq!(keys(&b.games), ["pc/c1", "gb/B", "snes/D", "gb/A"]);
        // in progress, then completed: the newest first, its start kept
        b.set_playing(&s(&["gb/B", "nope"]), true);
        assert!(!b.games[1].started.is_empty() && b.games[0].started.is_empty());
        b.complete(&s(&["gb/B"]));
        b.complete(&s(&["pc/c1"]));
        assert_eq!(keys(&b.games), ["snes/D", "gb/A"]);
        assert_eq!(keys(&b.done), ["pc/c1", "gb/B"]);
        assert!(!b.done[1].started.is_empty() && !b.done[1].completed.is_empty());
        // back to the list to play, where it's put; and added again from Completed
        b.move_to(&s(&["gb/B"]), 1);
        assert_eq!(keys(&b.games), ["snes/D", "gb/B", "gb/A"]);
        assert!(b.games[1].completed.is_empty() && b.games[1].started.is_empty());
        assert_eq!(b.add(&[g("pc/c1")], false), 1);
        assert_eq!((keys(&b.games), keys(&b.done)), (vec!["snes/D", "gb/B", "gb/A", "pc/c1"], vec![]));
        b.set_playing(&s(&["gb/A"]), true);
        b.set_playing(&s(&["gb/A"]), false);
        assert!(b.games.iter().all(|e| e.started.is_empty()));
        b.remove(&s(&["gb/B", "pc/c1"]));
        assert_eq!(keys(&b.games), ["snes/D", "gb/A"]);
    }

    #[test]
    fn kept_in_the_library_with_undo() {
        let (dir, lib, ix) = crate::collection::tests::fixture("backlog");
        browse::refresh(&ix, &lib).unwrap();
        assert_eq!(view(&ix, &lib).unwrap()["games"], json!([]));
        // (a game a search found comes by its key alone: its title and system are the Library's)
        let found = Game { key: "gb/Beta (Japan)".into(), ..Default::default() };
        let v = add(&ix, &lib, vec![found, g("gb/Gone (USA)")], false).unwrap();
        assert_eq!(v["added"], 2);
        assert_eq!(v["before"]["games"], json!([]));
        let first = &v["games"][0];
        assert_eq!((first["key"].as_str(), first["platform_name"].as_str(), first["have"].as_bool()), (Some("gb/Beta (Japan)"), Some("Game Boy"), Some(true)));
        assert!(first.get("missing").is_none());
        assert_eq!((load(&lib).unwrap().games[0].title.as_str(), load(&lib).unwrap().games[0].platform.as_str()), ("Beta", "gb"));
        // a game the Library doesn't have keeps its own title and system
        assert_eq!((v["games"][1]["missing"].as_bool(), v["games"][1]["title"].as_str()), (Some(true), Some("Gone (USA)")));
        let v = change(&ix, &lib, |b| {
            b.complete(&s(&["gb/Beta (Japan)"]));
            Ok(json!({}))
        })
        .unwrap();
        assert_eq!((v["games"].as_array().unwrap().len(), v["done"][0]["key"].as_str()), (1, Some("gb/Beta (Japan)")));
        assert_eq!(of_game(&lib, "gb/Beta (Japan)").unwrap()["list"], "done");
        assert_eq!(of_game(&lib, "gb/Gone (USA)").unwrap()["place"], 1);
        assert!(of_game(&lib, "gb/Delta (World)").is_none());
        // Undo puts back the list as it was before the change
        let back = restore(&ix, &lib, v["before"].clone()).unwrap();
        assert_eq!((back["games"].as_array().unwrap().len(), back["done"].as_array().unwrap().len()), (2, 0));
        assert_eq!(load(&lib).unwrap().games[0].key, "gb/Beta (Japan)");
        assert!(lib.root().join(APP_DIR).join(FILE).is_file());
        // a file that can't be read isn't taken for an empty backlog
        std::fs::write(lib.root().join(APP_DIR).join(FILE), b"{ nope").unwrap();
        assert!(view(&ix, &lib).is_err());
        drop(ix);
        let _ = std::fs::remove_dir_all(dir);
    }
}
