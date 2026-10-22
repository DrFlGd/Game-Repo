//! The sources that need an account or a key (docs/PLAN.md, "Phase 5 design", D24, D33):
//! ScreenScraper (looked up by a game's ROMs' hashes, or searched by name), IGDB (a
//! Twitch developer application) and TheGamesDB (an API key). Each answers Identify's
//! searches and gives a game's details and its pictures, video and manual as a
//! [`Match`]: `art` holds kind → URL for the kinds in [`crate::meta::MEDIA`] and
//! `manual`.
//!
//! Their request and answer shapes were taken from the open-source clients that use
//! them (ES-DE, Skyscraper, RomM) and TheGamesDB's own server source; each source's
//! address can be changed in config.json `source_urls` for the tests. Every answer is
//! kept on this computer for a week (`<data dir>/scrape-cache/`), and each source is
//! asked at its own pace. ScreenScraper is asked by as many threads at once as the
//! account allows (`ssuserInfos`' `maxthreads`, or fewer as Settings › Sources says), each
//! request still spaced to its `maxrequestspermin` ([`ss_configure`]).

use crate::config::AppConfig;
use crate::identify::{encode, url, year_of, Candidate};
use crate::meta::DatGame;
use crate::pcgames::Match;
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime};

/// The fields each source needs in Settings › Sources: (source, [(field, label, secret)]).
pub const KEYS: &[(&str, &[(&str, &str, bool)])] = &[
    ("screenscraper", &[("user", "User name", false), ("password", "Password", true), ("devid", "Developer id", false), ("devpassword", "Developer password", true), ("threads", "Threads at once", false)]),
    ("igdb", &[("client_id", "Client id", false), ("client_secret", "Client secret", true)]),
    ("thegamesdb", &[("apikey", "API key", true)]),
];

/// A source's key, if set.
fn key<'a>(cfg: &'a AppConfig, source: &str, field: &str) -> Option<&'a str> {
    cfg.source_keys.get(source).and_then(|m| m.get(field)).map(String::as_str).map(str::trim).filter(|s| !s.is_empty())
}

/// What a source still needs before it can be asked (None: it's ready).
pub fn needs(cfg: &AppConfig, source: &str) -> Option<String> {
    match source {
        "screenscraper" => (key(cfg, source, "devid").is_none() || key(cfg, source, "devpassword").is_none())
            .then(|| "Needs ScreenScraper's developer id and password for the app, and your account, in Settings › Sources.".to_string()),
        "igdb" => (key(cfg, source, "client_id").is_none() || key(cfg, source, "client_secret").is_none())
            .then(|| "Needs a Twitch developer application's client id and secret in Settings › Sources.".to_string()),
        "thegamesdb" => key(cfg, source, "apikey").is_none().then(|| "Needs a TheGamesDB API key in Settings › Sources.".to_string()),
        _ => None,
    }
}

/// The keys as Settings shows them: secrets are only said to be set. ScreenScraper's
/// threads also carry what the account allowed when it was last read (D142).
pub fn keys_json(cfg: &AppConfig, data_dir: &Path, source: &str) -> Value {
    let Some((_, fields)) = KEYS.iter().find(|(s, _)| *s == source) else { return Value::Null };
    json!(fields
        .iter()
        .map(|(f, label, secret)| {
            let v = key(cfg, source, f);
            let mut k = json!({ "field": f, "label": label, "secret": secret, "set": v.is_some(), "value": if *secret { None } else { v } });
            if (source, *f) == ("screenscraper", "threads") {
                k["account"] = json!(ss_threads_allowed(data_dir));
            }
            k
        })
        .collect::<Vec<_>>())
}

/// The ScreenScraper account as last read: kept in the app's data so Settings can show
/// how many threads it allows.
fn ss_account_file(data_dir: &Path) -> PathBuf {
    data_dir.join("screenscraper-account.json")
}

/// The threads ScreenScraper's account allowed when it was last read (Settings' *Check*, or
/// a scrape starting).
pub fn ss_threads_allowed(data_dir: &Path) -> Option<usize> {
    let v: Value = std::fs::read(ss_account_file(data_dir)).ok().and_then(|b| serde_json::from_slice(&b).ok())?;
    v["threads_allowed"].as_u64().filter(|n| *n > 0).map(|n| n as usize)
}

fn ss_keep_allowed(data_dir: &Path, threads: usize) {
    let _ = std::fs::create_dir_all(data_dir);
    let _ = std::fs::write(ss_account_file(data_dir), json!({ "threads_allowed": threads }).to_string());
}

/// The ids each source gives a system: (system, ScreenScraper, IGDB, TheGamesDB), as the
/// frontends' scrapers use them. A system without one is searched by name alone, and then
/// what a search finds is only a candidate (H19).
const SYSTEM_IDS: &[(&str, Option<u32>, Option<u32>, Option<&str>)] = &[
    ("nes", Some(3), Some(18), Some("7")),
    ("fds", Some(106), Some(51), Some("4936")),
    ("snes", Some(4), Some(19), Some("6")),
    ("n64", Some(14), Some(4), Some("3")),
    ("gb", Some(9), Some(33), Some("4")),
    ("gbc", Some(10), Some(22), Some("41")),
    ("gba", Some(12), Some(24), Some("5")),
    ("nds", Some(15), Some(20), Some("8")),
    ("virtualboy", Some(11), Some(87), Some("4918")),
    ("genesis", Some(1), Some(29), Some("18,36")),
    ("mastersystem", Some(2), Some(64), Some("35")),
    ("gamegear", Some(21), Some(35), Some("20")),
    ("sega32x", Some(19), Some(30), Some("33")),
    ("sg-1000", Some(109), Some(84), Some("4949")),
    ("atari2600", Some(26), Some(59), Some("22")),
    ("atari5200", Some(40), Some(66), Some("26")),
    ("atari7800", Some(41), Some(60), Some("27")),
    ("atarilynx", Some(28), Some(61), Some("4924")),
    ("atarijaguar", Some(27), Some(62), Some("28")),
    ("pcengine", Some(31), Some(86), Some("34")),
    ("ngp", Some(25), Some(119), Some("4922")),
    ("ngpc", Some(82), Some(120), Some("4923")),
    ("wonderswan", Some(45), Some(57), Some("4925")),
    ("wonderswancolor", Some(46), Some(123), Some("4926")),
    ("colecovision", Some(48), Some(68), Some("31")),
    ("intellivision", Some(115), Some(67), Some("32")),
    ("vectrex", Some(102), Some(70), Some("4939")),
    ("pokemini", Some(211), Some(166), Some("4957")),
    ("psx", Some(57), Some(7), Some("10")),
    ("ps2", Some(58), Some(8), Some("11")),
    ("psp", Some(61), Some(38), Some("13")),
    ("saturn", Some(22), Some(32), Some("17")),
    ("segacd", Some(20), Some(78), Some("21")),
    ("dreamcast", Some(23), Some(23), Some("16")),
    ("pcenginecd", Some(114), Some(150), Some("4955")),
    ("pcfx", Some(72), Some(274), Some("4930")),
    ("cdimono1", Some(133), Some(117), Some("4917")),
    ("neogeocd", Some(70), Some(136), Some("4956")),
    ("3do", Some(29), Some(50), Some("25")),
    ("atarijaguarcd", Some(171), Some(410), Some("29")),
    ("gc", Some(13), Some(21), Some("2")),
    ("wii", Some(16), Some(5), Some("9")),
    ("xbox", Some(32), Some(11), Some("14")),
    ("n3ds", Some(17), Some(37), Some("4912")),
    ("wiiu", Some(18), Some(41), Some("38")),
    ("switch", Some(225), Some(130), Some("4971")),
    ("ps3", Some(59), Some(9), Some("12")),
    ("ps4", Some(60), Some(48), Some("4919")),
    ("psvita", Some(62), Some(46), Some("39")),
    ("xbox360", Some(33), Some(12), Some("15")),
    // ScreenScraper's arcade is MAME's; IGDB's and TheGamesDB's arcade
    ("mame", Some(75), Some(52), Some("23")),
    ("fbneo", Some(75), Some(52), Some("23")),
    ("pc", Some(138), Some(6), Some("1")),
];

