//! Magazines (docs/PLAN.md, 3.8, "Phase 8 design", D68, D31): kept like PC games, a
//! folder per magazine in the library, `Magazines/<Title>/`, with `magazine.json` (its
//! publisher, and each issue's number, date, file and cover) and its issues' files, PDF,
//! CBZ or CBR, named `<Title> <number> (<YYYY-MM>).<ext>`. *Import magazines…* reads
//! folders, works out each file's magazine, number and date from its name, and brings them
//! in as an Organise change. A cover is the issue's first page (a CBZ's or CBR's first
//! picture, a scanned PDF's first page's one picture), else one dropped on it. Magazines
//! are audited against lists in `_library/dats/magazines/` (a DAT whose games are issues,
//! or a CSV of magazine, number and date). Their issues are documents too (`docs.rs`), so
//! they're read in the viewer and their text searched.

use crate::config::{read_json_object, sorted};
use crate::docs::{outside, text_op};
use crate::index::Index;
use crate::library::{now, Library, APP_DIR};
use crate::organise::{self, key, safe_name, Op, Plan, Src};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// The magazines' folder in the library.
pub const FOLDER: &str = "Magazines";
pub const MANIFEST: &str = "magazine.json";
/// The files an issue can be.
pub const EXTS: &[&str] = &["pdf", "cbz", "cbr"];
const PICTURES: &[&str] = &["jpg", "jpeg", "png", "webp", "gif", "bmp"];
const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];

pub fn folder(lib: &Library) -> PathBuf {
    lib.root().join(FOLDER)
}

/// Where the lists magazines are audited against are kept.
pub fn lists_dir(lib: &Library) -> PathBuf {
    lib.root().join(APP_DIR).join("dats").join("magazines")
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Issue {
    pub number: Option<String>,
    /// YYYY-MM, or YYYY
    pub date: Option<String>,
    /// Its file in the magazine's folder
    pub file: String,
    /// A picture of its cover in the magazine's `covers/` (one dropped on it); none: its first page
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover: Option<String>,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha1: Option<String>,
    pub added: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Magazine {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    pub issues: Vec<Issue>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Every magazine in the library, by title: (its folder, what its `magazine.json` says).
pub fn list(lib: &Library) -> Vec<(PathBuf, Magazine)> {
    let mut v: Vec<(PathBuf, Magazine)> = std::fs::read_dir(folder(lib))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join(MANIFEST).is_file())
        .filter_map(|e| Some((e.path(), serde_json::from_value(read_json_object(&e.path().join(MANIFEST))).ok()?)))
        .collect();
    v.sort_by(|a, b| a.1.title.to_lowercase().cmp(&b.1.title.to_lowercase()));
    v
}

fn by_title(lib: &Library, title: &str) -> Result<(PathBuf, Magazine)> {
    list(lib).into_iter().find(|(_, m)| m.title.eq_ignore_ascii_case(title)).ok_or_else(|| anyhow!("There's no magazine {title}."))
}

/// An issue's name in the library: `<Title> <number> (<YYYY-MM>)`, the number with three
/// digits so they sort.
pub fn issue_name(title: &str, number: Option<&str>, date: Option<&str>) -> String {
    let mut s = title.trim().to_string();
    if let Some(n) = number.filter(|n| !n.is_empty()) {
        let n = if n.chars().all(|c| c.is_ascii_digit()) { format!("{:0>3}", n) } else { n.to_string() };
        s.push_str(&format!(" {n}"));
    }
    if let Some(d) = date.filter(|d| !d.is_empty()) {
        s.push_str(&format!(" ({d})"));
    }
    s
}

/// How an issue is shown: "Nintendo Power 12 (1990-05)".
pub fn issue_title(title: &str, i: &Issue) -> String {
    let mut s = title.to_string();
    if let Some(n) = &i.number {
        s.push_str(&format!(" {n}"));
    }
    if let Some(d) = &i.date {
        s.push_str(&format!(" ({d})"));
    }
    s
}

// ------------------------------------------------------------ names

/// A file name's magazine, number and date: "Nintendo Power 012 (1990-05)",
/// "EGM_045_Apr1993", "Edge #100 - July 2001", "Retro Gamer Issue 7 (2004)".
pub fn parse(stem: &str) -> (String, Option<String>, Option<String>) {
    let s: String = stem.chars().map(|c| if matches!(c, '_' | '(' | ')' | '[' | ']' | ',') { ' ' } else { c }).collect();
    let tokens: Vec<&str> = s.split_whitespace().filter(|t| *t != "-").collect();
    let lower: Vec<String> = tokens.iter().map(|t| t.to_lowercase()).collect();
    let month = |t: &str| MONTHS.iter().position(|m| t.starts_with(m) && t.chars().all(|c| c.is_ascii_alphabetic()) && (t.len() == 3 || t.len() >= 4 && "january february march april may june july august september october november december sept".split(' ').any(|full| full == t)));
    let year = |t: &str| t.len() == 4 && t.chars().all(|c| c.is_ascii_digit()) && (1950..=2099).contains(&t.parse::<u32>().unwrap_or(0));
    let mut date: Option<String> = None;
    let mut number: Option<String> = None;
    let mut first_other: Option<usize> = None;
    let mut i = 0;
    while i < lower.len() {
        let t = lower[i].as_str();
        let mark = |at: usize, first: &mut Option<usize>| {
            if first.is_none() {
                *first = Some(at);
            }
        };
        // 1990-05, 1990-05-01
        let parts: Vec<&str> = t.split('-').collect();
        if parts.len() >= 2 && year(parts[0]) && parts[1].len() == 2 && parts[1].chars().all(|c| c.is_ascii_digit()) && date.is_none() {
            date = Some(format!("{}-{}", parts[0], parts[1]));
            mark(i, &mut first_other);
        } else if let Some(m) = month(t).filter(|_| i + 1 < lower.len() && year(&lower[i + 1]) && date.is_none()) {
            // Apr 1993, April 1993
            date = Some(format!("{}-{:02}", lower[i + 1], m + 1));
            mark(i, &mut first_other);
            i += 1;
        } else if let Some(m) = (t.chars().count() >= 7).then(|| t.char_indices().rev().nth(3).map(|(at, _)| t.split_at(at))).flatten().filter(|(a, y)| year(y) && month(a).is_some()).map(|(a, y)| (month(a).unwrap(), y.to_string())).filter(|_| date.is_none()) {
            // Apr1993
            date = Some(format!("{}-{:02}", m.1, m.0 + 1));
            mark(i, &mut first_other);
        } else if year(t) && date.is_none() && (number.is_some() || i > 0 && lower[..i].iter().any(|x| x.chars().all(|c| c.is_ascii_digit()))) {
            date = Some(t.to_string());
            mark(i, &mut first_other);
        } else if ["issue", "no", "no.", "nr", "nr.", "#", "vol", "vol.", "volume"].contains(&t) {
            mark(i, &mut first_other);
        } else {
            let n = t.trim_start_matches('#').trim_start_matches("no.");
            if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) && n.len() <= 4 && (number.is_none() || ["issue", "no", "no.", "nr", "#"].contains(&lower.get(i.wrapping_sub(1)).map(String::as_str).unwrap_or(""))) && !(year(n) && number.is_some()) {
                // the number after "Issue", "No." or "#" wins over one before it (a volume)
                let after_vol = i > 0 && ["vol", "vol.", "volume"].contains(&lower[i - 1].as_str());
                if !after_vol || number.is_none() {
                    let trimmed = n.trim_start_matches('0');
                    number = Some(if trimmed.is_empty() { "0".into() } else { trimmed.into() });
                }
                mark(i, &mut first_other);
            }
        }
        i += 1;
    }
    let title_end = first_other.unwrap_or(tokens.len());
    // (empty when the name is only a number or a date: "012.pdf")
    let title = tokens[..title_end].join(" ").trim().trim_end_matches(['-', '.']).trim().to_string();
    (title, number, date)
}

fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

fn same_number(a: Option<&str>, b: Option<&str>) -> bool {
    let n = |x: &str| x.trim_start_matches('0').to_lowercase();
    matches!((a, b), (Some(a), Some(b)) if n(a) == n(b))
}

// ------------------------------------------------------------ importing

/// A file to bring in, with what its name says (changed in the review before it's planned).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Item {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub sha1: String,
    pub title: String,
    pub number: Option<String>,
    pub date: Option<String>,
    /// The library has it already (the same file)
    pub have: bool,
    pub include: bool,
}

