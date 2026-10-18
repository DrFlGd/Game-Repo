//! A game's folder in the library (docs/PLAN.md, "Phase 5 design", D13, D32):
//! `_library/games/<system>/<DAT game name>/` with `game.json`, `media/` (scraped
//! pictures and video) and `docs/` (documents). The folder is found by the game's ROMs'
//! hashes, which `game.json` records, never by its name: the index keeps every
//! `game.json`'s ROMs (`meta_games`, `meta_roms`) so a DAT game is joined to its folder
//! by CRC32 and size. A ROM no DAT knows keeps Phase 4's `_library/games/<system>/<SHA1>/`.

use crate::config::{read_json_object, write_json};
use crate::identify;
use crate::index::Index;
use crate::library::{now, Library, APP_DIR};
use crate::organise::{new_id, safe_name};
use crate::platforms;
use anyhow::{anyhow, bail, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// The pictures (and video) a game can have: (kind, what it's called, ES-DE's folder for it).
pub const MEDIA: &[(&str, &str, &str)] = &[
    ("cover", "Box front", "covers"),
    ("back", "Box back", "backcovers"),
    ("box3d", "3D box", "3dboxes"),
    ("screenshot", "Screenshot", "screenshots"),
    ("titlescreen", "Title screen", "titlescreens"),
    ("marquee", "Marquee (logo)", "marquees"),
    ("fanart", "Fan art", "fanart"),
    ("physical", "Cartridge or disc", "physicalmedia"),
    ("video", "Video", "videos"),
];

/// A PC game's artwork beside its cover (Phase 4, kept as MEDIA is since D55): (kind, what
/// it's called, ES-DE's folder for it, none).
pub const PC_ART: &[(&str, &str, &str)] = &[("banner", "Banner", ""), ("hero", "Hero", ""), ("logo", "Logo", ""), ("icon", "Icon", "")];

/// Every kind of picture a game's folder can hold: MEDIA, then a PC game's.
pub fn all_media() -> impl Iterator<Item = &'static (&'static str, &'static str, &'static str)> {
    MEDIA.iter().chain(PC_ART.iter())
}

/// The details a game can have: (key, what it's called).
pub const FIELDS: &[(&str, &str)] = &[
    ("description", "Description"),
    ("developer", "Developer"),
    ("publisher", "Publisher"),
    ("released", "Release date"),
    ("genres", "Genres"),
    ("players", "Players"),
    ("rating", "Rating"),
];

/// Where every game's folder is.
pub fn games_root(lib: &Library) -> PathBuf {
    lib.root().join(APP_DIR).join("games")
}

/// A DAT game, as a scrape or a document's pairing needs it.
#[derive(Clone, Debug, Default)]
pub struct DatGame {
    pub id: i64,
    pub platform: String,
    pub dat: String,
    pub name: String,
    /// Its name without the tags ("Zeta (Japan) (Rev 1)" → "Zeta")
    pub title: String,
    pub regions: String,
    pub roms: Vec<Rom>,
}

#[derive(Clone, Debug, Default)]
pub struct Rom {
    pub name: String,
    pub size: Option<u64>,
    pub crc32: Option<String>,
    pub md5: Option<String>,
    pub sha1: Option<String>,
}

impl Rom {
    fn to_json(&self) -> Value {
        json!({ "name": self.name, "size": self.size, "crc32": self.crc32, "md5": self.md5, "sha1": self.sha1 })
    }
}

/// A DAT game by its id in the index.
pub fn dat_game(conn: &Connection, id: i64) -> Result<DatGame> {
    let mut g = conn
        .query_row(
            "SELECT g.id, d.platform, d.name, g.name, COALESCE(g.title, ''), COALESCE(g.regions, '') FROM dat_games g JOIN dats d ON d.id = g.dat_id WHERE g.id = ?1",
            [id],
            |r| Ok(DatGame { id: r.get(0)?, platform: r.get(1)?, dat: r.get(2)?, name: r.get(3)?, title: r.get(4)?, regions: r.get(5)?, roms: vec![] }),
        )
        .map_err(|_| anyhow!("There's no game {id} in the DATs."))?;
    if g.title.is_empty() {
        g.title = crate::dat::parse_name(&g.name).title;
    }
    let mut q = conn.prepare("SELECT name, size, crc32, md5, sha1 FROM dat_roms WHERE game_id = ?1 AND (status IS NULL OR status != 'nodump') ORDER BY id")?;
    g.roms = q
        .query_map([id], |r| Ok(Rom { name: r.get(0)?, size: r.get::<_, Option<i64>>(1)?.map(|s| s as u64), crc32: r.get(2)?, md5: r.get(3)?, sha1: r.get(4)? }))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(g)
}