/// Whether `source`'s search can be limited to `platform`'s games (else a game of the
/// same name on another system could be taken, H19).
pub fn searches_system(source: &str, platform: &str) -> bool {
    match (source, ids_of(platform)) {
        ("igdb", (_, Some(_), _)) | ("thegamesdb", (_, _, Some(_))) => true,
        ("igdb" | "thegamesdb", _) => false,
        _ => true,
    }
}

/// The other ids a system's games may have at ScreenScraper and IGDB: a PC game may be a
/// DOS or Windows 3.x game there, which they keep apart from Windows games.
fn also_ids(platform: &str) -> (&'static [u32], &'static [u32]) {
    match platform {
        "pc" => (&[135, 136], &[13]),
        _ => (&[], &[]),
    }
}

fn ids_of(platform: &str) -> (Option<u32>, Option<u32>, Option<&'static str>) {
    SYSTEM_IDS.iter().find(|s| s.0 == platform).map(|s| (s.1, s.2, s.3)).unwrap_or((None, None, None))
}

// ------------------------------------------------------------ asking, at each source's pace

static LAST: Mutex<Option<HashMap<String, Instant>>> = Mutex::new(None);

/// Wait until `source` may be asked again (`gap` after the last time).
fn pace(source: &str, gap: Duration) {
    loop {
        let wait = {
            let mut l = LAST.lock().unwrap();
            let m = l.get_or_insert_with(HashMap::new);
            let now = Instant::now();
            match m.get(source) {
                Some(t) if now < *t + gap => Some(*t + gap - now),
                _ => {
                    m.insert(source.to_string(), now);
                    None
                }
            }
        };
        match wait {
            Some(d) => std::thread::sleep(d),
            None => return,
        }
    }
}

fn gap_of(source: &str) -> Duration {
    if cfg!(test) {
        return Duration::ZERO;
    }
    match source {
        "igdb" => Duration::from_millis(260),
        "screenscraper" => Duration::from_millis(SS_GAP_MS.load(Ordering::Relaxed)),
        _ => Duration::from_millis(1000),
    }
}

/// How many requests ScreenScraper may have open at once, and the time between two of
/// them starting: one, and 1.2 seconds, until [`ss_configure`] reads the account.
static SS_THREADS: AtomicUsize = AtomicUsize::new(1);
static SS_GAP_MS: AtomicU64 = AtomicU64::new(1200);
static SS_OPEN: (Mutex<usize>, Condvar) = (Mutex::new(0), Condvar::new());

/// One of ScreenScraper's threads, held while a request is open.
struct Slot;

impl Slot {
    fn take() -> Slot {
        let (m, c) = &SS_OPEN;
        let mut open = m.lock().unwrap();
        while *open >= SS_THREADS.load(Ordering::Relaxed).max(1) {
            open = c.wait(open).unwrap();
        }
        *open += 1;
        Slot
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        let (m, c) = &SS_OPEN;
        *m.lock().unwrap() -= 1;
        c.notify_one();
    }
}

/// How many threads a scrape asks ScreenScraper with.
pub fn threads() -> usize {
    SS_THREADS.load(Ordering::Relaxed).max(1)
}

/// What a ScreenScraper account allows: (threads, requests a minute), from `ssuserInfos`.
fn ss_allows(v: &Value) -> (Option<usize>, Option<u64>) {
    let u = &v["response"]["ssuser"];
    let n = |k: &str| text(&u[k]).and_then(|t| t.parse::<u64>().ok()).filter(|n| *n > 0);
    (n("maxthreads").map(|n| n as usize), n("maxrequestspermin"))
}

/// Read the account (when ScreenScraper is to be asked) and set how many threads ask it
/// and how far apart their requests start: the account's `maxthreads` (at most 16, or
/// fewer if Settings › Sources says so) and `maxrequestspermin`. Returns the threads.
pub fn ss_configure(cfg: &AppConfig, data_dir: &Path) -> usize {
    SS_THREADS.store(1, Ordering::Relaxed);
    SS_GAP_MS.store(1200, Ordering::Relaxed);
    if needs(cfg, "screenscraper").is_some() {
        return 1;
    }
    // the account as it is now, not as it was kept
    let fresh = data_dir.join("scrape-cache-account");
    let _ = std::fs::remove_dir_all(&fresh);
    let (allowed, per_min) = ss_get(cfg, &fresh, "ssuserInfos.php", &[]).ok().flatten().map(|v| ss_allows(&v)).unwrap_or((None, None));
    let _ = std::fs::remove_dir_all(&fresh);
    if let Some(a) = allowed {
        ss_keep_allowed(data_dir, a);
    }
    let wanted = key(cfg, "screenscraper", "threads").and_then(|t| t.parse::<usize>().ok()).filter(|n| *n > 0);
    let n = match (allowed, wanted) {
        (Some(a), Some(w)) => a.min(w),
        (Some(a), None) => a,
        (None, _) => 1,
    }
    .clamp(1, 16);
    // a little under the account's rate, else 1.2 seconds shared between the threads
    let gap = match per_min {
        Some(r) => (60_000 / r) * 11 / 10,
        None => 1200 / n as u64,
    };
    SS_THREADS.store(n, Ordering::Relaxed);
    SS_GAP_MS.store(gap.max(50), Ordering::Relaxed);
    n
}

