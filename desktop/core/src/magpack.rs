//! A magazine's issues for Atlas (docs/PLAN.md, "Maps and Guides notes, part 12", D245–D249):
//! the Atlas pack contract's revision 49, Atlas 0.29.0's format 10. Each magazine the library
//! keeps (`Magazines/<Title>/`, [`crate::magazines`]) goes to Atlas in a pack of its own, one
//! file per magazine title, written by Atlas's own crate (`write_magazine_pack`):
//!
//! - its id is `magazines-<title id>` ([`pack_id_for`]): the magazine's folder's name made safe
//!   for the contract's id rule, with 8 hex digits of its SHA-1, so it's the same every time
//!   (a title changed in `magazine.json` keeps its folder, so its id) and a newer file replaces
//!   the older one;
//! - its issues are the library's, oldest first (Atlas shows them newest first), each with its
//!   date (`YYYY-MM` or `YYYY-MM-DD`, else none), number (when it's a whole number), title
//!   (*Issue 42*, else its file's name), its pages' text when the magazines' reader has read it
//!   ([`crate::magtext`]), a cover (the picture chosen for it, else its first page, made a JPEG
//!   of at most 512 KB with 512 pixels on its longest side) and the games ticked in it
//!   (`Issue.games`, D145), their pages as runs, each with its system's name and, when the game
//!   has a pack Game Repo sends Atlas, that pack's id as its Atlas game id ([`GameIds`]);
//! - a CBR (or a `.cbz` that isn't a ZIP) is made a CBZ on the way, as a guidebook is;
//! - an issue the contract's bounds can't take is left out, with why.
//!
//! Game Repo Server makes the pack when a device asks for it, into a cache of its own (the
//! guidebooks' [`Cache`], in `<data>/magazines/<packId>/`, never the library), keeps a ledger
//! so it's never made twice for the same magazine, and dates each one after the last.

use crate::guidebooks::{self, Cache, Made};
use crate::index::Index;
use crate::library::Library;
use crate::magazines::{self, Issue, Magazine};
use crate::maps;
use anyhow::{anyhow, bail, Result};
use atlas_pack::{GuidebookText, IssueGame, MagazineFacts, MagazinePack, NewIssue, COVER_SIDE, MAX_COVER_BYTES, MAX_GAME_PAGE_RUNS, MAX_GUIDEBOOK_BYTES, MAX_GUIDEBOOK_PAGES, MAX_GUIDEBOOK_TEXT_BYTES, MAX_ISSUES, MAX_ISSUE_GAMES, MAX_MAGAZINE_NAME, MAX_MAGAZINE_TITLE};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// The start of every magazine pack's id (the contract: `magazines-<title id>`).
pub const PACK_PREFIX: &str = atlas_pack::MAGAZINE_PACK_PREFIX;
/// Where Game Repo Server makes them, in its data folder.
pub const DIR: &str = "magazines";
/// The version of how Game Repo makes the pack: a change to it makes every pack again.
const MAKER: u32 = 1;

/// A magazine's pack id, from its folder's name in `Magazines/` (compared without case, as a
/// folder's name is on Windows): `magazines-nintendo-power-1a2b3c4d`. Always 1 to 64
/// characters that Atlas's `is_pack_id` takes.
pub fn pack_id_for(folder: &str) -> String {
    guidebooks::slug_id(PACK_PREFIX, &folder.to_lowercase())
}

/// Whether a pack id is one of Game Repo's magazine packs.
pub fn is_pack_id(id: &str) -> bool {
    id.starts_with(PACK_PREFIX) && atlas_pack::is_pack_id(id)
}

fn cut(s: &str, max: usize) -> String {
    s.trim().chars().take(max).collect::<String>().trim().to_string()
}

// ------------------------------------------------------------ the games' Atlas ids

/// The Atlas game id of each Library game that has a pack Game Repo sends Atlas: its first
/// linked pack of maps and guides (`packs.json`, by packId), else its pack of guidebooks
/// (D236). Atlas joins an issue to a game by this id first (a pack's id is its game's id in
/// Atlas), else by title and system, so a game with no pack is joined by title.
#[derive(Clone, Debug, Default)]
pub struct GameIds(HashMap<String, String>);

