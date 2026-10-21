//! Identify (docs/PLAN.md, "Phase 4 design", D24, D27): one search across the sources,
//! in the order Settings › Sources gives, for a game nothing recognised. Phase 4's
//! sources need no account: Steam's store and GOG's catalogue (PC games), and LaunchBox
//! Games DB (every system, searched on this computer). Picking a result stores the
//! source's id and details with the game and fetches its artwork.
//!
//! Every source's address can be changed in config.json (`source_urls`), which is how
//! the tests answer for them locally.

use crate::config::AppConfig;
use crate::launchbox;
use crate::pcgames::{norm_title, Match};
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The sources, in their default order: id, name, whether they know only PC games.
/// ScreenScraper first (Jerred, 2026-10-21, D138): it knows the games no store sells.
pub const SOURCES: &[(&str, &str, bool)] = &[
    ("screenscraper", "ScreenScraper", false),
    ("steam", "Steam", true),
    ("gog", "GOG", true),
    ("igdb", "IGDB", false),
    ("thegamesdb", "TheGamesDB", false),
    ("launchbox", "LaunchBox Games DB", false),
];

/// A source's name, as the page shows it.
pub fn source_name(id: &str) -> &'static str {
    SOURCES.iter().find(|s| s.0 == id).map(|s| s.1).unwrap_or("")
}

/// The default addresses (config.json `source_urls` overrides each).
pub const URLS: &[(&str, &str)] = &[
    ("steam_store", "https://store.steampowered.com"),
    ("steam_api", "https://api.steampowered.com"),
    ("steam_cdn", "https://shared.akamai.steamstatic.com/store_item_assets/steam/apps"),
    ("gog_catalog", "https://catalog.gog.com"),
    ("gog_api", "https://api.gog.com"),
    ("screenscraper", "https://api.screenscraper.fr/api2"),
    ("igdb", "https://api.igdb.com/v4"),
    ("igdb_images", "https://images.igdb.com/igdb/image/upload"),
    ("twitch", "https://id.twitch.tv"),
    ("thegamesdb", "https://api.thegamesdb.net/v1"),
    ("launchbox_zip", launchbox::ZIP_URL),
    ("launchbox_images", launchbox::IMAGES),
];

/// The default order before D138.
const OLD_ORDER: &[&str] = &["steam", "gog", "screenscraper", "igdb", "thegamesdb", "launchbox"];

/// The sources as this computer has them: (id, on), in order.
pub fn order(cfg: &AppConfig) -> Vec<(String, bool)> {
    let mut v: Vec<(String, bool)> = cfg.sources.iter().filter(|s| SOURCES.iter().any(|(id, _, _)| *id == s.id)).map(|s| (s.id.clone(), s.on)).collect();
    // an order saved before D138 that's still the old default: ScreenScraper goes first, as
    // it does now; one chosen since is left as it is
    if cfg.sources_rev < 1 {
        let ids: Vec<&str> = v.iter().map(|(id, _)| id.as_str()).collect();
        let old: Vec<&str> = OLD_ORDER.iter().copied().filter(|id| ids.contains(id)).collect();
        if ids == old {
            if let Some(i) = v.iter().position(|(id, _)| id == "screenscraper") {
                let ss = v.remove(i);
                v.insert(0, ss);
            }
        }
    }
    for (id, _, _) in SOURCES {
        if !v.iter().any(|(x, _)| x == id) {
            v.push((id.to_string(), true));
        }
    }
    v
}

/// What Settings › Sources shows.
pub fn sources(cfg: &AppConfig, data_dir: &Path) -> Value {
    json!(order(cfg)
        .into_iter()
        .map(|(id, on)| {
            let (_, name, pc) = SOURCES.iter().find(|(x, _, _)| *x == id).copied().unwrap_or(("", "", false));
            let mut v = json!({ "id": id, "name": name, "on": on, "pc_only": pc, "needs": crate::scrapers::needs(cfg, &id), "keys": crate::scrapers::keys_json(cfg, &id) });
            if id == "launchbox" {
                v["database"] = launchbox::status(data_dir);
            }
            v
        })
        .collect::<Vec<_>>())
}