/// Fetch several pictures at once (as many as ScreenScraper's threads), in order.
pub fn fetch_all(list: &[(&str, &str)]) -> Vec<Option<(Vec<u8>, String)>> {
    let n = threads();
    if n <= 1 || list.len() <= 1 {
        return list.iter().map(|(u, k)| fetch(u, k)).collect();
    }
    let mut out = vec![];
    for part in list.chunks(n) {
        std::thread::scope(|s| {
            let hs: Vec<_> = part.iter().map(|(u, k)| s.spawn(move || fetch(u, k))).collect();
            out.extend(hs.into_iter().map(|h| h.join().ok().flatten()));
        });
    }
    out
}

/// An HTTP answer: its status and body.
struct Answer {
    status: u16,
    body: Vec<u8>,
    ctype: String,
}

fn request(method: &str, u: &str, headers: &[(&str, String)], limit: u64) -> Result<Answer> {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(60))).http_status_as_error(false).build().into();
    let ua = concat!("GameRepo/", env!("CARGO_PKG_VERSION"));
    let r = if method == "POST" {
        let mut rq = agent.post(u).header("User-Agent", ua);
        for (k, v) in headers.iter().filter(|(k, _)| *k != "x-body") {
            rq = rq.header(*k, v.as_str());
        }
        let body = headers.iter().find(|(k, _)| *k == "x-body").map(|(_, v)| v.clone()).unwrap_or_default();
        rq.send(body.as_bytes())
    } else {
        let mut rq = agent.get(u).header("User-Agent", ua);
        for (k, v) in headers {
            rq = rq.header(*k, v.as_str());
        }
        rq.call()
    };
    let mut r = r.map_err(|e| anyhow!("{e}"))?;
    let status = r.status().as_u16();
    let ctype = r.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let mut body = vec![];
    r.body_mut().with_config().limit(limit).reader().read_to_end(&mut body)?;
    Ok(Answer { status, body, ctype })
}

/// Where answers are kept for a week.
fn cache_path(data_dir: &Path, source: &str, what: &str) -> PathBuf {
    use sha1::Digest;
    let h = hex::encode(sha1::Sha1::digest(what.as_bytes()));
    data_dir.join("scrape-cache").join(source).join(format!("{}.json", &h[..20]))
}

const WEEK: Duration = Duration::from_secs(7 * 86_400);

/// Set while a scrape asks the sources again (D48): the answers kept are passed over.
pub static FRESH: AtomicBool = AtomicBool::new(false);

fn cached(data_dir: &Path, source: &str, what: &str) -> Option<Value> {
    if FRESH.load(Ordering::Relaxed) {
        return None;
    }
    let p = cache_path(data_dir, source, what);
    let age = std::fs::metadata(&p).ok()?.modified().ok().and_then(|t| SystemTime::now().duration_since(t).ok())?;
    if age > WEEK {
        return None;
    }
    serde_json::from_slice(&std::fs::read(&p).ok()?).ok()
}

fn keep(data_dir: &Path, source: &str, what: &str, v: &Value) {
    let _ = crate::config::write_atomic(&cache_path(data_dir, source, what), v.to_string().as_bytes());
}

/// Text sent where JSON was expected, as one line.
fn short(b: &[u8]) -> String {
    let s = String::from_utf8_lossy(&b[..b.len().min(300)]).trim().replace('\n', " ");
    if s.is_empty() {
        "nothing".into()
    } else {
        s
    }
}

// ------------------------------------------------------------ ScreenScraper

