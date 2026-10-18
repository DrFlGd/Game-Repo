//! Documents (docs/PLAN.md, 3.8, "Phase 5 design", D30, D34): the manuals, guides,
//! FAQs, maps, posters, scans and soundtracks kept with a game in its folder's `docs/`,
//! or with no game in `_library/docs/<system or General>/`. Each `docs/` folder's
//! `docs.json` keeps its files' kinds, titles and sources; files it doesn't list are
//! guessed from their names. The index lists every document and keeps their text for
//! search (`docs`, `docs_text`), read once from PDFs, text and Markdown.
//!
//! Attaching documents to a game, and importing a folder of them with a pairing for
//! each, is one Organise change (copied or moved, journalled, undone).

use crate::config::{read_json_object, sorted, write_json};
use crate::index::Index;
use crate::library::{now, Library, APP_DIR};
use crate::meta::{self, DatGame};
use crate::organise::{self, key, safe_name, Op, Plan, Src};
use crate::pcgames::norm_title;
use crate::platforms;
use anyhow::{anyhow, bail, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// The kinds of document: (id, what it's called, words in a file's name that mean it).
pub const KINDS: &[(&str, &str, &[&str])] = &[
    ("manual", "Manual", &["manual", "instructions", "instruction", "booklet", "handbook", "manuel", "anleitung"]),
    ("guide", "Guide", &["guide", "walkthrough", "strategy", "solution", "hints", "hint", "tips"]),
    ("faq", "FAQ", &["faq", "faqs"]),
    ("map", "Map", &["map", "maps", "atlas"]),
    ("poster", "Poster", &["poster", "posters", "artwork"]),
    ("box", "Box scan", &["box", "cover", "inlay", "insert", "scan", "scans", "jewel", "case"]),
    ("media", "Cartridge or disc scan", &["cart", "cartridge", "disc", "disk", "label", "cd"]),
    ("soundtrack", "Soundtrack", &["ost", "soundtrack", "music"]),
    ("readme", "Readme", &["readme", "nfo", "notes", "changelog"]),
    ("magazine", "Magazine", &["magazine", "mag", "issue"]),
    ("other", "Other", &[]),
];

const AUDIO: &[&str] = &["mp3", "flac", "ogg", "wav", "m4a", "opus", "aac"];
const TEXT: &[&str] = &["txt", "md", "markdown", "nfo", "diz", "htm", "html", "rtf"];
const IMAGES: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff"];
/// Files a document folder can hold (anything else in a picked folder is left out).
pub const EXTS: &[&str] = &[
    "pdf", "txt", "md", "markdown", "nfo", "diz", "htm", "html", "rtf", "png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "cbz", "cbr", "epub", "djvu",
    "mp3", "flac", "ogg", "wav", "m4a", "opus", "aac", "doc", "docx", "odt", "xps",
];

pub const MANIFEST: &str = "docs.json";

fn ext(name: &str) -> String {
    platforms::ext_of(name)
}

/// What a file is: viewed in the page as pdf, image, text, audio, or opened outside.
pub fn view_of(name: &str) -> &'static str {
    let e = ext(name);
    match e.as_str() {
        "pdf" => "pdf",
        "cbz" | "cbr" => "comic",
        "md" | "markdown" => "markdown",
        _ if TEXT.contains(&e.as_str()) => "text",
        _ if IMAGES.contains(&e.as_str()) => "image",
        _ if AUDIO.contains(&e.as_str()) => "audio",
        _ => "outside",
    }
}

/// A file name's words, lower case.
fn words(s: &str) -> Vec<String> {
    crate::launchbox::words(s).split(' ').filter(|w| !w.is_empty()).map(String::from).collect()
}

/// The kind a file's name suggests.
pub fn guess_kind(name: &str) -> &'static str {
    let e = ext(name);
    if AUDIO.contains(&e.as_str()) {
        return "soundtrack";
    }
    if ["nfo", "diz"].contains(&e.as_str()) {
        return "readme";
    }
    let stem = Path::new(name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let w = words(&stem);
    for (id, _, ws) in KINDS {
        if w.iter().any(|x| ws.contains(&x.as_str())) {
            return id;
        }
    }
    if IMAGES.contains(&e.as_str()) {
        return "box";
    }
    "other"
}