fn sha1_of(p: &Path, cancel: &AtomicBool) -> Result<String> {
    use sha1::Digest;
    let mut f = std::fs::File::open(p).with_context(|| format!("couldn't read {}", p.display()))?;
    let mut h = sha1::Sha1::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        if cancel.load(Ordering::Relaxed) {
            bail!("stopped");
        }
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

fn is_issue(p: &Path) -> bool {
    EXTS.contains(&crate::platforms::ext_of(&p.to_string_lossy()).as_str())
}

/// The issues in the paths picked: files themselves, folders walked; each one's magazine,
/// number and date from its name (a folder named for a magazine names the files in it
/// that don't say).
pub fn read_items(lib: &Library, paths: &[PathBuf], cancel: &AtomicBool, progress: &mut dyn FnMut(Value)) -> Result<Vec<Item>> {
    let mut files: Vec<(PathBuf, Option<String>)> = vec![];
    for p in paths {
        if p.is_file() {
            if is_issue(p) {
                files.push((p.clone(), p.parent().and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned())));
            }
            continue;
        }
        let mut stack = vec![p.clone()];
        while let Some(d) = stack.pop() {
            let mut es: Vec<PathBuf> = std::fs::read_dir(&d).into_iter().flatten().flatten().map(|e| e.path()).collect();
            es.sort();
            for e in es {
                let name = e.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                if name.starts_with('.') {
                    continue;
                }
                if e.is_dir() {
                    stack.push(e);
                } else if is_issue(&e) {
                    files.push((e, d.file_name().map(|n| n.to_string_lossy().into_owned())));
                }
            }
        }
    }
    let have: HashSet<String> = list(lib).into_iter().flat_map(|(_, m)| m.issues.into_iter().filter_map(|i| i.sha1)).collect();
    let mut v = vec![];
    for (i, (p, parent)) in files.iter().enumerate() {
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        progress(json!({ "item": i + 1, "items": files.len(), "name": name }));
        let stem = p.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let (title, number, date) = parse(&stem);
        // "012.pdf" in a folder "Nintendo Power"
        let title = if title.is_empty() { parent.clone().map(|p| parse(&p).0).filter(|t| !t.is_empty()).unwrap_or_else(|| stem.clone()) } else { title };
        let sha1 = sha1_of(p, cancel)?;
        let had = have.contains(&sha1);
        v.push(Item { path: p.display().to_string(), name, size: std::fs::metadata(p)?.len(), sha1, title, number, date, have: had, include: !had });
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

/// Plan bringing the items into their magazines' folders (copied, hardlinked on the same
/// drive unless `always_copy`), each folder's `magazine.json` written with them.
pub fn plan(lib: &Library, items: &[Item], always_copy: bool) -> Result<Plan> {
    lib.writable()?;
    let items: Vec<&Item> = items.iter().filter(|i| i.include).collect();
    if items.is_empty() {
        bail!("Choose at least one issue to import.");
    }
    struct Target {
        dir: PathBuf,
        mag: Magazine,
        taken: HashSet<String>,
        ops: Vec<Op>,
    }
    let existing = list(lib);
    let mut targets: BTreeMap<String, Target> = BTreeMap::new();
    for it in &items {
        let src = PathBuf::from(&it.path);
        if !src.is_file() {
            bail!("There's no file {}.", it.path);
        }
        let title = it.title.trim();
        if title.is_empty() {
            bail!("{} needs the name of its magazine.", it.name);
        }
        let t = targets.entry(norm(title)).or_insert_with(|| {
            // by its title, else the folder it would have, whose magazine may have been
            // renamed since: its issues are kept (H20)
            let (dir, mag) = existing.iter().find(|(_, m)| norm(&m.title) == norm(title)).cloned().unwrap_or_else(|| {
                let dir = folder(lib).join(safe_name(title));
                let mag = existing.iter().find(|(d, _)| crate::organise::key(d) == crate::organise::key(&dir)).map(|(_, m)| m.clone()).unwrap_or_else(|| Magazine { title: title.to_string(), ..Default::default() });
                (dir, mag)
            });
            let taken = std::fs::read_dir(&dir).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().to_lowercase()).collect();
            Target { dir, mag, taken, ops: vec![] }
        });
        let number = it.number.as_deref().map(str::trim).filter(|n| !n.is_empty()).map(|n| n.trim_start_matches('#').to_string());
        let date = it.date.as_deref().map(str::trim).filter(|d| !d.is_empty()).map(String::from);
        let ext = crate::platforms::ext_of(&it.name);
        let name = free(&mut t.taken, &issue_name(&t.mag.title, number.as_deref(), date.as_deref()), &ext);
        let to = t.dir.join(&name);
        let size = std::fs::metadata(&src)?.len();
        let op = if !always_copy && organise::same_drive(&src, &t.dir) { "link" } else { "copy" };
        let s: Src = Src { size, sha1: Some(it.sha1.clone()).filter(|h| !h.is_empty()), ..outside(&src) };
        t.ops.push(Op { op: op.into(), game: t.mag.title.clone(), to: to.display().to_string(), from: vec![s], names: vec![], aside: vec![], size, note: None });
        t.mag.issues.push(Issue { number, date, file: name, size, sha1: Some(it.sha1.clone()).filter(|h| !h.is_empty()), added: now(), ..Default::default() });
    }
    let mut ops = vec![];
    let n = items.len();
    for mut t in targets.into_values() {
        t.mag.issues.sort_by(|a, b| issue_order(a).cmp(&issue_order(b)));
        ops.extend(t.ops);
        let text = serde_json::to_string_pretty(&sorted(&serde_json::to_value(&t.mag)?))? + "\n";
        ops.push(text_op(&t.mag.title, &t.dir.join(MANIFEST), text, "magazine.json: its issues"));
    }
    Ok(Plan {
        id: organise::new_id(),
        created: now(),
        kind: "magazines".into(),
        label: format!("Imported {n} magazine issue{}", if n == 1 { "" } else { "s" }),
        title: format!("Import {n} magazine issue{}", if n == 1 { "" } else { "s" }),
        dat: Value::Null,
        platform: String::new(),
        mode: "copy".into(),
        format: "keep".into(),
        out: Some(folder(lib).display().to_string()),
        space: organise::space_needed(&ops),
        ops,
        games: n,
        in_place: 0,
        conflicts: vec![],
        duplicates: vec![],
        left: vec![],
        new_root: None, new_roots: vec![],
        present: vec![],
        export: None,
    })
}

/// Issues by date, then number, then file.
fn issue_order(i: &Issue) -> (String, u64, String, String) {
    let n = i.number.as_deref().and_then(|n| n.parse::<u64>().ok()).unwrap_or(u64::MAX);
    (i.date.clone().unwrap_or_else(|| "9999".into()), n, i.number.clone().unwrap_or_default(), i.file.clone())
}

/// Change an issue's number or date, or the magazine's title or publisher (its
/// `magazine.json`; the files keep their names).
pub fn edit(lib: &Library, title: &str, file: Option<&str>, patch: &Value) -> Result<Value> {
    lib.writable()?;
    let (dir, mut m) = by_title(lib, title)?;
    let text = |k: &str| patch.get(k).map(|v| v.as_str().map(str::trim).filter(|s| !s.is_empty()).map(String::from));
    match file {
        Some(f) => {
            let i = m.issues.iter_mut().find(|i| i.file == f).ok_or_else(|| anyhow!("{title} has no issue {f}."))?;
            if let Some(n) = text("number") {
                i.number = n;
            }
            if let Some(d) = text("date") {
                i.date = d;
            }
        }
        None => {
            if let Some(Some(t)) = text("title") {
                if list(lib).iter().any(|(d, o)| *d != dir && norm(&o.title) == norm(&t)) {
                    bail!("There's a magazine called {t} already.");
                }
                m.title = t;
            }
            if let Some(p) = text("publisher") {
                m.publisher = p;
            }
        }
    }
    m.issues.sort_by(|a, b| issue_order(a).cmp(&issue_order(b)));
    crate::config::write_json(&dir.join(MANIFEST), &serde_json::to_value(&m)?)?;
    Ok(json!({ "ok": true }))
}

/// A picture dropped on an issue: kept in the magazine's `covers/` as its cover.
pub fn set_cover(lib: &Library, title: &str, file: &str, picture: &Path) -> Result<()> {
    lib.writable()?;
    let (dir, mut m) = by_title(lib, title)?;
    let ext = crate::platforms::ext_of(&picture.to_string_lossy());
    if !PICTURES.contains(&ext.as_str()) {
        bail!("A cover is a picture (JPEG, PNG, WebP).");
    }
    let i = m.issues.iter_mut().find(|i| i.file == file).ok_or_else(|| anyhow!("{title} has no issue {file}."))?;
    let stem = Path::new(file).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let rel = format!("covers/{}", safe_name(&format!("{stem}.{ext}")));
    std::fs::create_dir_all(dir.join("covers"))?;
    std::fs::copy(picture, dir.join(&rel)).with_context(|| format!("couldn't copy {}", picture.display()))?;
    i.cover = Some(rel);
    crate::config::write_json(&dir.join(MANIFEST), &serde_json::to_value(&m)?)
}

// ------------------------------------------------------------ pages and covers

/// What a CBZ or CBR really is, by its first bytes (a CBR is often a ZIP).
fn comic_kind(p: &Path) -> Option<&'static str> {
    let mut b = [0u8; 6];
    let n = std::fs::File::open(p).ok()?.read(&mut b).ok()?;
    let b = &b[..n];
    if b.starts_with(b"PK") {
        Some("zip")
    } else if b.starts_with(b"Rar!") {
        Some("rar")
    } else if b.starts_with(&[0x37, 0x7A, 0xBC, 0xAF]) {
        Some("7z")
    } else {
        None
    }
}

