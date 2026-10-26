//! The devices Game Repo knows (docs/PLAN.md, "Save sync and the Devices page", D187–D188):
//! each one's record in `_library/devices/<id>/` (`gamerepo_wire::devices`), what it held when
//! last connected, and the changes waiting for it. Game Repo writes the queue; Game Repo
//! Server writes what a Game Repo Go device sends and what became of each change; so the
//! Devices page works from any computer the library's on, and from a server on a NAS.

use crate::config::write_json;
use crate::library::APP_DIR;
use anyhow::{bail, Result};
use gamerepo_wire::devices::{folder_name, Change, Contents, Done, Info, Outcome, Queue, FOLDER};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// The devices' records under a library's folder.
pub struct Records {
    dir: PathBuf,
}

fn read<T: DeserializeOwned + Default>(p: &Path) -> T {
    std::fs::read(p).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn write<T: serde::Serialize>(p: &Path, v: &T) -> Result<()> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    write_json(p, &serde_json::to_value(v)?)
}

impl Records {
    pub fn of(library: &Path) -> Self {
        Self { dir: library.join(APP_DIR).join(FOLDER) }
    }

    fn at(&self, id: &str) -> PathBuf {
        self.dir.join(folder_name(id))
    }

    pub fn info(&self, id: &str) -> Option<Info> {
        let p = self.at(id).join("device.json");
        p.is_file().then(|| read(&p))
    }

    pub fn contents(&self, id: &str) -> Contents {
        read(&self.at(id).join("contents.json"))
    }

    pub fn queue(&self, id: &str) -> Queue {
        read(&self.at(id).join("queue.json"))
    }

    pub fn done(&self, id: &str) -> Done {
        read(&self.at(id).join("done.json"))
    }

    /// Every device with a record, the one seen last first.
    pub fn all(&self) -> Vec<Info> {
        let mut v: Vec<Info> = std::fs::read_dir(&self.dir).into_iter().flatten().flatten().filter_map(|e| {
            let p = e.path().join("device.json");
            p.is_file().then(|| read::<Info>(&p))
        }).filter(|i| !i.id.is_empty()).collect();
        v.sort_by(|a, b| b.seen.cmp(&a.seen).then(a.name.cmp(&b.name)));
        v
    }

    /// What a device sent when it connected (D188): its record and its contents, written
    /// by whoever it connected to.
    pub fn connected(&self, mut info: Info, contents: &Contents) -> Result<()> {
        info.seen = now();
        info.free = contents.free;
        info.size = contents.games.iter().map(|g| g.size).sum();
        info.games = contents.games.len() as u64;
        write(&self.at(&info.id).join("contents.json"), contents)?;
        write(&self.at(&info.id).join("device.json"), &info)
    }

    /// The changes waiting for a device.
    pub fn waiting(&self, id: &str) -> Vec<Change> {
        let q = self.queue(id);
        self.done(id).waiting(&q).into_iter().cloned().collect()
    }

    /// What became of some changes, added to `done.json`.
    pub fn record(&self, id: &str, outcomes: &[Outcome]) -> Result<()> {
        let mut d = self.done(id);
        for o in outcomes {
            let mut o = o.clone();
            if o.at == 0 {
                o.at = now();
            }
            d.outcomes.push(o);
        }
        write(&self.at(id).join("done.json"), &d)
    }

    /// Changes queued for a device (D187), each given an id; one already waiting (the same
    /// game, added or removed) isn't queued twice, and a game to add that's waiting to be
    /// removed (or the other way round) cancels that one instead.
    pub fn add(&self, id: &str, changes: Vec<Change>) -> Result<usize> {
        if self.info(id).is_none() {
            bail!("Game Repo doesn't know that device.");
        }
        let mut q = self.queue(id);
        let mut waiting: Vec<Change> = self.waiting(id);
        let mut added = 0;
        let mut n = q.changes.len();
        for mut c in changes {
            if c.action != "add" && c.action != "remove" {
                bail!("A device's change is add or remove, not {}.", c.action);
            }
            if waiting.iter().any(|w| w.key == c.key && w.action == c.action && w.game == c.game) {
                continue;
            }
            if let Some(at) = waiting.iter().position(|w| w.key == c.key && !c.key.is_empty() && w.action != c.action) {
                let w = waiting.remove(at);
                q.changes.retain(|x| x.id != w.id);
                added += 1;
                continue;
            }
            n += 1;
            // unique however many were cancelled before
            let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
            c.id = format!("{nanos:x}-{n}");
            c.made = now();
            waiting.push(c.clone());
            q.changes.push(c);
            added += 1;
        }
        write(&self.at(id).join("queue.json"), &q)?;
        Ok(added)
    }

    /// Changes taken back before the device took them.
    pub fn cancel(&self, id: &str, ids: &[String]) -> Result<()> {
        let waiting: Vec<String> = self.waiting(id).into_iter().map(|c| c.id).collect();
        let mut q = self.queue(id);
        q.changes.retain(|c| !(ids.contains(&c.id) && waiting.contains(&c.id)));
        write(&self.at(id).join("queue.json"), &q)
    }