pub fn url(cfg: &AppConfig, key: &str) -> String {
    cfg.source_urls.get(key).cloned().unwrap_or_else(|| URLS.iter().find(|(k, _)| *k == key).map(|(_, u)| u.to_string()).unwrap_or_default()).trim_end_matches('/').to_string()
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(20))).build().into()
}

/// GET a URL: its bytes, or None when it isn't there (404).
fn get(url: &str, limit: u64) -> Result<Option<(Vec<u8>, String)>> {
    match agent().get(url).header("User-Agent", concat!("GameRepo/", env!("CARGO_PKG_VERSION"))).header("Accept-Language", "en").call() {
        Ok(mut r) => {
            let ctype = r.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
            let mut out = vec![];
            r.body_mut().with_config().limit(limit).reader().read_to_end(&mut out)?;
            Ok(Some((out, ctype)))
        }
        Err(ureq::Error::StatusCode(404)) => Ok(None),
        Err(ureq::Error::StatusCode(429)) => bail!("the source says it's had too many requests: try again in a minute"),
        Err(e) => Err(anyhow!("{e}")),
    }
}

fn get_json(url: &str) -> Result<Value> {
    let (b, _) = get(url, 16 << 20)?.ok_or_else(|| anyhow!("not found"))?;
    serde_json::from_slice(&b).map_err(|e| anyhow!("an answer that isn't JSON ({e})"))
}

/// A part of a URL's query.
pub fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// The first year in a date as sources write them ("24 Feb, 2017", "2017.02.24", "2017-02-24T…").
pub fn year_of(s: &str) -> Option<i32> {
    s.split(|c: char| !c.is_ascii_digit()).find(|t| t.len() == 4).and_then(|t| t.parse().ok()).filter(|y| (1950..=2100).contains(y))
}

/// "//images.gog…" → "https://images.gog…".
fn https(u: &str) -> String {
    if let Some(r) = u.strip_prefix("//") {
        format!("https://{r}")
    } else {
        u.to_string()
    }
}

/// A result of a search, as Identify lists it.
#[derive(Clone, Debug, Default)]
pub struct Candidate {
    pub source: String,
    pub id: String,
    pub title: String,
    pub year: Option<i32>,
    pub platform: String,
    pub cover: Option<String>,
    pub url: Option<String>,
    pub score: u32,
    /// Its other titles at the source, for matching
    pub also: Vec<String>,
}

impl Candidate {
    pub fn to_json(&self) -> Value {
        json!({ "source": self.source, "source_name": SOURCES.iter().find(|s| s.0 == self.source).map(|s| s.1), "id": self.id, "title": self.title,
                "year": self.year, "platform": self.platform, "cover": self.cover, "url": self.url, "score": self.score, "also": self.also })
    }
}

// ------------------------------------------------------------ Steam

