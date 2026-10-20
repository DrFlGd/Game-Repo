//! The queue (docs/PLAN.md, "Phase 9b design", D99): games, not files, kept in
//! `queue.json` in the client's data folder until they're done. One game at a time, its
//! files fetched as [`crate::add_planned`] fetches them (three at once, each resumed and
//! checked), and its gamelist entry and record written only when its last file is in. When
//! the library can't be reached the queue waits and tries again by itself; a plan whose
//! files changed in the library since is asked for again. Closing and opening the client
//! goes on where it stopped.

use crate::client::Refused;
use crate::{add_planned, drop_parts, part_of, to_fetch, AddOptions, Damaged, Device, Server, Stopped, Watch};
use anyhow::Result;
use gamerepo_wire::api::HeldFile;
use gamerepo_wire::Plan;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// How long the queue waits before trying the library again.
pub const RETRY: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    #[default]
    Waiting,
    Downloading,
    Paused,
    /// Waiting for the library (it can't be reached)
    Offline,
    Done,
    Failed,
}

/// A game in the queue.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Item {
    pub id: u64,
    pub key: String,
    pub title: String,
    pub system: String,
    /// The media kinds asked for with it, and its manual
    pub media: Vec<String>,
    pub manuals: bool,
    /// Only its media and manual, for a game the device has (D102)
    pub media_only: bool,
    /// Its files on the device, for a game from elsewhere whose media is asked for (D102)
    pub adopt: Vec<HeldFile>,
    /// The ES-DE custom collection it's put in too (C16); empty: none
    pub collection: String,
    pub state: State,
    /// What went wrong
    pub error: String,
    /// Its files' bytes, and those in
    pub size: u64,
    pub done: u64,
    /// Bytes a second, while it's downloading
    #[serde(skip_deserializing)]
    pub speed: u64,
    /// When it was queued (seconds since 1970)
    pub added: u64,
    /// The plan its files come by, once asked for
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<Plan>,
    /// The plan was asked for again once already (its files changed while they came)
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub replanned: bool,
}

/// What a game is added with: the server, the device and the options from Settings.
pub struct Ctx {
    pub server: Server,
    pub device: Device,
    pub options: AddOptions,
}

/// The game downloading now.
struct Running {
    id: u64,
    stop: Arc<AtomicBool>,
    got: Arc<AtomicU64>,
    cancel: bool,
    samples: VecDeque<(Instant, u64)>,
    /// Its plan, once asked for (to drop what came of it when it's cancelled)
    plan: Option<Plan>,
}

#[derive(Default)]
struct Inner {
    items: Vec<Item>,
    running: Option<Running>,
    /// Not before then: the library couldn't be reached
    retry_at: Option<Instant>,
    /// Games added since ES-DE was last said to be restarted
    added: usize,
    quit: bool,
}

/// The queue's state as the page shows it.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Shown {
    pub items: Vec<Item>,
    /// Waiting for the library
    pub offline: bool,
    /// Games added since: "Restart ES-DE to see the new games"
    pub added: usize,
    /// The games still to come, their bytes and those in, and the speed
    pub left: usize,
    pub size: u64,
    pub done: u64,
    pub speed: u64,
}

pub struct Queue {
    path: PathBuf,
    inner: Mutex<Inner>,
    wake: Condvar,
    ctx: Box<dyn Fn() -> Result<Ctx> + Send + Sync>,
    changed: Box<dyn Fn() + Send + Sync>,
    /// How long it waits for the library before trying again, in milliseconds
    retry: AtomicU64,
    /// A game downloading pauses once this much of it is in (for the tests; u64::MAX: never)
    pause_at: AtomicU64,
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Whether an error is the library not being reached (the queue waits), rather than
/// something wrong with the game or the device.
pub fn is_offline(e: &anyhow::Error) -> bool {
    use std::io::ErrorKind::*;
    if e.is::<Refused>() || e.is::<Damaged>() || e.is::<Stopped>() {
        return false;
    }
    let net = e.chain().any(|c| c.downcast_ref::<std::io::Error>().is_some_and(|io| matches!(io.kind(), ConnectionRefused | ConnectionReset | ConnectionAborted | NotConnected | TimedOut | UnexpectedEof | WouldBlock | HostUnreachable | NetworkUnreachable | BrokenPipe | AddrNotAvailable)));
    let text = format!("{e:#}");
    net || text.contains("isn't answering") || text.contains("stopped sending") || text.contains("couldn't talk to the server")
}

impl Queue {
    /// The queue kept at `path`; a game that was downloading when the client closed waits
    /// to go on once it's [started](Self::start). `ctx` gives what a game is added with
    /// when it starts; `changed` is told whenever the queue changes (the page, the tray,
    /// the notification).
    pub fn open(path: PathBuf, ctx: Box<dyn Fn() -> Result<Ctx> + Send + Sync>, changed: Box<dyn Fn() + Send + Sync>) -> Arc<Queue> {
        let mut items: Vec<Item> = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        for it in &mut items {
            if matches!(it.state, State::Downloading | State::Offline) {
                it.state = State::Waiting;
            }
        }
        Arc::new(Queue { path, inner: Mutex::new(Inner { items, ..Default::default() }), wake: Condvar::new(), ctx, changed, retry: AtomicU64::new(RETRY.as_millis() as u64), pause_at: AtomicU64::new(u64::MAX) })
    }