/// A DAT game by its system and name (the system's current DATs, its audit DAT first).
pub fn dat_game_named(conn: &Connection, lib: &Library, platform: &str, name: &str) -> Result<Option<DatGame>> {
    let primary = crate::audit::primary_dat(conn, lib, platform)?;
    let id: Option<i64> = conn
        .query_row(
            "SELECT g.id FROM dat_games g JOIN dats d ON d.id = g.dat_id WHERE d.platform = ?1 AND g.name = ?2
             ORDER BY (d.id = ?3) DESC, d.current DESC, d.id DESC LIMIT 1",
            params![platform, name, primary.unwrap_or(-1)],
            |r| r.get(0),
        )
        .optional()?;
    id.map(|id| dat_game(conn, id)).transpose()
}

/// The folder of a DAT game, if it has one: the folder holding exactly its ROMs (by CRC32
/// and size), else one made for a game of its name on the same system (its DAT changed).
/// A ROM shared by two games (a disc's track, a BIOS) can't decide alone.
pub fn folder_of(conn: &Connection, g: &DatGame) -> Result<Option<String>> {
    let mut q = conn.prepare_cached("SELECT mr.folder FROM meta_roms mr JOIN meta_games mg ON mg.folder = mr.folder WHERE mr.crc32 = ?1 AND mr.size = ?2 AND mg.platform = ?3")?;
    let mut votes: HashMap<String, usize> = HashMap::new();
    let mut want = 0;
    for r in &g.roms {
        let (Some(crc), Some(size)) = (&r.crc32, r.size) else { continue };
        want += 1;
        for f in q.query_map(params![crc.to_lowercase(), size as i64, g.platform], |r| r.get::<_, String>(0))? {
            *votes.entry(f?).or_default() += 1;
        }
    }
    let mut count = conn.prepare_cached("SELECT COUNT(*) FROM meta_roms WHERE folder = ?1")?;
    let mut whole: Vec<String> = vec![];
    for (f, n) in &votes {
        if *n == want && count.query_row([f], |r| r.get::<_, i64>(0))? == want as i64 {
            whole.push(f.clone());
        }
    }
    whole.sort();
    if let Some(f) = whole.into_iter().next() {
        return Ok(Some(f));
    }
    Ok(conn
        .prepare_cached("SELECT folder FROM meta_games WHERE platform = ?1 AND name = ?2 ORDER BY folder LIMIT 1")?
        .query_row(params![g.platform, g.name], |r| r.get(0))
        .optional()?)
}

/// A game's folder from its library-relative form, checked to be one.
pub fn resolve(lib: &Library, folder: &str) -> Result<PathBuf> {
    let p = lib.resolve(folder)?;
    let root = games_root(lib);
    if !p.starts_with(&root) || p.parent().and_then(Path::parent) != Some(root.as_path()) {
        bail!("{folder} isn't a game's folder.");
    }
    Ok(p)
}

/// The folder a DAT game's things go in: its own, else a new one named after it.
pub fn ensure_folder(ix: &Index, lib: &Library, g: &DatGame) -> Result<PathBuf> {
    lib.writable()?;
    if let Some(f) = folder_of(ix.conn(), g)? {
        let p = lib.resolve(&f)?;
        if p.join("game.json").is_file() {
            return Ok(p);
        }
    }
    platforms::valid_id(&g.platform)?;
    let base = games_root(lib).join(&g.platform);
    let mut dir = base.join(safe_name(&g.name));
    let mut n = 2;
    // a folder of that name for another game (by its ROMs) gets a number
    while dir.join("game.json").is_file() {
        dir = base.join(format!("{} ({n})", safe_name(&g.name)));
        n += 1;
    }
    let v = json!({
        "id": new_id(), "platform": g.platform, "name": g.name, "dat": g.dat, "title": g.title,
        "roms": g.roms.iter().map(Rom::to_json).collect::<Vec<_>>(), "created": now(),
    });
    std::fs::create_dir_all(&dir)?;
    write_game(ix, lib, &dir, &v)?;
    Ok(dir)
}

