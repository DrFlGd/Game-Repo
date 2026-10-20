//! Removing games and media safely (docs/PLAN.md, "Phase 9b design", D101 and D102): what
//! goes is worked out and shown first (each file, its media and manual, its gamelist entry,
//! the room it frees), then its files are set aside in `.gamerepo-removing/` in their own
//! folder (a rename on the same drive), `ES-DE/gamerepo/removing.json` is written, the
//! gamelists, collections and record are changed, and only then is what was set aside
//! deleted. A removal cut short is found again and finished or put back. A file another
//! game on the device needs stays, and nothing outside ES-DE's folders is touched.

use crate::scan::Found;
use crate::{write_whole, Device, WRITING};
use anyhow::{bail, Result};
use gamerepo_wire::api::Held;
use gamerepo_wire::{gamelist, merge_lines, remove_lines};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

/// Where a removal's files are set aside, in each of ES-DE's folders.
pub const ASIDE: &str = ".gamerepo-removing";

/// What's to be removed: a game ES-DE lists (its path in the ROMs folder, as the scan gives
/// it) or one the record lists (its key), whole or only some of its media.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Ask {
    pub id: String,
    pub key: String,
    /// Only these media kinds (cover, manual…); empty: the whole game
    pub kinds: Vec<String>,
}

/// A file a removal takes, or keeps.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Going {
    pub place: String,
    pub path: String,
    pub size: u64,
    /// The game's name
    pub game: String,
    /// rom (its own files), or the media kind
    pub kind: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Removal {
    /// Its folder in `.gamerepo-removing/`
    pub id: String,
    /// The games going whole, by name
    pub games: Vec<String>,
    /// The games losing only some media, by name
    pub media_of: Vec<String>,
    /// The record's games going whole, by key
    pub keys: Vec<String>,
    pub files: Vec<Going>,
    /// Staying: another game on the device needs them
    pub kept: Vec<Going>,
    /// The record's files that aren't on the device any more, only taken off the record
    pub gone: Vec<Going>,
    /// The gamelist entries going (the list's place and path, and the entry's path)
    pub entries: Vec<(String, String, String)>,
    /// The collection lines going (the file's place and path, and the line)
    pub lines: Vec<(String, String, String)>,
    pub frees: u64,
    /// For putting back: the entries as they were written, and the record's games as they were
    pub blocks: Vec<(String, String, String)>,
    pub held: Vec<Held>,
    /// How far it got: aside, lists, deleting
    pub step: String,
}

/// A removal cut short, and whether it can still be put back (not once deleting began).
#[derive(Clone, Debug, Serialize)]
pub struct Pending {
    pub removal: Removal,
    pub can_put_back: bool,
}

fn journal(d: &Device) -> PathBuf {
    d.places["esde"].join("gamerepo").join("removing.json")
}

fn save(d: &Device, r: &Removal) -> Result<()> {
    write_whole(&journal(d), &serde_json::to_vec_pretty(r)?)
}

/// Every path a file's path is in, and itself.
fn ancestors(path: &str) -> Vec<String> {
    let parts: Vec<&str> = path.split('/').collect();
    (1..=parts.len()).map(|n| parts[..n].join("/")).collect()
}

/// The paths in the ROMs folder ES-DE could start a held game from (its own files', not a
/// BIOS or device set it shares).
pub fn starts(h: &Held) -> HashSet<String> {
    h.files.iter().filter(|f| f.place == "roms" && f.needed_by.is_empty()).flat_map(|f| ancestors(&f.path)).collect()
}

fn media_kind(role: &str) -> bool {
    role == gamerepo_wire::MANUAL.0 || gamerepo_wire::MEDIA.iter().any(|(k, _, _)| *k == role)
}