/// ScreenScraper's answer, or None when it has no such game.
fn ss_get(cfg: &AppConfig, data_dir: &Path, page: &str, params: &[(&str, String)]) -> Result<Option<Value>> {
    let what = format!("{page}?{}", params.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&"));
    if let Some(v) = cached(data_dir, "screenscraper", &what) {
        return Ok((!v.is_null()).then_some(v));
    }
    let (Some(devid), Some(devpw)) = (key(cfg, "screenscraper", "devid"), key(cfg, "screenscraper", "devpassword")) else {
        bail!("{}", needs(cfg, "screenscraper").unwrap_or_default());
    };
    let mut q = format!("devid={}&devpassword={}&softname={}&output=json", encode(devid), encode(devpw), encode(concat!("GameRepo-", env!("CARGO_PKG_VERSION"))));
    if let (Some(u), Some(p)) = (key(cfg, "screenscraper", "user"), key(cfg, "screenscraper", "password")) {
        q.push_str(&format!("&ssid={}&sspassword={}", encode(u), encode(p)));
    }
    for (k, v) in params {
        q.push_str(&format!("&{k}={}", encode(v)));
    }
    let u = format!("{}/{page}?{q}", url(cfg, "screenscraper"));
    for attempt in 0..3 {
        let slot = Slot::take();
        pace("screenscraper", gap_of("screenscraper"));
        let a = request("GET", &u, &[], 8 << 20);
        drop(slot);
        let a = a?;
        let text = String::from_utf8_lossy(&a.body[..a.body.len().min(400)]).to_string();
        // errors come as French text, before or instead of JSON
        if text.contains("Erreur de login") || a.status == 403 {
            bail!("ScreenScraper didn't accept the {} (check them in Settings › Sources)", if text.contains("veloppeur") { "developer id and password" } else { "user name and password" });
        }
        if a.status == 404 || a.status == 431 || text.starts_with("Erreur : Rom") || text.contains("non trouv") {
            keep(data_dir, "screenscraper", &what, &Value::Null);
            return Ok(None);
        }
        match a.status {
            429 if attempt < 2 => {
                std::thread::sleep(Duration::from_secs(5 << attempt));
                continue;
            }
            429 => bail!("ScreenScraper says it's had too many requests this minute: try again shortly"),
            430 => bail!("Your ScreenScraper account has used today's requests: they start again at midnight (Paris time)"),
            401 => bail!("ScreenScraper is only answering its members just now (it's busy): try again later"),
            423 => bail!("ScreenScraper's service is closed just now: try again later"),
            426 => bail!("ScreenScraper doesn't accept this version of Game Repo"),
            s if s >= 500 && attempt < 2 => {
                std::thread::sleep(Duration::from_secs(3));
                continue;
            }
            s if s >= 400 => bail!("ScreenScraper answered {s}: {}", short(&a.body)),
            _ => {}
        }
        if text.contains("maximum threads") || text.contains("API ferm") || text.contains("API closed") {
            if attempt < 2 {
                std::thread::sleep(Duration::from_secs(3));
                continue;
            }
            bail!("ScreenScraper is busy: try again later");
        }
        if text.contains("quota de scrape") {
            bail!("Your ScreenScraper account has used today's requests: they start again at midnight (Paris time)");
        }
        let v = parse_lenient(&a.body).ok_or_else(|| anyhow!("ScreenScraper answered {}", short(&a.body)))?;
        keep(data_dir, "screenscraper", &what, &v);
        return Ok(Some(v));
    }
    bail!("ScreenScraper didn't answer")
}

/// JSON as ScreenScraper sends it, which is now and then not quite JSON.
fn parse_lenient(b: &[u8]) -> Option<Value> {
    if let Ok(v) = serde_json::from_slice::<Value>(b) {
        return Some(v);
    }
    let s = String::from_utf8_lossy(b).replace("],\n\t\t}", "]\n\t\t}");
    if let Ok(v) = serde_json::from_str::<Value>(&s) {
        return Some(v);
    }
    // backslashes that aren't escapes, doubled
    let mut out = String::with_capacity(s.len());
    let cs: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        if cs[i] == '\\' {
            let next = cs.get(i + 1).copied().unwrap_or(' ');
            if "\"\\/bfnrtu".contains(next) {
                out.push('\\');
                out.push(next);
                i += 2;
                continue;
            }
            out.push_str("\\\\");
        } else {
            out.push(cs[i]);
        }
        i += 1;
    }
    serde_json::from_str(&out).ok()
}

/// A field ScreenScraper sends as a string, a number or `{ text }`.
fn text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        Value::Object(o) => o.get("text").and_then(text),
        _ => None,
    }
}

/// From a list of `{ region|langue, text }`, the one for the first of `prefs` there, else the first.
fn by_pref(list: &Value, key: &str, prefs: &[String]) -> Option<String> {
    let a = list.as_array()?;
    for p in prefs {
        if let Some(t) = a.iter().find(|x| x[key].as_str().is_some_and(|r| r.eq_ignore_ascii_case(p))).and_then(|x| text(&x["text"])) {
            return Some(t);
        }
    }
    a.iter().find_map(|x| text(&x["text"]))
}

fn unescape(s: &str) -> String {
    s.replace("&nbsp;", " ").replace("&quot;", "\"").replace("&copy;", "©").replace("&#039;", "'").replace("&#39;", "'").replace("&#x26;", "&").replace("&amp;", "&")
}

/// ScreenScraper's media types for each kind, in the order they're preferred.
const SS_MEDIA: &[(&str, &[&str])] = &[
    ("cover", &["box-2D"]),
    ("back", &["box-2D-back"]),
    ("box3d", &["box-3D"]),
    ("screenshot", &["ss"]),
    ("titlescreen", &["sstitle"]),
    ("marquee", &["wheel-hd", "wheel"]),
    ("fanart", &["fanart"]),
    ("physical", &["support-2D"]),
    ("video", &["video-normalized", "video"]),
    ("manual", &["manuel"]),
];

/// A game ScreenScraper sent, as a match: names, dates and pictures in the regions
/// preferred, the description in `lang`.
fn ss_match(jeu: &Value, regions: &[String], lang: &str) -> Option<Match> {
    let id = text(&jeu["id"])?;
    let mut name_prefs: Vec<String> = regions.to_vec();
    for r in ["wor", "us", "ss", "eu", "jp"] {
        if !name_prefs.iter().any(|x| x == r) {
            name_prefs.push(r.into());
        }
    }
    let title = by_pref(&jeu["noms"], "region", &name_prefs).map(|t| unescape(&t))?;
    if title.starts_with("ZZZ(NOTGAME)") || jeu["notgame"] == "true" {
        return None;
    }
    let mut details = Map::new();
    let langs: Vec<String> = [lang, "en"].iter().map(|s| s.to_string()).collect();
    if let Some(d) = by_pref(&jeu["synopsis"], "langue", &langs) {
        details.insert("description".into(), json!(unescape(&d)));
    }
    if let Some(d) = text(&jeu["developpeur"]) {
        details.insert("developer".into(), json!(unescape(&d)));
    }
    if let Some(d) = text(&jeu["editeur"]) {
        details.insert("publisher".into(), json!(unescape(&d)));
    }
    let date = by_pref(&jeu["dates"], "region", &name_prefs);
    if let Some(d) = &date {
        details.insert("released".into(), json!(d));
    }
    let genres: Vec<String> = jeu["genres"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|g| by_pref(&g["noms"], "langue", &langs))
        .collect();
    if !genres.is_empty() {
        details.insert("genres".into(), json!(genres));
    }
    if let Some(p) = text(&jeu["joueurs"]).filter(|p| p != "null") {
        details.insert("players".into(), json!(p));
    }
    if let Some(n) = text(&jeu["note"]).and_then(|n| n.parse::<f64>().ok()).filter(|n| *n > 0.0) {
        details.insert("rating".into(), json!(((n / 20.0) * 100.0).round() / 100.0));
    }
    let medias: Vec<&Value> = jeu["medias"].as_array().into_iter().flatten().filter(|m| m["parent"].as_str().is_none_or(|p| p == "jeu")).collect();
    let mut media_prefs: Vec<String> = regions.to_vec();
    for r in ["wor", "us", "eu", "jp", "cus", "ss"] {
        if !media_prefs.iter().any(|x| x == r) {
            media_prefs.push(r.into());
        }
    }
    let mut art = BTreeMap::new();
    for (kind, types) in SS_MEDIA {
        let mut pick: Option<&Value> = None;
        'types: for t in *types {
            let of: Vec<&&Value> = medias.iter().filter(|m| m["type"].as_str() == Some(t) && m["url"].is_string()).collect();
            if of.is_empty() {
                continue;
            }
            for r in &media_prefs {
                if let Some(m) = of.iter().find(|m| m["region"].as_str().is_some_and(|x| x.eq_ignore_ascii_case(r))) {
                    pick = Some(m);
                    break 'types;
                }
            }
            pick = of.iter().find(|m| m["region"].is_null()).or(of.first()).map(|m| **m);
            break;
        }
        if let Some(m) = pick {
            art.insert(kind.to_string(), m["url"].as_str().unwrap_or("").to_string());
        }
    }
    let mut also: Vec<String> = vec![];
    for n in jeu["noms"].as_array().into_iter().flatten().filter_map(|x| text(&x["text"])).map(|t| unescape(&t)) {
        if n != title && !also.contains(&n) {
            also.push(n);
        }
    }
    Some(Match { source: "screenscraper".into(), id, title, year: date.as_deref().and_then(year_of), details, art, also })
}