pub fn read_game(dir: &Path) -> Value {
    read_json_object(&dir.join("game.json"))
}

/// Write a game's `game.json` and bring the index's row for it up to date.
pub fn write_game(ix: &Index, lib: &Library, dir: &Path, v: &Value) -> Result<()> {
    lib.writable()?;
    write_json(&dir.join("game.json"), v)?;
    index_folder(ix.conn(), lib, dir)
}

/// A game folder's `game.json` size, and its modified time added to its `media/` folder's,
/// so pictures added or removed by hand are seen too. Added rather than the later of the
/// two: Windows can give both the same time when a picture comes just after an edit.
fn stamp(p: &Path) -> Option<(i64, i64)> {
    let ms = |m: &std::fs::Metadata| m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_millis() as i64);
    let m = std::fs::metadata(p).ok()?;
    let media = p.parent().and_then(|d| std::fs::metadata(d.join("media")).ok()).and_then(|m| ms(&m)).unwrap_or(0);
    Some((m.len() as i64, ms(&m)? + media))
}

/// The kinds of media a game's folder has.
pub fn media_kinds(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir.join("media"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_string_lossy().split_once('.').map(|(k, _)| k.to_string()))
        .collect();
    v.sort();
    v.dedup();
    v
}

/// The picture a game is shown by (its box front, else its 3D box, screenshot, title
/// screen or fan art): its file name in `media/`.
pub fn picture_of(dir: &Path) -> Option<String> {
    let names: Vec<String> = std::fs::read_dir(dir.join("media")).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    ["cover", "box3d", "screenshot", "titlescreen", "fanart"].iter().find_map(|k| {
        names.iter().filter(|n| n.split_once('.').is_some_and(|(s, x)| s == *k && ["png", "jpg", "jpeg", "webp", "gif", "bmp"].contains(&x.to_ascii_lowercase().as_str()))).min().cloned()
    })
}

/// Read one game's folder into the index.
pub fn index_folder(conn: &Connection, lib: &Library, dir: &Path) -> Result<()> {
    let Some(rel) = lib.relative(dir) else { return Ok(()) };
    let p = dir.join("game.json");
    let Some((size, mtime)) = stamp(&p) else {
        conn.execute("DELETE FROM meta_games WHERE folder = ?1", [&rel])?;
        return Ok(());
    };
    let v = read_json_object(&p);
    let platform = v["platform"].as_str().map(String::from).or_else(|| dir.parent().and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned())).unwrap_or_default();
    let details = v["details"].as_object().is_some_and(|d| FIELDS.iter().any(|(k, _)| d.get(*k).is_some_and(|x| !x.is_null() && x != "")));
    let review = v["review"].as_array().is_some_and(|a| !a.is_empty());
    let mut title = v["title"].as_str().filter(|s| !s.is_empty()).or(v["name"].as_str()).unwrap_or("").to_string();
    // a PC game's title is its own, in `PC/…/game.json` (D55)
    if title.is_empty() && crate::pcmeta::is_pc(lib, dir) {
        title = crate::pcmeta::title_of(lib, dir).unwrap_or_default();
    }
    // the details the Library page lists and searches (browse.rs), and the picture it shows
    let facts = v["details"].as_object().cloned().unwrap_or_default();
    conn.execute(
        "INSERT INTO meta_games (folder, platform, id, name, title, year, details, media, review, size, mtime, facts, picture) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
         ON CONFLICT(folder) DO UPDATE SET platform = excluded.platform, id = excluded.id, name = excluded.name, title = excluded.title, year = excluded.year,
           details = excluded.details, media = excluded.media, review = excluded.review, size = excluded.size, mtime = excluded.mtime,
           facts = excluded.facts, picture = excluded.picture",
        params![rel, platform, v["id"].as_str(), v["name"].as_str(), title, v["year"].as_i64(), details as i64, media_kinds(dir).join(","), review as i64, size, mtime,
                serde_json::to_string(&facts)?, picture_of(dir)],
    )?;
    conn.execute("DELETE FROM meta_roms WHERE folder = ?1", [&rel])?;
    // a ROM no DAT knows: its one file's hashes
    let roms = v["roms"].as_array().or(v["files"].as_array()).cloned().unwrap_or_default();
    let mut q = conn.prepare_cached("INSERT INTO meta_roms (folder, sha1, crc32, size) VALUES (?1, ?2, ?3, ?4)")?;
    for r in roms {
        q.execute(params![rel, r["sha1"].as_str().map(str::to_lowercase), r["crc32"].as_str().map(str::to_lowercase), r["size"].as_i64()])?;
    }
    Ok(())
}

