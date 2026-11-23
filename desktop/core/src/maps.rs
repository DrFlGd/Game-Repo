//! Maps and Guides (docs/PLAN.md, D214–D221): the Atlas packs (`.atlaspack`, written by
//! Atlas Importer) kept in the library's `Maps and Guides/` folder, laid out as the Atlas
//! pack contract says:
//!
//! ```text
//! Maps and Guides/
//!   <Game title>/<Game title> - <Source>.atlaspack
//!   No game yet/<Title> - <Source>.atlaspack
//!   packs.json
//! ```
//!
//! `packs.json` is Game Repo's record, keyed by each pack's `packId`: its games (Library
//! keys), the picture chosen for it, the file it was last seen as, when it was added and
//! linked. Which game a pack belongs to is the record, never the folder; the folder follows
//! the record (D216). The index's `atlas_packs` keeps each file's description by its path,
//! size and time, so only a file that changed is read again (D217). Every `.atlaspack` in
//! the folder and the folders inside it is a pack whatever it's named (a `.part` is one
//! still being written, and hidden folders and `Guidebooks/` are passed over); a pack moved
//! by hand is found again by its `packId`, and of two files of one pack the later
//! `pack.updated` is the pack, the other *Another copy*.
//!
//! Linking a pack to a game (by hand, or a sure match found by its title, D218) moves its
//! file into the game's folder as an Organise change of kind `maps_link`, with `packs.json`
//! written in the same change, so Undo puts both back. Importing (D220) is a change of kind
//! `maps_import` into `No game yet/`, then matched; removing is one of kind `maps_remove`,
//! setting the file aside. A pack is never opened, changed or unpacked: its small pictures
//! are read one at a time into the computer's cache (`<data dir>/thumbs/<library id>/atlas/`)
//! under names of Game Repo's own. *Open in Atlas* starts Atlas with the pack's path (D221).
//!
//! Game Repo Server (part 2, D222) takes a game's packs from [`packs_of`] (or every linked
//! one from [`linked`]): each pack's path, size, `packId` and description.

use crate::atlaspack::{self, Description};
use crate::config::{read_json_object, sorted};
use crate::docs::{outside, text_op};
use crate::index::Index;
use crate::library::{now, Library};
use crate::organise::{self, safe_name, Op, Plan, Src};
use crate::variants::{split_name, words};
use anyhow::{anyhow, bail, Context, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

/// The section's folder in the library.
pub const FOLDER: &str = "Maps and Guides";
/// Where a pack with no game is (the Importer writes new packs here).
pub const NO_GAME: &str = "No game yet";
/// Game Repo's record of the packs, in the folder.
pub const RECORD: &str = "packs.json";
/// A title's guidebooks (`guidebooks.rs`, D232), passed over by the packs' walk.
pub const GUIDEBOOKS: &str = "Guidebooks";
/// The pictures chosen for packs (hidden, so passed over).
const PICTURES: &str = ".pictures";

pub fn folder(lib: &Library) -> PathBuf {
    lib.root().join(FOLDER)
}

// ------------------------------------------------------------ the record

/// A game a pack is linked to.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PackGame {
    /// Its key in the Library: `<system>/<DAT name>`, `pc/<id>`
    pub key: String,
    pub title: String,
    pub platform: String,
}

/// What `packs.json` keeps of a pack.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Kept {
    /// The games it's linked to; the first names its folder
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub games: Vec<PackGame>,
    /// A picture chosen for it, below `Maps and Guides/`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub picture: Option<String>,
    /// The file it was last seen as, below `Maps and Guides/`, with `/`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    pub added: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub linked: Option<String>,
    /// Linked, or taken off a game, by hand (or a link Undo took back): never matched
    /// again on its own
    #[serde(skip_serializing_if = "is_false")]
    pub by_hand: bool,
    /// Left in `No game yet` by choice: not on the To review list
    #[serde(skip_serializing_if = "is_false")]
    pub left: bool,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// `packs.json`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Record {
    pub packs: BTreeMap<String, Kept>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

pub fn read_record(lib: &Library) -> Record {
    serde_json::from_value(read_json_object(&folder(lib).join(RECORD))).unwrap_or_default()
}

fn record_text(r: &Record) -> Result<String> {
    Ok(serde_json::to_string_pretty(&sorted(&serde_json::to_value(r)?))? + "\n")
}

fn write_record(lib: &Library, r: &Record) -> Result<()> {
    lib.writable()?;
    crate::config::write_json(&folder(lib).join(RECORD), &serde_json::to_value(r)?)
}

// ------------------------------------------------------------ the folder, read

/// A pack file in the folder, with its description or why it can't be read.
#[derive(Clone, Debug)]
pub struct Found {
    /// Below `Maps and Guides/`, with `/`
    pub rel: String,
    pub size: u64,
    /// Milliseconds since 1970
    pub mtime: i64,
    pub description: Option<Description>,
    /// The sentence when it can't be read (D215): the crate's, shown as it is
    pub error: Option<String>,
    /// Its kind (`atlaspack::kind_of`): not-a-pack-file, newer-format, damaged, cannot-open
    pub error_kind: Option<String>,
}

impl Found {
    pub fn pack_id(&self) -> Option<&str> {
        self.description.as_ref().and_then(|d| d.pack_id.as_deref())
    }
}

fn mtime_of(m: &std::fs::Metadata) -> i64 {
    m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Every pack file below `dir`: (path below it, size, time). Hidden folders, `Guidebooks/`
/// and files still being written (`.part`, Atlas Importer's `.atlas-importer.part` among
/// them: never read, moved or removed) are passed over.
fn walk(dir: &Path) -> Vec<(String, u64, i64)> {
    let mut out = vec![];
    let mut stack = vec![(dir.to_path_buf(), String::new())];
    while let Some((d, rel)) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let Ok(t) = e.file_type() else { continue };
            let r = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
            if t.is_dir() {
                if !name.eq_ignore_ascii_case(GUIDEBOOKS) {
                    stack.push((e.path(), r));
                }
            } else if atlaspack::is_pack_name(&name) {
                if let Ok(m) = std::fs::metadata(e.path()) {
                    out.push((r, m.len(), mtime_of(&m)));
                }
            }
        }
    }
    out.sort();
    out
}

/// The index's note of the release of Atlas's crate its descriptions were read with.
const READER_KEY: &str = "atlas_packs_reader";

/// The pack files in the folder, each one's description from the index while its size
/// and time are the same, else read again (and the index kept in step); every one read
/// again once after the crate's release changes ([`atlaspack::READER`]).
pub fn read_folder(ix: &Index, lib: &Library) -> Result<Vec<Found>> {
    let dir = folder(lib);
    let files = walk(&dir);
    // read with another release of Atlas's crate before: every pack is read again once
    if ix.meta(READER_KEY)?.as_deref() != Some(atlaspack::READER) {
        ix.conn().execute("DELETE FROM atlas_packs", [])?;
        ix.set_meta(READER_KEY, atlaspack::READER)?;
    }
    let conn = ix.conn();
    type Row = (Option<String>, Option<String>, Option<String>);
    let mut kept: HashMap<String, (i64, i64, Row)> = HashMap::new();
    {
        let mut q = conn.prepare("SELECT path, size, mtime, description, error, error_kind FROM atlas_packs")?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            kept.insert(r.get(0)?, (r.get(1)?, r.get(2)?, (r.get(3)?, r.get(4)?, r.get(5)?)));
        }
    }
    let tx = conn.unchecked_transaction()?;
    let mut out = vec![];
    let here: HashSet<&str> = files.iter().map(|f| f.0.as_str()).collect();
    // a file gone from one place and one of its size and time come in another: moved (by a
    // change, or by hand), so its description is the one read before
    let mut moved: HashMap<(i64, i64), Row> = HashMap::new();
    for (gone, (s, m, row)) in kept.iter().filter(|(k, _)| !here.contains(k.as_str())) {
        tx.execute("DELETE FROM atlas_packs WHERE path = ?1", [gone])?;
        moved.insert((*s, *m), row.clone());
    }
    let parse = |d: &Option<String>| d.as_deref().and_then(|t| serde_json::from_str::<Value>(t).ok()).map(Description::from_value);
    for (rel, size, mtime) in &files {
        let (description, error, error_kind) = match kept.get(rel) {
            Some((s, m, row)) if *s == *size as i64 && *m == *mtime => (parse(&row.0), row.1.clone(), row.2.clone()),
            _ => {
                let was = if kept.contains_key(rel) { None } else { moved.remove(&(*size as i64, *mtime)) };
                let (d, e, k) = match was {
                    Some(row) => (parse(&row.0), row.1, row.2),
                    None => match atlaspack::read_header(&dir.join(rel)) {
                        Ok(d) => (Some(d), None, None),
                        Err(r) => (None, Some(r.to_string()), Some(atlaspack::kind_of(&r).to_string())),
                    },
                };
                // a file that couldn't be opened (held by another program a moment) is read
                // again next time, not remembered by its size and time
                if k.as_deref() == Some("cannot-open") {
                    tx.execute("DELETE FROM atlas_packs WHERE path = ?1", [rel])?;
                } else {
                    tx.execute(
                        "INSERT OR REPLACE INTO atlas_packs (path, size, mtime, pack_id, description, error, error_kind) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        params![rel, *size as i64, mtime, d.as_ref().and_then(|d| d.pack_id.clone()), d.as_ref().map(|d| d.raw.to_string()), e, k],
                    )?;
                }
                (d, e, k)
            }
        };
        // the crate refuses a description without a packId that can be one, so every pack
        // read has one; a file that has none is one that can't be read
        out.push(Found { rel: rel.clone(), size: *size, mtime: *mtime, description: description.filter(|d| d.pack_id.is_some()), error, error_kind });
    }
    tx.commit()?;
    Ok(out)
}

/// A pack: its file (the newer of two with its `packId`), any other copies, and its record.
#[derive(Clone, Debug)]
pub struct Pack {
    pub id: String,
    pub file: Found,
    pub copies: Vec<Found>,
    pub kept: Kept,
}

impl Pack {
    pub fn description(&self) -> &Description {
        self.file.description.as_ref().expect("a pack's file has its description")
    }
}

/// The folder as it is: its packs, the files that can't be read, and the record.
#[derive(Clone, Debug, Default)]
pub struct State {
    pub packs: Vec<Pack>,
    pub unreadable: Vec<Found>,
    pub record: Record,
}

impl State {
    pub fn pack(&self, id: &str) -> Option<&Pack> {
        self.packs.iter().find(|p| p.id == id)
    }
}