/// A title for a file: its name without extension, underscores and dots made spaces.
pub fn guess_title(name: &str) -> String {
    let stem = Path::new(name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let s: String = stem.chars().map(|c| if c == '_' { ' ' } else { c }).collect();
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn kind_name(kind: &str) -> &'static str {
    KINDS.iter().find(|k| k.0 == kind).map(|k| k.1).unwrap_or("Other")
}

/// A file's entry in its folder's `docs.json`, or one guessed from its name.
fn entry_of(manifest: &Value, name: &str) -> (String, String, Option<String>) {
    let e = &manifest["files"][name];
    let kind = e["kind"].as_str().filter(|k| KINDS.iter().any(|x| x.0 == *k)).map(String::from).unwrap_or_else(|| guess_kind(name).into());
    let title = e["title"].as_str().filter(|t| !t.is_empty()).map(String::from).unwrap_or_else(|| guess_title(name));
    let source = e["source"].as_str().filter(|s| !s.is_empty()).map(String::from);
    (kind, title, source)
}

/// Where documents of no game are kept: `_library/docs/<system or General>/`.
pub fn loose_dir(lib: &Library, platform: Option<&str>) -> PathBuf {
    let d = lib.root().join(APP_DIR).join("docs");
    match platform.filter(|p| !p.is_empty()) {
        Some(p) => d.join(safe_name(p)),
        None => d.join("General"),
    }
}

// ------------------------------------------------------------ the index

fn stamp(p: &Path) -> Option<(i64, i64)> {
    let m = std::fs::metadata(p).ok()?;
    if !m.is_file() {
        return None;
    }
    let t = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as i64;
    Some((m.len() as i64, t))
}

/// Each documents folder in the library: (the folder, the game's folder if it's a game's,
/// its system, what its files are when it isn't its `docs.json` that says: a magazine's).
fn doc_folders(lib: &Library) -> Vec<(PathBuf, Option<String>, Option<String>, Option<Value>)> {
    let mut v = vec![];
    for sys in std::fs::read_dir(meta::games_root(lib)).into_iter().flatten().flatten() {
        let platform = sys.file_name().to_string_lossy().into_owned();
        for g in std::fs::read_dir(sys.path()).into_iter().flatten().flatten() {
            let d = g.path().join("docs");
            if d.is_dir() {
                v.push((d, lib.relative(&g.path()), Some(platform.clone()), None));
            }
        }
    }
    for sub in std::fs::read_dir(lib.root().join(APP_DIR).join("docs")).into_iter().flatten().flatten() {
        if sub.path().is_dir() {
            let name = sub.file_name().to_string_lossy().into_owned();
            let platform = (name != "General").then_some(name);
            v.push((sub.path(), None, platform, None));
        }
    }
    // each magazine's issues (D68), only the files its magazine.json lists
    for m in std::fs::read_dir(crate::magazines::folder(lib)).into_iter().flatten().flatten() {
        if let Some(manifest) = crate::magazines::docs_manifest(&m.path()) {
            v.push((m.path(), None, None, Some(manifest)));
        }
    }
    v
}

/// Bring the index's documents up to date with the library's documents folders.
pub fn sync(ix: &Index, lib: &Library) -> Result<()> {
    let conn = ix.conn();
    let known: HashMap<String, (i64, (i64, i64), String)> = {
        let mut q = conn.prepare("SELECT path, id, size, mtime, kind || char(9) || title || char(9) || COALESCE(source, '') FROM docs")?;
        let rows = q.query_map([], |r| Ok((r.get::<_, String>(0)?, (r.get(1)?, (r.get(2)?, r.get(3)?), r.get(4)?))))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut seen = HashSet::new();
    let tx = conn.unchecked_transaction()?;
    for (dir, folder, platform, given) in doc_folders(lib) {
        let only_listed = given.is_some();
        let manifest = given.unwrap_or_else(|| read_json_object(&dir.join(MANIFEST)));
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name == MANIFEST || name.starts_with('.') || name.ends_with(".gamerepo-tmp") || (only_listed && manifest["files"].get(&name).is_none()) {
                continue;
            }
            let p = e.path();
            let Some(st) = stamp(&p) else { continue };
            let Some(rel) = lib.relative(&p) else { continue };
            seen.insert(rel.clone());
            let (kind, title, source) = entry_of(&manifest, &name);
            let label = format!("{kind}\t{title}\t{}", source.clone().unwrap_or_default());
            match known.get(&rel) {
                Some((_, s, l)) if *s == st && *l == label => continue,
                Some((id, s, _)) => {
                    let changed = *s != st;
                    tx.execute(
                        "UPDATE docs SET folder = ?2, platform = ?3, name = ?4, title = ?5, kind = ?6, source = ?7, size = ?8, mtime = ?9, text_read = CASE WHEN ?10 THEN 0 ELSE text_read END WHERE id = ?1",
                        params![id, folder, platform, name, title, kind, source, st.0, st.1, changed],
                    )?;
                    tx.execute("UPDATE docs_text SET title = ?2, name = ?3 WHERE rowid = ?1", params![id, title, name])?;
                }
                None => {
                    tx.execute(
                        "INSERT INTO docs (path, folder, platform, name, title, kind, source, size, mtime) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                        params![rel, folder, platform, name, title, kind, source, st.0, st.1],
                    )?;
                    let id = tx.last_insert_rowid();
                    tx.execute("INSERT INTO docs_text (rowid, title, name, body) VALUES (?1, ?2, ?3, '')", params![id, title, name])?;
                }
            }
        }
    }
    for (rel, (id, _, _)) in known.iter().filter(|(r, _)| !seen.contains(*r)) {
        tx.execute("DELETE FROM docs WHERE id = ?1", [id])?;
        tx.execute("DELETE FROM docs_text WHERE rowid = ?1", [id])?;
        let _ = rel;
    }
    tx.commit()?;
    Ok(())
}

/// How many documents haven't had their text read yet.
pub fn unread(conn: &Connection) -> Result<i64> {
    let readable = EXT_SQL;
    Ok(conn.query_row(&format!("SELECT COUNT(*) FROM docs WHERE text_read = 0 AND {readable}"), [], |r| r.get(0))?)
}

/// The documents whose text can be read.
const EXT_SQL: &str = "(lower(name) LIKE '%.pdf' OR lower(name) LIKE '%.txt' OR lower(name) LIKE '%.md' OR lower(name) LIKE '%.markdown' OR lower(name) LIKE '%.nfo'
                        OR lower(name) LIKE '%.htm' OR lower(name) LIKE '%.html' OR lower(name) LIKE '%.diz')";

/// The text of a document, for search: a PDF's (when it has text, not just scans), a
/// text file's, a web page's without its tags. At most 1 MB of it.
pub fn text_of(path: &Path) -> String {
    const MAX: usize = 1 << 20;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let e = ext(&name);
    let mut s = if e == "pdf" {
        pdf_text(path).unwrap_or_default()
    } else {
        let Ok(b) = std::fs::read(path) else { return String::new() };
        let t = String::from_utf8_lossy(&b[..b.len().min(4 * MAX)]).into_owned();
        if e == "htm" || e == "html" {
            strip_html(&t)
        } else {
            t
        }
    };
    if s.len() > MAX {
        let mut cut = MAX;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
    }
    s
}