/// A file of a place on the device, refused if it isn't inside that place once links are
/// followed, or is in what's set aside. ES-DE's own folders in a place (its `gamelists` and
/// `collections`, a system's folder) may be links to elsewhere, as RetroDECK and EmuDeck
/// make them, and what's in one is inside the place; a link deeper down isn't followed out.
pub fn inside(d: &Device, place: &str, path: &str) -> Result<PathBuf> {
    let p = d.at(place, path)?;
    let first = path.split('/').next().unwrap_or("");
    if first == ASIDE {
        bail!("{path} is set aside already.");
    }
    let mut roots = vec![std::fs::canonicalize(&d.places[place])?];
    let top = d.places[place].join(first);
    if path.contains('/') && std::fs::symlink_metadata(&top).is_ok_and(|m| m.file_type().is_symlink()) {
        roots.extend(std::fs::canonicalize(&top));
    }
    let mut parent = p.parent().unwrap_or(&p).to_path_buf();
    while !parent.exists() {
        match parent.parent() {
            Some(x) => parent = x.to_path_buf(),
            None => break,
        }
    }
    if !std::fs::canonicalize(&parent).is_ok_and(|c| roots.iter().any(|r| c.starts_with(r))) {
        bail!("{path} leads out of ES-DE's {place} folder, so it isn't removed.");
    }
    Ok(p)
}

/// Whether a file of a place is on the device.
fn there(d: &Device, place: &str, path: &str) -> bool {
    d.at(place, path).is_ok_and(|p| std::fs::symlink_metadata(p).is_ok())
}

/// Whether the record's game has none of its own files on the device any more (its ROMs or
/// PC games folder changed since it was added, say): it isn't held then, and can be added
/// again or taken off the record.
pub fn missing(d: &Device, h: &Held) -> bool {
    let own: Vec<_> = h.files.iter().filter(|f| f.needed_by.is_empty() && !media_kind(&f.role) && (f.place == "roms" || f.place == "pc")).collect();
    !own.is_empty() && own.iter().all(|f| !there(d, &f.place, &f.path))
}

