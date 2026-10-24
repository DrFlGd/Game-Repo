//! Videos (docs/PLAN.md, D146): a section of their own, like magazines. Each is a file kept
//! in the library as `Videos/<Kind>/<Title>.<ext>` (Walkthroughs, Reviews, Retrospectives
//! or Other), or a web link (YouTube and the like, opened in the browser), and
//! `Videos/videos.json` lists them all: each one's title, kind, date, length, source and the
//! games it's about. *Import videos…* reads folders, takes each one's title, kind, date and
//! source from its name, finds the games in its title (the titles D145 looks for in
//! magazines' text, compared as D120 compares names), and brings them in as an Organise
//! change with Undo. A game's page lists its videos. Exports and devices don't take them.

use crate::config::{read_json_object, sorted};
use crate::docs::{outside, text_op};
use crate::index::Index;
use crate::library::{now, Library};
use crate::organise::{self, safe_name, Op, Plan, Src};
use crate::variants::{split_name, words};
use anyhow::{anyhow, bail, Context, Result};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// The videos' folder in the library.
pub const FOLDER: &str = "Videos";
pub const MANIFEST: &str = "videos.json";
/// The files a video can be.
pub const EXTS: &[&str] = &["mp4", "m4v", "mov", "webm", "mkv", "avi"];

/// The kinds of video: id, name, its folder, and the words in a name that say it's one.
pub const KINDS: &[(&str, &str, &str, &[&str])] = &[
    ("walkthrough", "Walkthrough", "Walkthroughs", &["walkthrough", "longplay", "long play", "playthrough", "lets play", "let s play", "speedrun", "speed run", "full game", "no commentary", "guide"]),
    ("retrospective", "Retrospective", "Retrospectives", &["retrospective", "history of", "documentary", "making of", "story of", "look back", "looking back", "rise and fall"]),
    ("review", "Review", "Reviews", &["review", "reviews", "reviewed", "avgn", "angry video game nerd", "worth playing", "worth it"]),
    ("other", "Other", "Other", &[]),
];

pub fn folder(lib: &Library) -> PathBuf {
    lib.root().join(FOLDER)
}

fn kind(id: &str) -> &'static (&'static str, &'static str, &'static str, &'static [&'static str]) {
    KINDS.iter().find(|k| k.0 == id).unwrap_or(&KINDS[KINDS.len() - 1])
}

/// A game a video is about.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct VideoGame {
    /// The game's key in the Library: `<system>/<DAT name>`, `pc/<id>`
    pub key: String,
    pub title: String,
    pub platform: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Video {
    pub id: String,
    pub title: String,
    /// walkthrough | review | retrospective | other
    pub kind: String,
    /// Its file below `Videos/`, with `/`; none: it's a web link
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// YYYY-MM-DD, YYYY-MM or YYYY
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    /// In seconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub length: Option<u64>,
    /// Who made it, or where it's from: "YouTube", a channel (shown as its creator, D153)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// The series it's part of: a show, a let's play in parts (D153)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series: Option<String>,
    /// Labels of your own: "Favourites", "To watch" (D153)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// A picture chosen for it, below `Videos/` (`pictures/<id>.<ext>`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub picture: Option<String>,
    #[serde(skip_serializing_if = "is_zero")]
    pub size: u64,
    /// The file's size and its first and last megabyte, hashed: the same video imported
    /// again is known without reading all of it
    #[serde(skip_serializing_if = "String::is_empty")]
    pub print: String,
    pub added: String,
    pub games: Vec<VideoGame>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Videos {
    pub videos: Vec<Video>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// What `videos.json` says (nothing yet: none).
pub fn read(lib: &Library) -> Videos {
    serde_json::from_value(read_json_object(&folder(lib).join(MANIFEST))).unwrap_or_default()
}

fn text_of(v: &Videos) -> Result<String> {
    Ok(serde_json::to_string_pretty(&sorted(&serde_json::to_value(v)?))? + "\n")
}

fn write(lib: &Library, v: &mut Videos) -> Result<()> {
    v.videos.sort_by(order);
    std::fs::create_dir_all(folder(lib))?;
    crate::config::write_json(&folder(lib).join(MANIFEST), &serde_json::to_value(&*v)?)
}

/// By title, then date.
fn order(a: &Video, b: &Video) -> std::cmp::Ordering {
    a.title.to_lowercase().cmp(&b.title.to_lowercase()).then_with(|| a.date.cmp(&b.date)).then_with(|| a.id.cmp(&b.id))
}

/// A new video's id: from what it is (its file's print, its link), made unique.
fn new_id(of: &str, taken: &HashSet<String>) -> String {
    use sha1::Digest;
    let base: String = hex::encode(sha1::Sha1::digest(of.as_bytes()))[..10].to_string();
    let mut id = base.clone();
    let mut i = 2;
    while taken.contains(&id) {
        id = format!("{base}-{i}");
        i += 1;
    }
    id
}

// ------------------------------------------------------------ names

/// What a video's file name says.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Named {
    pub title: String,
    pub kind: String,
    pub date: Option<String>,
    pub source: Option<String>,
    pub url: Option<String>,
}

/// A YouTube video's id at the end of a name, as yt-dlp leaves it: "Title [dQw4w9WgXcQ]".
fn youtube_id(stem: &str) -> Option<(&str, &str)> {
    let s = stem.trim_end();
    let inner = s.strip_suffix(']')?;
    let at = inner.rfind('[')?;
    let id = &inner[at + 1..];
    (id.len() == 11 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')).then(|| (&s[..at], id))
}

/// A date in a name: 2021-03-05, 2021-03, 20210305 (yt-dlp's upload date).
fn date_of(t: &str) -> Option<String> {
    let t = t.trim_matches(|c: char| matches!(c, '(' | ')' | '[' | ']' | ','));
    let year = |y: &str| y.len() == 4 && y.chars().all(|c| c.is_ascii_digit()) && (1970..=2099).contains(&y.parse::<u32>().unwrap_or(0));
    let two = |x: &str, hi: u32| x.len() == 2 && x.chars().all(|c| c.is_ascii_digit()) && (1..=hi).contains(&x.parse::<u32>().unwrap_or(0));
    let parts: Vec<&str> = t.split('-').collect();
    match parts.as_slice() {
        [y, m] if year(y) && two(m, 12) => Some(format!("{y}-{m}")),
        [y, m, d] if year(y) && two(m, 12) && two(d, 31) => Some(format!("{y}-{m}-{d}")),
        [n] if n.len() == 8 && year(&n[..4]) && two(&n[4..6], 12) && two(&n[6..], 31) => Some(format!("{}-{}-{}", &n[..4], &n[4..6], &n[6..])),
        _ => None,
    }
}

/// The kind a title says, by its words ("… Longplay", "… Review", "The History of …").
pub fn kind_of(title: &str) -> &'static str {
    let plain = format!(" {} ", title.to_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" "));
    KINDS.iter().find(|k| k.3.iter().any(|w| plain.contains(&format!(" {w} ")))).map(|k| k.0).unwrap_or("other")
}