/// The ScreenScraper region codes for a DAT game's regions, then the preferred ones.
pub fn regions_for(dat_regions: &str, prefs: &[String]) -> Vec<String> {
    let map = [("USA", "us"), ("Europe", "eu"), ("Japan", "jp"), ("World", "wor"), ("UK", "uk"), ("Australia", "au"), ("Brazil", "br"), ("Korea", "kr"), ("China", "cn"),
               ("France", "fr"), ("Germany", "de"), ("Spain", "sp"), ("Italy", "it"), ("Asia", "asi"), ("Canada", "ca"), ("Taiwan", "tw"), ("Netherlands", "nl"), ("Sweden", "se")];
    let mut v: Vec<String> = dat_regions.split(',').map(str::trim).filter_map(|r| map.iter().find(|(n, _)| n.eq_ignore_ascii_case(r)).map(|(_, c)| c.to_string())).collect();
    for p in prefs {
        let p = p.trim().to_lowercase();
        if !p.is_empty() && !v.contains(&p) {
            v.push(p);
        }
    }
    v
}

/// ScreenScraper's game for a DAT game, looked up by its ROMs' hashes (the first ROM
/// that ScreenScraper knows; a disc's data track is usually its biggest).
pub fn ss_lookup(cfg: &AppConfig, data_dir: &Path, g: &DatGame, regions: &[String], lang: &str) -> Result<Option<Match>> {
    let (sys, _, _) = ids_of(&g.platform);
    let mut roms: Vec<&crate::meta::Rom> = g.roms.iter().filter(|r| r.crc32.is_some() || r.sha1.is_some()).collect();
    let disc = crate::platforms::is_disc(&g.platform);
    if disc {
        // the data, not the sheet or the audio tracks
        roms.retain(|r| !r.name.to_lowercase().ends_with(".cue") && !r.name.to_lowercase().ends_with(".gdi"));
        roms.sort_by_key(|r| std::cmp::Reverse(r.size.unwrap_or(0)));
    }
    for r in roms.into_iter().take(2) {
        let mut p: Vec<(&str, String)> = vec![];
        if let Some(s) = sys {
            p.push(("systemeid", s.to_string()));
        }
        p.push(("romtype", if disc { "iso" } else { "rom" }.into()));
        if let Some(c) = &r.crc32 {
            p.push(("crc", format!("{:0>8}", c.to_uppercase())));
        }
        if let Some(m) = &r.md5 {
            p.push(("md5", m.to_uppercase()));
        }
        if let Some(s) = &r.sha1 {
            p.push(("sha1", s.to_uppercase()));
        }
        p.push(("romnom", r.name.rsplit('/').next().unwrap_or(&r.name).to_string()));
        if let Some(s) = r.size {
            p.push(("romtaille", s.to_string()));
        }
        if let Some(v) = ss_get(cfg, data_dir, "jeuInfos.php", &p)? {
            if let Some(m) = ss_match(&v["response"]["jeu"], regions, lang) {
                return Ok(Some(m));
            }
        }
    }
    Ok(None)
}

fn ss_search(cfg: &AppConfig, data_dir: &Path, query: &str, platform: &str) -> Result<Vec<Candidate>> {
    let q: String = query.trim().to_string();
    if q.chars().count() < 4 {
        // ScreenScraper won't search for fewer than four letters
        return Ok(vec![]);
    }
    // a PC game may be ScreenScraper's DOS or Windows 3.x game as well as its Windows one
    let systems: Vec<Option<u32>> = match ids_of(platform) {
        (Some(s), _, _) => std::iter::once(s).chain(also_ids(platform).0.iter().copied()).map(Some).collect(),
        _ => vec![None],
    };
    let mut jeux: Vec<Value> = vec![];
    for sys in systems {
        let mut p = vec![("recherche", q.clone())];
        if let Some(s) = sys {
            p.push(("systemeid", s.to_string()));
        }
        let Some(v) = ss_get(cfg, data_dir, "jeuRecherche.php", &p)? else { continue };
        for j in v["response"]["jeux"].as_array().into_iter().flatten() {
            if j.as_object().is_some_and(|o| !o.is_empty()) && !jeux.iter().any(|x| text(&x["id"]) == text(&j["id"])) {
                jeux.push(j.clone());
            }
        }
    }
    let regions: Vec<String> = ["us", "wor", "eu", "jp"].map(String::from).to_vec();
    Ok(jeux
        .iter()
        .filter_map(|j| ss_match(j, &regions, "en"))
        .map(|m| Candidate {
            score: crate::launchbox::score(&q, &m.title),
            cover: m.art.get("cover").cloned(),
            url: Some(format!("https://www.screenscraper.fr/gameinfos.php?gameid={}", m.id)),
            source: "screenscraper".into(),
            platform: platform.into(),
            id: m.id,
            title: m.title,
            year: m.year,
            also: m.also,
        })
        .collect())
}

fn ss_details(cfg: &AppConfig, data_dir: &Path, id: &str, regions: &[String]) -> Result<Match> {
    let v = ss_get(cfg, data_dir, "jeuInfos.php", &[("gameid", id.to_string())])?.ok_or_else(|| anyhow!("ScreenScraper has no game {id}"))?;
    ss_match(&v["response"]["jeu"], regions, "en").ok_or_else(|| anyhow!("ScreenScraper has no game {id}"))
}