/// What a removal takes (shown before it's asked to go ahead): `found` is what the scan
/// found, for the games' media by ES-DE's names and the games from elsewhere.
pub fn work_out(d: &Device, found: &[Found], asks: &[Ask]) -> Result<Removal> {
    let rec = d.record();
    let mut r = Removal::default();
    // the games that go whole, and whose files stay for those that don't
    let held_of = |a: &Ask| -> Option<&Held> {
        if !a.key.is_empty() {
            return rec.games.iter().find(|h| h.key == a.key);
        }
        rec.games.iter().find(|h| starts(h).contains(&a.id))
    };
    let found_of = |a: &Ask, h: Option<&Held>| -> Option<&Found> {
        if !a.id.is_empty() {
            return found.iter().find(|g| g.id == a.id);
        }
        let s = starts(h?);
        found.iter().find(|g| s.contains(&g.id))
    };
    let mut whole: HashSet<String> = HashSet::new();
    let mut whole_found: HashSet<String> = HashSet::new();
    for a in asks.iter().filter(|a| a.kinds.is_empty()) {
        let h = held_of(a);
        if h.is_none() && found_of(a, h).is_none() {
            bail!("{} isn't on this device.", if a.key.is_empty() { &a.id } else { &a.key });
        }
        if let Some(h) = h {
            whole.insert(h.key.clone());
        }
        if let Some(g) = found_of(a, h) {
            whole_found.insert(g.id.clone());
        }
    }
    // a file stays while a game that stays has it (its media of the kinds asked aside)
    let dropping = |h: &Held, role: &str| asks.iter().any(|a| !a.kinds.is_empty() && held_of(a).is_some_and(|x| x.key == h.key) && a.kinds.iter().any(|k| k == role));
    let needed: HashSet<(String, String)> = rec.games.iter().filter(|h| !whole.contains(&h.key)).flat_map(|h| h.files.iter().filter(move |f| !dropping(h, &f.role))).map(|f| (f.place.clone(), f.path.clone())).collect();
    // and media by ES-DE's names while another game that stays has the same name
    let mut media_users: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for g in found.iter().filter(|g| !whole_found.contains(&g.id)) {
        for m in &g.media {
            media_users.entry(&m.path).or_default().push(&g.id);
        }
    }
    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut held_changed: BTreeMap<String, Held> = BTreeMap::new();
    for a in asks {
        let h = held_of(a);
        let g = found_of(a, h);
        let name = h.map(|h| h.title.clone()).filter(|t| !t.is_empty()).or_else(|| g.map(|g| g.name.clone())).unwrap_or_else(|| a.id.clone());
        let me = g.map(|g| g.id.as_str()).unwrap_or("");
        let mut take = |place: &str, path: &str, size: u64, kind: &str, r: &mut Removal| {
            if !seen.insert((place.to_string(), path.to_string())) {
                return;
            }
            let going = Going { place: place.into(), path: path.into(), size, game: name.clone(), kind: kind.into() };
            if !there(d, place, path) {
                r.gone.push(going);
                return;
            }
            let shared = place == "media" && media_users.get(path).is_some_and(|ids| ids.iter().any(|id| *id != me));
            if needed.contains(&(place.to_string(), path.to_string())) || shared {
                r.kept.push(going);
            } else {
                r.frees += size;
                r.files.push(going);
            }
        };
        if a.kinds.is_empty() {
            r.games.push(name.clone());
            if let Some(h) = h {
                held_changed.insert(h.key.clone(), h.clone());
                r.keys.push(h.key.clone());
                for f in &h.files {
                    take(&f.place, &f.path, f.size, if media_kind(&f.role) { &f.role } else { "rom" }, &mut r);
                }
                for (place, path, entry) in &h.entries {
                    match entry.strip_prefix("line:") {
                        Some(line) => r.lines.push((place.clone(), path.clone(), line.to_string())),
                        None => r.entries.push((place.clone(), path.clone(), entry.clone())),
                    }
                }
            }
            if let Some(g) = g {
                for (path, size) in &g.files {
                    take("roms", path, *size, "rom", &mut r);
                }
                for m in &g.media {
                    take("media", &m.path, m.size, &m.kind, &mut r);
                }
                // its gamelist entry and custom collections' lines, as ES-DE names it
                let (system, rel) = g.id.split_once('/').unwrap_or(("", &g.id));
                let list = format!("gamelists/{system}/gamelist.xml");
                let text = std::fs::read_to_string(d.places["esde"].join(&list)).unwrap_or_default();
                if gamelist::blocks(&text).iter().any(|(p, _)| *p == gamelist::norm_path(rel)) {
                    let e = ("esde".to_string(), list, format!("./{rel}"));
                    if !r.entries.iter().any(|x| x.0 == e.0 && x.1 == e.1 && gamelist::norm_path(&x.2) == gamelist::norm_path(&e.2)) {
                        r.entries.push(e);
                    }
                }
                for c in std::fs::read_dir(d.places["esde"].join("collections")).into_iter().flatten().flatten() {
                    let file = c.file_name().to_string_lossy().into_owned();
                    if !file.ends_with(".cfg") {
                        continue;
                    }
                    for line in std::fs::read_to_string(c.path()).unwrap_or_default().lines() {
                        let l = line.trim().replace('\\', "/");
                        if l.split_once("%ROMPATH%/").is_some_and(|(_, p)| p == g.id) {
                            let e = ("esde".to_string(), format!("collections/{file}"), line.trim().to_string());
                            if !r.lines.contains(&e) {
                                r.lines.push(e);
                            }
                        }
                    }
                }
            }
        } else {
            r.media_of.push(name.clone());
            if let Some(h) = h {
                for f in h.files.iter().filter(|f| a.kinds.contains(&f.role)) {
                    take(&f.place, &f.path, f.size, &f.role, &mut r);
                }
                held_changed.entry(h.key.clone()).or_insert_with(|| h.clone());
            }
            if let Some(g) = g {
                for m in g.media.iter().filter(|m| a.kinds.contains(&m.kind)) {
                    take("media", &m.path, m.size, &m.kind, &mut r);
                }
            }
        }
    }
    r.held = held_changed.into_values().collect();
    Ok(r)
}

/// Where a file is set aside.
fn aside(d: &Device, r: &Removal, place: &str, path: &str) -> PathBuf {
    d.places[place].join(ASIDE).join(&r.id).join(path)
}