/// Bring the index's game folders and documents up to date with the library: each
/// `game.json` read again only when its size or modified time changed.
pub fn sync(ix: &Index, lib: &Library) -> Result<()> {
    let conn = ix.conn();
    let known: HashMap<String, (i64, i64)> = {
        let mut q = conn.prepare("SELECT folder, size, mtime FROM meta_games")?;
        let rows = q.query_map([], |r| Ok((r.get::<_, String>(0)?, (r.get(1)?, r.get(2)?))))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut seen = HashSet::new();
    let tx = conn.unchecked_transaction()?;
    for sys in std::fs::read_dir(games_root(lib)).into_iter().flatten().flatten() {
        // not the set-aside folder (metadata deleted, D48)
        if !sys.path().is_dir() || sys.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        for g in std::fs::read_dir(sys.path()).into_iter().flatten().flatten() {
            let dir = g.path();
            let Some(rel) = lib.relative(&dir) else { continue };
            let Some(st) = stamp(&dir.join("game.json")) else { continue };
            seen.insert(rel.clone());
            if known.get(&rel) != Some(&st) {
                index_folder(&tx, lib, &dir)?;
            }
        }
    }
    for f in known.keys().filter(|f| !seen.contains(*f)) {
        tx.execute("DELETE FROM meta_games WHERE folder = ?1", [f])?;
    }
    tx.commit()?;
    crate::docs::sync(ix, lib)?;
    Ok(())
}

/// Refresh one folder's media kinds in the index (after pictures were fetched or removed).
pub fn touch(ix: &Index, lib: &Library, dir: &Path) -> Result<()> {
    index_folder(ix.conn(), lib, dir)
}

// ------------------------------------------------------------ the Metadata page

/// What the Metadata page lists: `show` is have (the games in the library, the default),
/// all (every game in the DAT), look (needing a look) or none (with nothing scraped).
pub fn list(ix: &Index, lib: &Library, platform: &str, show: &str, search: &str, offset: i64, limit: i64) -> Result<Value> {
    let conn = ix.conn();
    let Some(dat) = crate::audit::primary_dat(conn, lib, platform)? else {
        return Ok(json!({ "platform": platform, "dat": null, "total": 0, "items": [], "counts": {} }));
    };
    let d = crate::audit::dat_ref(conn, dat)?;
    let base = format!(
        "WITH {},
              fv AS (SELECT r.game_id, mr.folder, COUNT(*) AS n FROM dat_roms r JOIN meta_roms mr ON mr.crc32 = r.crc32 AND mr.size = r.size
                     JOIN meta_games mg ON mg.folder = mr.folder AND mg.platform = ?2 WHERE r.dat_id = ?1 GROUP BY r.game_id, mr.folder),
              gr AS (SELECT game_id, COUNT(*) AS n FROM dat_roms WHERE dat_id = ?1 AND crc32 IS NOT NULL AND size IS NOT NULL GROUP BY game_id),
              fr AS (SELECT folder, COUNT(*) AS n FROM meta_roms GROUP BY folder),
              whole AS (SELECT fv.game_id, MIN(fv.folder) AS folder FROM fv JOIN gr ON gr.game_id = fv.game_id AND gr.n = fv.n
                        JOIN fr ON fr.folder = fv.folder AND fr.n = fv.n GROUP BY fv.game_id),
              -- as folder_of: the folder with exactly its ROMs, else one of its name
              fm AS (SELECT gs.id AS game_id, COALESCE(whole.folder, (SELECT mg.folder FROM meta_games mg WHERE mg.platform = ?2 AND mg.name = gs.name ORDER BY mg.folder LIMIT 1)) AS folder
                     FROM gs LEFT JOIN whole ON whole.game_id = gs.id),
              ls AS (SELECT gs.id, gs.name, gs.title, gs.status, gs.regions, mg.folder, COALESCE(mg.details, 0) AS details, COALESCE(mg.media, '') AS media,
                            COALESCE(mg.review, 0) AS review, mg.title AS meta_title, mg.year,
                            (SELECT COUNT(*) FROM docs dc WHERE dc.folder = mg.folder) AS docs
                     FROM gs LEFT JOIN fm ON fm.game_id = gs.id LEFT JOIN meta_games mg ON mg.folder = fm.folder)",
        crate::audit::GAMES
    );
    let counts: (i64, i64, i64, i64, i64) = conn.query_row(
        &format!("{base} SELECT COUNT(*), COALESCE(SUM(status != 'missing'), 0), COALESCE(SUM(details), 0), COALESCE(SUM(media != ''), 0), COALESCE(SUM(review), 0) FROM ls"),
        params![dat, platform],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
    )?;
    let filter = match show {
        "all" => "1",
        "look" => "review = 1",
        "none" => "details = 0 AND media = '' AND status != 'missing'",
        _ => "status != 'missing'",
    };
    let like = format!("%{}%", search.trim().replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
    let where_ = format!("{filter} AND (?3 = '%%' OR name LIKE ?3 ESCAPE '\\' OR COALESCE(meta_title, '') LIKE ?3 ESCAPE '\\')");
    let total: i64 = conn.query_row(&format!("{base} SELECT COUNT(*) FROM ls WHERE {where_}"), params![dat, platform, like], |r| r.get(0))?;
    let mut q = conn.prepare(&format!("{base} SELECT id, name, title, status, folder, details, media, review, meta_title, year, docs FROM ls WHERE {where_} ORDER BY name LIMIT ?4 OFFSET ?5"))?;
    let items: Vec<Value> = q
        .query_map(params![dat, platform, like, limit.clamp(1, 1000), offset.max(0)], |r| {
            let media: String = r.get(6)?;
            Ok(json!({
                "id": r.get::<_, i64>(0)?, "name": r.get::<_, String>(1)?, "title": r.get::<_, Option<String>>(8)?.or(r.get::<_, Option<String>>(2)?),
                "status": r.get::<_, String>(3)?, "folder": r.get::<_, Option<String>>(4)?, "details": r.get::<_, i64>(5)? == 1,
                "media": media.split(',').filter(|s| !s.is_empty()).collect::<Vec<_>>(), "review": r.get::<_, i64>(7)? == 1,
                "year": r.get::<_, Option<i64>>(9)?, "docs": r.get::<_, i64>(10)?,
            }))
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(json!({
        "platform": platform, "platform_name": platforms::name_of(platform),
        "dat": { "id": d.id, "name": d.name, "version": d.version },
        "total": total, "items": items,
        "counts": { "all": counts.0, "have": counts.1, "details": counts.2, "media": counts.3, "look": counts.4 },
    }))
}

/// One game as its panel shows it: the DAT game, what its folder holds, its documents.
pub fn game(ix: &Index, lib: &Library, dat_game_id: i64) -> Result<Value> {
    let conn = ix.conn();
    let g = dat_game(conn, dat_game_id)?;
    let folder = folder_of(conn, &g)?;
    let have: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM matches m JOIN dat_roms r ON r.id = m.rom_id WHERE r.game_id = ?1)",
        [g.id],
        |r| r.get(0),
    )?;
    let mut v = json!({
        "id": g.id, "platform": g.platform, "platform_name": platforms::name_of(&g.platform), "dat": g.dat, "name": g.name,
        "title": g.title, "regions": g.regions, "have": have, "folder": folder,
        "roms": g.roms.iter().map(Rom::to_json).collect::<Vec<_>>(),
        "details": {}, "media": {}, "docs": [], "ids": {}, "review": [], "scraped": {}, "edited": [], "pinned": {}, "sources": [], "variants": {},
    });
    if let Some(f) = &folder {
        let dir = lib.resolve(f)?;
        let j = read_game(&dir);
        for k in ["details", "ids", "review", "scraped", "edited", "pinned"] {
            if !j[k].is_null() {
                v[k] = j[k].clone();
            }
        }
        // what each source said, and each kind's pictures from each (D46, D47)
        v["sources"] = json!(crate::sources::answers(&dir).into_iter().map(|(s, mut a)| {
            a["name"] = json!(crate::sources::source_name(&s));
            a
        }).collect::<Vec<_>>());
        v["variants"] = json!(crate::sources::variants(lib, &dir, &j));
        if let Some(t) = j["title"].as_str().filter(|s| !s.is_empty()) {
            v["title"] = json!(t);
        }
        v["year"] = j["year"].clone();
        v["folder_path"] = json!(dir.join("game.json").display().to_string());
        v["media"] = json!(identify::media_of(lib, &dir.join("media")));
        v["docs"] = json!(crate::docs::of_folder(conn, f)?);
    }
    Ok(v)
}

/// Clear what was scraped for a game: its details, pictures and scraped documents (the
/// user's own documents stay), and its candidates.
pub fn clear(ix: &Index, lib: &Library, dat_game_id: i64) -> Result<Value> {
    lib.writable()?;
    let g = dat_game(ix.conn(), dat_game_id)?;
    if let Some(f) = folder_of(ix.conn(), &g)? {
        let dir = lib.resolve(&f)?;
        let _ = std::fs::remove_dir_all(dir.join("media"));
        let _ = std::fs::remove_dir_all(dir.join(crate::sources::DIR));
        crate::docs::remove_scraped(ix, lib, &dir)?;
        let mut v = read_game(&dir);
        if let Some(o) = v.as_object_mut() {
            for k in ["details", "review", "scraped", "year", "ids", "chosen", "pinned", "pinned_media"] {
                o.remove(k);
            }
            o.insert("title".into(), json!(g.title));
        }
        write_game(ix, lib, &dir, &v)?;
    }
    game(ix, lib, dat_game_id)
}

/// Change one of a game's details by hand (D44): `title`, `year` or one of FIELDS. It's
/// marked as edited, so later scrapes leave it; `None` takes the mark off (the sources'
/// value comes back, D46).
pub fn edit(ix: &Index, lib: &Library, dat_game_id: i64, field: &str, value: Option<&Value>) -> Result<Value> {
    lib.writable()?;
    edit_value(field, &Value::Null)?;
    let g = dat_game(ix.conn(), dat_game_id)?;
    let dir = ensure_folder(ix, lib, &g)?;
    edit_dir(ix, lib, &dir, field, value)?;
    game(ix, lib, dat_game_id)
}

/// A hand-edited value as it's kept: a year a number, genres a list, a rating 0 to 1.
pub fn edit_value(field: &str, x: &Value) -> Result<Value> {
    if field != "title" && field != "year" && !FIELDS.iter().any(|(k, _)| *k == field) {
        bail!("There's no detail called {field}.");
    }
    Ok(match field {
        "year" => match x.as_i64().or_else(|| x.as_str().and_then(|s| s.trim().parse().ok())) {
            Some(y) => json!(y),
            None if x.is_null() || x == "" => Value::Null,
            None => bail!("A year is a number, such as 1991."),
        },
        "genres" => match x {
            Value::String(s) => json!(s.split(',').map(str::trim).filter(|s| !s.is_empty()).collect::<Vec<_>>()),
            _ => x.clone(),
        },
        "rating" => match x.as_f64().or_else(|| x.as_str().and_then(|s| s.trim().parse().ok())) {
            Some(r) if (0.0..=1.0).contains(&r) => json!(r),
            Some(_) => bail!("A rating is from 0 to 1 (0.8 is four stars of five)."),
            None => Value::Null,
        },
        "title" => match x.as_str().map(str::trim) {
            Some(t) if !t.is_empty() => json!(t),
            _ if x.is_null() => Value::Null,
            _ => bail!("Give the game a title."),
        },
        _ => x.clone(),
    })
}

/// Edit one of the details in a game folder's `game.json` (D44).
pub fn edit_dir(ix: &Index, lib: &Library, dir: &Path, field: &str, value: Option<&Value>) -> Result<()> {
    lib.writable()?;
    let pc = crate::pcmeta::is_pc(lib, dir);
    if pc {
        // a PC game's details from before are brought across first (D55)
        crate::pcmeta::ensure_dir(ix, lib, dir)?;
        if field == "title" || field == "year" {
            bail!("A PC game's {field} is its own (its folder is named by it): change it on its page.");
        }
    }
    let mut v = read_game(dir);
    if !pc {
        // a folder from before is split while its edits are still marked
        crate::sources::prepare(dir, &v)?;
    }
    let mut edited: Vec<Value> = v["edited"].as_array().cloned().unwrap_or_default();
    edited.retain(|e| e != field);
    if let Some(x) = value {
        let x = edit_value(field, x)?;
        match field {
            "title" | "year" => v[field] = x,
            _ => {
                if !v["details"].is_object() {
                    v["details"] = json!({});
                }
                v["details"][field] = x;
            }
        }
        if !v["scraped"].is_object() {
            v["scraped"] = json!({});
        }
        v["scraped"][field] = json!("you");
        edited.push(json!(field));
    }
    v["edited"] = json!(edited);
    // the mark taken off: the sources' value again (D46)
    if value.is_none() {
        if pc {
            crate::pcmeta::make(lib, dir, &mut v);
        } else {
            crate::sources::make(&mut v, dir, &crate::sources::Rules::of(lib));
        }
    }
    write_game(ix, lib, dir, &v)
}

/// The game folders whose names are the old names of their games, after a DAT renamed
/// them (D45): a change renaming each to its game's name in the DAT in use, with a preview
/// and Undo. A folder is its game's by its ROMs; one whose ROMs are several games', or
/// whose new name is taken, is left (and said why).
pub fn rename_folders_plan(ix: &Index, lib: &Library) -> Result<crate::organise::Plan> {
    use crate::organise::{Op, Plan, Src};
    lib.writable()?;
    let conn = ix.conn();
    let platforms: Vec<String> = conn.prepare("SELECT DISTINCT platform FROM meta_games WHERE platform != 'pc' ORDER BY platform")?.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    let (mut ops, mut left, mut in_place) = (vec![], vec![], 0);
    let mut taken: HashSet<PathBuf> = HashSet::new();
    for p in &platforms {
        let Some(dat) = crate::audit::primary_dat(conn, lib, p)? else { continue };
        // each folder with exactly a DAT game's ROMs, and the games it's
        let mut q = conn.prepare(
            "WITH fv AS (SELECT r.game_id, mr.folder, COUNT(*) AS n FROM dat_roms r JOIN meta_roms mr ON mr.crc32 = r.crc32 AND mr.size = r.size
                         JOIN meta_games mg ON mg.folder = mr.folder AND mg.platform = ?2 WHERE r.dat_id = ?1 GROUP BY r.game_id, mr.folder),
                  gr AS (SELECT game_id, COUNT(*) AS n FROM dat_roms WHERE dat_id = ?1 AND crc32 IS NOT NULL AND size IS NOT NULL GROUP BY game_id),
                  fr AS (SELECT folder, COUNT(*) AS n FROM meta_roms GROUP BY folder)
             SELECT fv.folder, g.name FROM fv JOIN gr ON gr.game_id = fv.game_id AND gr.n = fv.n JOIN fr ON fr.folder = fv.folder AND fr.n = fv.n
             JOIN dat_games g ON g.id = fv.game_id ORDER BY fv.folder, g.name",
        )?;
        let mut by: Vec<(String, Vec<String>)> = vec![];
        for x in q.query_map(params![dat, p], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (f, g) = x?;
            match by.last_mut() {
                Some((lf, gs)) if *lf == f => gs.push(g),
                _ => by.push((f, vec![g])),
            }
        }
        for (folder, games) in by {
            let dir = lib.resolve(&folder)?;
            let now = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if games.iter().any(|g| safe_name(g) == now) {
                in_place += 1;
                continue;
            }
            if games.len() > 1 {
                left.push(json!({ "path": dir.display().to_string(), "why": format!("Its ROMs are those of {} games in the DAT ({}), so it keeps its name.", games.len(), games.join(", ")) }));
                continue;
            }
            let to = dir.with_file_name(safe_name(&games[0]));
            if to.exists() || !taken.insert(to.clone()) {
                left.push(json!({ "path": dir.display().to_string(), "why": format!("A folder called {} is there already.", safe_name(&games[0])) }));
                continue;
            }
            ops.push(Op {
                op: "rename".into(),
                game: games[0].clone(),
                to: to.display().to_string(),
                from: vec![Src { file_id: 0, root: String::new(), rel: folder.clone(), path: dir.display().to_string(), member: None, size: 0, crc32: String::new(), sha1: None }],
                names: vec![],
                aside: vec![],
                size: 0,
                note: Some(format!("was {now}")),
            });
        }
    }
    let n = ops.len();
    Ok(Plan {
        id: new_id(),
        created: now(),
        kind: "folders".into(),
        label: format!("Renamed {n} game{} library folder{} to {} DAT name{}", if n == 1 { "'s" } else { "s'" }, if n == 1 { "" } else { "s" }, if n == 1 { "its" } else { "their" }, if n == 1 { "" } else { "s" }),
        title: format!("Rename {n} game{} library folder{} to {} DAT name{}", if n == 1 { "'s" } else { "s'" }, if n == 1 { "" } else { "s" }, if n == 1 { "its" } else { "their" }, if n == 1 { "" } else { "s" }),
        dat: Value::Null,
        platform: if platforms.len() == 1 { platforms[0].clone() } else { String::new() },
        mode: "rename".into(),
        format: "keep".into(),
        out: Some(games_root(lib).display().to_string()),
        space: vec![],
        games: n,
        ops,
        in_place,
        conflicts: vec![],
        duplicates: vec![],
        left,
        new_root: None, new_roots: vec![],
        present: vec![],
        export: None,
    })
}

/// Put what a scrape (or Identify) found into a game's `game.json`.
pub struct Found<'a> {
    pub source: &'a str,
    pub id: &'a str,
    pub title: Option<&'a str>,
    pub year: Option<i32>,
    pub details: &'a Map<String, Value>,
}