impl GameIds {
    pub fn of(ix: &Index, lib: &Library) -> Result<GameIds> {
        let mut m: HashMap<String, String> = HashMap::new();
        for (id, k) in &maps::read_record(lib).packs {
            if atlas_pack::is_pack_id(id) {
                for g in &k.games {
                    m.entry(g.key.clone()).or_insert_with(|| id.clone());
                }
            }
        }
        if !guidebooks::list(lib).is_empty() {
            for f in guidebooks::folders(ix, lib)? {
                if let Some(id) = f.pack_id() {
                    for g in &f.games {
                        m.entry(g.key.clone()).or_insert_with(|| id.clone());
                    }
                }
            }
        }
        Ok(GameIds(m))
    }

    pub fn get(&self, key: &str) -> Option<&String> {
        self.0.get(key)
    }
}

// ------------------------------------------------------------ what a pack is to hold

/// A game an issue covers, as the pack says it.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct GameIn {
    pub title: String,
    pub system: Option<String>,
    pub game_id: Option<String>,
    /// Its pages (from 1), made runs when the issue's pages are counted
    pub pages: Vec<u32>,
}

/// An issue to put in a magazine's pack: what it's made from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IssueIn {
    /// Its file's name in the magazine's folder
    pub file: String,
    pub path: PathBuf,
    /// pdf | cbz | cbr
    pub kind: String,
    /// As Atlas shows it: "Issue 42", else its file's name
    pub title: String,
    pub date: Option<String>,
    pub number: Option<u32>,
    pub size: u64,
    pub mtime: i64,
    /// Its kept text's file's size and time, when there is one
    pub text: Option<(u64, i64)>,
    /// A picture chosen for its cover, with its size and time; none: its first page
    pub cover: Option<(PathBuf, u64, i64)>,
    pub games: Vec<GameIn>,
}

/// What a magazine's pack is to hold: its id, the magazine, its issues (oldest first), and
/// those left out by the contract's bounds with why, each (title, why).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Wanted {
    pub pack_id: String,
    pub title: String,
    pub publisher: Option<String>,
    /// The magazine's folder's name in `Magazines/`
    pub folder: String,
    pub items: Vec<IssueIn>,
    pub left: Vec<(String, String)>,
}

impl Wanted {
    /// Its recipe: the pack id, the magazine and each issue's file, size, time, title, date,
    /// number, text's and cover's size and time, and games, hashed. The same recipe is the
    /// same pack.
    pub fn recipe(&self) -> String {
        use sha2::Digest;
        let items: Vec<Value> = self
            .items
            .iter()
            .map(|i| json!([i.file, i.size, i.mtime, i.title, i.date, i.number, i.text, i.cover.as_ref().map(|c| (c.1, c.2)), i.games]))
            .collect();
        let r = serde_json::to_string(&(MAKER, &self.pack_id, &self.title, &self.publisher, items)).unwrap_or_default();
        hex::encode(sha2::Sha256::digest(r.as_bytes()))[..32].to_string()
    }

    /// The pack's size, reckoned before it's made: its issues' and a little more for each
    /// one's cover and text.
    pub fn estimate(&self) -> u64 {
        self.items.iter().map(|i| i.size + 64 * 1024).sum::<u64>() + 16 * 1024
    }

    /// The name of its file in Atlas's packs folder: `<Title> - Magazines.atlaspack`.
    pub fn file_name(&self) -> String {
        crate::organise::safe_name(&format!("{} - Magazines.{}", self.title, crate::atlaspack::EXTENSION))
    }
}

/// How an issue is shown in Atlas: *Issue 42* when it has a number, else its file's name.
pub fn issue_title(i: &Issue) -> String {
    let t = match i.number.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        // a number of digits only as the number it is: "042" is Issue 42
        Some(n) => format!("Issue {}", n.parse::<u64>().map_or_else(|_| n.to_string(), |x| x.to_string())),
        None => Path::new(&i.file).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
    };
    let t = cut(&t, MAX_MAGAZINE_TITLE);
    if t.is_empty() {
        "Issue".into()
    } else {
        t
    }
}