fn steam_search(cfg: &AppConfig, query: &str) -> Result<Vec<Candidate>> {
    let v = get_json(&format!("{}/api/storesearch/?term={}&l=english&cc=US", url(cfg, "steam_store"), encode(query)))?;
    if v.is_null() {
        // what the store answers when it's had too many requests
        bail!("Steam's store is busy: try again in a minute");
    }
    Ok(v["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|i| i["type"].as_str().unwrap_or("app") == "app")
        .filter_map(|i| {
            let id = i["id"].as_u64().map(|n| n.to_string()).or_else(|| i["id"].as_str().map(String::from))?;
            let title = i["name"].as_str()?.to_string();
            Some(Candidate {
                score: launchbox::score(query, &title),
                cover: i["tiny_image"].as_str().map(String::from),
                url: Some(format!("https://store.steampowered.com/app/{id}/")),
                source: "steam".into(),
                platform: "pc".into(),
                id,
                title,
                year: None,
                also: vec![],
            })
        })
        .collect())
}

fn steam_details(cfg: &AppConfig, id: &str) -> Result<Match> {
    let v = get_json(&format!("{}/api/appdetails?appids={}&l=english&cc=US", url(cfg, "steam_store"), encode(id)))?;
    if v.is_null() {
        bail!("Steam's store is busy: try again in a minute");
    }
    // keyed by the id asked for, though now and then by another
    let d = v.get(id).or_else(|| v.as_object().and_then(|o| o.values().next())).unwrap_or(&Value::Null);
    if d["success"] != true {
        bail!("Steam has no app {id}");
    }
    let d = &d["data"];
    let mut details = Map::new();
    let list = |k: &str| d[k].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(", ")).filter(|s| !s.is_empty());
    if let Some(v) = list("developers") {
        details.insert("developer".into(), json!(v));
    }
    if let Some(v) = list("publishers") {
        details.insert("publisher".into(), json!(v));
    }
    let genres: Vec<&str> = d["genres"].as_array().into_iter().flatten().filter_map(|g| g["description"].as_str()).collect();
    if !genres.is_empty() {
        details.insert("genres".into(), json!(genres));
    }
    if let Some(s) = d["short_description"].as_str().map(strip_tags).filter(|s| !s.is_empty()) {
        details.insert("description".into(), json!(s));
    }
    let date = d["release_date"]["date"].as_str().unwrap_or("");
    if !date.is_empty() {
        details.insert("released".into(), json!(steam_date(date)));
    }
    let cdn = url(cfg, "steam_cdn");
    let mut art = BTreeMap::new();
    art.insert("cover".into(), format!("{cdn}/{id}/library_600x900.jpg"));
    art.insert("banner".into(), d["header_image"].as_str().map(String::from).unwrap_or_else(|| format!("{cdn}/{id}/header.jpg")));
    art.insert("hero".into(), format!("{cdn}/{id}/library_hero.jpg"));
    art.insert("logo".into(), format!("{cdn}/{id}/logo.png"));
    // newer games keep their library art under a hashed folder, which the store's
    // browse service names; the fixed names above are the fallback
    art.extend(steam_assets(cfg, id).unwrap_or_default());
    Ok(Match { source: "steam".into(), id: id.to_string(), title: d["name"].as_str().unwrap_or("").to_string(), year: year_of(date), details, art, also: vec![] })
}

/// A Steam app's library artwork as the store's browse service names it (no key needed).
fn steam_assets(cfg: &AppConfig, id: &str) -> Result<BTreeMap<String, String>> {
    let input = json!({ "ids": [{ "appid": id.parse::<u64>()? }], "context": { "language": "english", "country_code": "US" }, "data_request": { "include_assets": true } });
    let v = get_json(&format!("{}/IStoreBrowseService/GetItems/v1/?input_json={}", url(cfg, "steam_api"), encode(&input.to_string())))?;
    let a = &v["response"]["store_items"][0]["assets"];
    let Some(format) = a["asset_url_format"].as_str() else { return Ok(BTreeMap::new()) };
    let cdn = url(cfg, "steam_cdn");
    let root = cdn.strip_suffix("/steam/apps").unwrap_or(&cdn);
    let mut art = BTreeMap::new();
    for (kind, keys) in [("cover", ["library_capsule_2x", "library_capsule"]), ("hero", ["library_hero_2x", "library_hero"]), ("banner", ["header_2x", "header"])] {
        if let Some(f) = keys.iter().find_map(|k| a[*k].as_str().filter(|s| !s.is_empty())) {
            art.insert(kind.to_string(), format!("{root}/{}", format.replace("${FILENAME}", f)));
        }
    }
    Ok(art)
}

// ------------------------------------------------------------ GOG

fn gog_search(cfg: &AppConfig, query: &str) -> Result<Vec<Candidate>> {
    let v = get_json(&format!(
        "{}/v1/catalog?limit=20&query=like:{}&order=desc:score&productType=in:game,pack&page=1&countryCode=US&locale=en-US&currencyCode=USD",
        url(cfg, "gog_catalog"),
        encode(query)
    ))?;
    Ok(v["products"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| {
            let id = p["id"].as_str().map(String::from).or_else(|| p["id"].as_u64().map(|n| n.to_string()))?;
            let title = p["title"].as_str()?.to_string();
            Some(Candidate {
                score: launchbox::score(query, &title),
                year: p["releaseDate"].as_str().and_then(year_of),
                cover: p["coverVertical"].as_str().or(p["coverHorizontal"].as_str()).map(https),
                url: p["slug"].as_str().map(|s| format!("https://www.gog.com/en/game/{s}")),
                source: "gog".into(),
                platform: "pc".into(),
                id,
                title,
                also: vec![],
            })
        })
        .collect())
}

