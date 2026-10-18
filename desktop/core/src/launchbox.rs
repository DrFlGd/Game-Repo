//! LaunchBox Games DB (docs/PLAN.md, "Phase 4 design", D27): LaunchBox publishes its
//! whole games database as one `Metadata.zip` (an XML file of every game, its other
//! names and its images). It's downloaded when asked (Settings › Sources), read into
//! `<data dir>/launchbox.sqlite` on this computer and searched there, so Identify can
//! answer for any system without an account and without asking a server each time.

use crate::platforms;
use anyhow::{anyhow, bail, Context, Result};
use quick_xml::events::Event;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Where LaunchBox publishes its database.
pub const ZIP_URL: &str = "https://gamesdb.launchbox-app.com/Metadata.zip";
/// Where its images are, by file name.
pub const IMAGES: &str = "https://images.launchbox-app.com/";

pub fn db_path(data_dir: &Path) -> PathBuf {
    data_dir.join("launchbox.sqlite")
}

/// LaunchBox's names for the app's systems (where they differ from the app's own).
pub fn platform_names(id: &str) -> Vec<String> {
    let names: &[&str] = match id {
        "pc" => &["Windows", "MS-DOS", "Windows 3.X"],
        "nes" => &["Nintendo Entertainment System"],
        "fds" => &["Nintendo Famicom Disk System"],
        "snes" => &["Super Nintendo Entertainment System"],
        "n64" => &["Nintendo 64"],
        "gb" => &["Nintendo Game Boy"],
        "gbc" => &["Nintendo Game Boy Color"],
        "gba" => &["Nintendo Game Boy Advance"],
        "nds" => &["Nintendo DS"],
        "virtualboy" => &["Nintendo Virtual Boy"],
        "pokemini" => &["Nintendo Pokemon Mini"],
        "genesis" => &["Sega Genesis"],
        "mastersystem" => &["Sega Master System"],
        "gamegear" => &["Sega Game Gear"],
        "sega32x" => &["Sega 32X"],
        "sg-1000" => &["Sega SG-1000"],
        "atari2600" => &["Atari 2600"],
        "atari5200" => &["Atari 5200"],
        "atari7800" => &["Atari 7800"],
        "atarilynx" => &["Atari Lynx"],
        "atarijaguar" => &["Atari Jaguar"],
        "atarijaguarcd" => &["Atari Jaguar CD"],
        "pcengine" => &["NEC TurboGrafx-16"],
        "pcenginecd" => &["NEC TurboGrafx-CD"],
        "pcfx" => &["NEC PC-FX"],
        "ngp" => &["SNK Neo Geo Pocket"],
        "ngpc" => &["SNK Neo Geo Pocket Color"],
        "neogeocd" => &["SNK Neo Geo CD"],
        "wonderswan" => &["WonderSwan"],
        "wonderswancolor" => &["WonderSwan Color"],
        "colecovision" => &["ColecoVision"],
        "intellivision" => &["Mattel Intellivision"],
        "vectrex" => &["GCE Vectrex"],
        "psx" => &["Sony Playstation"],
        "ps2" => &["Sony Playstation 2"],
        "psp" => &["Sony PSP"],
        "saturn" => &["Sega Saturn"],
        "segacd" => &["Sega CD"],
        "dreamcast" => &["Sega Dreamcast"],
        "3do" => &["3DO Interactive Multiplayer"],
        "cdimono1" => &["Philips CD-i"],
        "gc" => &["Nintendo GameCube"],
        "wii" => &["Nintendo Wii"],
        "xbox" => &["Microsoft Xbox"],
        "n3ds" => &["Nintendo 3DS"],
        "wiiu" => &["Nintendo Wii U"],
        "switch" => &["Nintendo Switch"],
        "ps3" => &["Sony Playstation 3"],
        "ps4" => &["Sony Playstation 4"],
        "psvita" => &["Sony Playstation Vita"],
        "xbox360" => &["Microsoft Xbox 360"],
        _ => &[],
    };
    if names.is_empty() {
        vec![platforms::name_of(id)]
    } else {
        names.iter().map(|s| s.to_string()).collect()
    }
}

/// A name as searched: lower case words of letters and digits, one space between.
pub fn words(s: &str) -> String {
    let s = s.to_lowercase().replace('&', " and ");
    s.chars().map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
}

const SCHEMA: &str = "
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE games (id INTEGER PRIMARY KEY, name TEXT NOT NULL, words TEXT NOT NULL, platform TEXT NOT NULL, year INTEGER,
                    released TEXT, developer TEXT, publisher TEXT, genres TEXT, players TEXT, overview TEXT, rating TEXT);