fn strip_html(s: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for c in s.chars() {
        match c {
            '<' => inside = true,
            '>' if inside => {
                inside = false;
                out.push(' ');
            }
            _ if !inside => out.push(c),
            _ => {}
        }
    }
    out
}

/// A PDF's text (a PDF of scans has none). Some PDFs upset the reader: they give none.
fn pdf_text(path: &Path) -> Option<String> {
    let size = std::fs::metadata(path).ok()?.len();
    if size > 400 << 20 {
        return None;
    }
    let p = path.to_path_buf();
    std::panic::catch_unwind(move || {
        let doc = lopdf::Document::load(&p).ok()?;
        let pages: Vec<u32> = doc.get_pages().keys().copied().take(2000).collect();
        let mut out = String::new();
        for n in pages {
            if let Ok(t) = doc.extract_text(&[n]) {
                out.push_str(&t);
                out.push('\n');
            }
            if out.len() > 1 << 20 {
                break;
            }
        }
        Some(out)
    })
    .ok()
    .flatten()
}

/// Read the text of the documents that haven't had it read (a job). Returns how many.
pub fn read_texts(ix: &Index, lib: &Library, cancel: &AtomicBool, progress: &mut dyn FnMut(Value)) -> Result<usize> {
    let conn = ix.conn();
    let todo: Vec<(i64, String, String)> = {
        let mut q = conn.prepare(&format!("SELECT id, path, title FROM docs WHERE text_read = 0 AND {EXT_SQL} ORDER BY id"))?;
        let rows = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut n = 0;
    for (i, (id, rel, title)) in todo.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        progress(json!({ "item": i + 1, "items": todo.len(), "name": title }));
        let text = lib.resolve(rel).map(|p| text_of(&p)).unwrap_or_default();
        conn.execute("UPDATE docs_text SET body = ?2 WHERE rowid = ?1", params![id, text])?;
        conn.execute("UPDATE docs SET text_read = 1 WHERE id = ?1", [id])?;
        n += 1;
    }
    Ok(n)
}

fn doc_json(r: &rusqlite::Row) -> rusqlite::Result<Value> {
    let name: String = r.get("name")?;
    let kind: String = r.get("kind")?;
    Ok(json!({
        "id": r.get::<_, i64>("id")?, "path": r.get::<_, String>("path")?, "folder": r.get::<_, Option<String>>("folder")?,
        "platform": r.get::<_, Option<String>>("platform")?, "name": name, "title": r.get::<_, String>("title")?,
        "kind": kind, "kind_name": kind_name(&kind), "source": r.get::<_, Option<String>>("source")?, "size": r.get::<_, i64>("size")?,
        "view": view_of(&name),
    }))
}

/// A game folder's documents.
pub fn of_folder(conn: &Connection, folder: &str) -> Result<Vec<Value>> {
    let mut q = conn.prepare("SELECT * FROM docs WHERE folder = ?1 ORDER BY kind, title")?;
    let v = q.query_map([folder], doc_json)?.collect::<rusqlite::Result<_>>()?;
    Ok(v)
}

/// A search's words as FTS5 wants them: each a prefix, all needed.
fn fts_query(q: &str) -> String {
    words(q).iter().map(|w| format!("\"{}\"*", w.replace('"', ""))).collect::<Vec<_>>().join(" ")
}