fn gog_details(cfg: &AppConfig, id: &str) -> Result<Match> {
    let v = get_json(&format!("{}/products/{}?expand=description", url(cfg, "gog_api"), encode(id)))?;
    let title = v["title"].as_str().ok_or_else(|| anyhow!("GOG has no game {id}"))?.to_string();
    let mut details = Map::new();
    let date = v["release_date"].as_str().unwrap_or("");
    if !date.is_empty() {
        details.insert("released".into(), json!(date.chars().take(10).collect::<String>()));
    }
    let desc = v["description"]["lead"].as_str().filter(|s| !s.trim().is_empty()).or(v["description"]["full"].as_str());
    if let Some(d) = desc {
        details.insert("description".into(), json!(strip_tags(d)));
    }
    if let Some(s) = v["slug"].as_str() {
        details.insert("gog_slug".into(), json!(s));
    }
    let mut art = BTreeMap::new();
    let img = |k: &str| v["images"][k].as_str().filter(|s| !s.is_empty()).map(https);
    if let Some(u) = img("background") {
        art.insert("hero".into(), u);
    }
    if let Some(u) = img("logo2x").or_else(|| img("logo")) {
        art.insert("banner".into(), u);
    }
    if let Some(u) = img("icon") {
        art.insert("icon".into(), u);
    }
    // the catalogue's portrait cover, when the search gave one
    if let Ok(found) = gog_search(cfg, &title) {
        if let Some(c) = found.into_iter().find(|c| c.id == id).and_then(|c| c.cover) {
            art.insert("cover".into(), c);
        }
    }
    Ok(Match { source: "gog".into(), id: id.to_string(), title, year: year_of(date), details, art, also: vec![] })
}

/// A date as Steam writes it ("24 Feb, 1999" or "Feb 24, 1999") as the other sources do
/// ("1999-02-24"), so they agree on a game's page; anything else as it is.
fn steam_date(s: &str) -> String {
    const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    let (mut day, mut month, mut year) = (None, None, None);
    for w in s.split(|c: char| c.is_whitespace() || c == ',').filter(|w| !w.is_empty()) {
        let l = w.to_ascii_lowercase();
        if let Some(m) = MONTHS.iter().position(|m| l.starts_with(m)) {
            month = Some(m + 1);
        } else if w.len() == 4 && w.chars().all(|c| c.is_ascii_digit()) {
            year = w.parse::<u32>().ok();
        } else if w.len() <= 2 && w.chars().all(|c| c.is_ascii_digit()) {
            day = w.parse::<u32>().ok().filter(|d| (1..=31).contains(d));
        } else {
            return s.to_string();
        }
    }
    match (year, month, day) {
        (Some(y), Some(m), Some(d)) => format!("{y}-{m:02}-{d:02}"),
        (Some(y), Some(m), None) => format!("{y}-{m:02}"),
        _ => s.to_string(),
    }
}

/// Text without its HTML tags.
fn strip_tags(s: &str) -> String {
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
    out.replace("&amp;", "&").replace("&quot;", "\"").replace("&#39;", "'").replace("&lt;", "<").replace("&gt;", ">").split_whitespace().collect::<Vec<_>>().join(" ")
}

// ------------------------------------------------------------ LaunchBox

fn launchbox_search(data_dir: &Path, query: &str, platform: Option<&str>) -> Result<Vec<Candidate>> {
    Ok(launchbox::search(data_dir, query, platform, 20)?
        .into_iter()
        .map(|f| Candidate { source: "launchbox".into(), id: f.id.to_string(), title: f.name, year: f.year, platform: f.platform, cover: None, url: Some(format!("https://gamesdb.launchbox-app.com/games/details/{}", f.id)), score: f.score, also: f.also })
        .collect())
}