CREATE TABLE names (game_id INTEGER NOT NULL, name TEXT NOT NULL, words TEXT NOT NULL, region TEXT);
CREATE TABLE images (game_id INTEGER NOT NULL, file TEXT NOT NULL, type TEXT NOT NULL, region TEXT);
CREATE INDEX games_platform ON games (platform);
CREATE INDEX names_game ON names (game_id);
CREATE INDEX images_game ON images (game_id);
";

/// What's known about the database on this computer: when it was fetched and how many games.
pub fn status(data_dir: &Path) -> Value {
    let p = db_path(data_dir);
    let Ok(c) = Connection::open_with_flags(&p, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY) else {
        return json!({ "have": false });
    };
    let get = |k: &str| c.query_row("SELECT value FROM meta WHERE key = ?1", [k], |r| r.get::<_, String>(0)).optional().ok().flatten();
    match get("games") {
        Some(n) => json!({ "have": true, "games": n.parse::<u64>().unwrap_or(0), "fetched": get("fetched"), "source": get("source") }),
        None => json!({ "have": false }),
    }
}

/// Download LaunchBox's Metadata.zip (from `url`) and read it into the database on this
/// computer, replacing the one there.
pub fn fetch(data_dir: &Path, url: &str, cancel: &AtomicBool, progress: &mut dyn FnMut(Value)) -> Result<Value> {
    std::fs::create_dir_all(data_dir)?;
    let zip_path = data_dir.join("launchbox-Metadata.zip");
    let tmp = zip_path.with_extension("zip.part");
    {
        let mut r = ureq::get(url).header("User-Agent", concat!("GameRepo/", env!("CARGO_PKG_VERSION"))).call().map_err(|e| anyhow!("couldn't download {url}: {e}"))?;
        let total: u64 = r.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok()).unwrap_or(0);
        let mut body = r.body_mut().with_config().limit(2 << 30).reader();
        let mut out = std::fs::File::create(&tmp)?;
        let mut buf = vec![0u8; 1 << 20];
        let mut done: u64 = 0;
        let mut last = 0;
        loop {
            if cancel.load(Ordering::Relaxed) {
                drop(out);
                let _ = std::fs::remove_file(&tmp);
                bail!("stopped");
            }
            let n = body.read(&mut buf)?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n])?;
            done += n as u64;
            if done - last > 4 << 20 {
                last = done;
                progress(json!({ "name": "Downloading Metadata.zip", "done": done, "total": total }));
            }
        }
        out.sync_all()?;
    }
    std::fs::rename(&tmp, &zip_path)?;
    let r = build(data_dir, &zip_path, url, cancel, progress);
    let _ = std::fs::remove_file(&zip_path);
    r
}