/// Read the folder (D217) and bring the record up to date: a new pack is added (no game
/// yet), one moved or renamed by hand is found again by its `packId`. A pack whose file is
/// gone keeps its record, so it's linked again when it's put back. Nothing is written in
/// a library that's read only.
pub fn sync(ix: &Index, lib: &Library) -> Result<State> {
    let found = read_folder(ix, lib)?;
    let mut record = read_record(lib);
    let before = record.clone();
    let mut by_id: BTreeMap<String, Vec<Found>> = BTreeMap::new();
    let mut unreadable = vec![];
    for f in found {
        match f.pack_id().map(String::from) {
            Some(id) => by_id.entry(id).or_default().push(f),
            None => unreadable.push(f),
        }
    }
    let mut packs = vec![];
    for (id, mut files) in by_id {
        let kept = record.packs.get(&id).cloned();
        let was = kept.as_ref().and_then(|k| k.file.clone());
        // the newer first: pack.updated, then exportedAt; then the file the record has
        files.sort_by(|a, b| {
            let (da, db) = (a.description.as_ref().unwrap(), b.description.as_ref().unwrap());
            db.newer(da).then_with(|| (Some(&b.rel) == was.as_ref()).cmp(&(Some(&a.rel) == was.as_ref()))).then_with(|| a.rel.cmp(&b.rel))
        });
        let file = files.remove(0);
        let mut kept = kept.unwrap_or_else(|| Kept { added: now(), ..Default::default() });
        if kept.file.as_deref() != Some(file.rel.as_str()) {
            kept.file = Some(file.rel.clone());
        }
        record.packs.insert(id.clone(), kept.clone());
        packs.push(Pack { id, file, copies: files, kept });
    }
    if record != before && lib.writable().is_ok() {
        write_record(lib, &record)?;
    }
    packs.sort_by(|a, b| {
        let t = |p: &Pack| p.description().game_title().unwrap_or("").to_lowercase();
        t(a).cmp(&t(b)).then_with(|| a.id.cmp(&b.id))
    });
    Ok(State { packs, unreadable, record })
}

// ------------------------------------------------------------ matching (D218)

/// A game a pack's title may be: the Library's games, and those of its systems' DATs.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Match {
    pub key: String,
    pub title: String,
    pub platform: String,
    pub platform_name: String,
    /// The library has it
    pub have: bool,
    /// Its title is the pack's, word for word (as D120 compares names)
    pub exact: bool,
    pub score: f64,
}

/// The games whose titles are like `title`, the closest first (`limit` of them): the
/// Library's titles found by their words, compared as videos' are (D145's titles, D120's
/// comparison).
pub fn matches(ix: &Index, title: &str, limit: usize) -> Result<Vec<Match>> {
    let want = words(split_name(title).0);
    if want.is_empty() {
        return Ok(vec![]);
    }
    // each word of the title (but the most common) in the game's title or DAT name
    let terms: Vec<String> = want
        .iter()
        .filter(|w| !crate::variants::COMMON.contains(&w.as_str()) || want.len() == 1)
        .map(|w| format!("{{title name}} : \"{}\"*", w.replace('"', "")))
        .collect();
    if terms.is_empty() {
        return Ok(vec![]);
    }
    let conn = ix.conn();
    let mut q = conn.prepare(
        "SELECT l.key, l.title, l.name, l.platform, l.have FROM library l
         WHERE l.addon = '' AND l.id IN (SELECT rowid FROM library_text WHERE library_text MATCH ?1)
         ORDER BY l.have DESC, CASE WHEN l.have THEN l.best_have ELSE l.best_all END DESC, l.sort LIMIT 2000",
    )?;
    let mut read = |m: String| -> Result<Vec<(String, String, String, String, bool)>> {
        Ok(q.query_map([m], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get::<_, i64>(4)? == 1)))?.collect::<rusqlite::Result<_>>().unwrap_or_default())
    };
    // every word first; else any of them (a pack's title with a word the game's hasn't, a
    // subtitle or an edition, still finds the game to offer)
    let mut rows = read(terms.join(" AND "))?;
    if rows.is_empty() && terms.len() > 1 {
        rows = read(terms.join(" OR "))?;
    }
    let mut out: Vec<Match> = vec![];
    // one game a system for each title: the best one (the one the Library shows first)
    let mut seen: HashSet<(String, Vec<String>)> = HashSet::new();
    for (key, t, name, platform, have) in rows {
        let shown = split_name(&t).0.to_string();
        let (tw, nw) = (words(&shown), words(split_name(&name).0));
        if !seen.insert((platform.clone(), tw.clone())) {
            continue;
        }
        let exact = tw == want || nw == want;
        let score = if exact { 1.0 } else { crate::variants::alike(&want, &tw).max(crate::variants::alike(&want, &nw)) };
        if score < 0.35 {
            continue;
        }
        out.push(Match { platform_name: crate::browse::system_name(&platform), key, title: shown, platform, have, exact, score });
    }
    out.sort_by(|a, b| b.exact.cmp(&a.exact).then(b.have.cmp(&a.have)).then(b.score.total_cmp(&a.score)).then_with(|| a.platform_name.cmp(&b.platform_name)));
    out.truncate(limit);
    Ok(out)
}

/// The games a sure match links a pack to (D218): the games you have whose title is the
/// pack's, one a system; `system` (`game.system`, as the site named it) only narrows them,
/// never proves one. None: not sure.
pub fn sure(found: &[Match], system: Option<&str>) -> Vec<Match> {
    let mut had: Vec<Match> = found.iter().filter(|m| m.exact && m.have).cloned().collect();
    if let Some(s) = system.map(words).filter(|w| !w.is_empty()) {
        let on: Vec<Match> = had.iter().filter(|m| words(&m.platform_name) == s || words(&m.platform) == s).cloned().collect();
        if !on.is_empty() {
            had = on;
        }
    }
    had
}

// ------------------------------------------------------------ changes: linking, importing, removing

pub(crate) fn folder_src(lib: &Library, rel: &str) -> Src {
    let base = folder(lib);
    let path = base.join(rel);
    Src {
        file_id: 0,
        root: format!("dir:{}", base.display()),
        rel: rel.to_string(),
        path: path.display().to_string(),
        member: None,
        size: std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0),
        crc32: String::new(),
        sha1: None,
    }
}

pub(crate) fn plan_of(lib: &Library, kind: &str, mode: &str, title: String, label: String, ops: Vec<Op>, n: usize, dat: Value) -> Plan {
    Plan {
        id: organise::new_id(),
        created: now(),
        kind: kind.into(),
        label,
        title,
        dat,
        platform: String::new(),
        mode: mode.into(),
        format: "keep".into(),
        out: Some(folder(lib).display().to_string()),
        space: organise::space_needed(&ops),
        ops,
        games: n,
        in_place: 0,
        conflicts: vec![],
        duplicates: vec![],
        left: vec![],
        new_root: None,
        new_roots: vec![],
        present: vec![],
        export: None,
        brought: vec![],
    }
}

/// A game in the Library, by its key, as a pack keeps it.
pub fn game_of(ix: &Index, key: &str) -> Result<PackGame> {
    let (platform, title): (String, String) = ix
        .conn()
        .query_row("SELECT platform, title FROM library WHERE key = ?1", [key], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?
        .ok_or_else(|| anyhow!("There's no game {key} in the library."))?;
    Ok(PackGame { key: key.into(), title: split_name(&title).0.to_string(), platform })
}

/// The folder (below `Maps and Guides/`) a pack linked to these games goes in: the first
/// game's title, its tags left out, made safe for Windows; with none, `No game yet`.
pub fn folder_for(games: &[PackGame]) -> String {
    match games.first() {
        Some(g) => {
            let t = safe_name(g.title.trim());
            if t.eq_ignore_ascii_case(NO_GAME) || t.eq_ignore_ascii_case(GUIDEBOOKS) || t.eq_ignore_ascii_case(RECORD) {
                format!("{t} (game)")
            } else {
                t
            }
        }
        None => NO_GAME.into(),
    }
}

/// A free name for `name` in `dir` (below the folder): itself, else "<stem> (2).atlaspack"…
pub(crate) fn free_name(base: &Path, dir: &str, name: &str, taken: &mut HashSet<String>, own: Option<&str>) -> String {
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) => (s.to_string(), format!(".{e}")),
        None => (name.to_string(), String::new()),
    };
    let mut n = safe_name(name);
    let mut i = 2;
    loop {
        let rel = format!("{dir}/{n}");
        let lower = rel.to_lowercase();
        let mine = own.is_some_and(|o| o.to_lowercase() == lower);
        if mine || (!taken.contains(&lower) && !base.join(&rel).exists()) {
            taken.insert(lower);
            return n;
        }
        n = safe_name(&format!("{stem} ({i}){ext}"));
        i += 1;
    }
}

/// A pack's games changed: the games it's to have.
#[derive(Clone, Debug)]
pub struct Change {
    pub pack: String,
    pub games: Vec<PackGame>,
}

/// Plan giving packs their games, each file moved to its first game's folder (or `No game
/// yet`) and `packs.json` written, as one change with Undo (D218). `auto`: sure matches
/// found by the app (they stay open to being matched by hand; by hand, never matched
/// again on their own). None: nothing would change.
pub fn link_plan(ix: &Index, lib: &Library, changes: &[Change], auto: bool) -> Result<Option<Plan>> {
    lib.writable()?;
    let state = sync(ix, lib)?;
    let base = folder(lib);
    // packs.json is written by the change: it's there first, so the change sets it aside
    if !base.join(RECORD).is_file() {
        write_record(lib, &state.record)?;
    }
    let mut record = state.record.clone();
    let mut taken: HashSet<String> = HashSet::new();
    let mut ops = vec![];
    let mut done: Vec<(String, String, Vec<PackGame>)> = vec![];
    for c in changes {
        let p = state.pack(&c.pack).ok_or_else(|| anyhow!("There's no pack {} in Maps and Guides.", c.pack))?;
        let k = record.packs.get_mut(&p.id).ok_or_else(|| anyhow!("There's no pack {} in packs.json.", c.pack))?;
        let mut games: Vec<PackGame> = vec![];
        for g in &c.games {
            if !games.iter().any(|x| x.key == g.key) {
                games.push(g.clone());
            }
        }
        if k.games == games && (auto || k.by_hand) {
            continue;
        }
        let dir = folder_for(&games);
        let from = p.file.rel.clone();
        let name = from.rsplit('/').next().unwrap_or(&from).to_string();
        let now_dir = from.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        let to = if now_dir.eq_ignore_ascii_case(&dir) {
            from.clone()
        } else {
            format!("{dir}/{}", free_name(&base, &dir, &name, &mut taken, None))
        };
        let title = p.description().game_title().unwrap_or(&p.id).to_string();
        if to != from {
            let s = folder_src(lib, &from);
            ops.push(Op { op: "move".into(), game: title.clone(), to: base.join(&to).display().to_string(), size: s.size, from: vec![s], names: vec![], aside: vec![], note: None });
        }
        k.linked = (!games.is_empty()).then(now);
        k.by_hand = k.by_hand || !auto;
        k.left = !auto && games.is_empty();
        k.games = games.clone();
        k.file = Some(to);
        done.push((p.id.clone(), title, games));
    }
    if done.is_empty() {
        return Ok(None);
    }
    ops.push(text_op("Maps and Guides", &base.join(RECORD), record_text(&record)?, "packs.json: the packs and their games"));
    let (title, label) = match done.as_slice() {
        [(_, t, g)] if g.is_empty() => (format!("Take {t} off its game"), format!("Took {t} off its game")),
        [(_, t, g)] if g.len() == 1 => (format!("Link {t} to {}", g[0].title), format!("Linked {t} to {}", g[0].title)),
        [(_, t, g)] => (format!("Link {t} to {} games", g.len()), format!("Linked {t} to {} games", g.len())),
        _ if done.iter().all(|d| d.2.is_empty()) => (format!("Take {} packs off their games", done.len()), format!("Took {} packs off their games", done.len())),
        _ => (format!("Link {} packs to their games", done.len()), format!("Linked {} packs to their games", done.len())),
    };
    let ids: Vec<&String> = done.iter().map(|d| &d.0).collect();
    Ok(Some(plan_of(lib, "maps_link", "move", title, label, ops, done.len(), json!({ "packs": ids, "auto": auto }))))
}

