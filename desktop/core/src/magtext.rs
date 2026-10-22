//! Magazines' text, and the games each issue covers (docs/PLAN.md, D145). Each issue's text
//! is read page by page, once for each file (by its size and time), and kept beside it in
//! `Magazines/<Title>/.text/<file>.json`: the documents index searches it, and the viewer
//! shows a page's text with the words searched for marked. A PDF's own text is used where
//! a page has some; any other page (a scanned PDF's, a CBZ's or CBR's) is read by text
//! recognition: Tesseract when it's found (Settings › Tools, D14), else Windows' own
//! (Windows.Media.Ocr, in the languages Windows has, nothing to install).
//!
//! Each page's text is then looked through for the titles of the library's games, and of
//! the games their systems' DATs list, so a game you don't have is found too: whole
//! titles, compared as D120 compares names. What's found is kept with the issue in
//! `magazine.json`, ticked when the title is sure (two words or more, long enough not to
//! be an everyday phrase); a tick changed or a game added by hand stays when the issue is
//! read again. A game's page lists the issues that cover it.

use crate::index::Index;
use crate::library::{now, Library};
use crate::magazines::{self, IssueGame, Magazine, MANIFEST};
use crate::variants::{split_name, words, COMMON};
use anyhow::{anyhow, bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Where an issue's text is kept, in its magazine's folder.
pub const TEXT_DIR: &str = ".text";
/// The script Windows PowerShell runs for Windows' own text recognition.
const WINDOWS_OCR: &str = include_str!("ocr.ps1");
/// A page with fewer letters and digits than this in a PDF's own text is read as a picture.
const FEW: usize = 20;

/// An issue's text, kept beside it.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Kept {
    /// The file's size and time when it was read
    pub size: u64,
    pub modified: i64,
    pub read: String,
    /// Each page's text, from the first
    pub pages: Vec<String>,
    /// The pages (from 1) text recognition read, not the PDF's own text
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub recognised: Vec<u32>,
    /// The pages (from 1) that couldn't be read, and why (`note`)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unread: Vec<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// What read the pictures: "tesseract" or "windows"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reader: Option<String>,
}

fn stamp(p: &Path) -> Option<(u64, i64)> {
    let m = std::fs::metadata(p).ok()?;
    let t = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as i64;
    Some((m.len(), t))
}

/// Where an issue's text is kept.
pub fn text_path(issue: &Path) -> Option<PathBuf> {
    let name = issue.file_name()?.to_string_lossy().into_owned();
    Some(issue.parent()?.join(TEXT_DIR).join(format!("{name}.json")))
}

/// An issue's kept text, when it was read from the file as it is now.
pub fn kept(issue: &Path) -> Option<Kept> {
    let k: Kept = serde_json::from_slice(&std::fs::read(text_path(issue)?).ok()?).ok()?;
    (stamp(issue)? == (k.size, k.modified)).then_some(k)
}

/// The text the documents index keeps for an issue (at most 1 MB), when it's been read.
pub fn kept_text(issue: &Path) -> Option<String> {
    let k = kept(issue)?;
    let mut s = k.pages.join("\n");
    if s.len() > 1 << 20 {
        let mut cut = 1 << 20;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
    }
    Some(s)
}

// ------------------------------------------------------------ text recognition

/// What reads the text in pictures.
pub enum Reader {
    Tesseract(PathBuf),
    /// Windows' own, through Windows PowerShell
    Windows(PathBuf),
}

impl Reader {
    pub fn id(&self) -> &'static str {
        match self {
            Reader::Tesseract(_) => "tesseract",
            Reader::Windows(_) => "windows",
        }
    }
}

/// Tesseract when it's found, else Windows' own text recognition; none on Linux without
/// Tesseract.
pub fn reader() -> Option<Reader> {
    if let Some(p) = crate::tools::find("tesseract") {
        return Some(Reader::Tesseract(p));
    }
    windows_powershell().map(Reader::Windows)
}

/// Windows PowerShell 5.1, which every Windows 10 and 11 has (PowerShell 7 can't call
/// Windows' own interfaces this way).
fn windows_powershell() -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    Some(root.join("System32").join("WindowsPowerShell").join("v1.0").join("powershell.exe")).filter(|p| p.is_file())
}

/// A page for text recognition: a picture, or a PDF's page (which Windows draws itself).
enum Picture {
    File(PathBuf),
    PdfPage(PathBuf),
}

/// What text recognition gave: each page's text (from 1), the pages it couldn't read, and
/// why when it read none.
#[derive(Default)]
struct Recognised {
    text: BTreeMap<u32, String>,
    failed: Vec<u32>,
    note: Option<String>,
}

const NO_LANGUAGE: &str = "Windows has no language it can read text in: add one in Windows' Settings › Time & language › Language (its \"Optical character recognition\" part), or put Tesseract in Game Repo's tools folder (Settings › Tools).";