/// Read a Metadata.zip into the database (through a new file, so a failure leaves the old one).
pub fn build(data_dir: &Path, zip_path: &Path, source: &str, cancel: &AtomicBool, progress: &mut dyn FnMut(Value)) -> Result<Value> {
    let f = std::fs::File::open(zip_path)?;
    let mut z = zip::ZipArchive::new(BufReader::new(f)).context("Metadata.zip isn't a ZIP")?;
    let idx = (0..z.len())
        .find(|&i| z.by_index(i).is_ok_and(|e| e.name().is_ok_and(|n| n.to_lowercase().ends_with("metadata.xml"))))
        .ok_or_else(|| anyhow!("Metadata.zip has no Metadata.xml"))?;
    let entry = z.by_index(idx)?;
    let total = entry.size();
    let counted = Counted { inner: entry, read: 0 };
    let new = db_path(data_dir).with_extension("sqlite.new");
    let _ = std::fs::remove_file(&new);
    let mut c = Connection::open(&new)?;
    c.execute_batch("PRAGMA journal_mode = OFF; PRAGMA synchronous = OFF;")?;
    c.execute_batch(SCHEMA)?;
    let tx = c.transaction()?;
    let mut games = 0u64;
    {
        let mut ins_game = tx.prepare("INSERT OR REPLACE INTO games (id, name, words, platform, year, released, developer, publisher, genres, players, overview, rating) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)")?;
        let mut ins_name = tx.prepare("INSERT INTO names (game_id, name, words, region) VALUES (?1, ?2, ?3, ?4)")?;
        let mut ins_image = tx.prepare("INSERT INTO images (game_id, file, type, region) VALUES (?1, ?2, ?3, ?4)")?;
        let mut r = quick_xml::Reader::from_reader(BufReader::with_capacity(1 << 20, counted));
        let mut buf = vec![];
        let mut record: Option<String> = None;
        let mut field: Option<String> = None;
        let mut fields: HashMap<String, String> = HashMap::new();
        let mut text = String::new();
        let mut depth = 0;
        loop {
            let ev = r.read_event_into(&mut buf).context("Metadata.xml can't be read")?;
            match ev {
                Event::Start(ref e) => {
                    depth += 1;
                    let name = e.local_name().as_ref().to_string();
                    if depth == 2 {
                        record = Some(name);
                        fields.clear();
                    } else if depth == 3 {
                        field = Some(name);
                        text.clear();
                    }
                }
                Event::Text(t) if field.is_some() => text.push_str(&t.xml10_content()),
                Event::CData(t) if field.is_some() => text.push_str(&t.into_inner()),
                Event::GeneralRef(g) if field.is_some() => {
                    let n = g.xml10_content().to_string();
                    let ch = match n.as_str() {
                        "amp" => Some('&'),
                        "lt" => Some('<'),
                        "gt" => Some('>'),
                        "quot" => Some('"'),
                        "apos" => Some('\''),
                        _ => g.resolve_char_ref().ok().flatten(),
                    };
                    if let Some(c) = ch {
                        text.push(c);
                    }
                }
                Event::End(_) => {
                    if depth == 3 {
                        if let Some(f) = field.take() {
                            fields.insert(f, text.trim().to_string());
                        }
                    } else if depth == 2 {
                        let get = |k: &str| fields.get(k).filter(|v| !v.is_empty()).cloned();
                        let id = get("DatabaseID").and_then(|v| v.parse::<i64>().ok());
                        match (record.as_deref(), id) {
                            (Some("Game"), Some(id)) => {
                                let name = get("Name").unwrap_or_default();
                                let released = get("ReleaseDate");
                                let year = get("ReleaseYear").and_then(|y| y.parse::<i64>().ok()).or_else(|| released.as_ref().and_then(|d| d.get(..4)?.parse().ok()));
                                ins_game.execute(params![
                                    id, name, words(&name), get("Platform").unwrap_or_default(), year, released.map(|d| d.chars().take(10).collect::<String>()),
                                    get("Developer"), get("Publisher"), get("Genres"), get("MaxPlayers"), get("Overview"), get("CommunityRating")
                                ])?;
                                games += 1;
                                if games.is_multiple_of(5000) {
                                    if cancel.load(Ordering::Relaxed) {
                                        bail!("stopped");
                                    }
                                    let read = r.get_ref().get_ref().read;
                                    progress(json!({ "name": "Reading LaunchBox's games", "item": games, "done": read, "total": total }));
                                }
                            }
                            (Some("GameAlternateName"), Some(id)) => {
                                if let Some(n) = get("AlternateName") {
                                    ins_name.execute(params![id, n, words(&n), get("Region")])?;
                                }
                            }
                            (Some("GameImage"), Some(id)) => {
                                if let (Some(f), Some(t)) = (get("FileName"), get("Type")) {
                                    ins_image.execute(params![id, f, t, get("Region")])?;
                                }
                            }
                            _ => {}
                        }
                        record = None;
                    }
                    depth -= 1;
                }
                Event::Empty(_) => {}
                Event::Eof => break,
                _ => {}
            }
            buf.clear();
        }
        let now = crate::library::now();
        tx.execute("INSERT INTO meta (key, value) VALUES ('games', ?1), ('fetched', ?2), ('source', ?3)", params![games.to_string(), now, source])?;
    }
    tx.commit()?;
    drop(c);
    if games == 0 {
        let _ = std::fs::remove_file(&new);
        bail!("Metadata.zip had no games in it.");
    }
    std::fs::rename(&new, db_path(data_dir))?;
    Ok(json!({ "games": games }))
}

/// A reader that counts what's been read through it (for the progress).
struct Counted<R> {
    inner: R,
    read: u64,
}

impl<R: Read> Read for Counted<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.read += n as u64;
        Ok(n)
    }
}

/// A game LaunchBox knows, as Identify shows it.
#[derive(Clone, Debug, Default)]
pub struct Found {
    pub id: i64,
    pub name: String,
    pub platform: String,
    pub year: Option<i32>,
    pub score: u32,
}