/// "page10" after "page9": names compared with their numbers as numbers.
fn natural(a: &str, b: &str) -> std::cmp::Ordering {
    let parts = |s: &str| -> Vec<(String, u64)> {
        let mut v = vec![];
        let mut cur = String::new();
        let mut digits = false;
        for c in s.to_lowercase().chars() {
            if c.is_ascii_digit() != digits && !cur.is_empty() {
                v.push(if digits { (String::new(), cur.parse().unwrap_or(0)) } else { (cur.clone(), 0) });
                cur.clear();
            }
            digits = c.is_ascii_digit();
            cur.push(c);
        }
        if !cur.is_empty() {
            v.push(if digits { (String::new(), cur.parse().unwrap_or(0)) } else { (cur, 0) });
        }
        v
    };
    parts(a).cmp(&parts(b))
}

fn is_picture(name: &str) -> bool {
    PICTURES.contains(&crate::platforms::ext_of(name).as_str())
}

/// A comic's pages: its pictures, in order of their names.
pub fn pages(p: &Path) -> Result<Vec<String>> {
    let kind = comic_kind(p).ok_or_else(|| anyhow!("{} isn't a CBZ or CBR that can be read.", p.display()))?;
    let mut names = vec![];
    crate::archive::each_wanted_as(p, kind, &|n| is_picture(n), &AtomicBool::new(false), &mut |name, _, r| {
        names.push(name.to_string());
        std::io::copy(r, &mut std::io::sink()).map(|_| ())
    })?;
    names.sort_by(|a, b| natural(a, b));
    Ok(names)
}