fn recognise(reader: &Reader, pages: &[(u32, Picture)], cancel: &AtomicBool, progress: &mut dyn FnMut(u32)) -> Result<Recognised> {
    let mut out = Recognised::default();
    match reader {
        Reader::Tesseract(exe) => {
            for (n, p) in pages {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                progress(*n);
                let Picture::File(file) = p else {
                    out.failed.push(*n);
                    continue;
                };
                let r = crate::tools::command(exe).arg(file).arg("stdout").stdin(std::process::Stdio::null()).output();
                match r {
                    Ok(o) if o.status.success() => {
                        out.text.insert(*n, String::from_utf8_lossy(&o.stdout).trim().to_string());
                    }
                    Ok(o) => {
                        out.failed.push(*n);
                        out.note.get_or_insert_with(|| format!("Tesseract couldn't read page {n}: {}", String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or("").trim()));
                    }
                    Err(e) => bail!("Couldn't run Tesseract ({}): {e}", exe.display()),
                }
            }
        }
        Reader::Windows(powershell) => {
            let dir = scratch()?;
            let list: Vec<Value> = pages
                .iter()
                .map(|(n, p)| match p {
                    Picture::File(f) => json!({ "page": n, "image": f }),
                    Picture::PdfPage(f) => json!({ "page": n, "pdf": f }),
                })
                .collect();
            let (script, input) = (dir.0.join("ocr.ps1"), dir.0.join("pages.json"));
            // with a byte order mark, as Windows PowerShell 5.1 reads a script's own text by it
            std::fs::write(&script, [&b"\xEF\xBB\xBF"[..], WINDOWS_OCR.as_bytes()].concat())?;
            std::fs::write(&input, serde_json::to_vec(&json!({ "pages": list }))?)?;
            let mut child = crate::tools::command(powershell)
                .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
                .arg(&script)
                .arg("-List")
                .arg(&input)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn()
                .with_context(|| format!("Couldn't run Windows PowerShell ({})", powershell.display()))?;
            let stdout = child.stdout.take().ok_or_else(|| anyhow!("no output"))?;
            for line in BufReader::new(stdout).split(b'\n') {
                if cancel.load(Ordering::Relaxed) {
                    let _ = child.kill();
                    break;
                }
                let line = line?;
                let line = String::from_utf8_lossy(&line);
                let Ok(v) = serde_json::from_str::<Value>(line.trim_start_matches('\u{feff}').trim()) else { continue };
                match (v["page"].as_u64(), v["error"].as_str()) {
                    (None, Some("language")) => out.note = Some(NO_LANGUAGE.into()),
                    (None, Some(_)) => out.note = Some(format!("Windows' text recognition didn't start: {}", v["detail"].as_str().unwrap_or("").trim())),
                    (Some(n), None) => {
                        progress(n as u32);
                        out.text.insert(n as u32, v["text"].as_str().unwrap_or("").to_string());
                    }
                    (Some(n), Some(e)) => {
                        out.failed.push(n as u32);
                        out.note.get_or_insert_with(|| format!("Couldn't read page {n}: {}", e.trim()));
                    }
                    _ => {}
                }
            }
            let _ = child.wait();
        }
    }
    // the pages it never came to (it stopped, or didn't start)
    for (n, _) in pages {
        if !out.text.contains_key(n) && !out.failed.contains(n) {
            out.failed.push(*n);
        }
    }
    out.failed.sort_unstable();
    Ok(out)
}