fn launchbox_details(cfg: &AppConfig, data_dir: &Path, id: &str) -> Result<Match> {
    let n: i64 = id.parse().map_err(|_| anyhow!("{id} isn't a LaunchBox id"))?;
    let (f, details, files) = launchbox::details(data_dir, n)?;
    let base = url(cfg, "launchbox_images");
    let art = files.into_iter().map(|(k, f)| (k, format!("{base}/{}", f.split('/').map(encode).collect::<Vec<_>>().join("/")))).collect();
    Ok(Match { source: "launchbox".into(), id: id.to_string(), title: f.name, year: f.year, details, art, also: f.also })
}

// ------------------------------------------------------------ all of them

/// Search every source that's on (and knows the system), in order, or just `only` (Identify's
/// choice of source, which asks it even when it's off). A source that fails says why; the
/// others still answer.
pub fn search(cfg: &AppConfig, data_dir: &Path, query: &str, platform: &str, year: Option<i32>, only: Option<&str>) -> Value {
    let query = query.trim();
    let mut results = vec![];
    let mut errors = vec![];
    // sources not ready yet (LaunchBox before its database is fetched): a note, not an error
    let mut needs = vec![];
    for (id, on) in order(cfg) {
        let (pc_only, name) = SOURCES.iter().find(|s| s.0 == id).map_or((false, ""), |s| (s.2, s.1));
        let on = match only {
            Some(o) => o == id,
            None => on,
        };
        if !on || query.is_empty() || (pc_only && platform != "pc") {
            continue;
        }
        if id == "launchbox" && launchbox::status(data_dir)["have"] != true {
            needs.push(json!({ "source": id, "name": name, "text": "Get its database in Settings › Sources to search it." }));
            continue;
        }
        if let Some(n) = crate::scrapers::needs(cfg, &id) {
            needs.push(json!({ "source": id, "name": name, "text": n }));
            continue;
        }
        let r = match id.as_str() {
            "steam" => steam_search(cfg, query),
            "gog" => gog_search(cfg, query),
            "launchbox" => launchbox_search(data_dir, query, Some(platform).filter(|p| !p.is_empty())),
            "screenscraper" | "igdb" | "thegamesdb" => crate::scrapers::search(cfg, data_dir, &id, query, platform),
            _ => continue,
        };
        match r {
            Ok(mut v) => {
                for c in v.iter_mut() {
                    if year.is_some() && c.year == year && c.score >= 70 {
                        c.score = (c.score + 5).min(100);
                    }
                }
                results.extend(v.into_iter().map(|c| c.to_json()));
            }
            Err(e) => errors.push(json!({ "source": id, "name": name, "error": format!("{e:#}") })),
        }
    }
    json!({ "results": results, "errors": errors, "needs": needs })
}

/// A source's details for one of its results.
pub fn details(cfg: &AppConfig, data_dir: &Path, source: &str, id: &str) -> Result<Match> {
    match source {
        "steam" => steam_details(cfg, id),
        "gog" => gog_details(cfg, id),
        "launchbox" => launchbox_details(cfg, data_dir, id),
        "screenscraper" | "igdb" | "thegamesdb" => crate::scrapers::details(cfg, data_dir, source, id, &["us".into(), "wor".into(), "eu".into(), "jp".into()]),
        other => bail!("There's no source {other}."),
    }
}

pub fn match_json(m: &Match) -> Value {
    json!({ "source": m.source, "id": m.id, "title": m.title, "year": m.year, "details": m.details, "art": m.art })
}

/// The best answer for an import item: the first source (in order) with a result whose
/// title is the same and whose year agrees when both have one.
pub fn best(cfg: &AppConfig, data_dir: &Path, title: &str, year: Option<i32>) -> Option<Match> {
    if title.trim().is_empty() {
        return None;
    }
    let found = search(cfg, data_dir, title, "pc", year, None);
    let agrees = |y: Option<i64>| year.is_none() || y.is_none() || y == year.map(i64::from);
    // its title, or one of the source's other titles for it (another region's, D138)
    let named = |r: &Value| {
        let want = norm_title(title);
        std::iter::once(&r["title"]).chain(r["also"].as_array().into_iter().flatten()).any(|t| norm_title(t.as_str().unwrap_or("")) == want)
    };
    for r in found["results"].as_array()?.iter().filter(|r| named(r) && agrees(r["year"].as_i64())).take(3) {
        // a search may not give the year: the details do
        if let Ok(m) = details(cfg, data_dir, r["source"].as_str()?, r["id"].as_str()?) {
            if agrees(m.year.map(i64::from)) {
                return Some(m);
            }
        }
    }
    None
}