fn stamp(p: &Path) -> Option<(u64, i64)> {
    let m = std::fs::metadata(p).ok()?;
    let t = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as i64;
    Some((m.len(), t))
}

/// What a magazine's pack holds: every issue the contract's bounds take, without reading
/// their files (their pages are counted as the pack is made, [`make`]). None: the folder has
/// no name.
pub fn wanted(dir: &Path, m: &Magazine, ids: &GameIds) -> Option<Wanted> {
    let folder = dir.file_name()?.to_string_lossy().into_owned();
    let title = Some(cut(&m.title, MAX_MAGAZINE_TITLE)).filter(|t| !t.is_empty()).unwrap_or_else(|| cut(&folder, MAX_MAGAZINE_TITLE));
    let mut w = Wanted {
        pack_id: pack_id_for(&folder),
        title,
        publisher: m.publisher.as_deref().map(|p| cut(p, MAX_MAGAZINE_NAME)).filter(|p| !p.is_empty()),
        folder,
        ..Default::default()
    };
    let mut issues: Vec<&Issue> = m.issues.iter().collect();
    issues.sort_by_key(|i| magazines::issue_order(i));
    for i in issues {
        let title = issue_title(i);
        let Some(path) = magazines::inside(dir, &i.file) else {
            w.left.push((title, "Its file isn't in the magazine's folder.".into()));
            continue;
        };
        let Some((size, mtime)) = stamp(&path).filter(|_| path.is_file()) else {
            w.left.push((title, "Its file isn't there.".into()));
            continue;
        };
        let kind = crate::platforms::ext_of(&i.file);
        if !magazines::EXTS.contains(&kind.as_str()) {
            w.left.push((title, "It isn't a PDF, CBZ or CBR file.".into()));
            continue;
        }
        if size == 0 {
            w.left.push((title, "It's empty, so it can't go to Atlas.".into()));
            continue;
        }
        if size > MAX_GUIDEBOOK_BYTES {
            w.left.push((title, "It's larger than 16 GB, the most an issue for Atlas can be.".into()));
            continue;
        }
        let games = i
            .games
            .iter()
            .filter(|g| g.ticked)
            .filter_map(|g| {
                let t = cut(&g.title, MAX_MAGAZINE_TITLE);
                let t = if t.is_empty() { cut(g.key.split_once('/').map_or(g.key.as_str(), |(_, n)| n), MAX_MAGAZINE_TITLE) } else { t };
                (!t.is_empty()).then(|| GameIn {
                    title: t,
                    system: Some(&g.platform).filter(|p| !p.is_empty()).map(|p| cut(&crate::browse::system_name(p), MAX_MAGAZINE_NAME)).filter(|s| !s.is_empty()),
                    game_id: ids.get(&g.key).cloned(),
                    pages: g.pages.clone(),
                })
            })
            .take(MAX_ISSUE_GAMES)
            .collect();
        let cover = i.cover.as_deref().and_then(|c| magazines::inside(dir, c)).and_then(|p| stamp(&p).filter(|_| p.is_file()).map(|(s, t)| (p, s, t)));
        w.items.push(IssueIn {
            file: i.file.clone(),
            text: crate::magtext::text_path(&path).and_then(|t| stamp(&t)),
            path,
            kind,
            title,
            date: i.date.as_deref().map(str::trim).filter(|d| atlas_pack::is_issue_date(d)).map(String::from),
            number: i.number.as_deref().map(str::trim).filter(|n| !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit())).and_then(|n| n.parse().ok()),
            size,
            mtime,
            cover,
            games,
        });
    }
    // at most 1,000 issues: the newest
    if w.items.len() > MAX_ISSUES {
        let over = w.items.len() - MAX_ISSUES;
        for i in w.items.drain(..over) {
            w.left.push((i.title, "A magazine's pack for Atlas holds at most 1,000 issues: the newest 1,000 go.".into()));
        }
    }
    Some(w)
}

/// Every magazine's pack, as it's wanted now (those with no issue that can go too).
pub fn all(ix: &Index, lib: &Library) -> Result<Vec<Wanted>> {
    let list = magazines::list(lib);
    if list.is_empty() {
        return Ok(vec![]);
    }
    let ids = GameIds::of(ix, lib)?;
    Ok(list.iter().filter_map(|(dir, m)| wanted(dir, m, &ids)).collect())
}