/// A file name's title, kind, date and source: "Super Mario World - Longplay
/// [dQw4w9WgXcQ]", "20190412 Mega Man 2 Review", "Sonic_the_Hedgehog_Retrospective".
pub fn parse(stem: &str) -> Named {
    let (rest, url, source) = match youtube_id(stem) {
        Some((rest, id)) => (rest, Some(format!("https://www.youtube.com/watch?v={id}")), Some("YouTube".to_string())),
        None => (stem, None, None),
    };
    // "Sonic_the_Hedgehog", "Sonic.the.Hedgehog"
    let rest: String = if rest.contains(' ') { rest.replace('_', " ") } else { rest.chars().map(|c| if c == '_' || c == '.' { ' ' } else { c }).collect() };
    let mut date = None;
    let mut kept: Vec<&str> = vec![];
    for t in rest.split_whitespace() {
        match date_of(t).filter(|_| date.is_none()) {
            Some(d) => date = Some(d),
            None => kept.push(t),
        }
    }
    let mut title = kept.join(" ").replace("()", "").replace("[]", "");
    loop {
        let t = title.trim().trim_matches(|c: char| matches!(c, '-' | '–' | '—' | '|' | '.' | ',' | ':')).trim().to_string();
        let t = t.replace("  ", " ").replace(" - - ", " - ");
        if t == title {
            break;
        }
        title = t;
    }
    let kind = kind_of(&title).to_string();
    Named { title, kind, date, source, url }
}

// ------------------------------------------------------------ a file's length

/// How long a video is, in seconds, from its file's header: MP4, M4V and MOV (`mvhd`),
/// Matroska and WebM (the segment's `Info`), AVI (`avih`, or OpenDML's `dmlh`).
pub fn length_of(p: &Path) -> Option<u64> {
    let mut f = File::open(p).ok()?;
    let secs = match crate::platforms::ext_of(&p.to_string_lossy()).as_str() {
        "mp4" | "m4v" | "mov" => mp4_length(&mut f),
        "mkv" | "webm" => mkv_length(&mut f),
        "avi" => avi_length(&mut f),
        _ => None,
    }?;
    (secs.is_finite() && secs > 0.0 && secs < 1e7).then(|| secs.round() as u64)
}

fn be(b: &[u8]) -> u64 {
    b.iter().fold(0u64, |n, &x| (n << 8) | x as u64)
}

/// An MP4 box named `name` between `start` and `end`: where its content starts and ends.
fn mp4_box(f: &mut File, start: u64, end: u64, name: &[u8; 4]) -> Option<(u64, u64)> {
    let mut at = start;
    for _ in 0..10_000 {
        if at + 8 > end {
            return None;
        }
        f.seek(SeekFrom::Start(at)).ok()?;
        let mut h = [0u8; 16];
        f.read_exact(&mut h[..8]).ok()?;
        let (mut size, mut head) = (be(&h[..4]), 8);
        if size == 1 {
            f.read_exact(&mut h[8..16]).ok()?;
            size = be(&h[8..16]);
            head = 16;
        } else if size == 0 {
            size = end - at;
        }
        if size < head {
            return None;
        }
        if &h[4..8] == name {
            return Some((at + head, (at + size).min(end)));
        }
        at += size;
    }
    None
}

fn mp4_length(f: &mut File) -> Option<f64> {
    let end = f.metadata().ok()?.len();
    let (from, to) = mp4_box(f, 0, end, b"moov")?;
    let (at, _) = mp4_box(f, from, to, b"mvhd")?;
    f.seek(SeekFrom::Start(at)).ok()?;
    let mut b = [0u8; 32];
    f.read_exact(&mut b).ok()?;
    let (scale, duration) = if b[0] == 1 { (be(&b[20..24]), be(&b[24..32])) } else { (be(&b[12..16]), be(&b[16..20])) };
    (scale > 0).then(|| duration as f64 / scale as f64)
}

/// An EBML number: an element's id (with its length marker) or size (without).
fn vint(f: &mut File, id: bool) -> Option<(u64, bool)> {
    let mut b = [0u8; 8];
    f.read_exact(&mut b[..1]).ok()?;
    let len = b[0].leading_zeros() as usize + 1;
    if len > 8 {
        return None;
    }
    f.read_exact(&mut b[1..len]).ok()?;
    let mut n = be(&b[..len]);
    if !id {
        n &= (1u64 << (7 * len)) - 1;
    }
    // a size of all ones: not known (a live recording's segment)
    let unknown = !id && n == (1u64 << (7 * len)) - 1;
    Some((n, unknown))
}

fn mkv_length(f: &mut File) -> Option<f64> {
    const EBML: u64 = 0x1A45DFA3;
    const SEGMENT: u64 = 0x18538067;
    const INFO: u64 = 0x1549A966;
    const CLUSTER: u64 = 0x1F43B675;
    const SCALE: u64 = 0x2AD7B1;
    const DURATION: u64 = 0x4489;
    let end = f.metadata().ok()?.len();
    let (id, _) = vint(f, true)?;
    if id != EBML {
        return None;
    }
    let (size, _) = vint(f, false)?;
    f.seek(SeekFrom::Current(size as i64)).ok()?;
    let (id, _) = vint(f, true)?;
    if id != SEGMENT {
        return None;
    }
    vint(f, false)?;
    for _ in 0..1_000 {
        let (id, _) = vint(f, true)?;
        let (size, unknown) = vint(f, false)?;
        let at = f.stream_position().ok()?;
        if id == CLUSTER || unknown || at + size > end {
            return None;
        }
        if id == INFO {
            let (mut scale, mut duration) = (1_000_000u64, None);
            while f.stream_position().ok()? < at + size {
                let (id, _) = vint(f, true)?;
                let (n, _) = vint(f, false)?;
                let mut b = vec![0u8; n.min(64) as usize];
                f.read_exact(&mut b).ok()?;
                f.seek(SeekFrom::Current(n as i64 - b.len() as i64)).ok()?;
                match (id, b.len()) {
                    (SCALE, 1..=8) => scale = be(&b),
                    (DURATION, 4) => duration = Some(f32::from_be_bytes(b[..4].try_into().ok()?) as f64),
                    (DURATION, 8) => duration = Some(f64::from_be_bytes(b[..8].try_into().ok()?)),
                    _ => {}
                }
            }
            return duration.map(|d| d * scale as f64 / 1e9);
        }
        f.seek(SeekFrom::Start(at + size)).ok()?;
    }
    None
}