// ------------------------------------------------------------ IGDB

static TOKEN: Mutex<Option<(String, String, Instant)>> = Mutex::new(None);

/// IGDB's access token (fetched from Twitch with the client id and secret, kept until it expires).
fn igdb_token(cfg: &AppConfig, renew: bool) -> Result<String> {
    let (Some(id), Some(secret)) = (key(cfg, "igdb", "client_id"), key(cfg, "igdb", "client_secret")) else {
        bail!("{}", needs(cfg, "igdb").unwrap_or_default());
    };
    {
        let t = TOKEN.lock().unwrap();
        if let Some((for_id, tok, until)) = t.as_ref() {
            if !renew && for_id == id && Instant::now() < *until {
                return Ok(tok.clone());
            }
        }
    }
    let u = format!("{}/oauth2/token?client_id={}&client_secret={}&grant_type=client_credentials", url(cfg, "twitch"), encode(id), encode(secret));
    let a = request("POST", &u, &[], 1 << 20)?;
    if a.status == 400 || a.status == 403 {
        bail!("Twitch didn't accept IGDB's client id and secret (check them in Settings › Sources)");
    }
    if a.status >= 400 {
        bail!("Twitch answered {}: {}", a.status, short(&a.body));
    }
    let v: Value = serde_json::from_slice(&a.body).map_err(|_| anyhow!("Twitch answered {}", short(&a.body)))?;
    let tok = v["access_token"].as_str().ok_or_else(|| anyhow!("Twitch gave no access token"))?.to_string();
    let secs = v["expires_in"].as_u64().unwrap_or(3600).saturating_sub(60);
    *TOKEN.lock().unwrap() = Some((id.to_string(), tok.clone(), Instant::now() + Duration::from_secs(secs)));
    Ok(tok)
}

const IGDB_FIELDS: &str = "name,first_release_date,summary,total_rating,cover.image_id,screenshots.image_id,artworks.image_id,artworks.artwork_type,genres.name,involved_companies.company.name,involved_companies.developer,involved_companies.publisher,platforms.id,platforms.name,game_modes.id,alternative_names.name";

fn igdb_post(cfg: &AppConfig, data_dir: &Path, endpoint: &str, body: &str) -> Result<Value> {
    let what = format!("{endpoint}:{body}");
    if let Some(v) = cached(data_dir, "igdb", &what) {
        return Ok(v);
    }
    let id = key(cfg, "igdb", "client_id").unwrap_or("").to_string();
    let mut renew = false;
    for attempt in 0..3 {
        let tok = igdb_token(cfg, renew)?;
        pace("igdb", gap_of("igdb"));
        let a = request(
            "POST",
            &format!("{}/{endpoint}", url(cfg, "igdb")),
            &[("Client-ID", id.clone()), ("Authorization", format!("Bearer {tok}")), ("Accept", "application/json".into()), ("x-body", body.to_string())],
            8 << 20,
        )?;
        match a.status {
            401 if attempt == 0 => {
                renew = true;
                continue;
            }
            401 | 403 => bail!("IGDB didn't accept the client id (check it in Settings › Sources)"),
            429 if attempt < 2 => {
                std::thread::sleep(Duration::from_secs(2));
                continue;
            }
            429 => bail!("IGDB says it's had too many requests: try again shortly"),
            s if s >= 400 => bail!("IGDB answered {s}: {}", short(&a.body)),
            _ => {}
        }
        let v: Value = serde_json::from_slice(&a.body).map_err(|_| anyhow!("IGDB answered {}", short(&a.body)))?;
        keep(data_dir, "igdb", &what, &v);
        return Ok(v);
    }
    bail!("IGDB didn't answer")
}

fn igdb_image(cfg: &AppConfig, id: &str, size: &str) -> String {
    format!("{}/t_{size}/{id}.jpg", url(cfg, "igdb_images"))
}

fn igdb_match(cfg: &AppConfig, g: &Value) -> Option<Match> {
    let id = g["id"].as_u64()?.to_string();
    let title = g["name"].as_str()?.to_string();
    let mut details = Map::new();
    if let Some(s) = g["summary"].as_str().filter(|s| !s.is_empty()) {
        details.insert("description".into(), json!(s));
    }
    let company = |role: &str| g["involved_companies"].as_array().into_iter().flatten().find(|c| c[role] == true).and_then(|c| c["company"]["name"].as_str()).map(String::from);
    if let Some(d) = company("developer") {
        details.insert("developer".into(), json!(d));
    }
    if let Some(p) = company("publisher") {
        details.insert("publisher".into(), json!(p));
    }
    let mut year = None;
    if let Some(t) = g["first_release_date"].as_i64() {
        let d = crate::library::iso_from_unix(t);
        year = year_of(&d);
        details.insert("released".into(), json!(d.chars().take(10).collect::<String>()));
    }
    let genres: Vec<&str> = g["genres"].as_array().into_iter().flatten().filter_map(|x| x["name"].as_str()).collect();
    if !genres.is_empty() {
        details.insert("genres".into(), json!(genres));
    }
    let modes: Vec<u64> = g["game_modes"].as_array().into_iter().flatten().filter_map(|m| m["id"].as_u64().or(m.as_u64())).collect();
    if !modes.is_empty() {
        details.insert("players".into(), json!(if modes.iter().any(|m| *m != 1) { "1-2" } else { "1" }));
    }
    if let Some(r) = g["total_rating"].as_f64().filter(|r| *r > 0.0) {
        details.insert("rating".into(), json!((r).round() / 100.0));
    }
    let mut art = BTreeMap::new();
    if let Some(c) = g["cover"]["image_id"].as_str() {
        art.insert("cover".into(), igdb_image(cfg, c, "1080p"));
    }
    let shots: Vec<&str> = g["screenshots"].as_array().into_iter().flatten().filter_map(|s| s["image_id"].as_str()).collect();
    // the first two are seldom in the game when there are more
    if let Some(s) = shots.get(if shots.len() >= 3 { 2 } else { 0 }) {
        art.insert("screenshot".into(), igdb_image(cfg, s, "1080p"));
    }
    let arts: Vec<&Value> = g["artworks"].as_array().into_iter().flatten().collect();
    if let Some(a) = [2, 1, 3].iter().find_map(|t| arts.iter().find(|a| a["artwork_type"].as_u64() == Some(*t))).or(arts.first()).and_then(|a| a["image_id"].as_str()) {
        art.insert("fanart".into(), igdb_image(cfg, a, "1080p"));
    }
    let also = g["alternative_names"].as_array().into_iter().flatten().filter_map(|n| n["name"].as_str()).filter(|n| *n != title).map(String::from).collect();
    Some(Match { source: "igdb".into(), id, title, year, details, art, also })
}