/// One page of a comic (from 0): its bytes and extension.
pub fn page(p: &Path, n: usize) -> Result<(Vec<u8>, String)> {
    let all = pages(p)?;
    let name = all.get(n).ok_or_else(|| anyhow!("There's no page {} in it.", n + 1))?.clone();
    let kind = comic_kind(p).unwrap_or("zip");
    let mut out = vec![];
    crate::archive::each_wanted_as(p, kind, &|x| x == name, &AtomicBool::new(false), &mut |_, _, r| r.read_to_end(&mut out).map(|_| ()))?;
    Ok((out, crate::platforms::ext_of(&name)))
}

/// A scanned PDF's first page: the one picture on it, when it's a JPEG.
fn pdf_cover(p: &Path) -> Option<Vec<u8>> {
    let doc = lopdf::Document::load(p).ok()?;
    let first = *doc.get_pages().values().next()?;
    let imgs = doc.get_page_images(first).ok()?;
    match imgs.as_slice() {
        [one] if one.filters.as_ref().is_some_and(|f| f.iter().any(|x| x == "DCTDecode")) => Some(one.content.to_vec()),
        _ => None,
    }
}

/// A file `magazine.json` names (an issue, its cover), when it stays inside the magazine's
/// folder: the file is the user's to edit, so it could name any file (M32).
fn inside(dir: &Path, rel: &str) -> Option<PathBuf> {
    crate::library::rel_inside(rel).ok().map(|r| dir.join(r))
}

/// An issue's cover from its file: (bytes, extension), none when it has none to give.
pub fn first_page(p: &Path) -> Option<(Vec<u8>, String)> {
    match crate::platforms::ext_of(&p.to_string_lossy()).as_str() {
        "pdf" => pdf_cover(p).map(|b| (b, "jpg".to_string())),
        _ => page(p, 0).ok(),
    }
}

/// The file served for `~cover/<issue's library path>`: the cover dropped on it, else its
/// first page, made smaller once and kept in `<data dir>/thumbs/<library id>/covers/`.
pub fn cover_file(lib: &Library, data_dir: &Path, rel: &str) -> Result<PathBuf> {
    let issue = lib.resolve(rel)?;
    let dir = issue.parent().ok_or_else(|| anyhow!("no folder"))?;
    let m: Magazine = serde_json::from_value(read_json_object(&dir.join(MANIFEST))).unwrap_or_default();
    let name = issue.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if let Some(p) = m.issues.iter().find(|i| i.file == name).and_then(|i| inside(dir, i.cover.as_deref()?)) {
        if p.is_file() {
            return Ok(p);
        }
    }
    let st = std::fs::metadata(&issue).with_context(|| format!("There's no issue {rel}."))?;
    let stamp = format!("{rel}|{}|{:?}", st.len(), st.modified().ok());
    let id = {
        use sha2::Digest;
        hex::encode(&sha2::Sha256::digest(stamp.as_bytes())[..12])
    };
    let cache = data_dir.join("thumbs").join(lib.id().unwrap_or_default()).join("covers");
    let at = cache.join(format!("{id}.jpg"));
    let plain = cache.join(format!("{id}.svg"));
    for p in [&at, &plain] {
        if p.is_file() {
            return Ok(p.clone());
        }
    }
    std::fs::create_dir_all(&cache)?;
    let picture = first_page(&issue).and_then(|(bytes, _)| ::image::load_from_memory(&bytes).ok());
    let Some(img) = picture else {
        // no picture to make it from: a plain cover with the magazine's name and the number,
        // until one is chosen
        let i = m.issues.iter().find(|i| i.file == name);
        let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
        let number = i.and_then(|i| i.number.clone()).map(|n| format!("No. {n}")).unwrap_or_default();
        let date = i.and_then(|i| i.date.clone()).unwrap_or_default();
        let svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 300 400\" width=\"300\" height=\"400\">\
             <rect width=\"300\" height=\"400\" fill=\"#e4e8ec\"/><rect x=\"12\" y=\"12\" width=\"276\" height=\"376\" fill=\"none\" stroke=\"#8a96a3\" stroke-width=\"2\"/>\
             <text x=\"150\" y=\"150\" font-family=\"sans-serif\" font-size=\"24\" font-weight=\"700\" fill=\"#3c4650\" text-anchor=\"middle\">{}</text>\
             <text x=\"150\" y=\"215\" font-family=\"sans-serif\" font-size=\"44\" font-weight=\"700\" fill=\"#1d242b\" text-anchor=\"middle\">{}</text>\
             <text x=\"150\" y=\"260\" font-family=\"sans-serif\" font-size=\"20\" fill=\"#5a6672\" text-anchor=\"middle\">{}</text></svg>",
            esc(&m.title.chars().take(22).collect::<String>()),
            esc(&number),
            esc(&date)
        );
        std::fs::write(&plain, svg)?;
        return Ok(plain);
    };
    let img = if img.width().max(img.height()) > 640 { img.thumbnail(640, 640) } else { img };
    let tmp = at.with_extension("tmp");
    img.to_rgb8().save_with_format(&tmp, ::image::ImageFormat::Jpeg)?;
    std::fs::rename(&tmp, &at)?;
    Ok(at)
}