    /// How long it waits for the library before trying again ([`RETRY`]; shorter in the tests).
    pub fn set_retry(&self, wait: Duration) {
        self.retry.store(wait.as_millis() as u64, Ordering::SeqCst);
    }

    /// A game downloading paused once `bytes` of it are in, as if *Pause* were pressed
    /// then (for the tests).
    #[doc(hidden)]
    pub fn pause_at(&self, bytes: Option<u64>) {
        self.pause_at.store(bytes.unwrap_or(u64::MAX), Ordering::SeqCst);
    }

    /// Its thread started: the games waiting come, one at a time.
    pub fn start(self: &Arc<Self>) {
        let runner = self.clone();
        std::thread::Builder::new().name("queue".into()).spawn(move || runner.run()).expect("the queue's thread");
    }

    fn save(&self, inner: &Inner) {
        if let Ok(b) = serde_json::to_vec(&inner.items) {
            let _ = crate::write_whole(&self.path, &b);
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Games added to the end of the queue (a game there already and not done isn't added
    /// twice). Their ids.
    pub fn add(&self, items: Vec<Item>) -> Vec<u64> {
        let mut inner = self.lock();
        let mut next = inner.items.iter().map(|i| i.id).max().unwrap_or(0) + 1;
        let mut ids = vec![];
        for mut it in items {
            if let Some(there) = inner.items.iter_mut().find(|x| x.key == it.key && x.media_only == it.media_only && x.state != State::Done) {
                if there.state == State::Failed || there.state == State::Paused {
                    there.state = State::Waiting;
                    there.error.clear();
                }
                ids.push(there.id);
                continue;
            }
            inner.items.retain(|x| !(x.key == it.key && x.media_only == it.media_only && x.state == State::Done));
            it.id = next;
            next += 1;
            it.state = State::Waiting;
            it.added = now();
            ids.push(it.id);
            inner.items.push(it);
        }
        self.save(&inner);
        drop(inner);
        self.wake.notify_all();
        (self.changed)();
        ids
    }

    /// The queue as it stands, with the game downloading's bytes and speed now.
    pub fn shown(&self) -> Shown {
        let mut inner = self.lock();
        let (mut live, mut speed) = (None, 0);
        if let Some(r) = inner.running.as_mut() {
            let got = r.got.load(Ordering::SeqCst);
            let t = Instant::now();
            r.samples.push_back((t, got));
            while r.samples.len() > 2 && t.duration_since(r.samples[0].0) > Duration::from_secs(5) {
                r.samples.pop_front();
            }
            let (t0, g0) = r.samples[0];
            let secs = t.duration_since(t0).as_secs_f64();
            speed = if secs > 0.2 { (got.saturating_sub(g0) as f64 / secs) as u64 } else { 0 };
            live = Some((r.id, got));
        }
        let mut s = Shown { offline: inner.retry_at.is_some(), added: inner.added, ..Default::default() };
        for it in &inner.items {
            let mut it = Item { plan: None, ..it.clone() };
            if let Some((_, got)) = live.filter(|(id, _)| *id == it.id) {
                it.done = got.min(it.size);
                it.speed = speed;
            }
            if !matches!(it.state, State::Done | State::Failed) {
                s.left += 1;
                s.size += it.size;
                s.done += it.done;
            }
            s.items.push(it);
        }
        s.speed = speed;
        s
    }

    /// Whether anything is still to come (the Windows client stays by the clock until not).
    pub fn busy(&self) -> bool {
        self.lock().items.iter().any(|i| matches!(i.state, State::Waiting | State::Downloading | State::Offline))
    }

    /// Games paused (none named: every one not done); what came is kept to go on from.
    pub fn pause(&self, ids: &[u64]) {
        let mut inner = self.lock();
        let all = ids.is_empty();
        for it in inner.items.iter_mut().filter(|i| all || ids.contains(&i.id)) {
            if matches!(it.state, State::Waiting | State::Offline | State::Downloading) {
                it.state = State::Paused;
            }
        }
        if let Some(r) = inner.running.as_ref().filter(|r| all || ids.contains(&r.id)) {
            r.stop.store(true, Ordering::SeqCst);
        }
        self.save(&inner);
        drop(inner);
        (self.changed)();
    }

    /// Paused games, and those that went wrong, to go on (none named: every one).
    pub fn resume(&self, ids: &[u64]) {
        let mut inner = self.lock();
        let all = ids.is_empty();
        for it in inner.items.iter_mut().filter(|i| all || ids.contains(&i.id)) {
            if matches!(it.state, State::Paused | State::Failed) {
                it.state = State::Waiting;
                it.error.clear();
                it.replanned = false;
            }
        }
        // a game asked for again goes now, without waiting out the retry
        inner.retry_at = None;
        self.save(&inner);
        drop(inner);
        self.wake.notify_all();
        (self.changed)();
    }

    /// Games taken off the queue, with what came of them.
    pub fn cancel(&self, ids: &[u64]) {
        let mut inner = self.lock();
        let mut gone = vec![];
        let running = inner.running.as_ref().map(|r| r.id);
        inner.items.retain(|i| {
            let go = ids.contains(&i.id) && i.state != State::Done;
            if go && Some(i.id) != running {
                gone.push(i.clone());
            }
            !go
        });
        if let Some(r) = inner.running.as_mut().filter(|r| ids.contains(&r.id)) {
            r.cancel = true;
            r.stop.store(true, Ordering::SeqCst);
        }
        self.save(&inner);
        drop(inner);
        if gone.iter().any(|i| i.plan.is_some()) {
            if let Ok(c) = (self.ctx)() {
                for p in gone.iter().filter_map(|i| i.plan.as_ref()) {
                    drop_parts(&c.device, p);
                }
            }
        }
        (self.changed)();
    }

    /// The games done taken off the list, and the restart notice with them.
    pub fn clear_done(&self) {
        let mut inner = self.lock();
        inner.items.retain(|i| i.state != State::Done);
        inner.added = 0;
        self.save(&inner);
        drop(inner);
        (self.changed)();
    }

    /// The thread stops (the client is closing); what's downloading carries on next time.
    pub fn quit(&self) {
        let mut inner = self.lock();
        inner.quit = true;
        if let Some(r) = &inner.running {
            r.stop.store(true, Ordering::SeqCst);
        }
        drop(inner);
        self.wake.notify_all();
    }

    /// The queue's thread: a game at a time, in order.
    fn run(&self) {
        loop {
            let mut inner = self.lock();
            let item = loop {
                if inner.quit {
                    return;
                }
                if let Some(at) = inner.retry_at {
                    let left = at.saturating_duration_since(Instant::now());
                    if !left.is_zero() {
                        inner = self.wake.wait_timeout(inner, left).unwrap_or_else(|e| e.into_inner()).0;
                        continue;
                    }
                    inner.retry_at = None;
                    for it in inner.items.iter_mut().filter(|i| i.state == State::Offline) {
                        it.state = State::Waiting;
                    }
                }
                // those waiting for the library too, once it's to be tried again
                let now_retry = inner.retry_at.is_none();
                match inner.items.iter_mut().find(|i| i.state == State::Waiting || (now_retry && i.state == State::Offline)) {
                    Some(it) => {
                        it.state = State::Downloading;
                        it.error.clear();
                        break it.clone();
                    }
                    None => inner = self.wake.wait(inner).unwrap_or_else(|e| e.into_inner()),
                }
            };
            let (stop, got) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicU64::new(item.done)));
            inner.running = Some(Running { id: item.id, stop: stop.clone(), got: got.clone(), cancel: false, samples: VecDeque::new(), plan: item.plan.clone() });
            self.save(&inner);
            drop(inner);
            (self.changed)();
            let result = self.one(&item, &stop, &got);
            let mut inner = self.lock();
            let running = inner.running.take();
            let quitting = inner.quit;
            let mut plan_to_drop = None;
            match inner.items.iter_mut().find(|i| i.id == item.id) {
                // cancelled while it came
                None => plan_to_drop = running.filter(|r| r.cancel && result.is_err()).and_then(|r| r.plan),
                Some(it) => {
                    it.done = got.load(Ordering::SeqCst).min(it.size);
                    match result {
                        Ok(()) => {
                            it.state = State::Done;
                            it.done = it.size;
                            it.plan = None;
                            inner.added += 1;
                        }
                        Err(e) if e.is::<Stopped>() => {
                            if quitting && it.state == State::Downloading {
                                it.state = State::Waiting;
                            } else if it.state == State::Downloading {
                                it.state = State::Paused;
                            }
                        }
                        Err(e) if (e.is::<Damaged>() || e.downcast_ref::<Refused>().is_some_and(|r| matches!(r.status, 404 | 410 | 416))) && !it.replanned => {
                            // its files changed in the library while they came: a new plan
                            it.plan = None;
                            it.replanned = true;
                            it.state = State::Waiting;
                        }
                        Err(e) if is_offline(&e) => {
                            it.state = State::Offline;
                            it.error = "Waiting for the library".into();
                            inner.retry_at = Some(Instant::now() + Duration::from_millis(self.retry.load(Ordering::SeqCst)));
                        }
                        Err(e) => {
                            it.state = State::Failed;
                            it.error = match e.downcast_ref::<Refused>() {
                                Some(r) if matches!(r.status, 401 | 403) => "The library doesn't know this device any more: pair it again in Settings.".into(),
                                _ => format!("{e:#}"),
                            };
                        }
                    }
                }
            }
            self.save(&inner);
            drop(inner);
            if let Some(p) = plan_to_drop {
                if let Ok(c) = (self.ctx)() {
                    drop_parts(&c.device, &p);
                }
            }
            (self.changed)();
        }
    }

    /// One game: its plan (asked for once and kept), then its files and its entries.
    fn one(&self, item: &Item, stop: &AtomicBool, got: &AtomicU64) -> Result<()> {
        let c = (self.ctx)()?;
        let mut o = c.options.clone();
        o.media = item.media.clone();
        o.manuals = item.manuals;
        o.media_only = item.media_only;
        o.collection = item.collection.clone();
        o.adopt = if item.adopt.is_empty() { vec![] } else { vec![(item.key.clone(), item.adopt.clone())] };
        let keys = [item.key.clone()];
        let plan = match &item.plan {
            Some(p) => p.clone(),
            None => {
                let p: Plan = c.server.call("POST", "/v1/plans", Some(&o.ask(&keys)))?;
                let mut inner = self.lock();
                if let Some(it) = inner.items.iter_mut().find(|i| i.id == item.id) {
                    it.plan = Some(p.clone());
                }
                if let Some(r) = inner.running.as_mut() {
                    r.plan = Some(p.clone());
                }
                self.save(&inner);
                p
            }
        };
        // its size, and what's in of it already (files there, and parts kept)
        let (todo, _) = to_fetch(&c.device, &plan)?;
        let size: u64 = plan.files.iter().map(|f| f.size).sum();
        let there: u64 = plan.files.iter().enumerate().filter(|(i, _)| !todo.contains(i)).map(|(_, f)| f.size).sum();
        let parts: u64 = todo.iter().filter_map(|&i| part_of(&c.device, &plan.files[i]).ok()).filter_map(|p| std::fs::metadata(p).ok()).map(|m| m.len()).sum();
        got.store(there + parts, Ordering::SeqCst);
        {
            let mut inner = self.lock();
            if let Some(it) = inner.items.iter_mut().find(|i| i.id == item.id) {
                it.size = size;
            }
        }
        (self.changed)();
        let until = Some(self.pause_at.load(Ordering::SeqCst)).filter(|&u| u != u64::MAX);
        add_planned(&c.server, &c.device, &plan, &keys, &o, Watch { stop: Some(stop), got: Some(got), until }, &mut |_| {})?;
        Ok(())
    }
}