pub fn igdb_search(cfg: &AppConfig, data_dir: &Path, query: &str, platform: &str) -> Result<Vec<Match>> {
    let q = query.replace('"', "").replace('\\', "");
    let mut body = format!("search \"{q}\"; fields {IGDB_FIELDS};");
    if let (_, Some(p), _) = ids_of(platform) {
        let all: Vec<String> = std::iter::once(p).chain(also_ids(platform).1.iter().copied()).map(|p| p.to_string()).collect();
        body.push_str(&format!(" where platforms = ({});", all.join(",")));
    }
    body.push_str(" limit 15;");
    let v = igdb_post(cfg, data_dir, "games", &body)?;
    Ok(v.as_array().into_iter().flatten().filter_map(|g| igdb_match(cfg, g)).collect())
}

fn igdb_details(cfg: &AppConfig, data_dir: &Path, id: &str) -> Result<Match> {
    let n: u64 = id.parse().map_err(|_| anyhow!("{id} isn't an IGDB id"))?;
    let v = igdb_post(cfg, data_dir, "games", &format!("fields {IGDB_FIELDS}; where id = {n};"))?;
    v.as_array().and_then(|a| a.first()).and_then(|g| igdb_match(cfg, g)).ok_or_else(|| anyhow!("IGDB has no game {id}"))
}

// ------------------------------------------------------------ TheGamesDB