/// The Documents page: every document, or those the search finds (in titles, names and
/// text), with filters. Each has its game's title when it has a game.
pub fn search(ix: &Index, query: &str, kind: &str, platform: &str, source: &str, offset: i64, limit: i64) -> Result<Value> {
    let conn = ix.conn();
    let fts = fts_query(query);
    let mut wh = vec!["1".to_string()];
    if !kind.is_empty() {
        wh.push("d.kind = :kind".into());
    }
    if !platform.is_empty() {
        wh.push("d.platform = :platform".into());
    }
    match source {
        "" => {}
        "yours" => wh.push("d.source IS NULL".into()),
        _ => wh.push("d.source = :source".into()),
    }
    let (from, snip, order) = if fts.is_empty() {
        ("docs d".to_string(), "NULL".to_string(), "d.title COLLATE NOCASE")
    } else {
        wh.push("docs_text MATCH :q".into());
        ("docs_text JOIN docs d ON d.id = docs_text.rowid".to_string(), "snippet(docs_text, 2, '[', ']', '…', 12)".to_string(), "bm25(docs_text, 5.0, 3.0, 1.0)")
    };
    let wh = wh.join(" AND ");
    let mut named: Vec<(&str, &dyn rusqlite::ToSql)> = vec![];
    if !kind.is_empty() {
        named.push((":kind", &kind));
    }
    if !platform.is_empty() {
        named.push((":platform", &platform));
    }
    if !source.is_empty() && source != "yours" {
        named.push((":source", &source));
    }
    if !fts.is_empty() {
        named.push((":q", &fts));
    }
    let total: i64 = conn.query_row(&format!("SELECT COUNT(*) FROM {from} WHERE {wh}"), named.as_slice(), |r| r.get(0))?;
    let lim = limit.clamp(1, 500);
    let off = offset.max(0);
    let mut all = named.clone();
    all.push((":lim", &lim));
    all.push((":off", &off));
    let mut q = conn.prepare(&format!(
        "SELECT d.*, {snip} AS snip, mg.title AS game_title, mg.name AS game_name FROM {from} LEFT JOIN meta_games mg ON mg.folder = d.folder
         WHERE {wh} ORDER BY {order} LIMIT :lim OFFSET :off"
    ))?;
    let items: Vec<Value> = q
        .query_map(all.as_slice(), |r| {
            let mut v = doc_json(r)?;
            v["snippet"] = json!(r.get::<_, Option<String>>("snip")?);
            v["game"] = json!(r.get::<_, Option<String>>("game_title")?.or(r.get::<_, Option<String>>("game_name")?));
            if let Some(p) = v["platform"].as_str() {
                v["platform_name"] = json!(crate::browse::system_name(p));
            }
            Ok(v)
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut kinds = conn.prepare("SELECT kind, COUNT(*) FROM docs GROUP BY kind")?;
    let kinds: BTreeMap<String, i64> = kinds.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    let mut ps = conn.prepare("SELECT DISTINCT platform FROM docs WHERE platform IS NOT NULL ORDER BY platform")?;
    let ps: Vec<String> = ps.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    let mut ss = conn.prepare("SELECT DISTINCT source FROM docs WHERE source IS NOT NULL ORDER BY source")?;
    let ss: Vec<String> = ss.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    Ok(json!({
        "total": total, "items": items, "unread": unread(conn)?,
        "kinds": KINDS.iter().map(|(id, name, _)| json!({ "id": id, "name": name, "count": kinds.get(*id).copied().unwrap_or(0) })).collect::<Vec<_>>(),
        "platforms": ps.iter().map(|p| json!({ "id": p, "name": platforms::name_of(p) })).collect::<Vec<_>>(),
        "sources": ss,
    }))
}

/// One document, to show it.
pub fn get(ix: &Index, id: i64) -> Result<Value> {
    let conn = ix.conn();
    conn.query_row("SELECT * FROM docs WHERE id = ?1", [id], doc_json).optional()?.ok_or_else(|| anyhow!("There's no document {id}."))
}

/// A text or Markdown document as HTML for the viewer (Model Library's readme view: raw
/// HTML in it is shown as text, links only to the web).
pub fn to_html(name: &str, bytes: &[u8]) -> String {
    use pulldown_cmark::{html, CowStr, Event, Options, Parser, Tag};
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(8 << 20)]);
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
    let e = ext(name);
    if e == "md" || e == "markdown" {
        fn safe(u: CowStr<'_>) -> CowStr<'_> {
            if u.starts_with("http://") || u.starts_with("https://") {
                u
            } else {
                CowStr::Borrowed("")
            }
        }
        let parser = Parser::new_ext(&text, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH).map(|ev| match ev {
            Event::Html(s) | Event::InlineHtml(s) => Event::Text(s),
            Event::Start(Tag::Link { link_type, dest_url, title, id }) => Event::Start(Tag::Link { link_type, dest_url: safe(dest_url), title, id }),
            Event::Start(Tag::Image { link_type, dest_url, title, id }) => Event::Start(Tag::Image { link_type, dest_url: safe(dest_url), title, id }),
            ev => ev,
        });
        let mut out = String::new();
        html::push_html(&mut out, parser);
        out
    } else if e == "htm" || e == "html" {
        format!("<pre class=\"doc-text\">{}</pre>", esc(&strip_html(&text)))
    } else {
        format!("<pre class=\"doc-text\">{}</pre>", esc(&text))
    }
}

// ------------------------------------------------------------ changing them

/// Change a document's kind or title (its folder's `docs.json`).
pub fn set(ix: &Index, lib: &Library, id: i64, kind: Option<&str>, title: Option<&str>) -> Result<Value> {
    lib.writable()?;
    let d = get(ix, id)?;
    let p = lib.resolve(d["path"].as_str().unwrap_or(""))?;
    if crate::magazines::is_magazine_path(lib, &p) {
        bail!("A magazine's issue is changed on the Magazines page.");
    }
    let dir = p.parent().ok_or_else(|| anyhow!("no folder"))?;
    let name = d["name"].as_str().unwrap_or("").to_string();
    let mp = dir.join(MANIFEST);
    let mut m = read_json_object(&mp);
    let (k, t, s) = entry_of(&m, &name);
    if let Some(k) = kind {
        if !KINDS.iter().any(|x| x.0 == k) {
            bail!("There's no kind of document {k}.");
        }
    }
    m["files"][&name] = json!({ "kind": kind.unwrap_or(&k), "title": title.map(str::trim).filter(|t| !t.is_empty()).unwrap_or(&t), "source": s });
    write_json(&mp, &m)?;
    sync(ix, lib)?;
    get(ix, id)
}

/// Keep a document a source gave (a scraped manual) in a game's folder: `<Kind>
/// (<Source>).<ext>`, replacing the one that source gave before, never the user's.
pub fn save_scraped(lib: &Library, game_dir: &Path, kind: &str, source_name: &str, source: &str, ext: &str, bytes: &[u8]) -> Result<PathBuf> {
    lib.writable()?;
    let dir = game_dir.join("docs");
    std::fs::create_dir_all(&dir)?;
    let name = format!("{} ({source_name}).{ext}", kind_name(kind));
    let p = dir.join(&name);
    crate::config::write_atomic(&p, bytes)?;
    let mp = dir.join(MANIFEST);
    let mut m = read_json_object(&mp);
    m["files"][&name] = json!({ "kind": kind, "title": kind_name(kind), "source": source, "added": now() });
    write_json(&mp, &m)?;
    Ok(p)
}

/// Whether a game's folder has a document of this kind from the user (not scraped).
pub fn has_users(game_dir: &Path, kind: &str) -> bool {
    let dir = game_dir.join("docs");
    let m = read_json_object(&dir.join(MANIFEST));
    std::fs::read_dir(&dir).into_iter().flatten().flatten().any(|e| {
        let n = e.file_name().to_string_lossy().into_owned();
        if n == MANIFEST {
            return false;
        }
        let (k, _, s) = entry_of(&m, &n);
        k == kind && s.is_none()
    })
}

/// Whether a game's folder has a document of this kind from a source.
pub fn has_scraped(game_dir: &Path, kind: &str) -> bool {
    let dir = game_dir.join("docs");
    let m = read_json_object(&dir.join(MANIFEST));
    std::fs::read_dir(&dir).into_iter().flatten().flatten().any(|e| {
        let n = e.file_name().to_string_lossy().into_owned();
        let (k, _, s) = entry_of(&m, &n);
        n != MANIFEST && k == kind && s.is_some()
    })
}

/// Remove the documents sources gave a game (Clear); the user's stay.
pub fn remove_scraped(ix: &Index, lib: &Library, game_dir: &Path) -> Result<()> {
    let dir = game_dir.join("docs");
    let mp = dir.join(MANIFEST);
    let mut m = read_json_object(&mp);
    let mut gone = vec![];
    for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        if n != MANIFEST && entry_of(&m, &n).2.is_some() {
            std::fs::remove_file(e.path())?;
            gone.push(n);
        }
    }
    if !gone.is_empty() {
        if let Some(files) = m["files"].as_object_mut() {
            for n in &gone {
                files.remove(n);
            }
        }
        write_json(&mp, &m)?;
    }
    sync(ix, lib)
}