/// One magazine's pack as it's wanted now, by its id.
pub fn by_id(ix: &Index, lib: &Library, pack_id: &str) -> Result<Option<Wanted>> {
    if !is_pack_id(pack_id) {
        return Ok(None);
    }
    Ok(all(ix, lib)?.into_iter().find(|w| w.pack_id == pack_id && !w.items.is_empty()))
}

// ------------------------------------------------------------ making it

/// Pages (from 1) as the runs the contract takes, each `[first, last]`, within `max` pages.
pub fn runs(pages: &[u32], max: u32) -> Vec<[u32; 2]> {
    let mut p: Vec<u32> = pages.iter().copied().filter(|x| (1..=max).contains(x)).collect();
    p.sort_unstable();
    p.dedup();
    let mut out: Vec<[u32; 2]> = vec![];
    for x in p {
        match out.last_mut() {
            Some(r) if r[1] + 1 == x => r[1] = x,
            _ => out.push([x, x]),
        }
    }
    out
}

/// An issue's cover for Atlas: the picture chosen for it, else its first page (a comic's first
/// picture, a scanned PDF's first page), at most [`COVER_SIDE`] pixels on its longest side, as
/// a JPEG of at most 512 KB (its quality lowered until it fits). None: it has no picture, or
/// none that fits.
fn cover_jpeg(i: &IssueIn, work: &Path, n: usize) -> Option<PathBuf> {
    let bytes = i.cover.as_ref().and_then(|(p, _, _)| std::fs::read(p).ok()).or_else(|| magazines::first_page(&i.path).map(|(b, _)| b))?;
    let img = ::image::load_from_memory(&bytes).ok()?;
    let img = if img.width().max(img.height()) > COVER_SIDE { img.resize(COVER_SIDE, COVER_SIDE, ::image::imageops::FilterType::Lanczos3) } else { img };
    let rgb = img.to_rgb8();
    for q in [85u8, 70, 55, 40] {
        let mut out = Vec::new();
        ::image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, q).encode_image(&rgb).ok()?;
        if out.len() as u64 <= MAX_COVER_BYTES {
            let at = work.join(format!("{n}.cover.jpg"));
            std::fs::write(&at, out).ok()?;
            return Some(at);
        }
    }
    None
}

/// An issue as the crate takes it: a PDF or a CBZ as it is, any other (a CBR, or a `.cbz`
/// that isn't a ZIP) made a CBZ in `work` first; its pages counted; its text, when it's read,
/// cut to its pages and left out when it's over the contract's 64 MB; its cover; its games'
/// pages as runs within the issue (a game with none, or none within it, is on all of them).
/// Err: why it can't go, in a sentence.
fn new_issue(i: &IssueIn, work: &Path, n: usize) -> std::result::Result<NewIssue, String> {
    let zip_already = i.kind == "cbz" && magazines::comic_kind(&i.path) == Some("zip");
    let (file, pages) = if i.kind == "pdf" || zip_already {
        let pages = guidebooks::pages_of(&i.path, &i.kind, i.size, i.mtime).map_err(|e| format!("{e} It can't go to Atlas."))?;
        (i.path.clone(), pages)
    } else {
        let to = work.join(format!("{n}.cbz"));
        let pages = guidebooks::to_cbz(&i.path, &to).map_err(|e| format!("It couldn't be made a CBZ for Atlas: {e:#}"))?;
        (to, pages)
    };
    if pages == 0 {
        return Err("It has no pages, so it can't go to Atlas.".into());
    }
    if pages > MAX_GUIDEBOOK_PAGES {
        return Err("It has more than 100,000 pages, the most an issue for Atlas can have.".into());
    }
    let text = crate::magtext::kept(&i.path).map(|k| k.pages).map(|mut p| {
        p.truncate(pages as usize);
        p
    });
    let text = text.filter(|p| {
        let t = GuidebookText::of_pages(p);
        !t.pages.is_empty() && serde_json::to_vec(&t).is_ok_and(|b| (b.len() as u64) <= MAX_GUIDEBOOK_TEXT_BYTES)
    });
    let games = i
        .games
        .iter()
        .map(|g| {
            let mut r = runs(&g.pages, pages);
            if r.is_empty() {
                r = vec![[1, pages]];
            }
            r.truncate(MAX_GAME_PAGE_RUNS);
            IssueGame { title: g.title.clone(), system: g.system.clone(), game_id: g.game_id.clone(), pages: r }
        })
        .collect();
    Ok(NewIssue { file, title: i.title.clone(), date: i.date.clone(), number: i.number, pages: Some(pages), text, cover: cover_jpeg(i, work, n), games })
}