/// The sure matches among the packs with no game yet (D218), linked as one change: those
/// in `No game yet` that nothing was decided for by hand. None: there are none.
pub fn auto_link_plan(ix: &Index, lib: &Library) -> Result<Option<Plan>> {
    if lib.writable().is_err() {
        return Ok(None);
    }
    let state = sync(ix, lib)?;
    // a pack Atlas Importer is writing again beside it waits until it's in (the next read)
    let base = folder(lib);
    let writing = |rel: &str| base.join(format!("{rel}{}", atlaspack::IMPORTER_PART)).exists();
    let waiting: Vec<&Pack> = state.packs.iter().filter(|p| p.kept.games.is_empty() && !p.kept.by_hand && !p.kept.left && in_no_game(&p.file.rel) && !writing(&p.file.rel)).collect();
    if waiting.is_empty() {
        return Ok(None);
    }
    crate::meta::sync(ix, lib)?;
    crate::browse::refresh(ix, lib)?;
    let mut changes = vec![];
    for p in waiting {
        let d = p.description();
        // a pack of magazines is of no game (D245): never linked by its magazine's title
        if d.sort() == Some(atlaspack::Sort::Magazines) {
            continue;
        }
        let Some(t) = d.game_title() else { continue };
        let s = sure(&matches(ix, t, 50)?, d.game.system.as_deref());
        if !s.is_empty() {
            changes.push(Change { pack: p.id.clone(), games: s.into_iter().map(|m| PackGame { key: m.key, title: m.title, platform: m.platform }).collect() });
        }
    }
    if changes.is_empty() {
        return Ok(None);
    }
    link_plan(ix, lib, &changes, true)
}

fn in_no_game(rel: &str) -> bool {
    rel.split_once('/').is_some_and(|(d, rest)| d.eq_ignore_ascii_case(NO_GAME) && !rest.contains('/'))
}

/// Link a pack to a game by hand: `also` adds it beside the games it has (a second link),
/// else it's the pack's only game. The plan, to be run.
pub fn link(ix: &Index, lib: &Library, pack: &str, key: &str, also: bool) -> Result<Option<Plan>> {
    let g = game_of(ix, key)?;
    let r = read_record(lib);
    let mut games = if also { r.packs.get(pack).map(|k| k.games.clone()).unwrap_or_default() } else { vec![] };
    games.retain(|x| x.key != g.key);
    games.push(g);
    link_plan(ix, lib, &[Change { pack: pack.into(), games }], false)
}

/// Take a pack off a game (or, with no key, off every one): a pack with no game left goes
/// back to `No game yet`. The plan, to be run.
pub fn unlink(ix: &Index, lib: &Library, pack: &str, key: Option<&str>) -> Result<Option<Plan>> {
    let r = read_record(lib);
    let mut games = r.packs.get(pack).map(|k| k.games.clone()).ok_or_else(|| anyhow!("There's no pack {pack} in Maps and Guides."))?;
    match key {
        Some(k) => games.retain(|g| g.key != k),
        None => games.clear(),
    }
    link_plan(ix, lib, &[Change { pack: pack.into(), games }], false)
}

/// Leave a pack in `No game yet` (off the To review list), or put it back on the list.
pub fn leave(lib: &Library, pack: &str, left: bool) -> Result<Value> {
    lib.writable()?;
    let mut r = read_record(lib);
    let k = r.packs.get_mut(pack).ok_or_else(|| anyhow!("There's no pack {pack} in Maps and Guides."))?;
    k.left = left;
    k.by_hand = true;
    write_record(lib, &r)?;
    Ok(json!({ "ok": true }))
}

/// The picture a pack's card shows when it has none of its own, or always (D219): copied
/// into `Maps and Guides/.pictures/`, or none to take it off (its file goes too).
pub fn set_picture(lib: &Library, pack: &str, picture: Option<&Path>) -> Result<Value> {
    lib.writable()?;
    let mut r = read_record(lib);
    let k = r.packs.get_mut(pack).ok_or_else(|| anyhow!("There's no pack {pack} in Maps and Guides."))?;
    let base = folder(lib);
    let new = match picture {
        Some(p) => {
            let ext = crate::platforms::ext_of(&p.to_string_lossy());
            if !["jpg", "jpeg", "png", "webp", "gif", "bmp"].contains(&ext.as_str()) {
                bail!("A pack's picture is a picture (JPEG, PNG, WebP).");
            }
            let id = {
                use sha1::Digest;
                hex::encode(&sha1::Sha1::digest(pack.as_bytes())[..8])
            };
            let rel = format!("{PICTURES}/{id}.{ext}");
            std::fs::create_dir_all(base.join(PICTURES))?;
            std::fs::copy(p, base.join(&rel)).with_context(|| format!("couldn't copy {}", p.display()))?;
            Some(rel)
        }
        None => None,
    };
    if let Some(old) = k.picture.take().filter(|o| Some(o) != new.as_ref()) {
        if let Ok(rel) = crate::library::rel_inside(&old) {
            let _ = std::fs::remove_file(base.join(rel));
        }
    }
    k.picture = new;
    write_record(lib, &r)?;
    Ok(json!({ "ok": true }))
}

/// Remove pack files (packs, or other copies of one), by their paths below the folder:
/// set aside by a change, with Undo (D219). Their record stays, so a pack put back is
/// linked again.
pub fn remove_plan(lib: &Library, rels: &[String]) -> Result<Plan> {
    lib.writable()?;
    let base = folder(lib);
    let mut ops = vec![];
    for rel in rels {
        crate::library::rel_inside(rel)?;
        if !atlaspack::is_pack_name(rel) || !base.join(rel).is_file() {
            bail!("There's no pack {rel} in Maps and Guides.");
        }
        let s = folder_src(lib, rel);
        let name = rel.rsplit('/').next().unwrap_or(rel).to_string();
        ops.push(Op { op: "aside".into(), game: name, to: s.path.clone(), size: s.size, from: vec![s], names: vec![], aside: vec![], note: None });
    }
    if ops.is_empty() {
        bail!("Choose the packs to remove.");
    }
    let (title, label) = match ops.as_slice() {
        [o] => (format!("Remove {}", o.game), format!("Removed {}", o.game)),
        _ => (format!("Remove {} packs", ops.len()), format!("Removed {} packs", ops.len())),
    };
    let n = ops.len();
    Ok(plan_of(lib, "maps_remove", "aside", title, label, ops, n, Value::Null))
}

/// A pack file to import, with what its description says (D220).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Item {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub pack_id: Option<String>,
    pub title: Option<String>,
    /// The site it was read from (`atlaspack::Description::site`)
    pub source: Option<String>,
    /// For a map made in Atlas or by Atlas Map Maker, the app it was made in ("Atlas")
    pub made_in: Option<String>,
    /// Where it's from, as a card says it: the site's name, "Made in Atlas", "Came with an
    /// app" (`atlaspack::Description::from_words`)
    pub from: Option<String>,
    /// "With your own locations and notes", when it carries the maker's own things
    pub own_things: Option<String>,
    pub version: Option<u64>,
    /// The sentence when it can't be read (it's brought in all the same)
    pub error: Option<String>,
    /// Its kind (`atlaspack::kind_of`)
    pub error_kind: Option<String>,
    /// The name it's given in `No game yet`
    pub to_name: String,
    /// new | newer (than the one kept, which it replaces) | same | older
    pub state: String,
    /// The file it replaces, below the folder
    pub replaces: Option<String>,
    pub include: bool,
}

/// Whether a pack's file name has the contract's shape: "<title> - <source>.atlaspack".
fn named_as_contract(name: &str) -> bool {
    atlaspack::is_pack_name(name) && name[..name.len() - atlaspack::EXTENSION.len() - 1].split_once(" - ").is_some_and(|(a, b)| !a.trim().is_empty() && !b.trim().is_empty())
}

/// The pack files in the paths picked (files themselves, folders walked), each one's
/// description read, and what the library has of it (D220).
pub fn read_items(ix: &Index, lib: &Library, paths: &[PathBuf]) -> Result<Vec<Item>> {
    let state = sync(ix, lib)?;
    let base = folder(lib);
    let mut files: Vec<PathBuf> = vec![];
    for p in paths {
        if p.is_file() {
            files.push(p.clone());
            continue;
        }
        files.extend(walk(p).into_iter().map(|(rel, _, _)| p.join(rel)));
    }
    let mut out = vec![];
    for p in files {
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if !atlaspack::is_pack_name(&name) {
            continue;
        }
        // a pack already in the folder isn't brought in again
        if crate::organise::key(&p).starts_with(&crate::organise::key(&base)) {
            continue;
        }
        let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        let mut it = Item { path: p.display().to_string(), name: name.clone(), size, state: "new".into(), include: true, ..Default::default() };
        match atlaspack::read_header(&p) {
            Ok(d) => {
                it.pack_id = d.pack_id.clone();
                it.title = d.game_title().map(String::from);
                it.source = d.site().map(String::from);
                it.made_in = d.made_in().map(String::from);
                it.from = d.from_words();
                it.own_things = d.own_things();
                it.version = Some(d.version);
                it.to_name = if named_as_contract(&name) { name.clone() } else { d.file_name().map(|n| safe_name(&n)).unwrap_or_else(|| name.clone()) };
                if let Some(kept) = d.pack_id.as_deref().and_then(|id| state.pack(id)) {
                    match d.newer(kept.description()) {
                        std::cmp::Ordering::Greater => {
                            it.state = "newer".into();
                            it.replaces = Some(kept.file.rel.clone());
                        }
                        std::cmp::Ordering::Equal => {
                            it.state = "same".into();
                            it.include = false;
                        }
                        std::cmp::Ordering::Less => {
                            it.state = "older".into();
                            it.include = false;
                        }
                    }
                }
            }
            Err(e) => {
                it.error = Some(e.to_string());
                it.error_kind = Some(atlaspack::kind_of(&e).into());
                it.to_name = name.clone();
            }
        }
        out.push(it);
    }
    Ok(out)
}