// ------------------------------------------------------------ attaching and importing

/// A file to attach or import, with the game it's paired with.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Item {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub kind: String,
    pub title: String,
    /// The DAT game it goes with (none: no game)
    pub game: Option<i64>,
    /// Or the folder of the game it goes with, in the library (a PC game's, D55)
    pub folder: Option<String>,
    pub game_name: Option<String>,
    pub game_title: Option<String>,
    pub platform: Option<String>,
    /// sure | likely | none | you (chosen by hand)
    pub pairing: String,
    pub score: u32,
    /// Other games it could be
    pub others: Vec<Value>,
    pub include: bool,
}

/// The files of the paths picked: files themselves, folders walked (documents only).
pub fn files_of(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = vec![];
    let mut stack: Vec<PathBuf> = paths.to_vec();
    while let Some(p) = stack.pop() {
        if p.is_dir() {
            let mut kids: Vec<PathBuf> = std::fs::read_dir(&p).into_iter().flatten().flatten().map(|e| e.path()).collect();
            kids.sort();
            kids.reverse();
            stack.extend(kids);
        } else if p.is_file() {
            let n = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if paths.contains(&p) || EXTS.contains(&ext(&n).as_str()) {
                out.push(p);
            }
        }
    }
    out
}

/// The titles of a system's DAT games, to pair files with: normalised title → games.
struct Titles {
    platform: String,
    by_title: HashMap<String, Vec<(i64, String, String, bool)>>,
    by_word: HashMap<String, Vec<String>>,
}

/// The words in a document's name that aren't the game's: kinds, tags, scan words.
fn title_words(name: &str) -> String {
    let stem = Path::new(name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| name.to_string());
    // tags in brackets go, as DAT names' do
    let mut s = String::new();
    let mut depth = 0;
    for c in stem.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = (depth - 1).max(0),
            _ if depth == 0 => s.push(c),
            _ => {}
        }
    }
    let drop: HashSet<&str> = KINDS.iter().flat_map(|k| k.2.iter().copied()).chain(["english", "en", "us", "usa", "eu", "jp", "scan", "hq", "hd", "full", "complete", "v1", "v2", "v1.0", "official"]).collect();
    words(&s.replace('_', " ")).into_iter().filter(|w| !drop.contains(w.as_str())).collect::<Vec<_>>().join(" ")
}

fn load_titles(conn: &Connection, lib: &Library, platform: &str) -> Result<Option<Titles>> {
    let Some(dat) = crate::audit::primary_dat(conn, lib, platform)? else { return Ok(None) };
    let mut q = conn.prepare(&format!("WITH {} SELECT id, name, COALESCE(title, ''), status != 'missing' FROM gs", crate::audit::GAMES))?;
    let rows: Vec<(i64, String, String, bool)> = q.query_map([dat], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<rusqlite::Result<_>>()?;
    let mut by_title: HashMap<String, Vec<(i64, String, String, bool)>> = HashMap::new();
    let mut by_word: HashMap<String, Vec<String>> = HashMap::new();
    for (id, name, title, have) in rows {
        let t = if title.is_empty() { crate::dat::parse_name(&name).title } else { title };
        let n = norm_title(&t);
        if n.is_empty() {
            continue;
        }
        if !by_title.contains_key(&n) {
            for w in words(&t) {
                by_word.entry(w).or_default().push(n.clone());
            }
        }
        by_title.entry(n).or_default().push((id, name, t, have));
    }
    Ok(Some(Titles { platform: platform.to_string(), by_title, by_word }))
}

/// The best of a title's games: one the user has, then by region (USA, World, Europe, Japan).
fn pick(games: &[(i64, String, String, bool)]) -> (i64, String, String, bool) {
    let rank = |n: &str| ["(USA", "(World", "(Europe", "(Japan"].iter().position(|r| n.contains(r)).unwrap_or(9);
    games.iter().min_by_key(|g| (!g.3, rank(&g.1), g.1.len())).cloned().unwrap_or_default()
}

/// Pair a file with a game of these systems: (game, score, others).
fn pair(t: &Titles, name: &str) -> Option<((i64, String, String), u32, Vec<Value>)> {
    let q = title_words(name);
    if q.is_empty() {
        return None;
    }
    let n = norm_title(&q);
    if let Some(gs) = t.by_title.get(&n) {
        let g = pick(gs);
        let others = gs.iter().filter(|o| o.0 != g.0).take(5).map(|o| json!({ "id": o.0, "name": o.1, "platform": t.platform })).collect();
        return Some(((g.0, g.1, g.2), 100, others));
    }
    // titles sharing a word with it, the rarest words first
    let mut ws: Vec<&Vec<String>> = words(&q).iter().filter_map(|w| t.by_word.get(w)).collect();
    ws.sort_by_key(|v| v.len());
    let mut cands: HashSet<&String> = HashSet::new();
    for v in ws.iter().take(3) {
        cands.extend(v.iter().take(2000));
    }
    let mut scored: Vec<(u32, &String)> = cands.into_iter().map(|c| (crate::launchbox::score(&q, &t.by_title[c][0].2), c)).filter(|(s, _)| *s >= 50).collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)));
    let (best, title) = scored.first()?;
    let g = pick(&t.by_title[*title]);
    let others = scored.iter().skip(1).take(5).map(|(_, c)| { let o = pick(&t.by_title[*c]); json!({ "id": o.0, "name": o.1, "platform": t.platform }) }).collect();
    Some(((g.0, g.1, g.2), *best, others))
}