/// A folder for the pages of one issue while they're read, removed when it's dropped.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch() -> Result<Scratch> {
    use std::sync::atomic::AtomicU64;
    static N: AtomicU64 = AtomicU64::new(0);
    let d = std::env::temp_dir().join(format!("gamerepo-ocr-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&d)?;
    Ok(Scratch(d))
}

/// The biggest picture on a PDF's page as a file Tesseract reads (a JPEG as it is; 8-bit
/// grey or colour pixels as a PNG), or none (no picture, or one in a kind it can't take).
fn pdf_picture(doc: &lopdf::Document, page: lopdf::ObjectId, to: &Path) -> Option<PathBuf> {
    let imgs = doc.get_page_images(page).ok()?;
    let img = imgs.iter().max_by_key(|i| i.width * i.height)?;
    let filters = img.filters.clone().unwrap_or_default();
    if filters.iter().any(|f| f == "DCTDecode") {
        let at = to.with_extension("jpg");
        std::fs::write(&at, img.content).ok()?;
        return Some(at);
    }
    if filters.iter().any(|f| f != "FlateDecode") || img.bits_per_component != Some(8) {
        return None;
    }
    let bytes = doc.get_object(img.id).ok()?.as_stream().ok()?.decompressed_content().ok()?;
    let (w, h) = (u32::try_from(img.width).ok()?, u32::try_from(img.height).ok()?);
    let picture: ::image::DynamicImage = match img.color_space.as_deref() {
        Some("DeviceGray") => ::image::GrayImage::from_raw(w, h, bytes)?.into(),
        Some("DeviceRGB") => ::image::RgbImage::from_raw(w, h, bytes)?.into(),
        _ => return None,
    };
    let at = to.with_extension("png");
    picture.save_with_format(&at, ::image::ImageFormat::Png).ok()?;
    Some(at)
}

fn enough(text: &str) -> bool {
    text.chars().filter(|c| c.is_alphanumeric()).count() >= FEW
}

/// Read an issue's text, page by page. `progress(page, pages)`.
pub fn read(issue: &Path, reader: Option<&Reader>, cancel: &AtomicBool, progress: &mut dyn FnMut(u32, u32)) -> Result<Kept> {
    let (size, modified) = stamp(issue).ok_or_else(|| anyhow!("There's no file {}.", issue.display()))?;
    let mut k = Kept { size, modified, read: now(), reader: reader.map(|r| r.id().to_string()), ..Default::default() };
    let tmp = scratch()?;
    let mut pictures: Vec<(u32, Picture)> = vec![];
    match crate::platforms::ext_of(&issue.to_string_lossy()).as_str() {
        "pdf" => {
            let p = issue.to_path_buf();
            let windows = matches!(reader, Some(Reader::Windows(_)));
            let dir = tmp.0.clone();
            // some PDFs upset the reader: they give no text, not a crash
            let got = std::panic::catch_unwind(move || -> Option<(Vec<String>, Vec<(u32, Picture)>)> {
                let doc = lopdf::Document::load(&p).ok()?;
                let mut pages = vec![];
                let mut pictures = vec![];
                for (n, id) in doc.get_pages() {
                    let t = doc.extract_text(&[n]).unwrap_or_default();
                    if !enough(&t) {
                        if windows {
                            pictures.push((n, Picture::PdfPage(p.clone())));
                        } else if let Some(f) = pdf_picture(&doc, id, &dir.join(format!("page-{n}"))) {
                            pictures.push((n, Picture::File(f)));
                        } else if t.trim().is_empty() {
                            pictures.push((n, Picture::PdfPage(p.clone())));
                        }
                    }
                    pages.push(t.trim().to_string());
                }
                Some((pages, pictures))
            });
            let (pages, pics) = got.ok().flatten().ok_or_else(|| anyhow!("{} isn't a PDF that can be read.", issue.display()))?;
            k.pages = pages;
            pictures = pics;
        }
        _ => {
            let names = magazines::pages(issue)?;
            let at: HashMap<String, u32> = names.iter().enumerate().map(|(i, n)| (n.clone(), i as u32 + 1)).collect();
            k.pages = vec![String::new(); names.len()];
            let kind = magazines::comic_kind(issue).unwrap_or("zip");
            let mut files: Vec<(u32, Picture)> = vec![];
            crate::archive::each_wanted_as(issue, kind, &|n| at.contains_key(n), cancel, &mut |name, _, r| {
                let n = at[name];
                let to = tmp.0.join(format!("page-{n}.{}", crate::platforms::ext_of(name)));
                let mut bytes = vec![];
                r.read_to_end(&mut bytes)?;
                std::fs::write(&to, bytes)?;
                files.push((n, Picture::File(to)));
                Ok(())
            })?;
            files.sort_by_key(|(n, _)| *n);
            pictures = files;
        }
    }
    let total = k.pages.len() as u32;
    if pictures.is_empty() {
        progress(total, total);
        return Ok(k);
    }
    let Some(reader) = reader else {
        k.unread = pictures.iter().map(|(n, _)| *n).collect();
        k.note = Some(if cfg!(windows) {
            "Windows PowerShell wasn't found to read the pages' text. Put Tesseract in Game Repo's tools folder (Settings › Tools).".into()
        } else {
            "Reading text in pictures needs Tesseract: install it (or put it in Game Repo's tools folder) and set it in Settings › Tools if it isn't on the PATH.".into()
        });
        return Ok(k);
    };
    // Tesseract can't draw a PDF's page: those it has no picture for stay unread
    let r = recognise(reader, &pictures, cancel, &mut |n| progress(n, total))?;
    for (n, t) in r.text {
        if let Some(p) = k.pages.get_mut(n as usize - 1) {
            if enough(&t) || p.trim().is_empty() {
                *p = t;
            }
            k.recognised.push(n);
        }
    }
    k.unread = r.failed;
    k.note = r.note.or_else(|| (!k.unread.is_empty()).then(|| "Some pages' pictures are of a kind Tesseract can't be given (it's given JPEG, PNG and plain pixels).".to_string()));
    Ok(k)
}

// ------------------------------------------------------------ the games in it

/// The titles to look for: the library's games, and those its systems' DATs list.
pub struct Titles {
    by_words: HashMap<String, Vec<usize>>,
    games: Vec<Titled>,
    longest: usize,
}

struct Titled {
    key: String,
    title: String,
    platform: String,
    have: bool,
    sure: bool,
}

/// Whether a title (its words) is sure to be the game, found in a page's text: two words
/// or more that aren't everyday small words or numbers, and long enough (a game you have
/// may be shorter). Shorter titles of games you have are found but not ticked; other
/// games' aren't looked for, as they'd be found in everyday sentences.
fn weigh(w: &[String], have: bool) -> Option<bool> {
    let content = w.iter().filter(|x| !COMMON.contains(&x.as_str()) && !x.chars().all(|c| c.is_ascii_digit())).count();
    let letters: usize = w.iter().map(|x| x.chars().count()).sum();
    if content >= 2 && letters >= if have { 6 } else { 8 } {
        Some(true)
    } else if have && content >= 1 && letters >= 4 {
        Some(false)
    } else {
        None
    }
}

impl Titles {
    pub fn of(conn: &Connection) -> Result<Titles> {
        let mut q = conn.prepare("SELECT key, platform, title, name, have FROM library ORDER BY have DESC, CASE WHEN have THEN best_have ELSE best_all END DESC, sort")?;
        let mut t = Titles { by_words: HashMap::new(), games: vec![], longest: 0 };
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            let (key, platform, title, name, have): (String, String, String, String, bool) = (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?);
            // its title, and the title in its DAT name (they can differ once it's scraped)
            let mut said = vec![words(split_name(&title).0)];
            let dat = words(split_name(&name).0);
            if dat != said[0] {
                said.push(dat);
            }
            for w in said {
                let Some(sure) = weigh(&w, have) else { continue };
                let k = w.join(" ");
                let list = t.by_words.entry(k).or_default();
                // one game a system for each title: the first, best one
                if list.iter().any(|&i| t.games[i].platform == platform) {
                    continue;
                }
                list.push(t.games.len());
                t.longest = t.longest.max(w.len());
                t.games.push(Titled { key: key.clone(), title: split_name(&title).0.to_string(), platform: platform.clone(), have, sure });
            }
        }
        Ok(t)
    }

    /// The games in an issue's pages (from 1), each with the pages it's on: for a title more
    /// than one system has, the games you have, else the first.
    pub fn find(&self, pages: &[String]) -> Vec<IssueGame> {
        let mut found: BTreeMap<String, IssueGame> = BTreeMap::new();
        for (i, text) in pages.iter().enumerate() {
            let w = words(text);
            let mut at = 0;
            while at < w.len() {
                let mut hit = None;
                for len in (1..=self.longest.min(w.len() - at)).rev() {
                    if let Some(list) = self.by_words.get(&w[at..at + len].join(" ")) {
                        hit = Some((len, list));
                        break;
                    }
                }
                let Some((len, list)) = hit else {
                    at += 1;
                    continue;
                };
                let have: Vec<usize> = list.iter().copied().filter(|&g| self.games[g].have).collect();
                for g in if have.is_empty() { vec![list[0]] } else { have } {
                    let t = &self.games[g];
                    let e = found.entry(t.key.clone()).or_insert_with(|| IssueGame {
                        key: t.key.clone(),
                        title: t.title.clone(),
                        platform: t.platform.clone(),
                        ticked: t.sure,
                        sure: t.sure,
                        ..Default::default()
                    });
                    if e.pages.last() != Some(&(i as u32 + 1)) {
                        e.pages.push(i as u32 + 1);
                    }
                }
                at += len;
            }
        }
        let mut v: Vec<IssueGame> = found.into_values().collect();
        v.sort_by(|a, b| b.ticked.cmp(&a.ticked).then(a.pages[0].cmp(&b.pages[0])).then(a.title.cmp(&b.title)));
        v
    }
}