// ------------------------------------------------------------ artwork

/// Fetch artwork (kind → URL) into a media folder; a picture a source doesn't have is
/// skipped. Returns the kinds fetched.
pub fn fetch_art(dir: &Path, art: &BTreeMap<String, String>, replace: bool) -> Vec<String> {
    let mut got = vec![];
    for (kind, u) in art {
        if !replace && has_media(dir, kind) {
            continue;
        }
        let Some((bytes, ext)) = fetch_picture(u) else { continue };
        if save_media(dir, kind, ext, &bytes).is_ok() {
            got.push(kind.clone());
        }
    }
    got
}

/// Fetch a picture: its bytes and the extension it's kept under, or None when the source
/// hasn't it (or answers with something that isn't one).
pub fn fetch_picture(u: &str) -> Option<(Vec<u8>, &'static str)> {
    let Ok(Some((bytes, ctype))) = get(u, 40 << 20) else { return None };
    if bytes.len() < 64 || ctype.starts_with("text/") {
        return None;
    }
    let ext = match () {
        _ if ctype.contains("png") || bytes.starts_with(b"\x89PNG") => "png",
        _ if ctype.contains("webp") || bytes.get(8..12) == Some(b"WEBP") => "webp",
        _ if ctype.contains("icon") || u.to_lowercase().ends_with(".ico") => "ico",
        _ => "jpg",
    };
    Some((bytes, ext))
}

/// The artwork in a media folder: kind → its path in the library.
pub fn media_of(lib: &crate::library::Library, dir: &Path) -> Map<String, Value> {
    let mut m = Map::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if let Some((kind, _)) = name.split_once('.') {
            if let Some(rel) = lib.relative(&e.path()) {
                m.insert(kind.to_string(), json!(rel));
            }
        }
    }
    m
}

fn has_media(dir: &Path, kind: &str) -> bool {
    std::fs::read_dir(dir).into_iter().flatten().flatten().any(|e| e.file_name().to_string_lossy().split_once('.').is_some_and(|(k, _)| k == kind))
}