/// Read files to attach or import, each paired with a game: of `platform` when given,
/// else of the system its folders name, else of every system with a DAT. `game` pairs
/// every file with that game (Attach documents… on a game).
pub fn read_items(ix: &Index, lib: &Library, paths: &[PathBuf], platform: Option<&str>, game: Option<i64>, cancel: &AtomicBool, progress: &mut dyn FnMut(Value)) -> Result<Vec<Item>> {
    let conn = ix.conn();
    let files = files_of(paths);
    if files.is_empty() {
        bail!("There are no documents there (PDFs, pictures, text, audio…).");
    }
    let fixed = game.map(|id| meta::dat_game(conn, id)).transpose()?;
    let mut cache: HashMap<String, Option<Titles>> = HashMap::new();
    let all_systems: Vec<String> = {
        let mut q = conn.prepare("SELECT DISTINCT platform FROM dats WHERE current = 1 AND platform != 'bios' ORDER BY platform")?;
        let v = q.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        v
    };
    let mut out = vec![];
    for (i, f) in files.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            bail!("stopped");
        }
        let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        progress(json!({ "item": i + 1, "items": files.len(), "name": name }));
        let mut it = Item {
            path: f.display().to_string(), size: std::fs::metadata(f).map(|m| m.len()).unwrap_or(0), kind: guess_kind(&name).into(), title: guess_title(&name),
            name: name.clone(), pairing: "none".into(), include: true, ..Default::default()
        };
        if let Some(g) = &fixed {
            it.game = Some(g.id);
            it.game_name = Some(g.name.clone());
            it.game_title = Some(g.title.clone());
            it.platform = Some(g.platform.clone());
            it.pairing = "you".into();
            it.score = 100;
            out.push(it);
            continue;
        }
        // the system: given, else named by a folder on the way, else every one with a DAT
        let dirs: Vec<String> = f.parent().map(|p| p.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect()).unwrap_or_default();
        let named = dirs.iter().rev().take(4).find_map(|d| platforms::from_folder(d)).map(|p| p.id.to_string());
        let systems: Vec<String> = match platform.filter(|p| !p.is_empty()).map(String::from).or(named) {
            Some(p) => vec![p],
            None => all_systems.clone(),
        };
        let mut best: Option<((i64, String, String), u32, Vec<Value>, String)> = None;
        for s in &systems {
            if !cache.contains_key(s) {
                cache.insert(s.clone(), load_titles(conn, lib, s)?);
            }
            let Some(t) = cache[s].as_ref() else { continue };
            if let Some((g, score, others)) = pair(t, &name) {
                if best.as_ref().is_none_or(|b| score > b.1) {
                    best = Some((g, score, others, s.clone()));
                }
            }
        }
        if let Some((g, score, others, s)) = best {
            it.game = Some(g.0);
            it.game_name = Some(g.1);
            it.game_title = Some(g.2);
            it.platform = Some(s);
            it.score = score;
            it.others = others;
            it.pairing = if score >= 100 { "sure" } else if score >= 70 { "likely" } else { "none" }.into();
            if it.pairing == "none" {
                it.game = None;
                it.game_name = None;
                it.game_title = None;
            }
        } else if systems.len() == 1 {
            it.platform = Some(systems[0].clone());
        }
        out.push(it);
    }
    Ok(out)
}

/// Pair every item with a game's folder (Attach documents… on a PC game's page, D55).
pub fn pair_with_folder(lib: &Library, items: &mut [Item], folder: &str, name: &str) -> Result<()> {
    meta::resolve(lib, folder)?;
    for it in items.iter_mut() {
        it.folder = Some(folder.to_string());
        it.game = None;
        it.game_name = Some(name.to_string());
        it.game_title = Some(name.to_string());
        it.platform = folder.split('/').nth(2).map(String::from);
        it.pairing = "you".into();
        it.score = 100;
        it.others = vec![];
    }
    Ok(())
}

/// A free name in a folder for a file, given the names taken.
fn free_name(taken: &mut HashSet<String>, name: &str) -> String {
    let p = Path::new(name);
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let mut n = safe_name(name);
    let mut i = 2;
    while taken.contains(&n.to_lowercase()) {
        n = safe_name(&format!("{stem} ({i}){ext}"));
        i += 1;
    }
    taken.insert(n.to_lowercase());
    n
}

