//! A device Game Repo knows, as the library keeps it (docs/PLAN.md, "Save sync and the
//! Devices page", D187–D189): `_library/devices/<id>/` with `device.json` (what it is and when
//! it was last connected), `contents.json` (what it held then), `queue.json` (the changes
//! waiting for it, written by Game Repo) and `done.json` (what became of each, written by
//! whoever applied it), so each file has one writer. Game Repo Go sends its contents and takes
//! its queue through Game Repo Server (`/v1/device/contents`, `/v1/device/queue`,
//! `/v1/device/done`).

use serde::{Deserialize, Serialize};

/// The folder of the devices' records in the library's `_library`.
pub const FOLDER: &str = "devices";

/// `device.json`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Info {
    pub id: String,
    pub name: String,
    /// android | windows | linux | steamdeck (Game Repo Go), or card (a folder device, D189)
    pub kind: String,
    /// How it's reached: `go` (Game Repo Go, through the server) or `folder`
    pub by: String,
    /// When it was last connected (seconds since 1970)
    pub seen: i64,
    pub free: u64,
    pub size: u64,
    pub games: u64,
}

/// A game on a device, as *On this device* lists it.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Held {
    /// Its path on the device (what removing it asks by)
    pub id: String,
    /// The library's game, when it's one
    #[serde(skip_serializing_if = "String::is_empty")]
    pub key: String,
    /// library (added from it), known (from elsewhere, but the library has it), elsewhere
    pub from: String,
    pub name: String,
    pub system: String,
    pub size: u64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub last_played: String,
    pub added: i64,
}

/// `contents.json`, and what Game Repo Go sends (`PUT /v1/device/contents`).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Contents {
    /// When it was read (seconds since 1970)
    pub at: i64,
    pub free: u64,
    pub games: Vec<Held>,
}

/// A change waiting for a device (D187): a game to add or one to remove.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Change {
    pub id: String,
    /// add | remove
    pub action: String,
    pub key: String,
    /// For a removal, the game's path on the device
    #[serde(skip_serializing_if = "String::is_empty")]
    pub game: String,
    pub title: String,
    pub system: String,
    pub size: u64,
    /// When it was asked for (seconds since 1970)
    pub made: i64,
}

/// `queue.json`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Queue {
    pub changes: Vec<Change>,
}

/// What became of a change.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Outcome {
    pub id: String,
    pub at: i64,
    /// done | failed | taken (the device has it in its own queue, downloading)
    pub state: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// `done.json`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Done {
    pub outcomes: Vec<Outcome>,
}

impl Done {
    /// The changes of `q` with no outcome yet, or one that failed and may be tried again.
    pub fn waiting<'a>(&self, q: &'a Queue) -> Vec<&'a Change> {
        q.changes.iter().filter(|c| !self.outcomes.iter().any(|o| o.id == c.id && o.state != "failed")).collect()
    }

    /// Each change's newest outcome.
    pub fn of(&self, id: &str) -> Option<&Outcome> {
        self.outcomes.iter().rev().find(|o| o.id == id)
    }
}

/// A device's id as a folder name.
pub fn folder_name(id: &str) -> String {
    id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_wait_until_done_and_a_failed_one_is_tried_again() {
        let c = |id: &str| Change { id: id.into(), action: "add".into(), ..Default::default() };
        let q = Queue { changes: vec![c("a"), c("b"), c("c")] };
        let done = Done { outcomes: vec![Outcome { id: "a".into(), state: "done".into(), ..Default::default() }, Outcome { id: "b".into(), state: "failed".into(), ..Default::default() }] };
        let ids: Vec<&str> = done.waiting(&q).iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["b", "c"]);
        assert_eq!(folder_name("dev/1 x"), "dev_1_x");
    }
}