/// The folders a file was in, up to its place, removed while they're empty.
fn tidy(d: &Device, place: &str, p: &Path) {
    let top = &d.places[place];
    let mut at = p.parent().map(Path::to_path_buf);
    while let Some(dir) = at {
        if !dir.starts_with(top) || dir == *top || std::fs::remove_dir(&dir).is_err() {
            break;
        }
        at = dir.parent().map(Path::to_path_buf);
    }
}

/// Removes what [`work_out`] works out for `asks`, safely: with a removal already cut short,
/// it's refused until that one is finished or put back.
pub fn remove(d: &Device, found: &[Found], asks: &[Ask]) -> Result<Removal> {
    remove_to(d, found, asks, None)
}

/// A removal cut short after a step (aside, lists or deleting), as a flat battery or a
/// crash would leave it: for the tests of finishing and putting back.
#[doc(hidden)]
pub fn cut_short(d: &Device, found: &[Found], asks: &[Ask], after: &str) -> Result<Removal> {
    remove_to(d, found, asks, Some(after))
}

fn remove_to(d: &Device, found: &[Found], asks: &[Ask], stop: Option<&str>) -> Result<Removal> {
    let _writing = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    if journal(d).exists() {
        bail!("A removal was cut short: finish it or put it back first.");
    }
    let mut r = work_out(d, found, asks)?;
    for f in &r.files {
        inside(d, &f.place, &f.path)?;
    }
    for (place, path, _) in r.entries.iter().chain(&r.lines) {
        inside(d, place, path)?;
    }
    r.id = format!("{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|t| t.as_millis()).unwrap_or(0));
    // the entries as they're written, to put back
    let mut lists: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for (place, path, entry) in &r.entries {
        lists.entry((place.clone(), path.clone())).or_default().push(gamelist::norm_path(entry));
    }
    for ((place, path), paths) in lists {
        let text = std::fs::read_to_string(d.at(&place, &path)?).unwrap_or_default();
        for (p, block) in gamelist::blocks(&text) {
            if paths.contains(&p) {
                r.blocks.push((place.clone(), path.clone(), block));
            }
        }
    }
    r.step = "aside".into();
    save(d, &r)?;
    if let Err(e) = set_aside(d, &r) {
        put_back_files(d, &r);
        let _ = std::fs::remove_file(journal(d));
        return Err(e.context("The removal was put back"));
    }
    if stop == Some("aside") {
        return Ok(r);
    }
    finish_from(d, &mut r, stop)?;
    Ok(r)
}

/// Its files moved into `.gamerepo-removing/`.
fn set_aside(d: &Device, r: &Removal) -> Result<()> {
    for f in &r.files {
        let from = inside(d, &f.place, &f.path)?;
        if std::fs::symlink_metadata(&from).is_err() {
            continue;
        }
        let to = aside(d, r, &f.place, &f.path);
        std::fs::create_dir_all(to.parent().unwrap())?;
        std::fs::rename(&from, &to)?;
        tidy(d, &f.place, &from);
    }
    Ok(())
}

/// The lists and record changed, then what was set aside deleted.
fn finish_from(d: &Device, r: &mut Removal, stop: Option<&str>) -> Result<()> {
    if r.step == "aside" {
        r.step = "lists".into();
        save(d, r)?;
    }
    if r.step == "lists" {
        let mut by_list: BTreeMap<(String, String), (Vec<String>, Vec<String>)> = BTreeMap::new();
        for (place, path, entry) in &r.entries {
            by_list.entry((place.clone(), path.clone())).or_default().0.push(entry.clone());
        }
        for (place, path, line) in &r.lines {
            by_list.entry((place.clone(), path.clone())).or_default().1.push(line.clone());
        }
        for ((place, path), (entries, lines)) in by_list {
            let p = d.at(&place, &path)?;
            let Ok(old) = std::fs::read_to_string(&p) else { continue };
            let mut text = old.clone();
            if let Some(t) = gamelist::remove(&text, &entries) {
                text = t;
            }
            if let Some(t) = remove_lines(&text, &lines) {
                text = t;
            }
            if text != old {
                write_whole(&p, text.as_bytes())?;
            }
        }
        let going: HashSet<(String, String)> = r.files.iter().map(|f| (f.place.clone(), f.path.clone())).collect();
        let mut rec = d.record();
        rec.games.retain(|h| !r.keys.contains(&h.key));
        for h in rec.games.iter_mut() {
            h.files.retain(|f| !going.contains(&(f.place.clone(), f.path.clone())));
        }
        d.save_record(&rec)?;
        if stop == Some("lists") {
            return Ok(());
        }
        r.step = "deleting".into();
        save(d, r)?;
        if stop == Some("deleting") {
            return Ok(());
        }
    }
    for place in d.places.keys() {
        let dir = d.places[place].join(ASIDE);
        let _ = std::fs::remove_dir_all(dir.join(&r.id));
        let _ = std::fs::remove_dir(&dir);
    }
    std::fs::remove_file(journal(d))?;
    Ok(())
}

/// A removal cut short, found when the client starts.
pub fn pending(d: &Device) -> Option<Pending> {
    let r: Removal = serde_json::from_slice(&std::fs::read(journal(d)).ok()?).ok()?;
    let can_put_back = r.step != "deleting";
    Some(Pending { removal: r, can_put_back })
}

/// A removal cut short, finished.
pub fn finish(d: &Device) -> Result<Removal> {
    let _writing = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    let Some(p) = pending(d) else { bail!("No removal was cut short.") };
    let mut r = p.removal;
    if r.step == "aside" {
        set_aside(d, &r)?;
    }
    finish_from(d, &mut r, None)?;
    Ok(r)
}

/// What was set aside moved back (a file that's back at its place since stays, and the
/// one set aside goes).
fn put_back_files(d: &Device, r: &Removal) {
    for f in &r.files {
        let from = aside(d, r, &f.place, &f.path);
        if std::fs::symlink_metadata(&from).is_err() {
            continue;
        }
        let Ok(to) = d.at(&f.place, &f.path) else { continue };
        if to.exists() {
            let _ = std::fs::remove_file(&from);
            continue;
        }
        if let Some(parent) = to.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::rename(&from, &to);
    }
    for place in d.places.keys() {
        let dir = d.places[place].join(ASIDE);
        let _ = std::fs::remove_dir_all(dir.join(&r.id));
        let _ = std::fs::remove_dir(&dir);
    }
}

/// A removal cut short, put back as it was: its files, gamelist entries, collection lines
/// and the record's games.
pub fn put_back(d: &Device) -> Result<Removal> {
    let _writing = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    let Some(p) = pending(d) else { bail!("No removal was cut short.") };
    if !p.can_put_back {
        bail!("Its files were being deleted, so it can only be finished.");
    }
    let r = p.removal;
    put_back_files(d, &r);
    if r.step == "lists" {
        let mut by_list: BTreeMap<(String, String), (Vec<String>, Vec<String>)> = BTreeMap::new();
        for (place, path, block) in &r.blocks {
            by_list.entry((place.clone(), path.clone())).or_default().0.push(block.clone());
        }
        for (place, path, line) in &r.lines {
            by_list.entry((place.clone(), path.clone())).or_default().1.push(line.clone());
        }
        for ((place, path), (blocks, lines)) in by_list {
            let p = d.at(&place, &path)?;
            let old = std::fs::read_to_string(&p).ok();
            let mut text = old.clone();
            if let Some(t) = gamelist::restore(text.as_deref(), &blocks) {
                text = Some(t);
            }
            if let Some(t) = merge_lines(text.as_deref(), &lines) {
                text = Some(t);
            }
            if text != old {
                write_whole(&p, text.unwrap_or_default().as_bytes())?;
            }
        }
        let mut rec = d.record();
        for h in &r.held {
            match rec.games.iter_mut().find(|x| x.key == h.key) {
                Some(x) => {
                    for f in &h.files {
                        if !x.files.iter().any(|y| (&y.place, &y.path) == (&f.place, &f.path)) {
                            x.files.push(f.clone());
                        }
                    }
                }
                None => rec.games.push(h.clone()),
            }
        }
        d.save_record(&rec)?;
    }
    std::fs::remove_file(journal(d))?;
    Ok(r)
}