/// Plan bringing the items in (D220): into `No game yet` under their names (copied,
/// hardlinked on the same drive unless `always_copy`, or moved), or a newer file of a pack
/// kept in place of the old one, which is set aside. Matched once in (`maps_sync`).
pub fn import_plan(lib: &Library, items: &[Item], mode: &str, always_copy: bool) -> Result<Plan> {
    lib.writable()?;
    let items: Vec<&Item> = items.iter().filter(|i| i.include).collect();
    if items.is_empty() {
        bail!("Choose at least one pack to import.");
    }
    let base = folder(lib);
    let mut taken: HashSet<String> = HashSet::new();
    let mut ops = vec![];
    let mut ids: HashSet<String> = HashSet::new();
    for it in &items {
        let src = PathBuf::from(&it.path);
        if !src.is_file() {
            bail!("There's no file {}.", it.path);
        }
        if let Some(id) = &it.pack_id {
            if !ids.insert(id.clone()) {
                bail!("{} and another file picked are the same pack: import one of them.", it.name);
            }
        }
        let title = it.title.clone().unwrap_or_else(|| it.name.clone());
        let to = match &it.replaces {
            Some(rel) => {
                crate::library::rel_inside(rel)?;
                let old = folder_src(lib, rel);
                ops.push(Op { op: "aside".into(), game: title.clone(), to: old.path.clone(), size: old.size, from: vec![old], names: vec![], aside: vec![], note: Some("the older file of this pack".into()) });
                taken.insert(rel.to_lowercase());
                base.join(rel)
            }
            None => {
                let mut name = if it.to_name.trim().is_empty() { it.name.clone() } else { it.to_name.clone() };
                if !atlaspack::is_pack_name(&name) {
                    name = format!("{name}.{}", atlaspack::EXTENSION);
                }
                base.join(NO_GAME).join(free_name(&base, NO_GAME, &name, &mut taken, None))
            }
        };
        let size = std::fs::metadata(&src)?.len();
        let op = match mode {
            "move" => "move",
            _ if !always_copy && organise::same_drive(&src, &base) => "link",
            _ => "copy",
        };
        let s: Src = Src { size, ..outside(&src) };
        ops.push(Op { op: op.into(), game: title, to: to.display().to_string(), from: vec![s], names: vec![], aside: vec![], size, note: None });
    }
    let n = items.len();
    let s = if n == 1 { "" } else { "s" };
    Ok(plan_of(lib, "maps_import", if mode == "move" { "move" } else { "copy" }, format!("Import {n} pack{s}"), format!("Imported {n} pack{s}"), ops, n, Value::Null))
}

/// After a Maps and Guides change runs or is undone (organise::run, undo): the title
/// folders its moves emptied are removed (never `No game yet`), a link Undo took back is
/// marked as decided by hand so it isn't matched again on its own, and the index is
/// brought up to date.
pub fn after(ix: &Index, lib: &Library, plan: &Plan, undone: bool) -> Result<()> {
    let base = folder(lib);
    let mut dirs: HashSet<PathBuf> = HashSet::new();
    for op in &plan.ops {
        for p in op.from.iter().map(|s| PathBuf::from(&s.path)).chain([PathBuf::from(&op.to)]) {
            if let Some(d) = p.parent() {
                dirs.insert(d.to_path_buf());
            }
        }
    }
    for d in dirs {
        let Ok(rel) = d.strip_prefix(&base) else { continue };
        let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
        if parts.len() != 1 || parts[0].eq_ignore_ascii_case(NO_GAME) || parts[0].starts_with('.') {
            continue;
        }
        if std::fs::read_dir(&d).is_ok_and(|mut rd| rd.next().is_none()) {
            let _ = std::fs::remove_dir(&d);
        }
    }
    if undone && plan.kind == "maps_link" && lib.writable().is_ok() {
        let mut r = read_record(lib);
        let mut changed = false;
        for id in plan.dat["packs"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            if let Some(k) = r.packs.get_mut(id).filter(|k| !k.by_hand) {
                k.by_hand = true;
                changed = true;
            }
        }
        if changed {
            write_record(lib, &r)?;
        }
    }
    read_folder(ix, lib)?;
    Ok(())
}

// ------------------------------------------------------------ pictures, Open in Atlas

/// The pack at a library path (it must be a pack in the folder).
fn pack_path(lib: &Library, rel: &str) -> Result<PathBuf> {
    let p = lib.resolve(rel)?;
    if !rel.starts_with(&format!("{FOLDER}/")) || !atlaspack::is_pack_name(rel) || !p.is_file() {
        bail!("There's no pack {rel} in Maps and Guides.");
    }
    Ok(p)
}

/// The file served for `~atlas/<n>/<packId>/<pack's library path>`: the pack's cover
/// (n = 0) or its n-th map's small picture, read out of the pack once and kept in the
/// computer's cache under a name of Game Repo's own (the pack's path, size and time
/// hashed), never in the library and never by the names inside the pack (D215). The pack
/// is found by its packId where packs.json has it now (a page drawn before it moved still
/// shows it), else at the path given.
pub fn preview_file(lib: &Library, data_dir: &Path, n: usize, id: &str, rel: &str) -> Result<PathBuf> {
    let base = folder(lib);
    let now = read_record(lib).packs.get(id).and_then(|k| k.file.clone()).filter(|f| crate::library::rel_inside(f).is_ok() && base.join(f).is_file());
    let rel = match now.and_then(|f| lib.relative(&base.join(f))) {
        Some(r) => r,
        None => rel.to_string(),
    };
    let rel = rel.as_str();
    let p = pack_path(lib, rel)?;
    let st = std::fs::metadata(&p)?;
    let stamp = format!("{rel}|{}|{}|{n}", st.len(), mtime_of(&st));
    let id = {
        use sha2::Digest;
        hex::encode(&sha2::Sha256::digest(stamp.as_bytes())[..12])
    };
    let cache = data_dir.join("thumbs").join(lib.id().unwrap_or_default()).join("atlas");
    let at = cache.join(format!("{id}.jpg"));
    if at.is_file() {
        return Ok(at);
    }
    let d = atlaspack::read_header(&p)?;
    let name = if n == 0 { d.cover.clone() } else { d.maps.get(n - 1).and_then(|m| m.preview.clone()) }.ok_or_else(|| anyhow!("That pack has no such picture."))?;
    let bytes = atlaspack::read_preview(&p, &name)?;
    std::fs::create_dir_all(&cache)?;
    let tmp = cache.join(format!("{id}.jpg.part"));
    std::fs::write(&tmp, &bytes)?;
    std::fs::rename(&tmp, &at)?;
    Ok(at)
}

/// *Open in Atlas* (D221): Atlas started with the pack's path as its one argument (Atlas
/// takes exactly that, block A2; a second start hands the path to the Atlas running), the
/// program set in Settings › Tools first (never one found on the PATH: another program may
/// be called atlas); else, on Windows, the program it opens `.atlaspack` files with (Atlas's
/// installer registers the type), which the page opens it with (`open_with`). Elsewhere no
/// program is registered for packs, so without Atlas set it says so.
pub fn open_in_atlas(lib: &Library, rel: &str) -> Result<Value> {
    let p = pack_path(lib, rel)?;
    if let Some(atlas) = crate::tools::find("atlas") {
        crate::tools::command(&atlas).arg(&p).spawn().with_context(|| format!("couldn't start {}", atlas.display()))?;
        return Ok(json!({ "opened": atlas.display().to_string() }));
    }
    if !cfg!(windows) {
        bail!("{}", NO_ATLAS_HERE);
    }
    if !registered() {
        bail!("Atlas isn't installed on this computer (nothing opens .atlaspack files): install it, or set where it is in Settings › Tools.");
    }
    Ok(json!({ "open_with": p.display().to_string() }))
}

/// Open in Atlas without Atlas set, where nothing is registered for packs (not Windows).
const NO_ATLAS_HERE: &str = "Set where Atlas is in Settings › Tools: on this system no program is registered for .atlaspack files, so Game Repo starts Atlas itself.";

/// Whether Windows has a program for `.atlaspack` files (Atlas's installer registers the type).
#[cfg(windows)]
fn registered() -> bool {
    crate::tools::command(Path::new("reg")).args(["query", r"HKCR\.atlaspack"]).output().is_ok_and(|o| o.status.success())
}

#[cfg(not(windows))]
fn registered() -> bool {
    false
}

// ------------------------------------------------------------ the page, a game's, and the server's

fn kinds_of(d: &Description) -> Vec<&'static str> {
    let mut k = vec![];
    if !d.maps.is_empty() || d.contents.get("maps").is_some_and(|n| *n > 0) {
        k.push("map");
    }
    if d.guides.iter().any(|g| g.kind.as_deref() != Some("wiki")) || (d.guides.is_empty() && d.contents.get("guides").is_some_and(|n| *n > 0)) {
        k.push("guide");
    }
    if d.guides.iter().any(|g| g.kind.as_deref() == Some("wiki")) {
        k.push("wiki");
    }
    k
}

fn iso_ms(ms: i64) -> String {
    crate::library::iso_from_unix(ms.div_euclid(1000))
}

fn file_json(lib: &Library, f: &Found) -> Value {
    let d = f.description.as_ref();
    json!({
        "path": lib.relative(&folder(lib).join(&f.rel)), "file": f.rel, "size": f.size, "modified": iso_ms(f.mtime),
        "updated": d.and_then(|d| d.updated.clone()), "exported_at": d.and_then(|d| d.exported_at.clone()), "error": f.error, "error_kind": f.error_kind,
    })
}

/// What a card shows of a pack: its description, its games (with their pictures), its
/// small pictures by number (`~atlas/<n>/<path>`), its other copies and the picture chosen.
fn card(lib: &Library, p: &Pack, pictures: &HashMap<String, String>, review: Option<Vec<Match>>) -> Value {
    let d = p.description();
    let mut v = atlaspack::card(d);
    let path = lib.relative(&folder(lib).join(&p.file.rel));
    let o = v.as_object_mut().expect("an object");
    o.insert("id".into(), json!(p.id));
    o.insert("path".into(), json!(path));
    o.insert("file".into(), json!(p.file.rel));
    o.insert("size".into(), json!(p.file.size));
    o.insert("modified".into(), json!(iso_ms(p.file.mtime)));
    o.insert("kinds".into(), json!(kinds_of(d)));
    o.insert("added".into(), json!(p.kept.added));
    o.insert("linked".into(), json!(p.kept.linked));
    o.insert("by_hand".into(), json!(p.kept.by_hand));
    o.insert("left".into(), json!(p.kept.left));
    o.insert("in_no_game".into(), json!(in_no_game(&p.file.rel)));
    // the cover is picture 0, each map's its place from 1
    let previews = d.previews();
    o.insert("cover_n".into(), if d.cover.as_ref().is_some_and(|c| previews.contains(c)) { json!(0) } else { Value::Null });
    let maps: Vec<Value> = d
        .maps
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let mut x = serde_json::to_value(m).unwrap_or_default();
            x["n"] = if m.preview.as_ref().is_some_and(|c| previews.contains(c)) { json!(i + 1) } else { Value::Null };
            x
        })
        .collect();
    o.insert("maps".into(), json!(maps));
    if let Some(pic) = p.kept.picture.as_deref().filter(|x| crate::library::rel_inside(x).is_ok()) {
        let at = folder(lib).join(pic);
        if at.is_file() {
            o.insert("picture_path".into(), json!(lib.relative(&at)));
        }
    }
    o.insert(
        "games".into(),
        json!(p
            .kept
            .games
            .iter()
            .map(|g| {
                let mut x = serde_json::to_value(g).unwrap_or_default();
                x["platform_name"] = json!(crate::browse::system_name(&g.platform));
                if let Some(pic) = pictures.get(&g.key) {
                    x["picture"] = json!(pic);
                }
                x
            })
            .collect::<Vec<_>>()),
    );
    o.insert("copies".into(), json!(p.copies.iter().map(|c| file_json(lib, c)).collect::<Vec<_>>()));
    if let Some(r) = review {
        o.insert("review".into(), json!(true));
        o.insert("suggestions".into(), json!(r));
    }
    v
}