fn avi_length(f: &mut File) -> Option<f64> {
    let mut b = vec![0u8; 64 * 1024];
    let n = f.read(&mut b).ok()?;
    let b = &b[..n];
    if n < 56 || &b[..4] != b"RIFF" || &b[8..12] != b"AVI " {
        return None;
    }
    let le = |x: &[u8]| u32::from_le_bytes(x.try_into().unwrap_or([0; 4])) as f64;
    let at = b.windows(4).position(|w| w == b"avih")?;
    let h = b.get(at + 8..at + 32)?;
    let (usec, mut frames) = (le(&h[..4]), le(&h[16..20]));
    // a file over 1 GB (OpenDML) counts all its frames in `dmlh`
    if let Some(d) = b.windows(4).position(|w| w == b"dmlh") {
        if let Some(x) = b.get(d + 8..d + 12) {
            frames = frames.max(le(x));
        }
    }
    (usec > 0.0).then(|| frames * usec / 1e6)
}

// ------------------------------------------------------------ importing

/// A video to bring in, with what its name says (changed in the review before it's planned).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Item {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub print: String,
    pub title: String,
    pub kind: String,
    pub date: Option<String>,
    pub source: Option<String>,
    pub url: Option<String>,
    pub length: Option<u64>,
    pub games: Vec<VideoGame>,
    /// The library has it already (the same file)
    pub have: bool,
    pub include: bool,
}

/// A file's print: its size and its first and last megabyte, hashed.
fn print_of(p: &Path, cancel: &AtomicBool) -> Result<String> {
    use sha1::Digest;
    if cancel.load(Ordering::Relaxed) {
        bail!("stopped");
    }
    let mut f = File::open(p).with_context(|| format!("couldn't read {}", p.display()))?;
    let size = f.metadata()?.len();
    let mut h = sha1::Sha1::new();
    h.update(size.to_le_bytes());
    let mut buf = vec![0u8; 1 << 20];
    let n = f.read(&mut buf)?;
    h.update(&buf[..n]);
    if size > 2 << 20 {
        f.seek(SeekFrom::End(-(1 << 20)))?;
        f.read_exact(&mut buf)?;
        h.update(&buf);
    }
    Ok(hex::encode(h.finalize()))
}

fn is_video(p: &Path) -> bool {
    EXTS.contains(&crate::platforms::ext_of(&p.to_string_lossy()).as_str())
}

/// The games a title names: the library's games, and those its systems' DATs list.
fn games_in(titles: &crate::magtext::Titles, title: &str) -> Vec<VideoGame> {
    titles.find(&[title.to_string()]).into_iter().map(|g| VideoGame { key: g.key, title: g.title, platform: g.platform }).collect()
}

/// The library's games, ready to be found in titles.
fn titles(ix: &Index, lib: &Library) -> Result<crate::magtext::Titles> {
    crate::meta::sync(ix, lib)?;
    crate::browse::refresh(ix, lib)?;
    crate::magtext::Titles::of(ix.conn())
}

/// The videos in the paths picked (files themselves, folders walked): each one's title,
/// kind, date and source from its name, its length from its header, and the games in its
/// title. A video in a folder named for a kind ("Reviews") is of that kind.
pub fn read_items(ix: &Index, lib: &Library, paths: &[PathBuf], cancel: &AtomicBool, progress: &mut dyn FnMut(Value)) -> Result<Vec<Item>> {
    let mut files: Vec<PathBuf> = vec![];
    for p in paths {
        if p.is_file() {
            if is_video(p) {
                files.push(p.clone());
            }
            continue;
        }
        let mut stack = vec![p.clone()];
        while let Some(d) = stack.pop() {
            let mut es: Vec<PathBuf> = std::fs::read_dir(&d).into_iter().flatten().flatten().map(|e| e.path()).collect();
            es.sort();
            for e in es {
                if e.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) {
                    continue;
                }
                if e.is_dir() {
                    stack.push(e);
                } else if is_video(&e) {
                    files.push(e);
                }
            }
        }
    }
    progress(json!({ "text": "Getting the games' titles" }));
    let titles = titles(ix, lib)?;
    let have: HashSet<String> = read(lib).videos.into_iter().map(|v| v.print).filter(|p| !p.is_empty()).collect();
    let mut v = vec![];
    for (i, p) in files.iter().enumerate() {
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        progress(json!({ "item": i + 1, "items": files.len(), "name": name }));
        let stem = p.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut named = parse(&stem);
        if named.kind == "other" {
            let parent = p.parent().and_then(Path::file_name).map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            if let Some(k) = KINDS.iter().find(|k| k.0 != "other" && (parent == k.0 || parent == k.2.to_lowercase())) {
                named.kind = k.0.to_string();
            }
        }
        if named.title.is_empty() {
            named.title = stem.clone();
        }
        let print = print_of(p, cancel)?;
        let had = have.contains(&print);
        v.push(Item {
            path: p.display().to_string(),
            name,
            size: std::fs::metadata(p)?.len(),
            games: games_in(&titles, &named.title),
            title: named.title,
            kind: named.kind,
            date: named.date,
            source: named.source,
            url: named.url,
            length: length_of(p),
            print,
            have: had,
            include: !had,
        });
    }
    Ok(v)
}