/// Keep a picture as `<kind>.<ext>` in a media folder, replacing the one of that kind.
pub fn save_media(dir: &Path, kind: &str, ext: &str, bytes: &[u8]) -> Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if e.file_name().to_string_lossy().split_once('.').is_some_and(|(k, _)| k == kind) {
            let _ = std::fs::remove_file(e.path());
        }
    }
    let p = dir.join(format!("{kind}.{ext}"));
    crate::config::write_atomic(&p, bytes)?;
    Ok(p)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::library::tests::temp_dir;
    use std::io::Write;

    /// A web server on 127.0.0.1 answering from `files` (path, query left out → body); 404 otherwise.
    pub(crate) fn serve(files: Vec<(&'static str, Vec<u8>)>) -> String {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let mut s = s;
                let mut buf = [0u8; 8192];
                let n = s.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let target = req.split(' ').nth(1).unwrap_or("").to_string();
                let path = target.split('?').next().unwrap_or("").to_string();
                let body = files.iter().find(|(p, _)| *p == path || *p == target).map(|(_, b)| b.clone());
                let (code, body) = match body {
                    Some(b) => ("200 OK", b),
                    None => ("404 Not Found", vec![]),
                };
                let _ = write!(s, "HTTP/1.1 {code}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", body.len());
                let _ = s.write_all(&body);
            }
        });
        format!("http://{addr}")
    }

    /// A config with every source at `base`.
    pub(crate) fn config_at(base: &str) -> AppConfig {
        let mut cfg = AppConfig::default();
        for (k, _) in URLS {
            let u = match *k {
                "steam_cdn" => format!("{base}/cdn"),
                "launchbox_images" => format!("{base}/lbimg"),
                "launchbox_zip" => format!("{base}/Metadata.zip"),
                _ => base.to_string(),
            };
            cfg.source_urls.insert(k.to_string(), u);
        }
        cfg
    }

    pub(crate) fn answers() -> Vec<(&'static str, Vec<u8>)> {
        let png = [b"\x89PNG\r\n\x1a\n".to_vec(), vec![0u8; 100]].concat();
        vec![
            ("/api/storesearch/", br#"{"total":2,"items":[{"type":"app","name":"Zeta Quest","id":4000,"tiny_image":"https://x/capsule.jpg","platforms":{"windows":true}},{"type":"app","name":"Zeta Quest II","id":4001}]}"#.to_vec()),
            ("/api/appdetails", br#"{"4000":{"success":true,"data":{"type":"game","name":"Zeta Quest","steam_appid":4000,"short_description":"Quest &amp; more.","developers":["Zeta Soft"],"publishers":["Zeta Pub"],"genres":[{"id":"1","description":"Action"}],"release_date":{"coming_soon":false,"date":"24 Feb, 1999"}}}}"#.to_vec()),
            ("/v1/catalog", br#"{"pages":1,"productCount":1,"products":[{"id":"1207658930","slug":"zeta_quest","title":"Zeta Quest","releaseDate":"1999.02.24","coverVertical":"//img/zeta_v.jpg","developers":["Zeta Soft"],"productType":"game"}]}"#.to_vec()),
            ("/products/1207658930", br#"{"id":1207658930,"title":"Zeta Quest","slug":"zeta_quest","release_date":"1999-02-24T00:00:00+0200","images":{"background":"//img/bg.jpg","logo":"//img/logo.jpg","icon":"//img/icon.png"},"description":{"lead":"<b>Quest</b> on.","full":"long"}}"#.to_vec()),
            ("/cdn/4000/library_600x900.jpg", [b"\xff\xd8\xff".to_vec(), vec![1u8; 100]].concat()),
            ("/cdn/4000/logo.png", png),
            ("/IStoreBrowseService/GetItems/v1/", br#"{"response":{"store_items":[{"id":4000,"success":1,"assets":{"asset_url_format":"steam/apps/4000/${FILENAME}?t=1","library_hero":"abc123/library_hero.jpg"}}]}}"#.to_vec()),
        ]
    }

    #[test]
    fn steam_gog_and_launchbox_answer_a_search_and_give_details() {
        let dir = temp_dir("identify");
        std::fs::create_dir_all(&dir).unwrap();
        let base = serve(answers());
        let cfg = config_at(&base);
        crate::launchbox::tests::metadata_zip(&dir.join("Metadata.zip"));
        crate::launchbox::build(&dir, &dir.join("Metadata.zip"), "test", &std::sync::atomic::AtomicBool::new(false), &mut |_| {}).unwrap();
        let r = search(&cfg, &dir, "zeta quest", "pc", Some(1999), None);
        assert!(r["errors"].as_array().unwrap().is_empty(), "{r}");
        let res = r["results"].as_array().unwrap();
        let sources: Vec<&str> = res.iter().map(|x| x["source"].as_str().unwrap()).collect();
        assert_eq!(sources, ["steam", "steam", "gog", "launchbox", "launchbox"]);
        assert_eq!((res[0]["id"].as_str(), res[0]["score"].as_u64()), (Some("4000"), Some(100)));
        assert_eq!(res[2]["cover"], "https://img/zeta_v.jpg");
        // not PC: only LaunchBox answers
        let r = search(&cfg, &dir, "alpha quest", "gb", None, None);
        assert_eq!(r["results"].as_array().unwrap().len(), 1);
        let s = details(&cfg, &dir, "steam", "4000").unwrap();
        assert_eq!((s.title.as_str(), s.year, s.details["developer"].as_str()), ("Zeta Quest", Some(1999), Some("Zeta Soft")));
        assert_eq!(s.art["cover"], format!("{base}/cdn/4000/library_600x900.jpg"));
        // the hashed folder the browse service names wins over the fixed name
        assert_eq!(s.art["hero"], format!("{base}/cdn/steam/apps/4000/abc123/library_hero.jpg?t=1"));
        assert_eq!(s.details["description"], "Quest & more.");
        assert_eq!(s.details["released"], "1999-02-24");
        assert_eq!([steam_date("Feb 24, 1999"), steam_date("Mar 2001"), steam_date("Coming soon")], ["1999-02-24", "2001-03", "Coming soon"]);
        let g = details(&cfg, &dir, "gog", "1207658930").unwrap();
        assert_eq!((g.year, g.details["description"].as_str(), g.art["hero"].as_str()), (Some(1999), Some("Quest on."), "https://img/bg.jpg"));
        let l = details(&cfg, &dir, "launchbox", "11").unwrap();
        assert_eq!(l.art["cover"], format!("{base}/lbimg/aaa-02.jpg"));
        // the best answer for an import: the same title and year
        assert_eq!(best(&cfg, &dir, "Zeta Quest", Some(1999)).map(|m| (m.source, m.id)), Some(("steam".into(), "4000".into())));
        assert!(best(&cfg, &dir, "Zeta Quest", Some(2005)).is_none(), "a different year");
        // or a source's other title for it (D138)
        assert_eq!(best(&cfg, &dir, "Zeta Saga", None).map(|m| (m.source, m.id, m.title)), Some(("launchbox".into(), "13".into(), "Zeta Quest II".into())));
        // one source asked, even one that's off (Identify's choice, D138)
        let mut steam_off = cfg.clone();
        steam_off.sources = vec![crate::config::SourceSetting { id: "steam".into(), on: false }];
        let r = search(&steam_off, &dir, "zeta quest", "pc", None, Some("steam"));
        assert!(r["results"].as_array().unwrap().iter().all(|x| x["source"] == "steam") && r["results"].as_array().unwrap().len() == 2, "{r}");
        // artwork: what the source has is kept, what it hasn't is skipped
        let media = dir.join("media");
        let got = fetch_art(&media, &s.art, true);
        assert_eq!(got, ["cover", "logo"]);
        assert!(media.join("cover.jpg").is_file() && media.join("logo.png").is_file());
        // a source that's off isn't asked; one that fails says so
        let mut off = cfg.clone();
        off.sources = vec![crate::config::SourceSetting { id: "steam".into(), on: false }];
        off.source_urls.insert("gog_catalog".into(), "http://127.0.0.1:9".into());
        let r = search(&off, &dir, "zeta quest", "pc", None, None);
        assert!(r["results"].as_array().unwrap().iter().all(|x| x["source"] == "launchbox"));
        assert_eq!(r["errors"][0]["source"], "gog");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn screenscraper_comes_first_unless_another_order_was_chosen() {
        let ids = |c: &AppConfig| order(c).into_iter().map(|(id, _)| id).collect::<Vec<_>>();
        let mut cfg = AppConfig::default();
        assert_eq!(ids(&cfg), ["screenscraper", "steam", "gog", "igdb", "thegamesdb", "launchbox"]);
        // saved before D138 in the old default order: ScreenScraper goes first, on or off kept
        let set = |v: &[(&str, bool)]| v.iter().map(|(id, on)| crate::config::SourceSetting { id: id.to_string(), on: *on }).collect::<Vec<_>>();
        cfg.sources = set(&[("steam", true), ("gog", false), ("screenscraper", true), ("igdb", true), ("thegamesdb", true), ("launchbox", true)]);
        assert_eq!(order(&cfg)[..3], [("screenscraper".to_string(), true), ("steam".to_string(), true), ("gog".to_string(), false)]);
        // one of its own is kept, and so is any chosen since
        cfg.sources = set(&[("gog", true), ("steam", true), ("screenscraper", true)]);
        assert_eq!(ids(&cfg)[..3], ["gog", "steam", "screenscraper"]);
        cfg.sources = set(&[("steam", true), ("gog", true), ("screenscraper", true)]);
        cfg.sources_rev = 1;
        assert_eq!(ids(&cfg)[..3], ["steam", "gog", "screenscraper"]);
    }
}