/// An issue's games after it's read again: what was found, with what was set by hand kept.
fn merge(was: &[IssueGame], found: Vec<IssueGame>) -> Vec<IssueGame> {
    let mut v: Vec<IssueGame> = found
        .into_iter()
        .map(|mut g| {
            if let Some(h) = was.iter().find(|h| h.hand && h.key == g.key) {
                g.ticked = h.ticked;
                g.hand = true;
            }
            g
        })
        .collect();
    let left: Vec<IssueGame> = was.iter().filter(|h| h.hand && !v.iter().any(|g| g.key == h.key)).cloned().collect();
    v.extend(left);
    v
}

// ------------------------------------------------------------ reading them (a job)

/// Read the text of each magazine's issues (or one magazine's) that hasn't been read since
/// its file changed (`again`: every one), keep it, give it to search, and find its games.
pub fn read_issues(ix: &Index, lib: &Library, title: Option<&str>, again: bool, cancel: &AtomicBool, progress: &mut dyn FnMut(Value)) -> Result<Value> {
    read_with(ix, lib, title, again, reader(), cancel, progress)
}

fn read_with(ix: &Index, lib: &Library, title: Option<&str>, again: bool, reader: Option<Reader>, cancel: &AtomicBool, progress: &mut dyn FnMut(Value)) -> Result<Value> {
    lib.writable()?;
    crate::docs::sync(ix, lib)?;
    crate::meta::sync(ix, lib)?;
    crate::browse::refresh(ix, lib)?;
    let mags: Vec<(PathBuf, Magazine)> = magazines::list(lib).into_iter().filter(|(_, m)| title.is_none_or(|t| m.title.eq_ignore_ascii_case(t))).collect();
    if let Some(t) = title {
        if mags.is_empty() {
            bail!("There's no magazine {t}.");
        }
    }
    let todo: Vec<(usize, String)> = mags
        .iter()
        .enumerate()
        .flat_map(|(m, (dir, mag))| mag.issues.iter().filter(|i| dir.join(&i.file).is_file()).map(move |i| (m, i.file.clone())))
        .filter(|(m, file)| {
            let p = mags[*m].0.join(file);
            again || match kept(&p) {
                None => true,
                // pages left unread for want of a reader, when there's one now
                Some(k) => !k.unread.is_empty() && k.reader.is_none() && reader.is_some(),
            }
        })
        .collect();
    let titles = Titles::of(ix.conn())?;
    let conn = ix.conn();
    let (mut issues, mut pages, mut recognised, mut unread, mut games) = (0, 0, 0, 0, 0);
    let mut notes: Vec<String> = vec![];
    for (n, (m, file)) in todo.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let (dir, mag) = &mags[*m];
        let issue = dir.join(file);
        let name = mag.issues.iter().find(|i| &i.file == file).map(|i| magazines::issue_title(&mag.title, i)).unwrap_or_else(|| file.clone());
        progress(json!({ "item": n + 1, "items": todo.len(), "name": name }));
        let k = read(&issue, reader.as_ref(), cancel, &mut |page, of| progress(json!({ "item": n + 1, "items": todo.len(), "name": name, "page": page, "pages": of })))?;
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let to = text_path(&issue).ok_or_else(|| anyhow!("no folder"))?;
        std::fs::create_dir_all(to.parent().unwrap_or(dir))?;
        crate::config::write_json(&to, &serde_json::to_value(&k)?)?;
        // search finds it
        if let Some(rel) = lib.relative(&issue) {
            let body = kept_text(&issue).unwrap_or_default();
            conn.execute("UPDATE docs_text SET body = ?2 WHERE rowid = (SELECT id FROM docs WHERE path = ?1)", params![rel, body])?;
            conn.execute("UPDATE docs SET text_read = 1 WHERE path = ?1", [&rel])?;
        }
        // its games, kept with it (magazine.json read again: it may have been changed meanwhile)
        let found = titles.find(&k.pages);
        let mut now_m: Magazine = serde_json::from_value(crate::config::read_json_object(&dir.join(MANIFEST)))?;
        if let Some(i) = now_m.issues.iter_mut().find(|i| &i.file == file) {
            i.games = merge(&i.games, found);
            games += i.games.iter().filter(|g| g.ticked).count();
        }
        crate::config::write_json(&dir.join(MANIFEST), &serde_json::to_value(&now_m)?)?;
        issues += 1;
        pages += k.pages.len();
        recognised += k.recognised.len();
        unread += k.unread.len();
        if let Some(note) = k.note {
            if !notes.contains(&note) {
                notes.push(note);
            }
        }
    }
    Ok(json!({ "issues": issues, "pages": pages, "recognised": recognised, "unread": unread, "games": games, "notes": notes, "reader": reader.as_ref().map(Reader::id) }))
}