fn free(taken: &mut HashSet<String>, name: &str, ext: &str) -> String {
    let mut n = safe_name(&format!("{name}.{ext}"));
    let mut i = 2;
    while taken.contains(&n.to_lowercase()) {
        n = safe_name(&format!("{name} ({i}).{ext}"));
        i += 1;
    }
    taken.insert(n.to_lowercase());
    n
}

fn plan_of(lib: &Library, kind: &str, title: String, label: String, ops: Vec<Op>, n: usize) -> Plan {
    Plan {
        id: organise::new_id(),
        created: now(),
        kind: kind.into(),
        label,
        title,
        dat: Value::Null,
        platform: String::new(),
        mode: if kind == "videos" { "copy".into() } else { "aside".into() },
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
        export: None, brought: vec![],
    }
}

/// Plan bringing the items into their kinds' folders (copied, hardlinked on the same drive
/// unless `always_copy`, or moved), `videos.json` written with them.
pub fn plan(lib: &Library, items: &[Item], mode: &str, always_copy: bool) -> Result<Plan> {
    lib.writable()?;
    let items: Vec<&Item> = items.iter().filter(|i| i.include).collect();
    if items.is_empty() {
        bail!("Choose at least one video to import.");
    }
    let mut all = read(lib);
    let mut ids: HashSet<String> = all.videos.iter().map(|v| v.id.clone()).collect();
    let mut taken: HashSet<String> = HashSet::new();
    for k in KINDS {
        for e in std::fs::read_dir(folder(lib).join(k.2)).into_iter().flatten().flatten() {
            taken.insert(format!("{}/{}", k.2, e.file_name().to_string_lossy()).to_lowercase());
        }
    }
    let mut ops = vec![];
    for it in &items {
        let src = PathBuf::from(&it.path);
        if !src.is_file() {
            bail!("There's no file {}.", it.path);
        }
        let title = it.title.trim();
        if title.is_empty() {
            bail!("{} needs a title.", it.name);
        }
        let k = kind(&it.kind);
        let ext = crate::platforms::ext_of(&it.name);
        let mut in_kind: HashSet<String> = taken.iter().filter_map(|t| t.strip_prefix(&format!("{}/", k.2.to_lowercase())).map(String::from)).collect();
        let name = free(&mut in_kind, title, &ext);
        let file = format!("{}/{name}", k.2);
        taken.insert(file.to_lowercase());
        let to = folder(lib).join(k.2).join(&name);
        let size = std::fs::metadata(&src)?.len();
        let op = match mode {
            "move" => "move",
            _ if !always_copy && organise::same_drive(&src, &folder(lib)) => "link",
            _ => "copy",
        };
        let s: Src = Src { size, ..outside(&src) };
        ops.push(Op { op: op.into(), game: title.to_string(), to: to.display().to_string(), from: vec![s], names: vec![], aside: vec![], size, note: None });
        let id = new_id(if it.print.is_empty() { &file } else { &it.print }, &ids);
        ids.insert(id.clone());
        let opt = |s: &Option<String>| s.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(String::from);
        all.videos.push(Video {
            id,
            title: title.to_string(),
            kind: k.0.to_string(),
            file: Some(file),
            url: opt(&it.url),
            date: opt(&it.date),
            length: it.length,
            source: opt(&it.source),
            size,
            print: it.print.clone(),
            added: now(),
            games: it.games.clone(),
            ..Default::default()
        });
    }
    all.videos.sort_by(order);
    ops.push(text_op("Videos", &folder(lib).join(MANIFEST), text_of(&all)?, "videos.json: the videos"));
    let n = items.len();
    let s = if n == 1 { "" } else { "s" };
    Ok(plan_of(lib, "videos", format!("Import {n} video{s}"), format!("Imported {n} video{s}"), ops, n))
}

// ------------------------------------------------------------ links, changes, removing

fn web_link(url: &str) -> Result<String> {
    let u = url.trim();
    if !(u.starts_with("https://") || u.starts_with("http://")) || u.contains(char::is_whitespace) || u.len() < 12 {
        bail!("A link starts with https:// (or http://), as the browser shows it.");
    }
    Ok(u.to_string())
}

/// Where a link is from, by its address: "YouTube", "Vimeo", else the site.
fn site_of(url: &str) -> Option<String> {
    let host = url.split("://").nth(1)?.split(['/', '?', '#']).next()?.trim_start_matches("www.").trim_start_matches("m.").to_lowercase();
    Some(match host.as_str() {
        "youtube.com" | "youtu.be" | "music.youtube.com" => "YouTube".into(),
        "vimeo.com" => "Vimeo".into(),
        "twitch.tv" => "Twitch".into(),
        "archive.org" => "Internet Archive".into(),
        "dailymotion.com" => "Dailymotion".into(),
        "" => return None,
        h => h.to_string(),
    })
}

/// A web link added as a video: its games found in its title.
pub fn add_link(ix: &Index, lib: &Library, url: &str, title: &str, kind_id: Option<&str>, date: Option<&str>) -> Result<Value> {
    lib.writable()?;
    let url = web_link(url)?;
    let title = title.trim();
    if title.is_empty() {
        bail!("Give the video a title.");
    }
    let mut all = read(lib);
    if let Some(v) = all.videos.iter().find(|v| v.url.as_deref() == Some(url.as_str()) && v.file.is_none()) {
        bail!("{} has that link already.", v.title);
    }
    let titles = titles(ix, lib)?;
    let ids: HashSet<String> = all.videos.iter().map(|v| v.id.clone()).collect();
    let id = new_id(&url, &ids);
    all.videos.push(Video {
        id: id.clone(),
        title: title.to_string(),
        kind: kind(kind_id.unwrap_or_else(|| kind_of(title))).0.to_string(),
        file: None,
        source: site_of(&url),
        url: Some(url),
        date: date.map(str::trim).filter(|d| !d.is_empty()).map(String::from),
        games: games_in(&titles, title),
        added: now(),
        ..Default::default()
    });
    write(lib, &mut all)?;
    Ok(json!({ "id": id }))
}