/// A magazine's pack for `w`, made into `cache` (or the one made already when it's the newest
/// of its id): every issue that can go, written by Atlas's crate, the others left out with
/// why (`Made.left`).
pub fn make(cache: &Cache, w: &Wanted, cancel: &AtomicBool) -> Result<Made> {
    cache.make_with(&w.pack_id, &w.recipe(), cancel, &mut |target, _before, work| {
        let mut left = w.left.clone();
        let mut issues = vec![];
        for (n, i) in w.items.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                bail!("stopped");
            }
            match new_issue(i, work, n) {
                Ok(x) => issues.push(x),
                Err(why) => left.push((i.title.clone(), why)),
            }
        }
        if issues.is_empty() {
            bail!("None of {}'s issues can go to Atlas.", w.title);
        }
        let pack = MagazinePack {
            pack_id: w.pack_id.clone(),
            magazine: MagazineFacts { title: w.title.clone(), publisher: w.publisher.clone() },
            written_by: guidebooks::written_by(),
            issues,
        };
        let header = atlas_pack::write_magazine_pack(&pack, target).map_err(|e| anyhow!("{e}"))?;
        Ok(Made {
            updated: header.pack.updated.clone().unwrap_or_default(),
            exported_at: header.exported_at.clone(),
            items: header.issues.len(),
            pages: header.issues.iter().map(|i| u64::from(i.pages)).sum(),
            left,
            ..Default::default()
        })
    })
}

// ------------------------------------------------------------ the Magazines page

/// What the Magazines page says of each magazine's pack for Atlas (D249): added to
/// [`magazines::overview`]'s magazines as `atlas` (its id, how many issues go, and its size and
/// when it was made, from Game Repo Server's cache in `server_data` when it's made for the
/// magazine as it is now, else the size of the one made before), and to each issue that can't
/// go as `atlas_left`, why. The files aren't read: an issue whose pages can't be counted is
/// said once the server has made the pack.
pub fn annotate(ix: &Index, lib: &Library, server_data: &Path, v: &mut Value) -> Result<()> {
    let list = magazines::list(lib);
    if list.is_empty() {
        return Ok(());
    }
    let ids = GameIds::of(ix, lib)?;
    let cache = Cache::new(server_data.join(DIR));
    let Some(mags) = v["magazines"].as_array_mut() else { return Ok(()) };
    for (dir, m) in &list {
        let Some(w) = wanted(dir, m, &ids) else { continue };
        let rel = lib.relative(dir);
        let Some(entry) = mags.iter_mut().find(|x| x["folder"].as_str() == rel.as_deref()) else { continue };
        let made = cache.current_of(&w.pack_id, &w.recipe());
        let before = if made.is_none() { cache.newest(&w.pack_id) } else { None };
        let left: Vec<(String, String)> = made.as_ref().map(|x| x.left.clone()).unwrap_or_else(|| w.left.clone());
        entry["atlas"] = json!({
            "pack_id": w.pack_id,
            "file": w.file_name(),
            "issues": made.as_ref().map_or(w.items.len(), |x| x.items),
            "size": made.as_ref().map(|x| x.size),
            "made": made.as_ref().map(|x| x.made.clone()),
            "before": before.map(|x| x.size),
        });
        if let Some(issues) = entry["issues"].as_array_mut() {
            for (i, e) in m.issues.iter().zip(issues.iter_mut()) {
                if let Some((_, why)) = left.iter().find(|(t, _)| *t == issue_title(i)) {
                    e["atlas_left"] = json!(why);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