pub(crate) fn outside(p: &Path) -> Src {
    Src {
        file_id: 0, root: format!("dir:{}", p.parent().unwrap_or(Path::new(".")).display()), rel: p.file_name().unwrap_or_default().to_string_lossy().into_owned(),
        path: p.display().to_string(), member: None, size: std::fs::metadata(p).map(|m| m.len()).unwrap_or(0), crc32: String::new(), sha1: None,
    }
}

pub(crate) fn text_op(game: &str, to: &Path, text: String, note: &str) -> Op {
    let from = if to.exists() { vec![outside(to)] } else { vec![] };
    Op { op: "text".into(), game: game.into(), to: to.display().to_string(), from, size: text.len() as u64, names: vec![text], aside: vec![], note: Some(note.into()) }
}

/// Plan putting the items into their games' `docs/` folders (or `_library/docs/` for
/// those with no game): copied (hardlinked on the same drive unless `always_copy`) or
/// moved, each folder's `docs.json` written with their kinds and titles.
pub fn plan(ix: &Index, lib: &Library, items: &[Item], mode: &str, always_copy: bool) -> Result<Plan> {
    lib.writable()?;
    let conn = ix.conn();
    let items: Vec<&Item> = items.iter().filter(|i| i.include).collect();
    if items.is_empty() {
        bail!("Choose at least one document to bring in.");
    }
    // each target folder: its manifest, the names taken, the ops, and the game's game.json if it's new
    struct Target {
        dir: PathBuf,
        label: String,
        manifest: Value,
        taken: HashSet<String>,
        ops: Vec<Op>,
        new_game: Option<Value>,
    }
    let mut targets: BTreeMap<String, Target> = BTreeMap::new();
    let mut games: HashSet<String> = HashSet::new();
    for it in &items {
        let src = PathBuf::from(&it.path);
        if !src.is_file() {
            bail!("There's no file {}.", it.path);
        }
        let (tkey, dir, label, new_game) = match (it.game, &it.folder) {
            (_, Some(f)) => {
                let d = meta::resolve(lib, f)?;
                (key(&d), d.join("docs"), it.game_name.clone().unwrap_or_default(), None)
            }
            (Some(id), None) => {
                let g: DatGame = meta::dat_game(conn, id)?;
                match meta::folder_of(conn, &g)?.and_then(|f| lib.resolve(&f).ok()).filter(|d| d.join("game.json").is_file()) {
                    Some(d) => (key(&d), d.join("docs"), g.name.clone(), None),
                    None => {
                        let d = meta::games_root(lib).join(&g.platform).join(safe_name(&g.name));
                        let gj = json!({
                            "id": organise::new_id(), "platform": g.platform, "name": g.name, "dat": g.dat, "title": g.title, "created": now(),
                            "roms": g.roms.iter().map(|r| json!({ "name": r.name, "size": r.size, "crc32": r.crc32, "md5": r.md5, "sha1": r.sha1 })).collect::<Vec<_>>(),
                        });
                        if d.join("game.json").is_file() {
                            bail!("{} has a folder of another game's name in the library.", g.name);
                        }
                        (key(&d), d.join("docs"), g.name.clone(), Some(gj))
                    }
                }
            }
            (None, None) => {
                let d = loose_dir(lib, it.platform.as_deref());
                (key(&d), d, String::new(), None)
            }
        };
        let t = targets.entry(tkey).or_insert_with(|| {
            let manifest = read_json_object(&dir.join(MANIFEST));
            let taken: HashSet<String> = std::fs::read_dir(&dir).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().to_lowercase()).collect();
            Target { dir: dir.clone(), label: label.clone(), manifest, taken, ops: vec![], new_game }
        });
        let name = free_name(&mut t.taken, &it.name);
        let to = t.dir.join(&name);
        let size = std::fs::metadata(&src)?.len();
        let op = match mode {
            "move" => "move",
            _ if !always_copy && organise::same_drive(&src, &t.dir) => "link",
            _ => "copy",
        };
        let s = Src { size, ..outside(&src) };
        t.ops.push(Op { op: op.into(), game: t.label.clone(), to: to.display().to_string(), from: vec![s], names: vec![], aside: vec![], size, note: None });
        let kind = if KINDS.iter().any(|k| k.0 == it.kind) { it.kind.clone() } else { guess_kind(&name).to_string() };
        t.manifest["files"][&name] = json!({ "kind": kind, "title": if it.title.trim().is_empty() { guess_title(&name) } else { it.title.trim().to_string() }, "added": now() });
        if !t.label.is_empty() {
            games.insert(t.label.clone());
        }
    }
    let mut ops = vec![];
    for t in targets.into_values() {
        if let Some(gj) = &t.new_game {
            let p = t.dir.parent().unwrap_or(&t.dir).join("game.json");
            ops.push(text_op(&t.label, &p, serde_json::to_string_pretty(&sorted(gj))? + "\n", "game.json"));
        }
        ops.extend(t.ops);
        ops.push(text_op(&t.label, &t.dir.join(MANIFEST), serde_json::to_string_pretty(&sorted(&t.manifest))? + "\n", "docs.json: the documents' kinds and titles"));
    }
    let n = items.len();
    Ok(Plan {
        id: organise::new_id(),
        created: now(),
        kind: "docs".into(),
        label: format!("Brought in {n} document{}", if n == 1 { "" } else { "s" }),
        title: format!("Bring in {n} document{}", if n == 1 { "" } else { "s" }),
        dat: Value::Null,
        platform: String::new(),
        mode: mode.into(),
        format: "keep".into(),
        out: Some(lib.root().join(APP_DIR).display().to_string()),
        space: organise::space_needed(&ops),
        ops,
        games: games.len(),
        in_place: 0,
        conflicts: vec![],
        duplicates: vec![],
        left: vec![],
        new_root: None,
        present: vec![],
        export: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_of_documents_is_paired_brought_in_found_and_undone() {
        let (dir, lib, ix) = crate::collection::tests::fixture("docs");
        let pack = dir.join("Downloads").join("Game Boy guides");
        std::fs::create_dir_all(pack.join("more")).unwrap();
        std::fs::write(pack.join("Alpha Manual.pdf"), b"%PDF-1.4 not a real one").unwrap();
        std::fs::write(pack.join("more").join("beta_strategy_guide.txt"), "Go left for the secret warp zone.").unwrap();
        std::fs::write(pack.join("Unrelated Thing.txt"), "nothing").unwrap();
        std::fs::write(pack.join("setup.exe"), "not a document").unwrap();
        let cancel = AtomicBool::new(false);
        // the folder's name says the system
        let items = read_items(&ix, &lib, &[pack.clone()], None, None, &cancel, &mut |_| {}).unwrap();
        assert_eq!(items.len(), 3, "{items:?}");
        let by = |n: &str| items.iter().find(|i| i.name == n).unwrap().clone();
        let a = by("Alpha Manual.pdf");
        // a game the user has, before the others of its title
        assert_eq!((a.game_name.as_deref(), a.pairing.as_str(), a.kind.as_str(), a.platform.as_deref()), (Some("Alpha (Europe)"), "sure", "manual", Some("gb")));
        assert_eq!(a.others.len(), 2);
        let b = by("beta_strategy_guide.txt");
        assert_eq!((b.game_name.as_deref(), b.kind.as_str(), b.title.as_str()), (Some("Beta (Europe)"), "guide", "beta strategy guide"));
        let u = by("Unrelated Thing.txt");
        assert!(u.game.is_none() && u.pairing == "none");
        // brought in: the guide's title changed, the unrelated one kept with no game
        let mut items = items;
        for i in items.iter_mut() {
            if i.name == "beta_strategy_guide.txt" {
                i.title = "Beta: the full guide".into();
            }
        }
        let plan = plan(&ix, &lib, &items, "copy", true).unwrap();
        assert_eq!((plan.kind.as_str(), plan.games), ("docs", 2));
        let r = organise::run(&ix, &lib, &plan, &cancel, &mut |_| {}).unwrap();
        assert_eq!(r["failed"], json!([]), "{r}");
        crate::meta::sync(&ix, &lib).unwrap();
        let games = lib.root().join("_library/games/gb");
        assert!(games.join("Alpha (Europe)/docs/Alpha Manual.pdf").is_file() && games.join("Alpha (Europe)/game.json").is_file());
        assert!(lib.root().join("_library/docs/gb/Unrelated Thing.txt").is_file());
        assert!(pack.join("Alpha Manual.pdf").is_file(), "copied, not moved");
        let all = search(&ix, "", "", "", "", 0, 50).unwrap();
        assert_eq!((all["total"].as_i64(), all["unread"].as_i64()), (Some(3), Some(3)), "{all}");
        let guide = all["items"].as_array().unwrap().iter().find(|d| d["kind"] == "guide").unwrap().clone();
        assert_eq!((guide["title"].as_str(), guide["game"].as_str()), (Some("Beta: the full guide"), Some("Beta")));
        // the text inside, once read
        assert_eq!(search(&ix, "warp", "", "", "", 0, 50).unwrap()["total"], 0);
        assert_eq!(read_texts(&ix, &lib, &cancel, &mut |_| {}).unwrap(), 3);
        let found = search(&ix, "warp zo", "", "", "", 0, 50).unwrap();
        assert_eq!(found["total"], 1, "{found}");
        assert!(found["items"][0]["snippet"].as_str().unwrap().contains("[warp]"), "{found}");
        assert_eq!(search(&ix, "", "manual", "gb", "yours", 0, 50).unwrap()["total"], 1);
        // a kind changed by hand is kept in docs.json
        let id = guide["id"].as_i64().unwrap();
        assert_eq!(set(&ix, &lib, id, Some("faq"), None).unwrap()["kind"], "faq");
        let m = read_json_object(&games.join("Beta (Europe)/docs/docs.json"));
        assert_eq!(m["files"]["beta_strategy_guide.txt"]["kind"], "faq");
        assert!(to_html("x.md", b"# Hi <script>x</script> [a](javascript:alert(1))").contains("&lt;script&gt;"));
        // Undo takes them out again
        organise::undo(&ix, &lib, &plan.id, &cancel, &mut |_| {}).unwrap();
        crate::meta::sync(&ix, &lib).unwrap();
        assert_eq!(search(&ix, "", "", "", "", 0, 50).unwrap()["total"], 0);
        assert!(!games.join("Alpha (Europe)/docs/Alpha Manual.pdf").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn kinds_and_titles_are_guessed_from_names() {
        assert_eq!(guess_kind("Zelda (USA) Manual.pdf"), "manual");
        assert_eq!(guess_kind("ff7_walkthrough.txt"), "guide");
        assert_eq!(guess_kind("World Map.png"), "map");
        assert_eq!(guess_kind("cover.jpg"), "box");
        assert_eq!(guess_kind("01 Title.mp3"), "soundtrack");
        assert_eq!(guess_kind("notes.pdf"), "readme");
        assert_eq!(title_words("Super_Metroid_(USA)_Manual [scan].pdf"), "super metroid");
    }
}