/// The file served for `~page/<n>/<issue's library path>`: page n (from 1) of a comic,
/// kept in `<data dir>/thumbs/<library id>/pages/` once read.
pub fn page_file(lib: &Library, data_dir: &Path, n: usize, rel: &str) -> Result<PathBuf> {
    let issue = lib.resolve(rel)?;
    let st = std::fs::metadata(&issue).with_context(|| format!("There's no issue {rel}."))?;
    let stamp = format!("{rel}|{}|{:?}", st.len(), st.modified().ok());
    let id = {
        use sha2::Digest;
        hex::encode(&sha2::Sha256::digest(stamp.as_bytes())[..12])
    };
    let cache = data_dir.join("thumbs").join(lib.id().unwrap_or_default()).join("pages");
    if let Some(e) = std::fs::read_dir(&cache).into_iter().flatten().flatten().find(|e| e.file_name().to_string_lossy().starts_with(&format!("{id}-{n}."))) {
        return Ok(e.path());
    }
    let (bytes, ext) = page(&issue, n.saturating_sub(1))?;
    std::fs::create_dir_all(&cache)?;
    let at = cache.join(format!("{id}-{n}.{ext}"));
    std::fs::write(&at, bytes)?;
    Ok(at)
}

// ------------------------------------------------------------ the page

/// The Magazines page: each magazine with its issues (their documents' ids, to read them),
/// and how each stands against the lists.
pub fn overview(ix: &Index, lib: &Library) -> Result<Value> {
    let conn = ix.conn();
    let audit = audit(lib).unwrap_or_default();
    let mut out = vec![];
    for (dir, m) in list(lib) {
        let mut issues = vec![];
        for i in &m.issues {
            let rel = lib.relative(&dir.join(&i.file)).unwrap_or_default();
            let doc: Option<i64> = conn.query_row("SELECT id FROM docs WHERE path = ?1", [&rel], |r| r.get(0)).ok();
            issues.push(json!({
                "number": i.number, "date": i.date, "file": i.file, "path": rel, "size": i.size, "doc": doc, "title": issue_title(&m.title, i),
                "cover": i.cover, "gone": !dir.join(&i.file).is_file(),
            }));
        }
        let a = audit.iter().find(|a| norm(a["title"].as_str().unwrap_or("")) == norm(&m.title));
        out.push(json!({
            "title": m.title, "publisher": m.publisher, "folder": lib.relative(&dir), "issues": issues,
            "missing": a.map(|a| a["missing"].clone()).unwrap_or(json!([])), "listed": a.map(|a| a["listed"].clone()).unwrap_or(json!(0)),
        }));
    }
    // magazines a list has that the library has no issue of
    for a in &audit {
        if !out.iter().any(|m| norm(m["title"].as_str().unwrap_or("")) == norm(a["title"].as_str().unwrap_or(""))) {
            out.push(json!({ "title": a["title"], "publisher": null, "folder": null, "issues": [], "missing": a["missing"], "listed": a["listed"], "none": true }));
        }
    }
    Ok(json!({ "magazines": out, "lists": lists(lib) }))
}

// ------------------------------------------------------------ lists to audit against

/// An issue a list says there is.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Listed {
    pub title: String,
    pub number: Option<String>,
    pub date: Option<String>,
    pub sha1: Option<String>,
}

/// The lists kept.
pub fn lists(lib: &Library) -> Vec<Value> {
    let mut v: Vec<Value> = std::fs::read_dir(lists_dir(lib))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_file())
        .map(|e| json!({ "name": e.file_name().to_string_lossy(), "issues": read_list(&e.path()).map(|l| l.len()).unwrap_or(0) }))
        .collect();
    v.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    v
}

fn csv_fields(line: &str) -> Vec<String> {
    let mut v = vec![];
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => v.push(std::mem::take(&mut cur).trim().to_string()),
            _ => cur.push(c),
        }
    }
    v.push(cur.trim().to_string());
    v
}

/// A list's issues: a CSV of magazine, number and date (a header line is passed over), or a
/// DAT whose games are issues (their names read as file names are, their files' SHA1s kept).
pub fn read_list(p: &Path) -> Result<Vec<Listed>> {
    let ext = crate::platforms::ext_of(&p.to_string_lossy());
    if ext == "csv" || ext == "txt" {
        let text = std::fs::read_to_string(p).with_context(|| format!("couldn't read {}", p.display()))?;
        let mut v = vec![];
        for (i, line) in text.lines().enumerate() {
            let f = csv_fields(line);
            if f.iter().all(String::is_empty) {
                continue;
            }
            if i == 0 && f.first().is_some_and(|x| ["magazine", "title", "name"].contains(&x.to_lowercase().as_str())) {
                continue;
            }
            let opt = |k: usize| f.get(k).map(|s| s.trim_start_matches('#').trim().to_string()).filter(|s| !s.is_empty());
            v.push(Listed { title: f[0].clone(), number: opt(1).map(|n| n.trim_start_matches('0').to_string()).map(|n| if n.is_empty() { "0".into() } else { n }), date: opt(2), sha1: None });
        }
        return Ok(v);
    }
    let dats = crate::dat::read_file(p)?;
    Ok(dats
        .into_iter()
        .flat_map(|d| d.games)
        .map(|g| {
            let (title, number, date) = parse(&g.name);
            Listed { title: if title.is_empty() { g.name.clone() } else { title }, number, date, sha1: g.roms.first().and_then(|r| r.sha1.clone()).map(|h| h.to_lowercase()) }
        })
        .collect())
}