/// Merge found details into a game: `replace` writes over what's there, else only
/// what's missing is filled; `fields` limits which are taken. Returns the fields set.
pub fn merge_details(v: &mut Value, f: &Found, fields: &[String], replace: bool) -> Vec<String> {
    if !v["details"].is_object() {
        v["details"] = json!({});
    }
    if !v["scraped"].is_object() {
        v["scraped"] = json!({});
    }
    if !v["ids"].is_object() {
        v["ids"] = json!({});
    }
    v["ids"][f.source] = json!(f.id);
    let mut set = vec![];
    // what was edited by hand stays (D44)
    let edited: Vec<String> = v["edited"].as_array().into_iter().flatten().filter_map(|e| e.as_str().map(String::from)).collect();
    for (k, x) in f.details {
        if !fields.iter().any(|w| w == k) || x.is_null() || x == "" || edited.contains(k) {
            continue;
        }
        let has = v["details"].get(k).is_some_and(|y| !y.is_null() && y != "");
        if has && !replace {
            continue;
        }
        v["details"][k] = x.clone();
        v["scraped"][k] = json!(f.source);
        set.push(k.clone());
    }
    if let Some(t) = f.title.filter(|t| !t.is_empty() && !edited.iter().any(|e| e == "title")) {
        if replace || v["scraped"]["title"].is_null() {
            v["title"] = json!(t);
            v["scraped"]["title"] = json!(f.source);
        }
    }
    if f.year.is_some() && (replace || v["year"].is_null()) && !edited.iter().any(|e| e == "year") {
        v["year"] = json!(f.year);
    }
    v["scraped"]["when"] = json!(now());
    set
}