    /// A device's record gone (its saves stay in `Saves/`).
    pub fn forget(&self, id: &str) -> Result<()> {
        let d = self.at(id);
        if d.is_dir() {
            std::fs::remove_dir_all(&d)?;
        }
        Ok(())
    }

    /// The devices that held a game when last connected, for its page.
    pub fn holding(&self, key: &str) -> Vec<Value> {
        self.all().into_iter().filter(|i| self.contents(&i.id).games.iter().any(|g| g.key == key)).map(|i| json!({ "id": i.id, "name": i.name, "seen": i.seen })).collect()
    }

    /// The Devices page's list (D187): each device with its changes waiting.
    pub fn list(&self) -> Value {
        let v: Vec<Value> = self.all().into_iter().map(|i| {
            let w = self.waiting(&i.id);
            json!({ "info": i, "waiting": w.len(), "waiting_size": w.iter().filter(|c| c.action == "add").map(|c| c.size).sum::<u64>() })
        }).collect();
        json!(v)
    }

    /// A device as its page shows it: its record, its contents, and each change with what
    /// became of it (the newest hundred done).
    pub fn view(&self, id: &str) -> Result<Value> {
        let Some(info) = self.info(id) else { bail!("Game Repo doesn't know that device.") };
        let q = self.queue(id);
        let done = self.done(id);
        let contents = self.contents(id);
        let mut changes: Vec<Value> = q.changes.iter().map(|c| {
            let o = done.of(&c.id);
            // an added game the device has since is done, whatever it said when it took it
            let there = c.action == "add" && contents.games.iter().any(|g| g.key == c.key);
            let state = match (o.map(|o| o.state.as_str()), there) {
                (_, true) => "done",
                (Some(s), _) => s,
                (None, _) => "waiting",
            };
            json!({ "change": c, "state": state, "note": o.map(|o| o.note.clone()).unwrap_or_default(), "at": o.map(|o| o.at).unwrap_or(0) })
        }).collect();
        changes.reverse();
        let mut finished = 0;
        changes.retain(|c| {
            if c["state"] == "done" {
                finished += 1;
                finished <= 100
            } else {
                true
            }
        });
        Ok(json!({ "info": info, "contents": contents, "changes": changes }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gamerepo_wire::devices::Held;

    #[test]
    fn a_devices_changes_wait_for_it_and_follow_what_it_did() {
        let t = crate::library::tests::temp_dir("devices-records");
        let r = Records::of(&t);
        let info = Info { id: "dev-1".into(), name: "Steam Deck".into(), kind: "steamdeck".into(), by: "go".into(), ..Default::default() };
        let held = Held { id: "gb/Alpha.gb".into(), key: "gb/Alpha".into(), from: "library".into(), name: "Alpha".into(), system: "gb".into(), size: 10, ..Default::default() };
        r.connected(info, &Contents { at: 1, free: 99, games: vec![held] }).unwrap();
        assert_eq!(r.all()[0].games, 1);
        let add = |key: &str| Change { action: "add".into(), key: key.into(), size: 5, ..Default::default() };
        let remove = Change { action: "remove".into(), key: "gb/Alpha".into(), game: "gb/Alpha.gb".into(), ..Default::default() };
        assert_eq!(r.add("dev-1", vec![add("gb/Beta"), add("gb/Beta"), remove.clone()]).unwrap(), 2);
        assert_eq!(r.add("dev-1", vec![add("gb/Beta")]).unwrap(), 0, "waiting already");
        let w = r.waiting("dev-1");
        assert_eq!(w.len(), 2);
        assert_eq!(r.list()[0]["waiting_size"], 5);
        // adding back what's waiting to be removed takes the removal back
        assert_eq!(r.add("dev-1", vec![add("gb/Alpha")]).unwrap(), 1);
        assert_eq!(r.waiting("dev-1").len(), 1);
        // the device took Beta into its queue: it's shown done once it's there
        r.record("dev-1", &[Outcome { id: w[0].id.clone(), state: "taken".into(), ..Default::default() }]).unwrap();
        assert!(r.waiting("dev-1").is_empty());
        assert_eq!(r.view("dev-1").unwrap()["changes"][0]["state"], "taken");
        let beta = Held { id: "gb/Beta.gb".into(), key: "gb/Beta".into(), from: "library".into(), ..Default::default() };
        r.connected(r.info("dev-1").unwrap(), &Contents { at: 2, free: 90, games: vec![beta] }).unwrap();
        assert_eq!(r.view("dev-1").unwrap()["changes"][0]["state"], "done");
        assert_eq!(r.holding("gb/Beta").len(), 1);
        assert!(r.add("nobody", vec![add("x")]).is_err());
        r.forget("dev-1").unwrap();
        assert!(r.all().is_empty());
    }
}