/// The pictures of the games packs are linked to, by their keys.
fn pictures_of(ix: &Index, packs: &[Pack]) -> HashMap<String, String> {
    let keys: HashSet<&str> = packs.iter().flat_map(|p| p.kept.games.iter().map(|g| g.key.as_str())).collect();
    let mut out = HashMap::new();
    let Ok(mut q) = ix.conn().prepare("SELECT picture FROM library WHERE key = ?1 AND picture IS NOT NULL") else { return out };
    for k in keys {
        if let Ok(Some(p)) = q.query_row([k], |r| r.get::<_, String>(0)).optional() {
            out.insert(k.to_string(), p);
        }
    }
    out
}

/// The Maps and Guides page (D219): every pack's card, the files that can't be read (with
/// their sentences), and the To review list's packs with the closest games offered.
pub fn overview(ix: &Index, lib: &Library) -> Result<Value> {
    let base = folder(lib);
    if lib.writable().is_ok() && !base.join(NO_GAME).is_dir() {
        // the folder Atlas Importer is pointed at, there from the start
        std::fs::create_dir_all(base.join(NO_GAME))?;
    }
    let state = sync(ix, lib)?;
    let pictures = pictures_of(ix, &state.packs);
    let waiting = state.packs.iter().any(|p| p.kept.games.is_empty() && !p.kept.left);
    if waiting {
        crate::meta::sync(ix, lib)?;
        crate::browse::refresh(ix, lib)?;
    }
    let mut packs = vec![];
    for p in &state.packs {
        let review = if p.kept.games.is_empty() && !p.kept.left {
            Some(p.description().game_title().map(|t| matches(ix, t, 6)).transpose()?.unwrap_or_default())
        } else {
            None
        };
        packs.push(card(lib, p, &pictures, review));
    }
    Ok(json!({
        "packs": packs,
        "unreadable": state.unreadable.iter().map(|f| {
            let mut x = file_json(lib, f);
            x["id"] = json!(format!("file:{}", f.rel));
            x["name"] = json!(f.rel.rsplit('/').next().unwrap_or(&f.rel));
            x
        }).collect::<Vec<_>>(),
        "folder": lib.relative(&base),
        "newest": atlaspack::VERSION,
        "atlas": crate::tools::find("atlas").map(|p| p.display().to_string()),
    }))
}

/// A game's packs, for its page (D219): by its key.
pub fn of_game(ix: &Index, lib: &Library, key: &str) -> Vec<Value> {
    let r = read_record(lib);
    if !r.packs.values().any(|k| k.games.iter().any(|g| g.key == key)) {
        return vec![];
    }
    let Ok(state) = sync(ix, lib) else { return vec![] };
    state.packs.iter().filter(|p| p.kept.games.iter().any(|g| g.key == key)).map(|p| card(lib, p, &HashMap::new(), None)).collect()
}

/// A pack linked to a game, as Game Repo Server takes it (part 2, D222).
#[derive(Clone, Debug)]
pub struct Linked {
    pub pack_id: String,
    /// Its file on this computer
    pub path: PathBuf,
    pub size: u64,
    pub games: Vec<PackGame>,
    /// Its description (empty when it can't be read: see `error`)
    pub description: Description,
    /// Why Game Repo can't read it (a newer format, say): it's sent on as it is all the same
    /// (D215, D222), since the Atlas on the device may be newer
    pub error: Option<String>,
}

/// Every pack linked to a game, with its file, size and description (the folder read
/// again first; nothing is written in a library that's read only). A linked pack whose
/// file can't be read now (the Importer wrote a newer format in its place) is still one:
/// the record has its file, and it goes as it is.
pub fn linked(ix: &Index, lib: &Library) -> Result<Vec<Linked>> {
    let state = sync(ix, lib)?;
    let base = folder(lib);
    let mut out: Vec<Linked> = vec![];
    for p in state.packs.iter().filter(|p| !p.kept.games.is_empty()) {
        out.push(Linked { path: base.join(&p.file.rel), size: p.file.size, games: p.kept.games.clone(), description: p.file.description.clone().unwrap_or_default(), pack_id: p.id.clone(), error: None });
    }
    for (id, k) in state.record.packs.iter().filter(|(id, k)| !k.games.is_empty() && state.pack(id).is_none()) {
        let Some(f) = k.file.as_deref().and_then(|rel| state.unreadable.iter().find(|f| f.rel == rel)) else { continue };
        out.push(Linked { pack_id: id.clone(), path: base.join(&f.rel), size: f.size, games: k.games.clone(), description: Description::default(), error: f.error.clone() });
    }
    Ok(out)
}