/// Keep a list (copied into `_library/dats/magazines/`).
pub fn import_list(lib: &Library, p: &Path) -> Result<Value> {
    lib.writable()?;
    let l = read_list(p)?;
    if l.is_empty() {
        bail!("{} lists no issues.", p.display());
    }
    let name = p.file_name().ok_or_else(|| anyhow!("no file"))?;
    std::fs::create_dir_all(lists_dir(lib))?;
    std::fs::copy(p, lists_dir(lib).join(name))?;
    Ok(json!({ "name": name.to_string_lossy(), "issues": l.len() }))
}

pub fn remove_list(lib: &Library, name: &str) -> Result<()> {
    lib.writable()?;
    if name.contains(['/', '\\']) || name.starts_with('.') {
        bail!("There's no list {name}.");
    }
    std::fs::remove_file(lists_dir(lib).join(name)).map_err(|_| anyhow!("There's no list {name}."))
}

/// Each magazine the lists name: how many issues they list, and those the library doesn't
/// have (matched by the file's SHA1, else by magazine and number, else by date).
pub fn audit(lib: &Library) -> Result<Vec<Value>> {
    let mut wanted: BTreeMap<String, (String, Vec<Listed>)> = BTreeMap::new();
    for e in std::fs::read_dir(lists_dir(lib)).into_iter().flatten().flatten() {
        for l in read_list(&e.path()).unwrap_or_default() {
            let w = wanted.entry(norm(&l.title)).or_insert_with(|| (l.title.clone(), vec![]));
            // the same issue in two lists counts once
            if !w.1.iter().any(|x| same_number(x.number.as_deref(), l.number.as_deref()) && x.date == l.date) {
                w.1.push(l);
            }
        }
    }
    let have = list(lib);
    let mut out = vec![];
    for (k, (title, issues)) in wanted {
        let mine: Vec<&Issue> = have.iter().filter(|(_, m)| norm(&m.title) == k).flat_map(|(_, m)| m.issues.iter()).collect();
        let found = |l: &Listed| {
            mine.iter().any(|i| {
                (l.sha1.is_some() && i.sha1.as_deref().map(str::to_lowercase) == l.sha1)
                    || (l.number.is_some() && same_number(i.number.as_deref(), l.number.as_deref()))
                    || (l.number.is_none() && l.date.is_some() && i.date == l.date)
            })
        };
        let missing: Vec<Value> = issues.iter().filter(|l| !found(l)).map(|l| json!({ "number": l.number, "date": l.date, "title": issue_name(&title, l.number.as_deref(), l.date.as_deref()) })).collect();
        out.push(json!({ "title": title, "listed": issues.len(), "missing": missing }));
    }
    Ok(out)
}

/// The issues missing (all magazines', or one's) as a CSV list, written to `to`.
pub fn save_missing(lib: &Library, title: Option<&str>, to: &Path) -> Result<usize> {
    let quote = |s: &str| if s.contains([',', '"']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() };
    let mut text = String::from("magazine,number,date\n");
    let mut n = 0;
    for a in audit(lib)? {
        let t = a["title"].as_str().unwrap_or("");
        if title.is_some_and(|x| norm(x) != norm(t)) {
            continue;
        }
        for m in a["missing"].as_array().into_iter().flatten() {
            text.push_str(&format!("{},{},{}\n", quote(t), m["number"].as_str().unwrap_or(""), m["date"].as_str().unwrap_or("")));
            n += 1;
        }
    }
    std::fs::write(to, text).with_context(|| format!("couldn't write {}", to.display()))?;
    Ok(n)
}

/// For an export: each issue's file and cover, and where it goes under `out`
/// (`Magazines/<Title>/`).
pub fn export_files(lib: &Library, out: &Path) -> Vec<(PathBuf, PathBuf, String)> {
    let mut v = vec![];
    for (dir, m) in list(lib) {
        let name = dir.file_name().map(|n| n.to_os_string()).unwrap_or_default();
        let to_dir = out.join(FOLDER).join(&name);
        for i in &m.issues {
            if let Some(from) = inside(&dir, &i.file).filter(|p| p.is_file()) {
                v.push((from, to_dir.join(&i.file), issue_title(&m.title, i)));
            }
            if let Some(c) = &i.cover {
                if let Some(from) = inside(&dir, c).filter(|p| p.is_file()) {
                    v.push((from, to_dir.join(c), issue_title(&m.title, i)));
                }
            }
        }
    }
    v
}

/// What a library folder's documents are, when it's a magazine's: its issues, as the
/// documents index lists them (kind `magazine`, titled by magazine, number and date).
pub fn docs_manifest(dir: &Path) -> Option<Value> {
    let m: Magazine = serde_json::from_value(read_json_object(&dir.join(MANIFEST))).ok()?;
    let mut files = Map::new();
    for i in &m.issues {
        files.insert(i.file.clone(), json!({ "kind": "magazine", "title": issue_title(&m.title, i) }));
    }
    Some(json!({ "files": files }))
}