/// How well a name answers a search, 0–100: the same words 100, then by the words shared.
pub fn score(query: &str, name: &str) -> u32 {
    let (q, n) = (crate::pcgames::norm_title(query), crate::pcgames::norm_title(name));
    if q.is_empty() || n.is_empty() {
        return 0;
    }
    if q == n {
        return 100;
    }
    let qw: Vec<String> = words(query).split(' ').map(String::from).collect();
    let nw: Vec<String> = words(name).split(' ').map(String::from).collect();
    let shared = qw.iter().filter(|w| nw.contains(w)).count();
    let base = (shared * 80 / qw.len().max(nw.len()).max(1)) as u32;
    if n.starts_with(&q) || q.starts_with(&n) {
        base.max(70)
    } else {
        base
    }
}

/// Search the database for a name on a system (any system when none).
pub fn search(data_dir: &Path, query: &str, platform: Option<&str>, limit: usize) -> Result<Vec<Found>> {
    let p = db_path(data_dir);
    if !p.is_file() {
        bail!("LaunchBox's database isn't on this computer yet (Settings › Sources › Get LaunchBox's database).");
    }
    let c = Connection::open_with_flags(&p, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let w = words(query);
    let toks: Vec<&str> = w.split(' ').filter(|t| !t.is_empty()).take(6).collect();
    if toks.is_empty() {
        return Ok(vec![]);
    }
    let plats: Vec<String> = platform.map(platform_names).unwrap_or_default();
    let mut sql = String::from(
        "SELECT g.id, g.name, g.platform, g.year, x.name FROM (SELECT id AS game_id, words, name FROM games UNION ALL SELECT game_id, words, name FROM names) x
         JOIN games g ON g.id = x.game_id WHERE 1",
    );
    let mut args: Vec<String> = vec![];
    for t in &toks {
        args.push(format!("%{t}%"));
        sql.push_str(&format!(" AND x.words LIKE ?{}", args.len()));
    }
    if !plats.is_empty() {
        let marks: Vec<String> = plats
            .iter()
            .map(|p| {
                args.push(p.clone());
                format!("?{}", args.len())
            })
            .collect();
        sql.push_str(&format!(" AND g.platform IN ({})", marks.join(", ")));
    }
    sql.push_str(" LIMIT 500");
    let mut q = c.prepare(&sql)?;
    let mut best: BTreeMap<i64, Found> = BTreeMap::new();
    let mut rows = q.query(rusqlite::params_from_iter(args.iter()))?;
    while let Some(r) = rows.next()? {
        let id: i64 = r.get(0)?;
        let matched: String = r.get(4)?;
        let s = score(query, &matched);
        let e = best.entry(id).or_insert_with(|| Found { id, name: r.get(1).unwrap_or_default(), platform: r.get(2).unwrap_or_default(), year: r.get::<_, Option<i64>>(3).ok().flatten().map(|y| y as i32), score: 0 });
        e.score = e.score.max(s);
    }
    let mut v: Vec<Found> = best.into_values().collect();
    v.sort_by(|a, b| b.score.cmp(&a.score).then(a.name.len().cmp(&b.name.len())).then(a.name.cmp(&b.name)));
    v.truncate(limit);
    Ok(v)
}

/// A game's details and the images Identify takes from it: kind → file name.
pub fn details(data_dir: &Path, id: i64) -> Result<(Found, Map<String, Value>, BTreeMap<String, String>)> {
    let c = Connection::open_with_flags(db_path(data_dir), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let (found, details) = c
        .query_row(
            "SELECT id, name, platform, year, released, developer, publisher, genres, players, overview, rating FROM games WHERE id = ?1",
            [id],
            |r| {
                let mut d = Map::new();
                for (i, k) in ["released", "developer", "publisher", "genres", "players", "description", "rating"].iter().enumerate() {
                    if let Some(v) = r.get::<_, Option<String>>(i + 4)?.filter(|v| !v.is_empty()) {
                        d.insert(k.to_string(), if *k == "genres" { json!(v.split(';').map(str::trim).filter(|g| !g.is_empty()).collect::<Vec<_>>()) } else { json!(v) });
                    }
                }
                Ok((Found { id: r.get(0)?, name: r.get(1)?, platform: r.get(2)?, year: r.get::<_, Option<i64>>(3)?.map(|y| y as i32), score: 0 }, d))
            },
        )
        .optional()?
        .ok_or_else(|| anyhow!("LaunchBox has no game {id}."))?;
    let mut q = c.prepare("SELECT file, type, COALESCE(region, '') FROM images WHERE game_id = ?1")?;
    let imgs: Vec<(String, String, String)> = q.query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
    let mut art = BTreeMap::new();
    let region_rank = |r: &str| ["", "North America", "World", "United States", "Europe", "United Kingdom"].iter().position(|x| x.eq_ignore_ascii_case(r)).unwrap_or(9);
    for (kind, types) in [
        ("cover", &["Box - Front", "Box - Front - Reconstructed", "Fanart - Box - Front"][..]),
        ("banner", &["Steam Banner", "Banner"][..]),
        ("hero", &["Fanart - Background", "Screenshot - Gameplay"][..]),
        ("logo", &["Clear Logo"][..]),
    ] {
        let pick = types.iter().find_map(|t| imgs.iter().filter(|(_, ty, _)| ty == t).min_by_key(|(_, _, r)| region_rank(r)));
        if let Some((f, _, _)) = pick {
            art.insert(kind.to_string(), f.clone());
        }
    }
    Ok((found, details, art))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::library::tests::temp_dir;

    /// A small Metadata.zip, as LaunchBox lays it out.
    pub(crate) fn metadata_zip(p: &Path) {
        let xml = r#"<?xml version="1.0" standalone="yes"?>
<LaunchBox>
  <Platform><Name>Nintendo Game Boy</Name></Platform>
  <Game>
    <Name>Alpha Quest</Name><ReleaseYear>1990</ReleaseYear><ReleaseDate>1990-04-21T00:00:00-07:00</ReleaseDate>
    <Overview>Alpha &amp; friends.</Overview><MaxPlayers>1</MaxPlayers><DatabaseID>11</DatabaseID>
    <Platform>Nintendo Game Boy</Platform><Genres>Action; Platform</Genres><Developer>Alpha Soft</Developer><Publisher>Nin</Publisher>
  </Game>
  <Game><Name>Zeta Quest</Name><ReleaseYear>1999</ReleaseYear><DatabaseID>12</DatabaseID><Platform>Windows</Platform><Developer>Zeta Soft</Developer></Game>
  <Game><Name>Zeta Quest II</Name><DatabaseID>13</DatabaseID><Platform>Windows</Platform></Game>
  <GameAlternateName><AlternateName>Arufa Kuesuto</AlternateName><DatabaseID>11</DatabaseID><Region>Japan</Region></GameAlternateName>
  <GameImage><DatabaseID>11</DatabaseID><FileName>aaa-01.jpg</FileName><Type>Box - Front</Type><Region>Japan</Region></GameImage>
  <GameImage><DatabaseID>11</DatabaseID><FileName>aaa-02.jpg</FileName><Type>Box - Front</Type><Region>North America</Region></GameImage>
  <GameImage><DatabaseID>11</DatabaseID><FileName>aaa-03.png</FileName><Type>Clear Logo</Type></GameImage>
</LaunchBox>"#;
        crate::pcgames::tests::zip_of(p, &[("Metadata.xml", xml.as_bytes()), ("Platforms.xml", b"<LaunchBox/>")]);
    }

    #[test]
    fn the_database_is_read_and_searched() {
        let dir = temp_dir("launchbox");
        std::fs::create_dir_all(&dir).unwrap();
        metadata_zip(&dir.join("Metadata.zip"));
        let r = build(&dir, &dir.join("Metadata.zip"), "test", &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(r["games"], 3);
        assert_eq!(status(&dir)["games"], 3);
        let found = search(&dir, "alpha quest", Some("gb"), 10).unwrap();
        assert_eq!((found.len(), found[0].id, found[0].score, found[0].year), (1, 11, 100, Some(1990)));
        assert!(search(&dir, "alpha quest", Some("pc"), 10).unwrap().is_empty(), "not a PC game");
        assert_eq!(search(&dir, "arufa", None, 10).unwrap()[0].id, 11, "by another name");
        let z = search(&dir, "Zeta Quest", Some("pc"), 10).unwrap();
        assert_eq!(z.iter().map(|f| f.id).collect::<Vec<_>>(), [12, 13], "the same name first");
        let (f, d, art) = details(&dir, 11).unwrap();
        assert_eq!(f.name, "Alpha Quest");
        assert_eq!((d["developer"].as_str(), d["released"].as_str(), d["description"].as_str()), (Some("Alpha Soft"), Some("1990-04-21"), Some("Alpha & friends.")));
        assert_eq!(d["genres"], json!(["Action", "Platform"]));
        assert_eq!((art["cover"].as_str(), art["logo"].as_str()), ("aaa-02.jpg", "aaa-03.png"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