/// The packs linked to one game, by its key.
pub fn packs_of(ix: &Index, lib: &Library, key: &str) -> Result<Vec<Linked>> {
    Ok(linked(ix, lib)?.into_iter().filter(|p| p.games.iter().any(|g| g.key == key)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atlaspack::tests::{format6, format7, guidebook_pack, jpeg, made_by_map_maker, made_in_atlas, mine_entries, write_pack, write_pack_with};
    use std::sync::atomic::AtomicBool;

    fn run(ix: &Index, lib: &Library, plan: &Plan) -> Value {
        let r = organise::run(ix, lib, plan, &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert!(r["failed"].as_array().unwrap().is_empty(), "{r}");
        after(ix, lib, plan, false).unwrap();
        r
    }

    fn undo(ix: &Index, lib: &Library, id: &str) {
        let r = organise::undo(ix, lib, id, &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert!(r["problems"].as_array().unwrap().is_empty(), "{r}");
        let plan = organise::load(lib, id).unwrap();
        after(ix, lib, &plan, true).unwrap();
    }

    #[test]
    fn packs_are_kept_found_again_and_linked_to_their_games() {
        // the collection fixture's Game Boy games: Alpha, Beta, Gamma, Delta and Epsilon had; Zeta not
        let (dir, lib, ix) = crate::collection::tests::fixture("maps");
        let base = folder(&lib);
        let no_game = base.join(NO_GAME);
        // the Importer's new packs: one for a game you have (format 7, its pictures), one
        // for a game you don't (format 6), one still being written, one too new, one not a pack
        let delta = no_game.join("Delta - Example Maps.atlaspack");
        write_pack(&delta, &format7("delta-example-maps", "Delta", "2026-11-17T08:30:00Z"), &[("preview/map-one.jpg", jpeg(3)), ("preview/map-two.jpg", jpeg(4))]);
        write_pack(&no_game.join("Zeta - Example Maps.atlaspack"), &format6("zeta-example-maps", "Zeta", "2026-11-10T08:30:00Z"), &[]);
        write_pack(&no_game.join("Beta - Example Maps.atlaspack.part"), &format6("beta-example-maps", "Beta", "2026-11-10T08:30:00Z"), &[]);
        let mut eleven = format7("future", "Epsilon", "2026-11-10T08:30:00Z");
        eleven["version"] = json!(11);
        write_pack(&no_game.join("Epsilon - Example Maps.atlaspack"), &eleven, &[]);
        std::fs::write(no_game.join("notes.atlaspack"), b"some notes").unwrap();
        // a packId that can't be one (upper case, a space): damaged, by the crate's check
        write_pack(&no_game.join("Theta - Example Maps.atlaspack"), &format6("Theta Maps", "Theta", "2026-11-10T08:30:00Z"), &[]);
        // a title's guidebooks are guidebooks.rs's (D232), not packs: even a pack file there is passed over
        write_pack(&base.join("Delta").join(GUIDEBOOKS).join("guide.atlaspack"), &format6("guidebook", "Delta", "2026-11-10T08:30:00Z"), &[]);

        let state = sync(&ix, &lib).unwrap();
        assert_eq!(state.packs.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["delta-example-maps", "zeta-example-maps"], "the .part and the guidebooks passed over");
        let said = |state: &State| {
            let mut v: Vec<(String, String)> = state.unreadable.iter().map(|f| (f.error_kind.clone().unwrap(), f.error.clone().unwrap())).collect();
            v.sort();
            v
        };
        // the crate's own sentences, as they are, with their kinds
        let sentences = vec![
            ("damaged".to_string(), "This pack file names a game id that cannot be used.".to_string()),
            ("newer-format".to_string(), "This pack file was made by Atlas Importer 0.22.0 in a newer format (version 11). A newer version of this app is needed to open it.".to_string()),
            ("not-a-pack-file".to_string(), "This file is not an Atlas pack file.".to_string()),
        ];
        assert_eq!(said(&state), sentences);
        // read back from the index the same
        assert_eq!(said(&sync(&ix, &lib).unwrap()), sentences);
        assert_eq!(overview(&ix, &lib).unwrap()["unreadable"].as_array().map(|u| u.iter().filter(|f| f["error_kind"] == "newer-format").count()), Some(1));
        let r = read_record(&lib);
        assert_eq!(r.packs["zeta-example-maps"].file.as_deref(), Some("No game yet/Zeta - Example Maps.atlaspack"));
        assert!(r.packs.values().all(|k| k.games.is_empty() && !k.added.is_empty()));
        // read again only when a file changes: the index has them
        let n: i64 = ix.conn().query_row("SELECT COUNT(*) FROM atlas_packs", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 5);

        // the sure match (Delta, which you have) is linked and its file moved into its game's folder, as one change
        let plan = auto_link_plan(&ix, &lib).unwrap().unwrap();
        assert_eq!((plan.kind.as_str(), plan.label.as_str()), ("maps_link", "Linked Delta to Delta"));
        run(&ix, &lib, &plan);
        let moved = base.join("Delta").join("Delta - Example Maps.atlaspack");
        assert!(moved.is_file() && !delta.exists());
        let k = read_record(&lib).packs["delta-example-maps"].clone();
        assert_eq!((k.games.iter().map(|g| g.key.as_str()).collect::<Vec<_>>(), k.file.as_deref(), k.by_hand), (vec!["gb/Delta (World)"], Some("Delta/Delta - Example Maps.atlaspack"), false));
        assert!(auto_link_plan(&ix, &lib).unwrap().is_none(), "Zeta isn't had: not sure, so it's for review");
        let page = overview(&ix, &lib).unwrap();
        let zeta = page["packs"].as_array().unwrap().iter().find(|p| p["id"] == "zeta-example-maps").unwrap().clone();
        assert_eq!(zeta["review"], true);
        assert_eq!(zeta["suggestions"][0]["key"], "gb/Zeta (Japan)");
        let card = page["packs"].as_array().unwrap().iter().find(|p| p["id"] == "delta-example-maps").unwrap().clone();
        assert_eq!((card["cover_n"].clone(), card["maps"][1]["n"].clone(), card["maps"][2]["n"].clone(), card["kinds"].clone()), (json!(0), json!(2), Value::Null, json!(["map", "wiki"])));
        assert_eq!(card["games"][0]["platform_name"], crate::browse::system_name("gb"));
        assert_eq!(page["unreadable"].as_array().unwrap().len(), 3);
        // a game's page lists its packs
        assert_eq!(of_game(&ix, &lib, "gb/Delta (World)").len(), 1);
        assert_eq!(packs_of(&ix, &lib, "gb/Delta (World)").unwrap()[0].path, moved);

        // a small picture read once into the cache, by a name of Game Repo's own
        let data = dir.join("data");
        let rel = lib.relative(&moved).unwrap();
        let cover = preview_file(&lib, &data, 0, "delta-example-maps", &rel).unwrap();
        assert_eq!(std::fs::read(&cover).unwrap(), jpeg(3));
        assert!(cover.starts_with(data.join("thumbs")) && cover.extension().unwrap() == "jpg");
        assert_eq!(std::fs::read(preview_file(&lib, &data, 2, "delta-example-maps", &rel).unwrap()).unwrap(), jpeg(4));
        assert!(preview_file(&lib, &data, 3, "delta-example-maps", &rel).is_err());
        assert!(preview_file(&lib, &data, 0, "no-such-pack", "Maps and Guides/packs.json").is_err());
        // by its packId where it is now, whatever path a page drawn earlier had; else by the path
        assert_eq!(std::fs::read(preview_file(&lib, &data, 2, "delta-example-maps", "Maps and Guides/No game yet/Delta - Example Maps.atlaspack").unwrap()).unwrap(), jpeg(4));
        assert_eq!(std::fs::read(preview_file(&lib, &data, 2, "no-such-pack", &rel).unwrap()).unwrap(), jpeg(4));

        // moved by hand: found again by its packId, its link held
        std::fs::create_dir_all(base.join("Elsewhere")).unwrap();
        std::fs::rename(&moved, base.join("Elsewhere/renamed.atlaspack")).unwrap();
        let state = sync(&ix, &lib).unwrap();
        let p = state.pack("delta-example-maps").unwrap();
        assert_eq!((p.file.rel.as_str(), p.kept.games.len()), ("Elsewhere/renamed.atlaspack", 1));
        assert_eq!(read_record(&lib).packs["delta-example-maps"].file.as_deref(), Some("Elsewhere/renamed.atlaspack"));
        std::fs::rename(base.join("Elsewhere/renamed.atlaspack"), &moved).unwrap();
        std::fs::remove_dir(base.join("Elsewhere")).unwrap();

        // two files of one pack: the later pack.updated is the pack, the other another copy, removed with Undo
        let older = no_game.join("Delta (old).atlaspack");
        write_pack(&older, &format7("delta-example-maps", "Delta", "2026-11-01T08:30:00Z"), &[]);
        let state = sync(&ix, &lib).unwrap();
        let p = state.pack("delta-example-maps").unwrap();
        assert_eq!((p.file.rel.as_str(), p.copies.iter().map(|c| c.rel.as_str()).collect::<Vec<_>>()), ("Delta/Delta - Example Maps.atlaspack", vec!["No game yet/Delta (old).atlaspack"]));
        let rm = remove_plan(&lib, &[p.copies[0].rel.clone()]).unwrap();
        assert_eq!((rm.kind.as_str(), rm.title.as_str()), ("maps_remove", "Remove Delta (old).atlaspack"));
        let r2 = run(&ix, &lib, &rm);
        assert!(!older.exists() && sync(&ix, &lib).unwrap().pack("delta-example-maps").unwrap().copies.is_empty());
        undo(&ix, &lib, r2["id"].as_str().unwrap());
        assert!(older.is_file());
        std::fs::remove_file(&older).unwrap();

        // linked to a second game, by hand: it stays in its first game's folder
        let plan = link(&ix, &lib, "delta-example-maps", "gb/Beta (Japan)", true).unwrap().unwrap();
        assert!(plan.ops.iter().all(|o| o.op == "text"), "{:?}", plan.ops);
        run(&ix, &lib, &plan);
        let k = read_record(&lib).packs["delta-example-maps"].clone();
        assert_eq!((k.games.len(), k.by_hand), (2, true));
        // linked to another game instead: the file moves to its folder and Delta's emptied folder goes
        let plan = link(&ix, &lib, "delta-example-maps", "gb/Gamma (USA) (Beta)", false).unwrap().unwrap();
        let r3 = run(&ix, &lib, &plan);
        assert!(base.join("Gamma/Delta - Example Maps.atlaspack").is_file());
        assert!(!base.join("Delta").join("Delta - Example Maps.atlaspack").exists());
        assert!(base.join("Delta").join(GUIDEBOOKS).is_dir(), "a folder with guidebooks in it stays");
        undo(&ix, &lib, r3["id"].as_str().unwrap());
        assert!(moved.is_file());
        assert_eq!(read_record(&lib).packs["delta-example-maps"].games.len(), 2, "Undo puts its games back");
        // taken off every game: back to No game yet, and not matched again on its own
        let plan = unlink(&ix, &lib, "delta-example-maps", None).unwrap().unwrap();
        assert_eq!(plan.label, "Took Delta off its game");
        run(&ix, &lib, &plan);
        assert!(no_game.join("Delta - Example Maps.atlaspack").is_file());
        let k = read_record(&lib).packs["delta-example-maps"].clone();
        assert!(k.games.is_empty() && k.by_hand && k.left);
        assert!(auto_link_plan(&ix, &lib).unwrap().is_none());

        // and Zeta, never sure, is still waiting for review in No game yet
        let state = sync(&ix, &lib).unwrap();
        let z = state.pack("zeta-example-maps").unwrap();
        assert!(z.kept.games.is_empty() && in_no_game(&z.file.rel));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// v0.25.0 (D231): a pack of guidebooks (format 8) is a pack like any other, never shown
    /// as one that can't be read, and its card says Guidebooks; an index whose packs were read
    /// with the crate before (format 8 kept as refused, `source.origin` left out) reads every
    /// pack again once. v0.26.0 (D241): so is a pack of format 9 (routes, the map's own
    /// icons), refused by the crate before; v0.29.0 (D245): and one of format 10, a pack of a
    /// magazine's issues, whose card says Magazines.
    #[test]
    fn a_pack_of_guidebooks_is_read_and_packs_are_read_again_with_a_newer_crate() {
        let (dir, lib, ix) = crate::collection::tests::fixture("maps-guidebooks");
        let base = folder(&lib);
        let books = base.join(NO_GAME).join("Hollow Marsh Guidebooks.atlaspack");
        guidebook_pack(&books, "guidebooks-hollow-marsh", "Hollow Marsh");
        let mut site = format7("delta-example-maps", "Delta", "2026-11-17T08:30:00Z");
        site["source"]["origin"] = json!("made-in-atlas");
        write_pack(&base.join(NO_GAME).join("Delta - Example Maps.atlaspack"), &site, &[]);
        // v0.26.0: a pack of format 9, with routes and the map's own icons
        let (routes, more) = crate::atlaspack::tests::format9("zeta-example-maps", "Zeta", "2026-11-20T08:30:00Z");
        crate::atlaspack::tests::write_pack_with(&base.join(NO_GAME).join("Zeta - Example Maps.atlaspack"), &routes, &[("preview/map-one.jpg", jpeg(3))], &more);
        // v0.29.0: a pack of format 10, a magazine's issues
        crate::atlaspack::tests::magazine_pack(&base.join(NO_GAME).join("Marsh Monthly.atlaspack"), "magazines-marsh-monthly", "Marsh Monthly");
        let state = sync(&ix, &lib).unwrap();
        assert!(state.unreadable.is_empty(), "{:?}", state.unreadable.iter().map(|f| f.error.clone()).collect::<Vec<_>>());
        let card_of = |id: &str| overview(&ix, &lib).unwrap()["packs"].as_array().unwrap().iter().find(|p| p["id"] == id).cloned().unwrap();
        let b = card_of("guidebooks-hollow-marsh");
        assert_eq!((b["sort"].clone(), b["from"].clone(), b["version"].clone()), (json!("guidebooks"), json!("Guidebooks"), json!(8)), "{b}");
        assert_eq!((card_of("delta-example-maps")["sort"].clone(), card_of("delta-example-maps")["from"].clone()), (json!("made-in-atlas"), json!("Made in Atlas")));
        // as the crate before left them in the index: the guidebooks refused as a newer
        // format, the other pack's description without its origin
        let conn = ix.conn();
        conn.execute("UPDATE atlas_packs SET description = NULL, error = 'This pack file was made by Example Shelf 1.0.0 in a newer format (version 8). A newer version of this app is needed to open it.', error_kind = 'newer-format' WHERE pack_id = 'guidebooks-hollow-marsh'", []).unwrap();
        let mut old = site.clone();
        old["source"].as_object_mut().unwrap().remove("origin");
        conn.execute("UPDATE atlas_packs SET description = ?1 WHERE pack_id = 'delta-example-maps'", [old.to_string()]).unwrap();
        conn.execute("UPDATE atlas_packs SET description = NULL, error = 'This pack file was made by Atlas Importer 0.22.0 in a newer format (version 9). A newer version of this app is needed to open it.', error_kind = 'newer-format' WHERE pack_id = 'zeta-example-maps'", []).unwrap();
        conn.execute("UPDATE atlas_packs SET description = NULL, error = 'This pack file was made by Example Shelf 1.0.0 in a newer format (version 10). A newer version of this app is needed to open it.', error_kind = 'newer-format' WHERE pack_id = 'magazines-marsh-monthly'", []).unwrap();
        // the same crate: kept as the index has them
        assert_eq!(sync(&ix, &lib).unwrap().unreadable.len(), 3);
        // another one: read again once, so the guidebooks are read and the origin is there
        ix.set_meta(READER_KEY, "atlas-pack v0.26.0").unwrap();
        let state = sync(&ix, &lib).unwrap();
        assert!(state.unreadable.is_empty());
        let m = card_of("magazines-marsh-monthly");
        assert_eq!((m["sort"].clone(), m["from"].clone(), m["version"].clone()), (json!("magazines"), json!("Magazines"), json!(10)), "{m}");
        assert_eq!(card_of("guidebooks-hollow-marsh")["sort"], "guidebooks");
        assert_eq!(card_of("delta-example-maps")["sort"], "made-in-atlas");
        let z = card_of("zeta-example-maps");
        assert_eq!((z["version"].clone(), z["contents"]["typePictures"].clone(), z["own_things"].clone(), z["personal_use"].clone()), (json!(9), json!(2), json!("With your own lists, locations and routes"), json!(true)), "{z}");
        assert_eq!(ix.meta(READER_KEY).unwrap().as_deref(), Some(atlaspack::READER));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_linked_pack_rewritten_in_a_newer_format_is_still_sent_as_it_is() {
        // D222: Game Repo Server sends a game's linked packs; one the Importer has since
        // written in a format this Game Repo can't read keeps its link and goes on as it is
        let (dir, lib, ix) = crate::collection::tests::fixture("maps-linked");
        let base = folder(&lib);
        write_pack(&base.join(NO_GAME).join("Delta - Example Maps.atlaspack"), &format7("delta-example-maps", "Delta", "2026-11-17T08:30:00Z"), &[]);
        write_pack(&base.join(NO_GAME).join("Zeta - Example Maps.atlaspack"), &format6("zeta-example-maps", "Zeta", "2026-11-10T08:30:00Z"), &[]);
        sync(&ix, &lib).unwrap();
        run(&ix, &lib, &auto_link_plan(&ix, &lib).unwrap().unwrap());
        let all = linked(&ix, &lib).unwrap();
        assert_eq!(all.iter().map(|p| (p.pack_id.as_str(), p.error.is_none())).collect::<Vec<_>>(), [("delta-example-maps", true)], "only the packs linked to a game");
        let file = base.join("Delta/Delta - Example Maps.atlaspack");
        let mut eleven = format7("delta-example-maps", "Delta", "2026-11-19T08:30:00Z");
        eleven["version"] = json!(11);
        write_pack(&file, &eleven, &[]);
        let p = packs_of(&ix, &lib, "gb/Delta (World)").unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!((p[0].pack_id.as_str(), &p[0].path, p[0].size), ("delta-example-maps", &file, std::fs::metadata(&file).unwrap().len()));
        assert!(p[0].error.as_deref().is_some_and(|e| e.contains("newer format (version 11)")), "{:?}", p[0].error);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_automatic_link_undone_goes_to_review_and_packs_are_imported() {
        let (dir, lib, ix) = crate::collection::tests::fixture("maps-import");
        let base = folder(&lib);
        let inbox = dir.join("downloads");
        // a pack named as the contract names one, another not, one that isn't a pack
        write_pack(&inbox.join("Delta - Example Maps.atlaspack"), &format7("delta-example-maps", "Delta", "2026-11-17T08:30:00Z"), &[]);
        write_pack(&inbox.join("export-1234.atlaspack"), &format6("beta-other-site", "Beta", "2026-11-17T08:30:00Z"), &[]);
        std::fs::write(inbox.join("broken.atlaspack"), b"not a zip").unwrap();
        std::fs::write(inbox.join("readme.txt"), b"hello").unwrap();
        let items = read_items(&ix, &lib, &[inbox.clone()]).unwrap();
        let by = |n: &str| items.iter().find(|i| i.name == n).cloned().unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(by("export-1234.atlaspack").to_name, "Beta - Example Maps.atlaspack", "named as the contract says");
        assert_eq!(by("Delta - Example Maps.atlaspack").to_name, "Delta - Example Maps.atlaspack");
        assert_eq!((by("broken.atlaspack").error.as_deref(), by("broken.atlaspack").include), (Some("This file is not an Atlas pack file."), true));
        let plan = import_plan(&lib, &items, "copy", true).unwrap();
        assert_eq!((plan.kind.as_str(), plan.label.as_str()), ("maps_import", "Imported 3 packs"));
        run(&ix, &lib, &plan);
        assert!(base.join(NO_GAME).join("Beta - Example Maps.atlaspack").is_file() && base.join(NO_GAME).join("broken.atlaspack").is_file());
        assert!(inbox.join("export-1234.atlaspack").is_file(), "copied, not moved");
        // matched once in: Delta and Beta are sure
        let plan = auto_link_plan(&ix, &lib).unwrap().unwrap();
        assert_eq!(plan.label, "Linked 2 packs to their games");
        let r = run(&ix, &lib, &plan);
        assert!(base.join("Delta/Delta - Example Maps.atlaspack").is_file() && base.join("Beta/Beta - Example Maps.atlaspack").is_file());
        undo(&ix, &lib, r["id"].as_str().unwrap());
        assert!(base.join(NO_GAME).join("Delta - Example Maps.atlaspack").is_file() && !base.join("Delta").exists());
        let rec = read_record(&lib);
        assert!(rec.packs["delta-example-maps"].games.is_empty() && rec.packs["delta-example-maps"].by_hand && !rec.packs["delta-example-maps"].left);
        assert!(auto_link_plan(&ix, &lib).unwrap().is_none(), "not linked again on its own");
        let page = overview(&ix, &lib).unwrap();
        assert!(page["packs"].as_array().unwrap().iter().all(|p| p["review"] == true), "both on To review");
        leave(&lib, "beta-other-site", true).unwrap();
        let page = overview(&ix, &lib).unwrap();
        assert_eq!(page["packs"].as_array().unwrap().iter().filter(|p| p["review"] == true).count(), 1);

        // the same pack again isn't brought in; a newer one replaces it in place, the old one set aside
        let again = read_items(&ix, &lib, &[inbox.join("Delta - Example Maps.atlaspack")]).unwrap();
        assert_eq!((again[0].state.as_str(), again[0].include), ("same", false));
        let newer = dir.join("newer");
        write_pack(&newer.join("Delta - Example Maps.atlaspack"), &format7("delta-example-maps", "Delta", "2026-11-18T08:30:00Z"), &[]);
        let items = read_items(&ix, &lib, &[newer.clone()]).unwrap();
        assert_eq!((items[0].state.as_str(), items[0].replaces.as_deref()), ("newer", Some("No game yet/Delta - Example Maps.atlaspack")));
        let plan = import_plan(&lib, &items, "copy", true).unwrap();
        assert_eq!(plan.ops.iter().map(|o| o.op.as_str()).collect::<Vec<_>>(), ["aside", "copy"]);
        let r = run(&ix, &lib, &plan);
        let kept = sync(&ix, &lib).unwrap();
        assert_eq!(kept.pack("delta-example-maps").unwrap().description().updated.as_deref(), Some("2026-11-18T08:30:00Z"));
        undo(&ix, &lib, r["id"].as_str().unwrap());
        let kept = sync(&ix, &lib).unwrap();
        assert_eq!(kept.pack("delta-example-maps").unwrap().description().updated.as_deref(), Some("2026-11-17T08:30:00Z"), "Undo puts the old one back");
        // an older file isn't brought in
        let older = dir.join("older");
        write_pack(&older.join("Delta - Example Maps.atlaspack"), &format7("delta-example-maps", "Delta", "2026-10-01T08:30:00Z"), &[]);
        assert_eq!(read_items(&ix, &lib, &[older]).unwrap()[0].state, "older");
        // a picture chosen for a pack
        let pic = dir.join("pic.png");
        ::image::RgbImage::from_pixel(4, 4, ::image::Rgb([1, 2, 3])).save(&pic).unwrap();
        set_picture(&lib, "beta-other-site", Some(&pic)).unwrap();
        let page = overview(&ix, &lib).unwrap();
        let beta = page["packs"].as_array().unwrap().iter().find(|p| p["id"] == "beta-other-site").unwrap().clone();
        assert!(beta["picture_path"].as_str().unwrap().starts_with("Maps and Guides/.pictures/"), "{beta}");
        assert!(set_picture(&lib, "beta-other-site", Some(&inbox.join("readme.txt"))).is_err());
        set_picture(&lib, "beta-other-site", None).unwrap();
        assert!(read_record(&lib).packs["beta-other-site"].picture.is_none());
        // packs.json keeps keys it doesn't know
        let mut raw = read_json_object(&base.join(RECORD));
        raw["fromLater"] = json!(1);
        raw["packs"]["beta-other-site"]["note"] = json!("mine");
        crate::config::write_json(&base.join(RECORD), &raw).unwrap();
        leave(&lib, "beta-other-site", false).unwrap();
        let raw = read_json_object(&base.join(RECORD));
        assert_eq!((raw["fromLater"].clone(), raw["packs"]["beta-other-site"]["note"].clone()), (json!(1), json!("mine")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Block A6: maps made in Atlas or by Atlas Map Maker are packs from no site, named by
    /// their game's title alone (never "Delta - .atlaspack"), matched and linked by it, shown
    /// as made in the app, and the maker's own things under `mine/` go through import and
    /// link byte for byte: Game Repo never reads or strips them.
    #[test]
    fn a_map_made_in_atlas_comes_from_no_site_and_goes_whole() {
        let (dir, lib, ix) = crate::collection::tests::fixture("maps-made");
        let base = folder(&lib);
        let inbox = dir.join("made");
        write_pack_with(&inbox.join("export-77.atlaspack"), &made_in_atlas("delta-my-map", "Delta", "2026-11-18T08:30:00Z"), &[], &mine_entries());
        write_pack(&inbox.join("Beta.atlaspack"), &made_by_map_maker("beta-map-maker", "Beta", "2026-11-18T08:30:00Z"), &[]);
        let original = std::fs::read(inbox.join("export-77.atlaspack")).unwrap();
        let items = read_items(&ix, &lib, &[inbox.clone()]).unwrap();
        let by = |n: &str| items.iter().find(|i| i.name == n).cloned().unwrap();
        let made = by("export-77.atlaspack");
        assert_eq!((made.to_name.as_str(), made.source.as_deref(), made.made_in.as_deref()), ("Delta.atlaspack", None, Some("Atlas")), "named by its game alone");
        assert_eq!((made.own_things.as_deref(), made.from.as_deref()), (Some("With your own locations, types and notes"), Some("Made in Atlas")));
        let mm = by("Beta.atlaspack");
        assert_eq!(mm.from.as_deref(), Some("Made in Atlas Map Maker"));
        assert_eq!((mm.to_name.as_str(), mm.made_in.as_deref(), mm.own_things.as_deref()), ("Beta.atlaspack", Some("Atlas Map Maker"), None));
        // copied (never hardlinked here), then matched by the game's title and linked
        let plan = import_plan(&lib, &items, "copy", true).unwrap();
        run(&ix, &lib, &plan);
        assert_eq!(std::fs::read(base.join(NO_GAME).join("Delta.atlaspack")).unwrap(), original, "imported whole, mine/ and all");
        let plan = auto_link_plan(&ix, &lib).unwrap().unwrap();
        assert_eq!(plan.label, "Linked 2 packs to their games");
        run(&ix, &lib, &plan);
        assert_eq!(std::fs::read(base.join("Delta").join("Delta.atlaspack")).unwrap(), original, "linked whole, mine/ and all");
        assert!(base.join("Beta").join("Beta.atlaspack").is_file());
        // the page: no site, made in the app, and what of the maker's own it carries
        let page = overview(&ix, &lib).unwrap();
        let card = |id: &str| page["packs"].as_array().unwrap().iter().find(|p| p["id"] == id).cloned().unwrap();
        let delta = card("delta-my-map");
        assert_eq!((delta["source_name"].clone(), delta["made_in"].clone(), delta["own_things"].clone()), (Value::Null, json!("Atlas"), json!("With your own locations, types and notes")), "{delta}");
        assert_eq!((delta["file"].as_str(), delta["games"][0]["title"].as_str()), (Some("Delta/Delta.atlaspack"), Some("Delta")), "{delta}");
        assert_eq!((delta["sort"].as_str(), delta["from"].as_str()), (Some("made-in-atlas"), Some("Made in Atlas")), "{delta}");
        assert_eq!((card("beta-map-maker")["made_in"].as_str(), card("beta-map-maker")["from"].as_str()), (Some("Atlas Map Maker"), Some("Made in Atlas Map Maker")));
        // Game Repo Server takes it as it is, for a device
        let linked = linked(&ix, &lib).unwrap();
        let l = linked.iter().find(|l| l.pack_id == "delta-my-map").unwrap();
        assert_eq!((l.description.made_in(), std::fs::read(&l.path).unwrap() == original), (Some("Atlas"), true));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The pack files below the folder outside any folder whose name begins with a dot (where
    /// Atlas Importer looks: it skips those, D216), and those inside one, `.part` files too.
    fn where_packs_are(base: &Path) -> (Vec<String>, Vec<String>) {
        let (mut plain, mut dotted) = (vec![], vec![]);
        let mut stack = vec![base.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                    continue;
                }
                let rel = p.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/");
                let lower = rel.to_lowercase();
                if lower.ends_with(".atlaspack") || lower.ends_with(".atlaspack.part") {
                    if rel.split('/').any(|c| c.starts_with('.')) { dotted.push(rel) } else { plain.push(rel) }
                }
            }
        }
        plain.sort();
        dotted.sort();
        (plain, dotted)
    }

    #[test]
    fn a_pack_set_aside_is_never_where_atlas_importer_looks() {
        // the contract (as built in block A3): the Importer reads every pack file in the folder
        // but those in a folder whose name begins with a dot, and knows a pack by its packId,
        // so a copy Game Repo sets aside must be in such a folder or it may be updated in
        // place of the pack. Remove…, Another copy, an import replacing an older file and a
        // link's Undo all leave only the packs themselves in plain folders.
        let (dir, lib, ix) = crate::collection::tests::fixture("maps-aside");
        let base = folder(&lib);
        let no_game = base.join(NO_GAME);
        write_pack(&no_game.join("Delta - Example Maps.atlaspack"), &format7("delta-example-maps", "Delta", "2026-11-17T08:30:00Z"), &[]);
        write_pack(&no_game.join("Zeta - Example Maps.atlaspack"), &format6("zeta-example-maps", "Zeta", "2026-11-10T08:30:00Z"), &[]);
        sync(&ix, &lib).unwrap();
        let r1 = run(&ix, &lib, &auto_link_plan(&ix, &lib).unwrap().unwrap());
        assert_eq!(where_packs_are(&base), (vec!["Delta/Delta - Example Maps.atlaspack".to_string(), "No game yet/Zeta - Example Maps.atlaspack".to_string()], vec![]));
        // another copy of Delta, removed: set aside below the folder, in a dot folder
        write_pack(&no_game.join("Delta (old).atlaspack"), &format7("delta-example-maps", "Delta", "2026-11-01T08:30:00Z"), &[]);
        let copy = sync(&ix, &lib).unwrap().pack("delta-example-maps").unwrap().copies[0].rel.clone();
        let r2 = run(&ix, &lib, &remove_plan(&lib, &[copy]).unwrap());
        // Zeta itself removed
        let r3 = run(&ix, &lib, &remove_plan(&lib, &["No game yet/Zeta - Example Maps.atlaspack".to_string()]).unwrap());
        // a newer file of Delta imported: the kept one set aside
        let newer = dir.join("newer");
        write_pack(&newer.join("Delta - Example Maps.atlaspack"), &format7("delta-example-maps", "Delta", "2026-11-18T08:30:00Z"), &[]);
        let items = read_items(&ix, &lib, &[newer.clone()]).unwrap();
        let r4 = run(&ix, &lib, &import_plan(&lib, &items, "copy", true).unwrap());
        let (plain, dotted) = where_packs_are(&base);
        assert_eq!(plain, ["Delta/Delta - Example Maps.atlaspack"], "only the pack itself where the Importer looks");
        assert_eq!(dotted.len(), 3, "{dotted:?}");
        assert!(dotted.iter().all(|d| d.starts_with(&format!("{}/", organise::SET_ASIDE))), "{dotted:?}");
        // and Game Repo sees what the Importer sees: one file of each pack, no copies
        let state = sync(&ix, &lib).unwrap();
        assert_eq!((state.packs.len(), state.packs[0].copies.len(), state.unreadable.len()), (1, 0, 0));
        // Undo of each puts the files back where they were, and nothing is left in a plain folder but packs
        undo(&ix, &lib, r4["id"].as_str().unwrap());
        undo(&ix, &lib, r3["id"].as_str().unwrap());
        undo(&ix, &lib, r2["id"].as_str().unwrap());
        undo(&ix, &lib, r1["id"].as_str().unwrap());
        let (plain, _) = where_packs_are(&base);
        assert_eq!(plain, ["No game yet/Delta (old).atlaspack", "No game yet/Delta - Example Maps.atlaspack", "No game yet/Zeta - Example Maps.atlaspack"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_cover_is_the_first_map_with_a_picture_and_pages_of_a_type_not_known_are_a_guide() {
        // as built in block A3: `cover` is the first map that has a small picture, not always
        // the first map; `guides` holds one entry, whose type a reader may not know
        let (dir, lib, ix) = crate::collection::tests::fixture("maps-cover");
        let base = folder(&lib);
        let mut v = format7("delta-example-maps", "Delta", "2026-11-17T08:30:00Z");
        v["maps"][0].as_object_mut().unwrap().remove("preview");
        v["cover"] = json!("preview/map-two.jpg");
        v["guides"] = json!([{ "title": "Delta Atlas", "type": "atlas-of-the-future", "pages": 12 }]);
        write_pack(&base.join(NO_GAME).join("Delta - Example Maps.atlaspack"), &v, &[("preview/map-two.jpg", jpeg(4))]);
        let page = overview(&ix, &lib).unwrap();
        let card = page["packs"][0].clone();
        assert_eq!((card["cover_n"].clone(), card["maps"][0]["n"].clone(), card["maps"][1]["n"].clone()), (json!(0), Value::Null, json!(2)), "{card}");
        assert_eq!((card["kinds"].clone(), card["guides"][0]["kind"].clone(), card["guides"][0]["pages"].clone()), (json!(["map", "guide"]), json!("other"), json!(12)));
        let data = dir.join("data");
        let rel = lib.relative(&base.join(NO_GAME).join("Delta - Example Maps.atlaspack")).unwrap();
        assert_eq!(std::fs::read(preview_file(&lib, &data, 0, "delta-example-maps", &rel).unwrap()).unwrap(), jpeg(4), "the cover is the second map's picture");
        assert_eq!(std::fs::read(preview_file(&lib, &data, 2, "delta-example-maps", &rel).unwrap()).unwrap(), jpeg(4));
        assert!(preview_file(&lib, &data, 1, "delta-example-maps", &rel).is_err(), "the first map has none");
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn atlas_importers_unfinished_files_are_never_shown_read_moved_or_removed() {
        // the contract: while Atlas Importer writes a pack, its unfinished file lies beside
        // the target as `<name>.atlas-importer.part`; Game Repo never shows, reads, moves or
        // deletes one, and a title folder holding one is never removed
        let (dir, lib, ix) = crate::collection::tests::fixture("maps-importer");
        let base = folder(&lib);
        let no_game = base.join(NO_GAME);
        write_pack(&no_game.join("Delta - Example Maps.atlaspack"), &format7("delta-example-maps", "Delta", "2026-11-17T08:30:00Z"), &[]);
        sync(&ix, &lib).unwrap();
        run(&ix, &lib, &auto_link_plan(&ix, &lib).unwrap().unwrap());
        // the Importer writes Delta again where Game Repo put it, Gamma again beside the one
        // it wrote before, and Alpha for the first time
        let delta_part = base.join("Delta").join(format!("Delta - Example Maps.atlaspack{}", atlaspack::IMPORTER_PART));
        let gamma = no_game.join("Gamma - Example Maps.atlaspack");
        let gamma_part = no_game.join(format!("Gamma - Example Maps.atlaspack{}", atlaspack::IMPORTER_PART));
        let alpha_part = no_game.join(format!("Alpha - Example Maps.atlaspack{}", atlaspack::IMPORTER_PART));
        write_pack(&gamma, &format6("gamma-example-maps", "Gamma", "2026-11-10T08:30:00Z"), &[]);
        write_pack(&delta_part, &format7("delta-example-maps", "Delta", "2026-11-19T08:30:00Z"), &[]);
        write_pack(&gamma_part, &format6("gamma-example-maps", "Gamma", "2026-11-19T08:30:00Z"), &[]);
        // as written later (the same size: only its time tells it from the file it replaces)
        std::fs::File::options().write(true).open(&gamma_part).unwrap().set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5)).unwrap();
        write_pack(&alpha_part, &format6("alpha-example-maps", "Alpha", "2026-11-19T08:30:00Z"), &[]);
        let parts = [delta_part.clone(), gamma_part.clone(), alpha_part.clone()];
        let stamp = |p: &Path| (std::fs::read(p).unwrap(), std::fs::metadata(p).unwrap().modified().unwrap());
        let before: Vec<_> = parts.iter().map(|p| stamp(p)).collect();
        let untouched = || assert_eq!(parts.iter().map(|p| stamp(p)).collect::<Vec<_>>(), before, "the Importer's files are as it left them");

        // not shown or read: neither on the page nor in the index
        let state = sync(&ix, &lib).unwrap();
        assert_eq!(state.packs.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["delta-example-maps", "gamma-example-maps"]);
        assert!(state.packs.iter().all(|p| p.copies.is_empty()) && state.unreadable.is_empty());
        let n: i64 = ix.conn().query_row("SELECT COUNT(*) FROM atlas_packs WHERE path LIKE '%.atlas-importer.part'", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0);
        assert!(!overview(&ix, &lib).unwrap().to_string().contains("atlas-importer"));
        // Gamma, a sure match, waits while the Importer writes it again beside it
        assert!(auto_link_plan(&ix, &lib).unwrap().is_none());
        // Delta taken off its game: the pack goes back to No game yet, its title's folder stays,
        // as it holds the Importer's file; Undo puts the pack back beside it
        let r = run(&ix, &lib, &unlink(&ix, &lib, "delta-example-maps", None).unwrap().unwrap());
        assert!(no_game.join("Delta - Example Maps.atlaspack").is_file() && base.join("Delta").is_dir());
        untouched();
        undo(&ix, &lib, r["id"].as_str().unwrap());
        assert!(base.join("Delta").join("Delta - Example Maps.atlaspack").is_file());
        // Remove… never takes one
        assert!(remove_plan(&lib, &[format!("Delta/Delta - Example Maps.atlaspack{}", atlaspack::IMPORTER_PART)]).is_err());
        // an import passes one over, picked or in a folder picked
        let inbox = dir.join("inbox");
        write_pack(&inbox.join(format!("Beta - Example Maps.atlaspack{}", atlaspack::IMPORTER_PART)), &format6("beta-example-maps", "Beta", "2026-11-19T08:30:00Z"), &[]);
        assert!(read_items(&ix, &lib, &[inbox.clone()]).unwrap().is_empty());
        assert!(read_items(&ix, &lib, &[inbox.join(format!("Beta - Example Maps.atlaspack{}", atlaspack::IMPORTER_PART))]).unwrap().is_empty());
        untouched();
        // the Importer renames Gamma's into place: read, and now linked
        std::fs::rename(&gamma_part, &gamma).unwrap();
        let state = sync(&ix, &lib).unwrap();
        assert_eq!(state.pack("gamma-example-maps").unwrap().description().updated.as_deref(), Some("2026-11-19T08:30:00Z"));
        run(&ix, &lib, &auto_link_plan(&ix, &lib).unwrap().unwrap());
        assert!(base.join("Gamma").join("Gamma - Example Maps.atlaspack").is_file());
        assert_eq!((stamp(&delta_part), stamp(&alpha_part)), (before[0].clone(), before[2].clone()));
        // Open in Atlas off Windows with no Atlas set: nothing is registered for packs there
        // (only Atlas's installer for Windows registers them), so it says to set Atlas
        let rel = lib.relative(&base.join("Gamma").join("Gamma - Example Maps.atlaspack")).unwrap();
        if !cfg!(windows) && crate::tools::find("atlas").is_none() {
            assert_eq!(open_in_atlas(&lib, &rel).unwrap_err().to_string(), NO_ATLAS_HERE);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