/// Change a video's title, kind, date, source or link (`videos.json`; its file keeps its
/// name and folder).
pub fn edit(lib: &Library, id: &str, patch: &Value) -> Result<Value> {
    lib.writable()?;
    let mut all = read(lib);
    let v = all.videos.iter_mut().find(|v| v.id == id).ok_or_else(|| anyhow!("There's no video {id}."))?;
    let text = |k: &str| patch.get(k).map(|v| v.as_str().map(str::trim).filter(|s| !s.is_empty()).map(String::from));
    if let Some(t) = text("title") {
        v.title = t.ok_or_else(|| anyhow!("A video needs a title."))?;
    }
    if let Some(Some(k)) = text("kind") {
        v.kind = kind(&k).0.to_string();
    }
    if let Some(d) = text("date") {
        v.date = d;
    }
    if let Some(s) = text("source") {
        v.source = s;
    }
    if let Some(s) = text("series") {
        v.series = s;
    }
    if let Some(u) = text("url") {
        match u {
            Some(u) => v.url = Some(web_link(&u)?),
            None if v.file.is_none() => bail!("A video that's a link needs its link."),
            None => v.url = None,
        }
    }
    write(lib, &mut all)?;
    Ok(json!({ "ok": true }))
}

/// A tag in the order it's kept: trimmed, and the same as one there is but for case taken as that one.
fn tag_of(all: &[String], t: &str) -> Option<String> {
    let t = t.trim();
    (!t.is_empty()).then(|| all.iter().find(|x| x.eq_ignore_ascii_case(t)).cloned().unwrap_or_else(|| t.to_string()))
}