/// Whether a library path is in the magazines' folder.
pub fn is_magazine_path(lib: &Library, p: &Path) -> bool {
    key(p).starts_with(&format!("{}/", key(&folder(lib)).trim_end_matches('/')))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_files_name_says_its_magazine_number_and_date() {
        let p = |s: &str| {
            let (t, n, d) = parse(s);
            (t, n.unwrap_or_default(), d.unwrap_or_default())
        };
        assert_eq!(p("Nintendo Power 012 (1990-05)"), ("Nintendo Power".into(), "12".into(), "1990-05".into()));
        assert_eq!(p("EGM_045_Apr1993"), ("EGM".into(), "45".into(), "1993-04".into()));
        assert_eq!(p("Edge #100 - July 2001"), ("Edge".into(), "100".into(), "2001-07".into()));
        assert_eq!(p("Retro Gamer Issue 7 (2004)"), ("Retro Gamer".into(), "7".into(), "2004".into()));
        assert_eq!(p("GamePro Vol 3 No 4"), ("GamePro".into(), "4".into(), "".into()));
        assert_eq!(p("Mean Machines (October 1990)"), ("Mean Machines".into(), "".into(), "1990-10".into()));
        // H21: names in other scripts are cut by letters, not bytes
        assert_eq!(p("週刊ファミ通 0123 (1995-01)"), ("週刊ファミ通".into(), "123".into(), "1995-01".into()));
        assert_eq!(p("ファミコン通信ゲーム 0002 (1990-01)"), ("ファミコン通信ゲーム".into(), "2".into(), "1990-01".into()));
        assert_eq!(issue_name("Nintendo Power", Some("12"), Some("1990-05")), "Nintendo Power 012 (1990-05)");
        assert_eq!(csv_fields("\"Edge, the\",12,2001-07"), ["Edge, the", "12", "2001-07"]);
        let mut v = vec!["p10.jpg", "p2.jpg", "p1.jpg"];
        v.sort_by(|a, b| natural(a, b));
        assert_eq!(v, ["p1.jpg", "p2.jpg", "p10.jpg"]);
    }

    fn png(seed: u8) -> Vec<u8> {
        let mut out = std::io::Cursor::new(vec![]);
        ::image::RgbImage::from_pixel(40, 60, ::image::Rgb([seed, 20, 200])).write_to(&mut out, ::image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    fn comic(path: &Path, pages: &[(&str, Vec<u8>)]) {
        let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (n, b) in pages {
            z.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap();
            std::io::Write::write_all(&mut z, b).unwrap();
        }
        z.finish().unwrap();
    }

    /// A PDF of one page that's one JPEG, as a scan is.
    fn scan_pdf(path: &Path, jpeg: &[u8]) {
        use lopdf::{dictionary, Object, Stream};
        let mut doc = lopdf::Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let img = doc.add_object(Stream::new(dictionary! { "Type" => "XObject", "Subtype" => "Image", "Width" => 40, "Height" => 60, "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8, "Filter" => "DCTDecode" }, jpeg.to_vec()));
        let content = doc.add_object(Stream::new(dictionary! {}, b"q 40 0 0 60 0 0 cm /Im0 Do Q".to_vec()));
        let page = doc.add_object(dictionary! { "Type" => "Page", "Parent" => pages_id, "MediaBox" => vec![0.into(), 0.into(), 40.into(), 60.into()], "Contents" => content, "Resources" => dictionary! { "XObject" => dictionary! { "Im0" => img } } });
        doc.objects.insert(pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 }));
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog);
        doc.save(path).unwrap();
    }

    #[test]
    fn a_scanned_pdfs_cover_is_its_first_pages_picture() {
        let dir = crate::library::tests::temp_dir("magazine-pdf");
        let mut jpeg = std::io::Cursor::new(vec![]);
        ::image::RgbImage::from_pixel(40, 60, ::image::Rgb([200, 10, 10])).write_to(&mut jpeg, ::image::ImageFormat::Jpeg).unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        scan_pdf(&dir.join("scan.pdf"), jpeg.get_ref());
        let (bytes, ext) = first_page(&dir.join("scan.pdf")).unwrap();
        assert_eq!((bytes, ext.as_str()), (jpeg.into_inner(), "jpg"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn magazines_are_brought_in_read_audited_exported_and_undone() {
        let _turn = crate::platforms::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (dir, lib, ix) = crate::collection::tests::fixture("magazines");
        let from = dir.join("Downloads").join("Nintendo Power");
        std::fs::create_dir_all(&from).unwrap();
        comic(&from.join("Nintendo Power 012 (1990-05).cbz"), &[("page2.png", png(2)), ("page10.png", png(10)), ("page1.png", png(1))]);
        // a CBR that's a ZIP inside, as many are
        comic(&from.join("Nintendo_Power_013_Jun1990.cbr"), &[("01.png", png(13))]);
        std::fs::write(from.join("014.pdf"), b"%PDF-1.4 not a scan").unwrap();
        std::fs::write(from.join("notes.txt"), b"not an issue").unwrap();
        let cancel = AtomicBool::new(false);
        let items = read_items(&lib, &[from.clone()], &cancel, &mut |_| {}).unwrap();
        let mut seen: Vec<(String, Option<String>, Option<String>)> = items.iter().map(|i| (i.title.clone(), i.number.clone(), i.date.clone())).collect();
        seen.sort();
        assert_eq!(seen, [
            ("Nintendo Power".to_string(), Some("12".to_string()), Some("1990-05".to_string())),
            ("Nintendo Power".to_string(), Some("13".to_string()), Some("1990-06".to_string())),
            ("Nintendo Power".to_string(), Some("14".to_string()), None),
        ]);
        let p = plan(&lib, &items, true).unwrap();
        assert_eq!((p.kind.as_str(), p.games), ("magazines", 3));
        let r = organise::run(&ix, &lib, &p, &cancel, &mut |_| {}).unwrap();
        assert_eq!(r["failed"], json!([]), "{r}");
        let mag = folder(&lib).join("Nintendo Power");
        let mut names: Vec<String> = std::fs::read_dir(&mag).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["Nintendo Power 012 (1990-05).cbz", "Nintendo Power 013 (1990-06).cbr", "Nintendo Power 014.pdf", "magazine.json"]);
        assert!(from.join("014.pdf").is_file(), "copied, not moved");
        // they're documents of the kind magazine, and the ones there already aren't brought in twice
        crate::docs::sync(&ix, &lib).unwrap();
        let found = crate::docs::search(&ix, "", "magazine", "", "", 0, 50).unwrap();
        assert_eq!(found["total"], 3, "{found}");
        assert!(found["items"].as_array().unwrap().iter().any(|d| d["title"] == "Nintendo Power 12 (1990-05)" && d["view"] == "comic"));
        let again = read_items(&lib, &[from.clone()], &cancel, &mut |_| {}).unwrap();
        assert!(again.iter().all(|i| i.have && !i.include));
        assert!(plan(&lib, &again, true).is_err());
        // pages in order, the first the cover; a PDF that isn't a scan has none until one's dropped on it
        let cbz = mag.join("Nintendo Power 012 (1990-05).cbz");
        assert_eq!(pages(&cbz).unwrap(), ["page1.png", "page2.png", "page10.png"]);
        let data = dir.join("data");
        let rel = lib.relative(&cbz).unwrap();
        let c = cover_file(&lib, &data, &rel).unwrap();
        assert!(c.starts_with(&data) && ::image::open(&c).unwrap().width() == 40);
        assert_eq!(std::fs::read(page_file(&lib, &data, 2, &rel).unwrap()).unwrap(), png(2));
        assert_eq!(pages(&mag.join("Nintendo Power 013 (1990-06).cbr")).unwrap(), ["01.png"]);
        let pdf = lib.relative(&mag.join("Nintendo Power 014.pdf")).unwrap();
        // a PDF without a picture on its first page gets a plain cover with its number
        let plain = cover_file(&lib, &data, &pdf).unwrap();
        assert!(plain.extension().unwrap() == "svg" && std::fs::read_to_string(&plain).unwrap().contains("No. 14"));
        std::fs::write(dir.join("cover.png"), png(99)).unwrap();
        set_cover(&lib, "nintendo power", "Nintendo Power 014.pdf", &dir.join("cover.png")).unwrap();
        assert_eq!(cover_file(&lib, &data, &pdf).unwrap(), mag.join("covers/Nintendo Power 014.png"));
        // M32: a cover magazine.json names outside the magazine's folder isn't served
        let manifest = mag.join(MANIFEST);
        let kept = std::fs::read(&manifest).unwrap();
        std::fs::write(dir.join("secret.png"), png(7)).unwrap();
        let mut crafted: Magazine = serde_json::from_slice(&kept).unwrap();
        crafted.issues[2].cover = Some("../../../../secret.png".into());
        std::fs::write(&manifest, serde_json::to_vec(&crafted).unwrap()).unwrap();
        assert!(!cover_file(&lib, &data, &pdf).unwrap().starts_with(&dir.join("secret.png")));
        assert!(export_files(&lib, &dir.join("Out")).iter().all(|(from, _, _)| from.starts_with(&mag)));
        std::fs::write(&manifest, &kept).unwrap();
        // an issue's number put right
        edit(&lib, "Nintendo Power", Some("Nintendo Power 014.pdf"), &json!({ "date": "1990-07" })).unwrap();
        assert_eq!(by_title(&lib, "Nintendo Power").unwrap().1.issues[2].date.as_deref(), Some("1990-07"));
        // audited against a list: 12 and 15 listed, 15 missing; and a magazine none of whose issues you have
        let csv = dir.join("np.csv");
        std::fs::write(&csv, "magazine,number,date\nNintendo Power,012,1990-05\nNintendo Power,15,1990-08\n\"Edge, the\",1,1993-10\n").unwrap();
        assert_eq!(import_list(&lib, &csv).unwrap()["issues"], 3);
        let o = overview(&ix, &lib).unwrap();
        let np = o["magazines"].as_array().unwrap().iter().find(|m| m["title"] == "Nintendo Power").unwrap().clone();
        assert_eq!((np["issues"].as_array().unwrap().len(), np["listed"].as_u64()), (3, Some(2)), "{np}");
        assert_eq!(np["missing"], json!([{ "number": "15", "date": "1990-08", "title": "Nintendo Power 015 (1990-08)" }]));
        assert!(np["issues"][0]["doc"].is_i64());
        assert!(o["magazines"].as_array().unwrap().iter().any(|m| m["title"] == "Edge, the" && m["none"] == true));
        let out = dir.join("missing.csv");
        assert_eq!(save_missing(&lib, None, &out).unwrap(), 2);
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "magazine,number,date\n\"Edge, the\",1,1993-10\nNintendo Power,15,1990-08\n");
        // an export takes them, each magazine in a folder of its own
        let c = crate::collection::save(&lib, crate::collection::Collection { name: "Mags".into(), rules: crate::collection::Rules { platforms: vec!["gb".into()], ..Default::default() }, ..serde_json::from_value(json!({ "name": "x" })).unwrap() }).unwrap();
        let to = dir.join("Card");
        let o = crate::export::Options { collection: c.id.clone(), target: "folder".into(), out: to.clone(), carry: crate::export::Carry { magazines: true, ..Default::default() }, ..Default::default() };
        let ep = crate::export::plan(&ix, &lib, &o).unwrap();
        let r = organise::run(&ix, &lib, &ep, &cancel, &mut |_| {}).unwrap();
        assert_eq!(r["failed"], json!([]), "{r}");
        assert!(to.join("Magazines/Nintendo Power/Nintendo Power 012 (1990-05).cbz").is_file() && to.join("Magazines/Nintendo Power/covers/Nintendo Power 014.png").is_file());
        // H20: renamed, then another issue imported under its old name: the issues stay
        edit(&lib, "Nintendo Power", None, &json!({ "title": "Nintendo Power (US)" })).unwrap();
        let more = dir.join("More");
        std::fs::create_dir_all(&more).unwrap();
        comic(&more.join("Nintendo Power 016 (1990-09).cbz"), &[("01.png", png(16))]);
        let items = read_items(&lib, &[more.join("Nintendo Power 016 (1990-09).cbz")], &cancel, &mut |_| {}).unwrap();
        assert_eq!(items[0].title, "Nintendo Power");
        let p16 = plan(&lib, &items, true).unwrap();
        organise::run(&ix, &lib, &p16, &cancel, &mut |_| {}).unwrap();
        let (_, m) = by_title(&lib, "Nintendo Power (US)").unwrap();
        assert_eq!(m.issues.len(), 4, "{m:?}");
        organise::undo(&ix, &lib, &p16.id, &cancel, &mut |_| {}).unwrap();
        // Undo of the import (after the export's) takes the issues out again
        organise::undo(&ix, &lib, &ep.id, &cancel, &mut |_| {}).unwrap();
        organise::undo(&ix, &lib, &p.id, &cancel, &mut |_| {}).unwrap();
        assert!(!mag.join("Nintendo Power 012 (1990-05).cbz").exists());
        crate::docs::sync(&ix, &lib).unwrap();
        assert_eq!(crate::docs::search(&ix, "", "magazine", "", "", 0, 50).unwrap()["total"], 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