fn tgdb_get(cfg: &AppConfig, data_dir: &Path, path: &str, params: &[(&str, String)]) -> Result<Value> {
    let what = format!("{path}?{}", params.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&"));
    if let Some(v) = cached(data_dir, "thegamesdb", &what) {
        return Ok(v);
    }
    let Some(k) = key(cfg, "thegamesdb", "apikey") else { bail!("{}", needs(cfg, "thegamesdb").unwrap_or_default()) };
    let mut u = format!("{}/{path}?apikey={}", url(cfg, "thegamesdb"), encode(k));
    for (k, v) in params {
        u.push_str(&format!("&{}={}", k.replace('[', "%5B").replace(']', "%5D"), encode(v)));
    }
    pace("thegamesdb", gap_of("thegamesdb"));
    let a = request("GET", &u, &[], 16 << 20)?;
    match a.status {
        401 => bail!("TheGamesDB didn't accept the API key (check it in Settings › Sources)"),
        403 => bail!("TheGamesDB refused the API key: it's wrong, or it has used this month's requests"),
        429 => bail!("TheGamesDB says it's had too many requests: try again later"),
        s if s >= 400 => bail!("TheGamesDB answered {s}: {}", short(&a.body)),
        _ => {}
    }
    let v: Value = serde_json::from_slice(&a.body).map_err(|_| anyhow!("TheGamesDB answered {}", short(&a.body)))?;
    keep(data_dir, "thegamesdb", &what, &v);
    Ok(v)
}

/// TheGamesDB's names for its genre, developer and publisher ids (fetched once, kept a week).
fn tgdb_names(cfg: &AppConfig, data_dir: &Path, what: &str) -> HashMap<String, String> {
    let Ok(v) = tgdb_get(cfg, data_dir, what, &[]) else { return HashMap::new() };
    let key = what.to_lowercase();
    let d = &v["data"][&key];
    let items: Vec<&Value> = match d {
        Value::Object(o) => o.values().collect(),
        Value::Array(a) => a.iter().collect(),
        _ => vec![],
    };
    items.into_iter().filter_map(|x| Some((x["id"].as_u64().map(|n| n.to_string()).or(x["id"].as_str().map(String::from))?, x["name"].as_str()?.to_string()))).collect()
}

fn tgdb_title(s: &str) -> String {
    // it adds "(1993)" and the like
    s.split(" (").next().unwrap_or(s).trim().to_string()
}

fn tgdb_match(cfg: &AppConfig, data_dir: &Path, g: &Value, images: &Value, base: &str, full: bool) -> Option<Match> {
    let id = g["id"].as_u64()?.to_string();
    let title = tgdb_title(g["game_title"].as_str()?);
    let mut details = Map::new();
    let date = g["release_date"].as_str().unwrap_or("");
    if !date.is_empty() {
        details.insert("released".into(), json!(date));
    }
    if let Some(o) = g["overview"].as_str().filter(|s| !s.is_empty()) {
        details.insert("description".into(), json!(o.replace('\r', "")));
    }
    if let Some(p) = g["players"].as_u64().filter(|p| *p > 0) {
        details.insert("players".into(), json!(if p == 1 { "1".to_string() } else { format!("1-{p}") }));
    }
    if full {
        let ids = |k: &str| g[k].as_array().map(|a| a.iter().filter_map(|x| x.as_u64().map(|n| n.to_string())).collect::<Vec<_>>()).unwrap_or_default();
        for (k, table, field) in [("developers", "Developers", "developer"), ("publishers", "Publishers", "publisher")] {
            let want = ids(k);
            if !want.is_empty() {
                let names = tgdb_names(cfg, data_dir, table);
                if let Some(n) = want.iter().find_map(|i| names.get(i)) {
                    details.insert(field.into(), json!(n));
                }
            }
        }
        let want = ids("genres");
        if !want.is_empty() {
            let names = tgdb_names(cfg, data_dir, "Genres");
            let gs: Vec<&String> = want.iter().filter_map(|i| names.get(i)).collect();
            if !gs.is_empty() {
                details.insert("genres".into(), json!(gs));
            }
        }
    }
    let list: Vec<&Value> = images[&id].as_array().into_iter().flatten().collect();
    let find = |t: &str, side: Option<&str>| list.iter().find(|x| x["type"] == t && side.is_none_or(|s| x["side"] == s)).and_then(|x| x["filename"].as_str()).map(|f| format!("{base}{f}"));
    let mut art = BTreeMap::new();
    for (kind, t, side) in [("cover", "boxart", Some("front")), ("back", "boxart", Some("back")), ("screenshot", "screenshot", None), ("titlescreen", "titlescreen", None), ("fanart", "fanart", None), ("marquee", "clearlogo", None)] {
        if let Some(u) = find(t, side) {
            art.insert(kind.to_string(), u);
        }
    }
    Some(Match { source: "thegamesdb".into(), id, title, year: year_of(date), details, art, also: vec![] })
}

fn base_url(v: &Value) -> String {
    v["large"].as_str().or(v["original"].as_str()).unwrap_or("https://cdn.thegamesdb.net/images/large/").to_string()
}

fn tgdb_search(cfg: &AppConfig, data_dir: &Path, query: &str, platform: &str) -> Result<Vec<Match>> {
    let mut p = vec![("name", query.trim().to_string()), ("fields", "players,publishers,genres,overview".to_string()), ("include", "boxart".to_string())];
    if let (_, _, Some(t)) = ids_of(platform) {
        p.push(("filter[platform]", t.to_string()));
    }
    let v = tgdb_get(cfg, data_dir, "Games/ByGameName", &p)?;
    let base = base_url(&v["include"]["boxart"]["base_url"]);
    let imgs = &v["include"]["boxart"]["data"];
    Ok(v["data"]["games"].as_array().into_iter().flatten().take(15).filter_map(|g| tgdb_match(cfg, data_dir, g, imgs, &base, false)).collect())
}

fn tgdb_details(cfg: &AppConfig, data_dir: &Path, id: &str) -> Result<Match> {
    let v = tgdb_get(cfg, data_dir, "Games/ByGameID", &[("id", id.to_string()), ("fields", "players,publishers,genres,overview".to_string())])?;
    let g = v["data"]["games"].as_array().and_then(|a| a.first()).cloned().ok_or_else(|| anyhow!("TheGamesDB has no game {id}"))?;
    let im = tgdb_get(cfg, data_dir, "Games/Images", &[("games_id", id.to_string())]).unwrap_or(Value::Null);
    let base = base_url(&im["data"]["base_url"]);
    tgdb_match(cfg, data_dir, &g, &im["data"]["images"], &base, true).ok_or_else(|| anyhow!("TheGamesDB has no game {id}"))
}

// ------------------------------------------------------------ for Identify and the scrape

fn candidate(m: Match, query: &str, platform: &str) -> Candidate {
    let url = match m.source.as_str() {
        "igdb" => None,
        _ => Some(format!("https://thegamesdb.net/game.php?id={}", m.id)),
    };
    Candidate { score: crate::launchbox::score(query, &m.title), cover: m.art.get("cover").cloned(), url, source: m.source, platform: platform.into(), id: m.id, title: m.title, year: m.year, also: m.also }
}

/// A source's search, for Identify.
pub fn search(cfg: &AppConfig, data_dir: &Path, source: &str, query: &str, platform: &str) -> Result<Vec<Candidate>> {
    match source {
        "screenscraper" => ss_search(cfg, data_dir, query, platform),
        "igdb" => Ok(igdb_search(cfg, data_dir, query, platform)?.into_iter().map(|m| candidate(m, query, platform)).collect()),
        "thegamesdb" => Ok(tgdb_search(cfg, data_dir, query, platform)?.into_iter().map(|m| candidate(m, query, platform)).collect()),
        other => bail!("There's no source {other}."),
    }
}

/// A source's details for one of its games.
pub fn details(cfg: &AppConfig, data_dir: &Path, source: &str, id: &str, regions: &[String]) -> Result<Match> {
    match source {
        "screenscraper" => ss_details(cfg, data_dir, id, regions),
        "igdb" => igdb_details(cfg, data_dir, id),
        "thegamesdb" => tgdb_details(cfg, data_dir, id),
        other => bail!("There's no source {other}."),
    }
}

/// Try a source's keys: a search it should answer.
pub fn check(cfg: &AppConfig, data_dir: &Path, source: &str) -> Result<String> {
    if let Some(n) = needs(cfg, source) {
        bail!("{n}");
    }
    let fresh = data_dir.join("scrape-cache-check");
    let _ = std::fs::remove_dir_all(&fresh);
    match source {
        "screenscraper" => {
            let v = ss_get(cfg, &fresh, "ssuserInfos.php", &[])?;
            let _ = std::fs::remove_dir_all(&fresh);
            if let (Some(n), _) = v.as_ref().map(ss_allows).unwrap_or((None, None)) {
                ss_keep_allowed(data_dir, n);
                return Ok(format!("ScreenScraper answered: your account can scrape {n} game{} at once.", if n == 1 { "" } else { "s" }));
            }
        }
        "igdb" => {
            igdb_token(cfg, true)?;
        }
        "thegamesdb" => {
            tgdb_get(cfg, &fresh, "Genres", &[])?;
        }
        _ => {}
    }
    let _ = std::fs::remove_dir_all(&fresh);
    Ok(format!("{} answered.", crate::identify::source_name(source)))
}

/// The pictures a source sent: fetched, each checked to be what it says. A manual goes to
/// the game's documents; the rest to its media folder.
pub fn fetch(u: &str, kind: &str) -> Option<(Vec<u8>, String)> {
    let source = if u.contains("screenscraper") { "screenscraper" } else { "media" };
    let slot = (source == "screenscraper").then(|| {
        let s = Slot::take();
        pace("screenscraper", gap_of("screenscraper"));
        s
    });
    let a = request("GET", u, &[], 400 << 20);
    drop(slot);
    let a = a.ok()?;
    if a.status >= 400 || a.body.len() < 80 || a.ctype.starts_with("text/") {
        return None;
    }
    let b = &a.body;
    let ext = if kind == "manual" {
        if !b.starts_with(b"%PDF") {
            return None;
        }
        "pdf"
    } else if kind == "video" {
        if b.get(4..8) == Some(b"ftyp") {
            "mp4"
        } else if b.starts_with(b"\x1a\x45\xdf\xa3") {
            if a.ctype.contains("webm") { "webm" } else { "mkv" }
        } else if b.starts_with(b"RIFF") {
            "avi"
        } else {
            return None;
        }
    } else if b.starts_with(b"\x89PNG") {
        "png"
    } else if b.starts_with(b"\xff\xd8") {
        "jpg"
    } else if b.get(8..12) == Some(b"WEBP") {
        "webp"
    } else if b.starts_with(b"GIF8") {
        "gif"
    } else {
        return None;
    };
    Some((a.body, ext.to_string()))
}