/// Change several videos at once (D153): `patch` may set their `kind`, `series` and
/// `source` (empty clears it), add a tag (`tag_add`), take one off (`tag_remove`), add a
/// game (`game_add`, its key) or take one off (`game_remove`).
pub fn edit_many(ix: &Index, lib: &Library, ids: &[String], patch: &Value) -> Result<Value> {
    lib.writable()?;
    let mut all = read(lib);
    let want: HashSet<&str> = ids.iter().map(String::as_str).collect();
    if !all.videos.iter().any(|v| want.contains(v.id.as_str())) {
        bail!("Choose the videos to change.");
    }
    let text = |k: &str| patch.get(k).map(|v| v.as_str().map(str::trim).filter(|s| !s.is_empty()).map(String::from));
    let known: Vec<String> = all.videos.iter().flat_map(|v| v.tags.iter().cloned()).collect();
    let tag_add = patch["tag_add"].as_str().and_then(|t| tag_of(&known, t));
    let tag_remove = patch["tag_remove"].as_str().map(str::trim).filter(|t| !t.is_empty());
    let game_add = match patch["game_add"].as_str().filter(|k| !k.is_empty()) {
        Some(key) => {
            let (platform, title): (String, String) = ix
                .conn()
                .query_row("SELECT platform, title FROM library WHERE key = ?1", [key], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()?
                .ok_or_else(|| anyhow!("There's no game {key} in the library."))?;
            Some(VideoGame { key: key.into(), title: split_name(&title).0.to_string(), platform })
        }
        None => None,
    };
    let game_remove = patch["game_remove"].as_str();
    let mut n = 0;
    for v in all.videos.iter_mut().filter(|v| want.contains(v.id.as_str())) {
        n += 1;
        if let Some(Some(k)) = text("kind") {
            v.kind = kind(&k).0.to_string();
        }
        if let Some(x) = text("series") {
            v.series = x;
        }
        if let Some(x) = text("source") {
            v.source = x;
        }
        if let Some(t) = &tag_add {
            if !v.tags.iter().any(|x| x.eq_ignore_ascii_case(t)) {
                v.tags.push(t.clone());
                v.tags.sort_by_key(|x| x.to_lowercase());
            }
        }
        if let Some(t) = tag_remove {
            v.tags.retain(|x| !x.eq_ignore_ascii_case(t));
        }
        if let Some(g) = &game_add {
            if !v.games.iter().any(|x| x.key == g.key) {
                v.games.push(g.clone());
            }
        }
        if let Some(k) = game_remove {
            v.games.retain(|x| x.key != k);
        }
    }
    write(lib, &mut all)?;
    Ok(json!({ "changed": n }))
}

/// The picture a tile shows for a video (D153): `picture` copied into `Videos/pictures/`,
/// or none to take it off (its file goes too).
pub fn set_picture(lib: &Library, id: &str, picture: Option<&Path>) -> Result<Value> {
    lib.writable()?;
    let mut all = read(lib);
    let v = all.videos.iter_mut().find(|v| v.id == id).ok_or_else(|| anyhow!("There's no video {id}."))?;
    let base = folder(lib);
    let new = match picture {
        Some(p) => {
            let ext = crate::platforms::ext_of(&p.to_string_lossy());
            if !["jpg", "jpeg", "png", "webp", "gif", "bmp"].contains(&ext.as_str()) {
                bail!("A video's picture is a picture (JPEG, PNG, WebP).");
            }
            let rel = format!("pictures/{}.{ext}", safe_name(&v.id));
            std::fs::create_dir_all(base.join("pictures"))?;
            std::fs::copy(p, base.join(&rel)).with_context(|| format!("couldn't copy {}", p.display()))?;
            Some(rel)
        }
        None => None,
    };
    if let Some(old) = v.picture.take().filter(|o| Some(o) != new.as_ref()) {
        if let Ok(rel) = crate::library::rel_inside(&old) {
            let _ = std::fs::remove_file(base.join(rel));
        }
    }
    v.picture = new;
    write(lib, &mut all)?;
    Ok(json!({ "ok": true }))
}

/// Add a game to a video (by its key, from the Library), or take one off (`on: false`).
pub fn set_game(ix: &Index, lib: &Library, id: &str, key: &str, on: bool) -> Result<Value> {
    lib.writable()?;
    let mut all = read(lib);
    let v = all.videos.iter_mut().find(|v| v.id == id).ok_or_else(|| anyhow!("There's no video {id}."))?;
    let at = v.games.iter().position(|g| g.key == key);
    match (at, on) {
        (Some(i), false) => {
            v.games.remove(i);
        }
        (None, true) => {
            let (platform, title): (String, String) = ix
                .conn()
                .query_row("SELECT platform, title FROM library WHERE key = ?1", [key], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()?
                .ok_or_else(|| anyhow!("There's no game {key} in the library."))?;
            v.games.push(VideoGame { key: key.into(), title: split_name(&title).0.to_string(), platform });
        }
        _ => {}
    }
    write(lib, &mut all)?;
    Ok(json!({ "ok": true }))
}

/// Remove videos. Links are taken off the list; files (and the pictures chosen for them)
/// are set aside by a change, with the preview and Undo (the plan is returned to be run).
pub fn remove(lib: &Library, ids: &[String]) -> Result<Option<Plan>> {
    lib.writable()?;
    let mut all = read(lib);
    let want: HashSet<&str> = ids.iter().map(String::as_str).collect();
    let (gone, kept): (Vec<Video>, Vec<Video>) = all.videos.drain(..).partition(|v| want.contains(v.id.as_str()));
    all.videos = kept;
    if gone.is_empty() {
        bail!("There's no video {}.", ids.first().map(String::as_str).unwrap_or(""));
    }
    let base = folder(lib);
    let src = |rel: &str| {
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
    };
    let mut ops = vec![];
    for v in &gone {
        for rel in [v.file.as_deref(), v.picture.as_deref()].into_iter().flatten() {
            if crate::library::rel_inside(rel).is_ok() && base.join(rel).is_file() {
                let s = src(rel);
                ops.push(Op { op: "aside".into(), game: v.title.clone(), to: s.path.clone(), size: s.size, from: vec![s], names: vec![], aside: vec![], note: None });
            }
        }
    }
    if ops.is_empty() {
        write(lib, &mut all)?;
        return Ok(None);
    }
    ops.push(text_op("Videos", &base.join(MANIFEST), text_of(&all)?, "videos.json: the videos"));
    let (title, label) = match gone.as_slice() {
        [v] => (format!("Remove the video {}", v.title), format!("Removed the video {}", v.title)),
        _ => (format!("Remove {} videos", gone.len()), format!("Removed {} videos", gone.len())),
    };
    Ok(Some(plan_of(lib, "video_remove", title, label, ops, gone.len())))
}

// ------------------------------------------------------------ the page, and a game's

/// The pictures of the games videos are about, by their keys (D153: a tile shows its first game's cover).
fn pictures_of(ix: &Index, all: &Videos) -> std::collections::HashMap<String, String> {
    let keys: HashSet<&str> = all.videos.iter().flat_map(|v| v.games.iter().map(|g| g.key.as_str())).collect();
    let mut out = std::collections::HashMap::new();
    let Ok(mut q) = ix.conn().prepare("SELECT picture FROM library WHERE key = ?1 AND picture IS NOT NULL") else { return out };
    for k in keys {
        if let Ok(Some(p)) = q.query_row([k], |r| r.get::<_, String>(0)).optional() {
            out.insert(k.to_string(), p);
        }
    }
    out
}

fn shown(lib: &Library, v: &Video, pictures: &std::collections::HashMap<String, String>) -> Value {
    let mut o = serde_json::to_value(v).unwrap_or_default();
    o["kind_name"] = json!(kind(&v.kind).1);
    if let Some(p) = v.picture.as_deref().filter(|p| crate::library::rel_inside(p).is_ok()) {
        let at = folder(lib).join(p);
        if at.is_file() {
            o["picture_path"] = json!(lib.relative(&at));
        }
    }
    if let Some(f) = &v.file {
        let p = folder(lib).join(f);
        o["path"] = json!(lib.relative(&p));
        o["gone"] = json!(!p.is_file());
    }
    o["games"] = json!(v
        .games
        .iter()
        .map(|g| {
            let mut x = serde_json::to_value(g).unwrap_or_default();
            x["platform_name"] = json!(crate::browse::system_name(&g.platform));
            if let Some(p) = pictures.get(&g.key) {
                x["picture"] = json!(p);
            }
            x
        })
        .collect::<Vec<_>>());
    o
}

/// The Videos page: every video, with its games' systems named and their pictures.
pub fn overview(ix: &Index, lib: &Library) -> Value {
    let all = read(lib);
    let pictures = pictures_of(ix, &all);
    json!({
        "videos": all.videos.iter().map(|v| shown(lib, v, &pictures)).collect::<Vec<_>>(),
        "kinds": KINDS.iter().map(|k| json!({ "id": k.0, "name": k.1 })).collect::<Vec<_>>(),
        "folder": lib.relative(&folder(lib)),
    })
}

/// The videos about a game: by its key, or its title on its system.
pub fn of_game(lib: &Library, key: &str, platform: &str, title: &str) -> Vec<Value> {
    let want = words(split_name(title).0);
    read(lib)
        .videos
        .iter()
        .filter(|v| v.games.iter().any(|g| g.key == key || (g.platform == platform && !want.is_empty() && words(&g.title) == want)))
        .map(|v| shown(lib, v, &Default::default()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_names_title_kind_date_and_source_are_read() {
        let n = parse("Super Mario World - Longplay [dQw4w9WgXcQ]");
        assert_eq!(n, Named { title: "Super Mario World - Longplay".into(), kind: "walkthrough".into(), date: None, source: Some("YouTube".into()), url: Some("https://www.youtube.com/watch?v=dQw4w9WgXcQ".into()) });
        let n = parse("20190412 Mega Man 2 Review");
        assert_eq!((n.title.as_str(), n.kind.as_str(), n.date.as_deref()), ("Mega Man 2 Review", "review", Some("2019-04-12")));
        let n = parse("Sonic_the_Hedgehog_Retrospective");
        assert_eq!((n.title.as_str(), n.kind.as_str()), ("Sonic the Hedgehog Retrospective", "retrospective"));
        let n = parse("The History of Metroid (2021-03)");
        assert_eq!((n.title.as_str(), n.kind.as_str(), n.date.as_deref()), ("The History of Metroid", "retrospective", Some("2021-03")));
        // a game's year isn't the video's date
        let n = parse("Doom (1993) - Let's Play part 1");
        assert_eq!((n.title.as_str(), n.kind.as_str(), n.date), ("Doom (1993) - Let's Play part 1", "walkthrough", None));
        assert_eq!(parse("Speedrunners.Talk.Zelda").kind, "other");
        assert_eq!(site_of("https://youtu.be/abc").as_deref(), Some("YouTube"));
        assert_eq!(site_of("https://www.example.org/v/1").as_deref(), Some("example.org"));
    }

    fn mp4(duration: u32, scale: u32) -> Vec<u8> {
        let mut mvhd = vec![0u8; 100];
        mvhd[12..16].copy_from_slice(&scale.to_be_bytes());
        mvhd[16..20].copy_from_slice(&duration.to_be_bytes());
        let boxed = |name: &[u8], body: &[u8]| [&((body.len() + 8) as u32).to_be_bytes()[..], name, body].concat();
        let moov = boxed(b"moov", &boxed(b"mvhd", &mvhd));
        [boxed(b"ftyp", b"isom\0\0\x02\0isomiso2"), boxed(b"mdat", &[0u8; 300]), moov].concat()
    }

    fn mkv(seconds: f64) -> Vec<u8> {
        // ids with their markers, sizes one byte (0x80 | n)
        let el = |id: &[u8], body: &[u8]| [id, &[0x80 | body.len() as u8], body].concat();
        let info = [el(&[0x2A, 0xD7, 0xB1], &[0x0F, 0x42, 0x40]), el(&[0x44, 0x89], &(seconds * 1000.0).to_be_bytes())].concat();
        let seek = el(&[0x11, 0x4D, 0x9B, 0x74], &[0u8; 6]);
        let segment = [seek, el(&[0x15, 0x49, 0xA9, 0x66], &info), el(&[0x1F, 0x43, 0xB6, 0x75], &[0u8; 8])].concat();
        // the segment's size unknown (all ones), as a recording leaves it
        [el(&[0x1A, 0x45, 0xDF, 0xA3], &[0x42, 0x86, 0x81, 0x01]), vec![0x18, 0x53, 0x80, 0x67, 0xFF], segment].concat()
    }

    fn avi(usec: u32, frames: u32) -> Vec<u8> {
        let mut b = vec![0u8; 120];
        b[..4].copy_from_slice(b"RIFF");
        b[8..12].copy_from_slice(b"AVI ");
        b[12..16].copy_from_slice(b"LIST");
        b[20..24].copy_from_slice(b"hdrl");
        b[24..28].copy_from_slice(b"avih");
        b[28..32].copy_from_slice(&56u32.to_le_bytes());
        b[32..36].copy_from_slice(&usec.to_le_bytes());
        b[48..52].copy_from_slice(&frames.to_le_bytes());
        b
    }

    #[test]
    fn a_videos_length_is_read_from_its_header() {
        let d = std::env::temp_dir().join(format!("gamerepo-test-videolen-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let put = |name: &str, b: Vec<u8>| {
            let p = d.join(name);
            std::fs::write(&p, b).unwrap();
            p
        };
        assert_eq!(length_of(&put("a.mp4", mp4(125_000, 1000))), Some(125));
        assert_eq!(length_of(&put("b.mkv", mkv(90.4))), Some(90));
        assert_eq!(length_of(&put("c.avi", avi(40_000, 1500))), Some(60));
        assert_eq!(length_of(&put("d.webm", b"not a video".to_vec())), None);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn videos_are_imported_by_kind_their_games_found_and_a_link_added_and_removed() {
        use crate::audit::tests::{dat_xml, rom, rom_xml};
        let (dir, lib, ix) = crate::collection::tests::fixture("videos");
        let nes = dat_xml("Nintendo - Nintendo Entertainment System", "2026", &[
            ("Super Mario Bros. 3 (USA)", None, rom_xml("Super Mario Bros. 3 (USA).nes", &rom(40, 64))),
            ("Mega Man 2 (USA)", None, rom_xml("Mega Man 2 (USA).nes", &rom(41, 64))),
            ("Mega Man (USA)", None, rom_xml("Mega Man (USA).nes", &rom(42, 64))),
            ("Golf (USA)", None, rom_xml("Golf (USA).nes", &rom(43, 64))),
        ]);
        std::fs::write(dir.join("nes.dat"), nes).unwrap();
        crate::dat::import(&ix, &lib, &dir.join("nes.dat"), &serde_json::Map::new()).unwrap();
        let inbox = dir.join("inbox");
        std::fs::create_dir_all(inbox.join("Reviews")).unwrap();
        std::fs::write(inbox.join("Super Mario Bros 3 - Longplay [dQw4w9WgXcQ].mp4"), mp4(3_600_000, 1000)).unwrap();
        std::fs::write(inbox.join("Reviews").join("20200101 Mega Man 2.mkv"), mkv(600.0)).unwrap();
        std::fs::write(inbox.join("notes.txt"), b"not a video").unwrap();
        let cancel = AtomicBool::new(false);
        let mut items = read_items(&ix, &lib, &[inbox.clone()], &cancel, &mut |_| {}).unwrap();
        assert_eq!(items.len(), 2);
        let it = |items: &[Item], name: &str| items.iter().find(|i| i.name.starts_with(name)).cloned().unwrap();
        let smb = it(&items, "Super Mario");
        assert_eq!((smb.title.as_str(), smb.kind.as_str(), smb.length, smb.source.as_deref()), ("Super Mario Bros 3 - Longplay", "walkthrough", Some(3600), Some("YouTube")));
        assert_eq!(smb.games.iter().map(|g| g.title.as_str()).collect::<Vec<_>>(), ["Super Mario Bros. 3"]);
        let mm = it(&items, "20200101");
        assert_eq!((mm.title.as_str(), mm.kind.as_str(), mm.date.as_deref(), mm.length), ("Mega Man 2", "review", Some("2020-01-01"), Some(600)));
        assert_eq!(mm.games.iter().map(|g| g.key.as_str()).collect::<Vec<_>>(), ["nes/Mega Man 2 (USA)"]);
        for i in items.iter_mut() {
            if i.name.starts_with("20200101") {
                i.title = "Mega Man 2, reviewed".into();
            }
        }
        let plan = plan(&lib, &items, "copy", true).unwrap();
        assert_eq!((plan.kind.as_str(), plan.games), ("videos", 2));
        let r = organise::run(&ix, &lib, &plan, &cancel, &mut |_| {}).unwrap();
        assert!(r["failed"].as_array().unwrap().is_empty(), "{r}");
        let v = folder(&lib);
        assert!(v.join("Walkthroughs").join("Super Mario Bros 3 - Longplay.mp4").is_file());
        assert!(v.join("Reviews").join("Mega Man 2, reviewed.mkv").is_file());
        assert!(inbox.join("Reviews").join("20200101 Mega Man 2.mkv").is_file(), "copied, not moved");
        let all = read(&lib);
        assert_eq!(all.videos.iter().map(|v| v.title.as_str()).collect::<Vec<_>>(), ["Mega Man 2, reviewed", "Super Mario Bros 3 - Longplay"]);
        // read again: both are had already
        let again = read_items(&ix, &lib, &[inbox.clone()], &cancel, &mut |_| {}).unwrap();
        assert!(again.iter().all(|i| i.have && !i.include));

        // a game's page lists its videos, by key or by its title on its system
        let mm_key = "nes/Mega Man 2 (USA)";
        let of = of_game(&lib, mm_key, "nes", "Mega Man 2 (USA)");
        assert_eq!(of.len(), 1);
        assert_eq!(of[0]["kind_name"], "Review");
        assert_eq!(of[0]["path"], "Videos/Reviews/Mega Man 2, reviewed.mkv");
        assert_eq!(of[0]["games"][0]["platform_name"], crate::browse::system_name("nes"));

        // a link, its games found in its title; a game added and taken off by hand
        assert!(add_link(&ix, &lib, "youtube.com/watch?v=x", "Mega Man retrospective", None, None).is_err());
        let id = add_link(&ix, &lib, "https://www.youtube.com/watch?v=abcdefghijk", "The Mega Man 2 Story of the Year", None, Some("2015")).unwrap()["id"].as_str().unwrap().to_string();
        let link = read(&lib).videos.into_iter().find(|v| v.id == id).unwrap();
        assert_eq!((link.kind.as_str(), link.source.as_deref(), link.date.as_deref(), link.file), ("retrospective", Some("YouTube"), Some("2015"), None));
        assert_eq!(link.games.iter().map(|g| g.key.as_str()).collect::<Vec<_>>(), [mm_key]);
        assert!(add_link(&ix, &lib, "https://www.youtube.com/watch?v=abcdefghijk", "Again", None, None).is_err());
        set_game(&ix, &lib, &id, mm_key, false).unwrap();
        let golf = "nes/Golf (USA)".to_string();
        set_game(&ix, &lib, &id, &golf, true).unwrap();
        assert!(set_game(&ix, &lib, &id, "nes/Nothing", true).is_err());
        edit(&lib, &id, &json!({ "kind": "other", "title": "Mega Man's story", "date": "" })).unwrap();
        let link = read(&lib).videos.into_iter().find(|v| v.id == id).unwrap();
        assert_eq!((link.title.as_str(), link.kind.as_str(), link.date, link.games.iter().map(|g| g.key.clone()).collect::<Vec<_>>()), ("Mega Man's story", "other", None, vec![golf]));
        assert!(edit(&lib, &id, &json!({ "url": "" })).is_err());

        // several at once (D153): a series, a creator, a tag (one case for it), a game
        let ids: Vec<String> = read(&lib).videos.iter().map(|v| v.id.clone()).collect();
        assert_eq!(edit_many(&ix, &lib, &ids, &json!({ "series": "Mega Man week", "tag_add": "Favourites" })).unwrap()["changed"], 3);
        edit_many(&ix, &lib, &ids[..1], &json!({ "tag_add": "favourites", "source": "Someone" })).unwrap();
        let all = read(&lib);
        assert!(all.videos.iter().all(|v| v.series.as_deref() == Some("Mega Man week") && v.tags == ["Favourites"]), "{:?}", all.videos);
        assert_eq!(all.videos[0].source.as_deref(), Some("Someone"));
        edit_many(&ix, &lib, &ids, &json!({ "tag_remove": "FAVOURITES", "series": "", "game_add": "nes/Golf (USA)" })).unwrap();
        let all = read(&lib);
        assert!(all.videos.iter().all(|v| v.series.is_none() && v.tags.is_empty() && v.games.iter().filter(|g| g.key == "nes/Golf (USA)").count() == 1));
        assert!(edit_many(&ix, &lib, &["nothing".into()], &json!({ "series": "x" })).is_err());
        // a picture of its own, shown on its tile; its game's cover when it has none
        let pic = dir.join("pic.png");
        ::image::RgbImage::from_pixel(4, 4, ::image::Rgb([1, 2, 3])).save(&pic).unwrap();
        assert!(set_picture(&lib, &ids[0], Some(&dir.join("notes.txt"))).is_err());
        set_picture(&lib, &ids[0], Some(&pic)).unwrap();
        let with = read(&lib).videos.into_iter().find(|v| v.id == ids[0]).unwrap();
        assert!(v.join(with.picture.as_deref().unwrap()).is_file());
        let shown = overview(&ix, &lib);
        let first = shown["videos"].as_array().unwrap().iter().find(|x| x["id"] == ids[0].as_str()).unwrap().clone();
        assert!(first["picture_path"].as_str().unwrap().starts_with("Videos/pictures/"), "{first}");

        assert!(remove(&lib, &[id.clone()]).unwrap().is_none());
        assert!(read(&lib).videos.iter().all(|v| v.id != id));

        // a file is set aside by a change, and Undo brings it back
        let mm_id = read(&lib).videos.iter().find(|v| v.title.starts_with("Mega Man 2")).unwrap().id.clone();
        let rm = remove(&lib, &[mm_id.clone()]).unwrap().unwrap();
        assert_eq!(rm.kind, "video_remove");
        assert_eq!(rm.ops.iter().filter(|o| o.op == "aside").count(), 2, "its file and its picture");
        assert!(read(&lib).videos.iter().any(|v| v.id == mm_id), "listed until the change runs");
        let r = organise::run(&ix, &lib, &rm, &cancel, &mut |_| {}).unwrap();
        assert!(r["failed"].as_array().unwrap().is_empty(), "{r}");
        assert!(!v.join("Reviews").join("Mega Man 2, reviewed.mkv").exists());
        assert_eq!(read(&lib).videos.len(), 1);
        organise::undo(&ix, &lib, r["id"].as_str().unwrap(), &cancel, &mut |_| {}).unwrap();
        assert!(v.join("Reviews").join("Mega Man 2, reviewed.mkv").is_file());
        assert_eq!(read(&lib).videos.len(), 2);
        assert!(v.join(with.picture.as_deref().unwrap()).is_file(), "its picture back too");

        // several removed in one change
        let ids: Vec<String> = read(&lib).videos.iter().map(|v| v.id.clone()).collect();
        let rm = remove(&lib, &ids).unwrap().unwrap();
        assert_eq!((rm.title.as_str(), rm.ops.iter().filter(|o| o.op == "aside").count()), ("Remove 2 videos", 3));
    }
}