// ------------------------------------------------------------ the page, and a game's

/// An issue's text for the viewer (its library path): each page's, and those not read.
pub fn text_of(lib: &Library, rel: &str) -> Result<Value> {
    let issue = lib.resolve(rel)?;
    if !magazines::is_magazine_path(lib, &issue) {
        bail!("{rel} isn't a magazine's issue.");
    }
    Ok(match kept(&issue) {
        Some(k) => json!({ "pages": k.pages, "recognised": k.recognised, "unread": k.unread, "note": k.note, "read": k.read }),
        None => Value::Null,
    })
}

/// Tick or untick a game found in an issue, or add one by its key (from the Library) with
/// the page it's on; `ticked: None` takes a game added by hand off again.
pub fn set_game(ix: &Index, lib: &Library, title: &str, file: &str, key: &str, ticked: Option<bool>, page: Option<u32>) -> Result<Value> {
    lib.writable()?;
    let (dir, mut m) = magazines::by_title(lib, title)?;
    let i = m.issues.iter_mut().find(|i| i.file == file).ok_or_else(|| anyhow!("{title} has no issue {file}."))?;
    match (i.games.iter().position(|g| g.key == key), ticked) {
        (Some(at), None) => {
            i.games.remove(at);
        }
        (Some(at), Some(t)) => {
            let g = &mut i.games[at];
            g.ticked = t;
            g.hand = true;
            if let Some(p) = page.filter(|p| *p > 0 && !g.pages.contains(p)) {
                g.pages.push(p);
                g.pages.sort_unstable();
            }
        }
        (None, None) => {}
        (None, Some(t)) => {
            let (platform, game_title): (String, String) = ix
                .conn()
                .query_row("SELECT platform, title FROM library WHERE key = ?1", [key], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()?
                .ok_or_else(|| anyhow!("There's no game {key} in the library."))?;
            i.games.push(IssueGame { key: key.into(), title: split_name(&game_title).0.to_string(), platform, pages: page.filter(|p| *p > 0).into_iter().collect(), ticked: t, hand: true, sure: false });
        }
    }
    crate::config::write_json(&dir.join(MANIFEST), &serde_json::to_value(&m)?)?;
    Ok(json!({ "ok": true }))
}

/// The issues that cover a game (ticked in them): by its key, or its title on its system.
pub fn issues_of(conn: &Connection, lib: &Library, key: &str, platform: &str, title: &str) -> Result<Vec<Value>> {
    let want = words(split_name(title).0);
    let mut out = vec![];
    for (dir, m) in magazines::list(lib) {
        for i in &m.issues {
            let Some(g) = i.games.iter().find(|g| g.ticked && (g.key == key || (g.platform == platform && !want.is_empty() && words(&g.title) == want))) else { continue };
            let rel = lib.relative(&dir.join(&i.file)).unwrap_or_default();
            let doc: Option<i64> = conn.query_row("SELECT id FROM docs WHERE path = ?1", [&rel], |r| r.get(0)).optional()?;
            out.push(json!({ "magazine": m.title, "title": magazines::issue_title(&m.title, i), "path": rel, "doc": doc, "pages": g.pages, "date": i.date }));
        }
    }
    Ok(out)
}

/// How far a magazine's issues have been read: (read, not read since their file changed).
pub fn state(dir: &Path, m: &Magazine) -> (usize, usize) {
    let have: Vec<PathBuf> = m.issues.iter().map(|i| dir.join(&i.file)).filter(|p| p.is_file()).collect();
    let read = have.iter().filter(|p| kept(p).is_some()).count();
    (read, have.len() - read)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::tests::{dat_xml, rom, rom_xml};
    use lopdf::{dictionary, Object, Stream};

    /// A PDF whose pages are lines of text, or (`None`) a JPEG as a scan's are.
    fn pdf(path: &Path, pages: &[Option<&str>], jpeg: &[u8], size: (i64, i64)) {
        let mut doc = lopdf::Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let font = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
        let img = doc.add_object(Stream::new(dictionary! { "Type" => "XObject", "Subtype" => "Image", "Width" => size.0, "Height" => size.1, "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8, "Filter" => "DCTDecode" }, jpeg.to_vec()));
        let mut kids = vec![];
        for p in pages {
            let content = match p {
                Some(t) => format!("BT /F1 12 Tf 20 120 Td ({t}) Tj ET"),
                None => format!("q {} 0 0 {} 0 0 cm /Im0 Do Q", size.0 / 2, size.1 / 2),
            };
            let c = doc.add_object(Stream::new(dictionary! {}, content.into_bytes()));
            let page = doc.add_object(dictionary! {
                "Type" => "Page", "Parent" => pages_id, "MediaBox" => vec![0.into(), 0.into(), (size.0 / 2).into(), (size.1 / 2).into()], "Contents" => c,
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => font }, "XObject" => dictionary! { "Im0" => img } },
            });
            kids.push(page.into());
        }
        let n = kids.len() as i64;
        doc.objects.insert(pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => n }));
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog);
        doc.save(path).unwrap();
    }

    fn jpeg() -> Vec<u8> {
        let mut out = std::io::Cursor::new(vec![]);
        ::image::RgbImage::from_pixel(40, 60, ::image::Rgb([240, 240, 240])).write_to(&mut out, ::image::ImageFormat::Jpeg).unwrap();
        out.into_inner()
    }

    fn cbz(path: &Path, pages: &[(&str, &[u8])]) {
        crate::audit::tests::zip(path, pages);
    }

    /// The Game Boy fixture's library (one-word titles, most of them had), a NES DAT of games
    /// not had, and a magazine with a PDF issue and a CBZ one.
    fn library(name: &str) -> (PathBuf, Library, Index, PathBuf) {
        let (dir, lib, ix) = crate::collection::tests::fixture(name);
        let nes = dat_xml("Nintendo - Nintendo Entertainment System", "2026", &[
            ("Super Mario Bros. 3 (USA)", None, rom_xml("Super Mario Bros. 3 (USA).nes", &rom(40, 64))),
            ("Mega Man 2 (USA)", None, rom_xml("Mega Man 2 (USA).nes", &rom(41, 64))),
            ("Mega Man (USA)", None, rom_xml("Mega Man (USA).nes", &rom(42, 64))),
            ("Golf (USA)", None, rom_xml("Golf (USA).nes", &rom(43, 64))),
        ]);
        std::fs::write(dir.join("nes.dat"), nes).unwrap();
        crate::dat::import(&ix, &lib, &dir.join("nes.dat"), &serde_json::Map::new()).unwrap();
        let mag = magazines::folder(&lib).join("Game Mag");
        std::fs::create_dir_all(&mag).unwrap();
        pdf(&mag.join("Game Mag 001.pdf"), &[Some("Reviews: Super Mario Bros. 3 is the best, and Alpha is back."), Some("Mega Man II tips, and golf for everyone."), None], &jpeg(), (80, 120));
        cbz(&mag.join("Game Mag 002.cbz"), &[("01.jpg", &jpeg()), ("02.jpg", &jpeg())]);
        let m = json!({ "title": "Game Mag", "issues": [
            { "number": "1", "file": "Game Mag 001.pdf", "size": 1, "added": "2026-10-22" },
            { "number": "2", "file": "Game Mag 002.cbz", "size": 1, "added": "2026-10-22" },
        ] });
        crate::config::write_json(&mag.join(MANIFEST), &m).unwrap();
        (dir, lib, ix, mag)
    }

    fn issue(lib: &Library, file: &str) -> crate::magazines::Issue {
        magazines::by_title(lib, "Game Mag").unwrap().1.issues.into_iter().find(|i| i.file == file).unwrap()
    }

    #[test]
    fn an_issues_text_is_kept_searched_and_its_games_found_with_the_ticks_set_by_hand_kept() {
        let (dir, lib, ix, mag) = library("magtext");
        let cancel = AtomicBool::new(false);
        let r = read_with(&ix, &lib, Some("game mag"), false, None, &cancel, &mut |_| {}).unwrap();
        assert_eq!((r["issues"].as_u64(), r["pages"].as_u64(), r["unread"].as_u64()), (Some(2), Some(5), Some(3)), "{r}");
        assert!(r["notes"][0].as_str().unwrap().contains("Tesseract"), "{r}");
        // the PDF's own text, kept beside it; its scanned page and the CBZ's pages wait for a reader
        let k = kept(&mag.join("Game Mag 001.pdf")).unwrap();
        assert!(k.pages[0].contains("Super Mario Bros. 3") && k.pages[1].contains("Mega Man II"), "{k:?}");
        assert_eq!((k.unread.clone(), k.reader.clone()), (vec![3], None));
        assert!(mag.join(".text/Game Mag 001.pdf.json").is_file());
        // its games: whole titles of two words or more ticked, a one-word title of a game you
        // have found but not ticked, and one of a game you don't (Golf) not looked for
        let games = issue(&lib, "Game Mag 001.pdf").games;
        let seen: Vec<(&str, &str, bool, Vec<u32>)> = games.iter().map(|g| (g.key.as_str(), g.title.as_str(), g.ticked, g.pages.clone())).collect();
        assert_eq!(seen, [
            ("nes/Super Mario Bros. 3 (USA)", "Super Mario Bros. 3", true, vec![1]),
            ("nes/Mega Man 2 (USA)", "Mega Man 2", true, vec![2]),
            ("gb/Alpha (Europe)", "Alpha", false, vec![1]),
        ]);
        // search finds an issue by its text
        let found = crate::docs::search(&ix, "mario", "magazine", "", "", 0, 10).unwrap();
        assert_eq!(found["total"], 1, "{found}");
        assert_eq!(text_of(&lib, "Magazines/Game Mag/Game Mag 001.pdf").unwrap()["unread"], json!([3]));
        // ticks set by hand, and a game added by hand on a page, stay when it's read again
        set_game(&ix, &lib, "Game Mag", "Game Mag 001.pdf", "gb/Alpha (Europe)", Some(true), None).unwrap();
        set_game(&ix, &lib, "Game Mag", "Game Mag 001.pdf", "nes/Mega Man 2 (USA)", Some(false), None).unwrap();
        set_game(&ix, &lib, "Game Mag", "Game Mag 001.pdf", "gb/Delta (World)", Some(true), Some(3)).unwrap();
        assert!(set_game(&ix, &lib, "Game Mag", "Game Mag 001.pdf", "gb/Nothing", Some(true), None).is_err());
        // nothing changed: nothing read again
        assert_eq!(read_with(&ix, &lib, None, false, None, &cancel, &mut |_| {}).unwrap()["issues"], 0);
        read_with(&ix, &lib, None, true, None, &cancel, &mut |_| {}).unwrap();
        let ticks: Vec<(String, bool, bool)> = issue(&lib, "Game Mag 001.pdf").games.iter().map(|g| (g.key.clone(), g.ticked, g.hand)).collect();
        assert_eq!(ticks, [
            ("nes/Super Mario Bros. 3 (USA)".to_string(), true, false),
            ("nes/Mega Man 2 (USA)".to_string(), false, true),
            ("gb/Alpha (Europe)".to_string(), true, true),
            ("gb/Delta (World)".to_string(), true, true),
        ]);
        // a game's page lists the issues that cover it (ticked), at their pages, whichever
        // release of the title it is
        let of = issues_of(ix.conn(), &lib, "gb/Alpha (USA)", "gb", "Alpha").unwrap();
        assert_eq!((of.len(), of[0]["title"].as_str(), of[0]["pages"].clone()), (1, Some("Game Mag 1"), json!([1])));
        assert!(of[0]["doc"].is_i64());
        assert!(issues_of(ix.conn(), &lib, "nes/Mega Man 2 (USA)", "nes", "Mega Man 2").unwrap().is_empty());
        let g = crate::browse::game(&ix, &lib, "gb/Delta (World)").unwrap();
        assert_eq!(g["magazines"][0]["pages"], json!([3]), "{g}");
        // taken off again
        set_game(&ix, &lib, "Game Mag", "Game Mag 001.pdf", "gb/Delta (World)", None, None).unwrap();
        assert!(!issue(&lib, "Game Mag 001.pdf").games.iter().any(|g| g.key == "gb/Delta (World)"));
        // a changed file is read again; its text isn't the old one's
        let o = magazines::overview(&ix, &lib).unwrap();
        assert_eq!((o["magazines"][0]["read"].as_u64(), o["magazines"][0]["unread"].as_u64()), (Some(2), Some(0)), "{o}");
        pdf(&mag.join("Game Mag 001.pdf"), &[Some("Nothing here today.")], &jpeg(), (80, 120));
        assert!(kept(&mag.join("Game Mag 001.pdf")).is_none() && crate::docs::text_of(&mag.join("Game Mag 001.pdf")).contains("Nothing here"));
        assert_eq!(magazines::overview(&ix, &lib).unwrap()["magazines"][0]["unread"], 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn pictures_are_read_by_tesseract_and_pages_left_unread_for_want_of_it_are_read_once_its_there() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, lib, ix, mag) = library("magtext-tesseract");
        let cancel = AtomicBool::new(false);
        read_with(&ix, &lib, None, false, None, &cancel, &mut |_| {}).unwrap();
        // a stand-in for Tesseract that reads every page the same, but the CBZ's second
        let fake = dir.join("tesseract");
        std::fs::write(&fake, "#!/bin/sh\ncase \"$1\" in *page-2.*) echo 'Mega Man 2 and Super Mario Bros. 3, reviewed';; *) echo 'Letters to the editor, nothing else today';; esac\n").unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut seen = vec![];
        let r = read_with(&ix, &lib, None, false, Some(Reader::Tesseract(fake.clone())), &cancel, &mut |p| seen.push(p)).unwrap();
        assert_eq!((r["issues"].as_u64(), r["recognised"].as_u64(), r["unread"].as_u64(), r["reader"].as_str()), (Some(2), Some(3), Some(0), Some("tesseract")), "{r}");
        assert!(seen.iter().any(|p| p["page"] == 2 && p["pages"] == 2), "{seen:?}");
        let k = kept(&mag.join("Game Mag 002.cbz")).unwrap();
        assert_eq!((k.recognised.clone(), k.pages[0].as_str()), (vec![1, 2], "Letters to the editor, nothing else today"));
        // the scanned page of the PDF too: its picture given to Tesseract; its own text kept
        let k = kept(&mag.join("Game Mag 001.pdf")).unwrap();
        assert!(k.pages[0].contains("Super Mario Bros. 3") && k.pages[2].starts_with("Letters") && k.recognised == [3], "{k:?}");
        let games: Vec<(String, Vec<u32>)> = issue(&lib, "Game Mag 002.cbz").games.iter().filter(|g| g.ticked).map(|g| (g.key.clone(), g.pages.clone())).collect();
        assert_eq!(games, [("nes/Mega Man 2 (USA)".to_string(), vec![2]), ("nes/Super Mario Bros. 3 (USA)".to_string(), vec![2])]);
        // a page Tesseract fails on is left unread, with what it said
        std::fs::write(&fake, "#!/bin/sh\necho 'Error opening data file' >&2\nexit 1\n").unwrap();
        let r = read_with(&ix, &lib, Some("Game Mag"), true, Some(Reader::Tesseract(fake)), &cancel, &mut |_| {}).unwrap();
        assert_eq!(r["unread"], 3, "{r}");
        assert!(r["notes"][0].as_str().unwrap().contains("Error opening data file"), "{r}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Windows' own text recognition, on a page of print Windows draws: a CBZ's picture,
    /// and a scanned PDF's page that Windows draws from the PDF itself.
    #[cfg(windows)]
    #[test]
    fn windows_reads_the_text_of_a_comics_page_and_of_a_scanned_pdfs_page() {
        let dir = crate::library::tests::temp_dir("magtext-windows");
        std::fs::create_dir_all(&dir).unwrap();
        let (png, jpg) = (dir.join("print.png"), dir.join("print.jpg"));
        let draw = format!(
            "Add-Type -AssemblyName System.Drawing; $b = New-Object System.Drawing.Bitmap 1600, 400; $g = [System.Drawing.Graphics]::FromImage($b); \
             $g.Clear([System.Drawing.Color]::White); $f = New-Object System.Drawing.Font('Arial', 64); \
             $g.DrawString('SUPER MARIO WORLD', $f, [System.Drawing.Brushes]::Black, 40, 130); \
             $b.Save('{}', [System.Drawing.Imaging.ImageFormat]::Png); $b.Save('{}', [System.Drawing.Imaging.ImageFormat]::Jpeg)",
            png.display(),
            jpg.display()
        );
        let powershell = windows_powershell().expect("Windows PowerShell");
        assert!(crate::tools::command(&powershell).args(["-NoProfile", "-NonInteractive", "-Command", &draw]).status().unwrap().success());
        cbz(&dir.join("comic.cbz"), &[("01.png", &std::fs::read(&png).unwrap())]);
        pdf(&dir.join("scan.pdf"), &[None], &std::fs::read(&jpg).unwrap(), (1600, 400));
        let reader = Reader::Windows(powershell);
        let cancel = AtomicBool::new(false);
        for file in ["comic.cbz", "scan.pdf"] {
            let k = read(&dir.join(file), Some(&reader), &cancel, &mut |_, _| {}).unwrap();
            assert!(k.pages[0].to_uppercase().contains("MARIO WORLD") && k.recognised == [1], "{file}: {k:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_title_is_looked_for_when_its_long_enough_and_sure_with_two_words() {
        let w = |s: &str| words(s);
        assert_eq!(weigh(&w("Super Mario World"), false), Some(true));
        assert_eq!(weigh(&w("Mega Man 2"), false), Some(true));
        // one word, or two short ones: only a game you have, and not ticked
        assert_eq!(weigh(&w("Tetris"), true), Some(false));
        assert_eq!(weigh(&w("Tetris"), false), None);
        assert_eq!(weigh(&w("Ys II"), false), None);
        assert_eq!(weigh(&w("Pac-Man"), true), Some(true));
        assert_eq!(weigh(&w("Pac-Man"), false), None);
        // numbers and small words aren't a title's words
        assert_eq!(weigh(&w("1942"), true), None);
        assert_eq!(weigh(&w("The Game"), true), None);
    }
}