/// A system's DAT games whose names have these words (for pairing a document by hand).
pub fn find_games(ix: &Index, lib: &Library, platform: &str, query: &str, limit: i64) -> Result<Value> {
    let conn = ix.conn();
    let Some(dat) = crate::audit::primary_dat(conn, lib, platform)? else { return Ok(json!([])) };
    let words: Vec<String> = crate::launchbox::words(query).split(' ').filter(|w| !w.is_empty()).map(|w| format!("%{w}%")).take(6).collect();
    let mut sql = format!("WITH {} SELECT id, name, COALESCE(title, ''), status FROM gs WHERE 1", crate::audit::GAMES);
    for i in 0..words.len() {
        sql.push_str(&format!(" AND lower(name) LIKE ?{}", i + 2));
    }
    sql.push_str(&format!(" ORDER BY status = 'missing', name LIMIT {}", limit.clamp(1, 200)));
    let mut q = conn.prepare(&sql)?;
    let mut p: Vec<&dyn rusqlite::ToSql> = vec![&dat];
    for w in &words {
        p.push(w);
    }
    let v: Vec<Value> = q
        .query_map(p.as_slice(), |r| Ok(json!({ "id": r.get::<_, i64>(0)?, "name": r.get::<_, String>(1)?, "title": r.get::<_, String>(2)?, "have": r.get::<_, String>(3)? != "missing", "platform": platform })))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(json!(v))
}
