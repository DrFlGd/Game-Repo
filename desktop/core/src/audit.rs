//! Matching files with DAT ROMs, and the audit of a DAT (docs/PLAN.md, "Phase 1
//! design", Matching and the audit). Everything here is SQL over the index, paged,
//! so a DAT of 50,000 games is audited without loading it.

use crate::index::Index;
use crate::library::{now, Library, APP_DIR};
use crate::platforms;
use crate::roots;
use anyhow::{anyhow, bail, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// A file matches a ROM by the strongest hash both have: SHA1, else MD5, else CRC32
/// and size (quick-scanned ZIP members have only those). `{p}` is `""` for the
/// file's own hashes and `"hl_"` for them without its header.
fn match_sql(p: &str, scope: &str) -> String {
    let suffix = if p.is_empty() { "" } else { "-hl" };
    format!(
        "INSERT OR IGNORE INTO matches (file_id, rom_id, method)
         SELECT f.id, r.id, CASE WHEN r.sha1 IS NOT NULL AND b.{p}sha1 IS NOT NULL THEN 'sha1{suffix}'
                                 WHEN r.md5 IS NOT NULL AND b.{p}md5 IS NOT NULL THEN 'md5{suffix}' ELSE 'crc{suffix}' END
         FROM files f JOIN blobs b ON b.id = f.blob_id
         JOIN dat_roms r ON r.crc32 = b.{p}crc32 AND r.size = b.{p}size
         WHERE {scope}
           AND (r.sha1 IS NULL OR b.{p}sha1 IS NULL OR r.sha1 = b.{p}sha1)
           AND (r.md5 IS NULL OR b.{p}md5 IS NULL OR r.md5 = b.{p}md5)"
    )
}

/// An arcade disk matches a CHD by the CHD's own SHA1 (D58).
fn disk_sql(scope: &str) -> String {
    format!(
        "INSERT OR IGNORE INTO matches (file_id, rom_id, method)
         SELECT f.id, r.id, 'chd' FROM files f JOIN dat_roms r ON r.disk = 1 AND r.sha1 = f.chd_sha1
         WHERE f.chd_sha1 IS NOT NULL AND f.chd_sha1 != '' AND f.parent_id IS NULL AND {scope}"
    )
}

/// Something matched changed: summaries worked out before are stale.
pub fn changed(conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM index_meta WHERE key LIKE 'summary:%'", [])?;
    // and collections worked out before (collection.rs keeps the last one)
    conn.execute(
        "INSERT INTO index_meta (key, value) VALUES ('generation', '1')
         ON CONFLICT(key) DO UPDATE SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)",
        [],
    )?;
    Ok(())
}

/// How many times what matched has changed (collection.rs's cache key).
pub fn generation(conn: &Connection) -> Result<String> {
    Ok(conn.query_row("SELECT value FROM index_meta WHERE key = 'generation'", [], |r| r.get(0)).optional()?.unwrap_or_default())
}

/// Match the files hashed since they were last matched. Returns how many matches were made.
pub fn match_files(conn: &Connection) -> Result<usize> {
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM matches WHERE file_id IN (SELECT id FROM files WHERE matched = 0)", [])?;
    let scope = "f.matched = 0";
    let n = tx.execute(&match_sql("", scope), [])? + tx.execute(&match_sql("hl_", &format!("{scope} AND b.hl_crc32 IS NOT NULL")), [])?;
    let mut sheets = unmatched_sheets(&tx, "f.matched = 0")?;
    // a sheet scanned before its tracks is looked at again once files beside it are (review M12)
    let dirs: HashSet<(String, String)> = tx
        .prepare("SELECT DISTINCT root_id, rel_path FROM files WHERE matched = 0 AND parent_id IS NULL")?
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .map(|r| r.map(|(root, rel)| (root, folder_of(&rel).to_string())))
        .collect::<rusqlite::Result<_>>()?;
    if !dirs.is_empty() {
        sheets.extend(
            unmatched_sheets(&tx, "f.matched != 0")?
                .into_iter()
                .filter(|(_, root, rel, _)| dirs.contains(&(root.clone(), folder_of(rel).to_string()))),
        );
    }
    let touched = tx.execute("UPDATE files SET matched = 1 WHERE matched = 0 AND (blob_id IS NOT NULL OR kind IN ('zip', '7z', 'rar', 'chd', 'cso', 'zso', 'rvz'))", [])?;
    let n = n + match_sheets(&tx, &sheets, None)?;
    let disks = tx.execute(&disk_sql("1"), [])?;
    let n = n + disks;
    if touched > 0 || n > 0 {
        changed(&tx)?;
    }
    tx.commit()?;
    Ok(n)
}

/// Match a newly loaded DAT's ROMs with every file.
pub fn match_dat(conn: &Connection, dat_id: i64) -> Result<usize> {
    let tx = conn.unchecked_transaction()?;
    let scope = format!("r.dat_id = {dat_id}");
    let n = tx.execute(&match_sql("", &scope), [])? + tx.execute(&match_sql("hl_", &format!("{scope} AND b.hl_crc32 IS NOT NULL")), [])?;
    let sheets = unmatched_sheets(&tx, "1")?;
    let n = n + match_sheets(&tx, &sheets, Some(dat_id))? + tx.execute(&disk_sql(&scope), [])?;
    changed(&tx)?;
    tx.commit()?;
    Ok(n)
}

/// The folder a file is in, relative to its root ("" at the top).
fn folder_of(rel: &str) -> &str {
    rel.rsplit_once('/').map_or("", |(d, _)| d)
}

/// A cue sheet or GDI with no match: (file id, root id, its path relative to the root, the root's path).
type Sheet = (i64, String, String, String);

fn unmatched_sheets(conn: &Connection, scope: &str) -> Result<Vec<Sheet>> {
    let mut q = conn.prepare(&format!(
        "SELECT f.id, f.root_id, f.rel_path, rt.path FROM files f JOIN roots rt ON rt.id = f.root_id
         WHERE {scope} AND f.parent_id IS NULL AND f.kind = 'file' AND f.missing_since IS NULL
           AND (lower(f.name) LIKE '%.cue' OR lower(f.name) LIKE '%.gdi')
           AND NOT EXISTS (SELECT 1 FROM matches m WHERE m.file_id = f.id)"
    ))?;
    let rows = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

/// *Cue differs*: a cue sheet (or GDI) that isn't the DAT's own but whose files are
/// beside it and all matched to one game is matched with that game's cue (method
/// `cue`), so the game is still had (docs/PLAN.md, "2b", Disc sets and the DAT).
fn match_sheets(conn: &Connection, sheets: &[Sheet], dat_id: Option<i64>) -> Result<usize> {
    let mut made = 0;
    let mut games_of = conn.prepare_cached(
        "SELECT DISTINCT r.game_id FROM files f JOIN matches m ON m.file_id = f.id JOIN dat_roms r ON r.id = m.rom_id
         WHERE f.root_id = ?1 AND f.rel_path = ?2 AND f.parent_id IS NULL AND (?3 IS NULL OR r.dat_id = ?3)",
    )?;
    let mut ins = conn.prepare_cached(
        "INSERT OR IGNORE INTO matches (file_id, rom_id, method)
         SELECT ?1, r.id, 'cue' FROM dat_roms r WHERE r.game_id = ?2 AND lower(r.name) LIKE ?3",
    )?;
    for (id, root, rel, root_path) in sheets {
        let path = rel.split('/').fold(PathBuf::from(root_path), |p, part| p.join(part));
        let gdi = platforms::ext_of(rel) == "gdi";
        let Ok(bytes) = std::fs::read(&path) else { continue };
        if bytes.len() > 1 << 20 {
            continue;
        }
        let names = crate::cue::file_names(&String::from_utf8_lossy(&bytes), gdi);
        if names.is_empty() {
            continue;
        }
        let dir = rel.rsplit_once('/').map(|(d, _)| format!("{d}/")).unwrap_or_default();
        let mut common: Option<Vec<i64>> = None;
        for n in &names {
            let games: Vec<i64> = games_of
                .query_map(params![root, format!("{dir}{}", n.replace('\\', "/")), dat_id], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            common = Some(match common {
                None => games,
                Some(c) => c.into_iter().filter(|g| games.contains(g)).collect(),
            });
        }
        for g in common.unwrap_or_default() {
            made += ins.execute(params![id, g, if gdi { "%.gdi" } else { "%.cue" }])?;
        }
    }
    Ok(made)
}

// ------------------------------------------------------------ the audit

/// Each game of the DAT with how many of its ROMs (nodumps aside) are matched.
/// A disc's cue sheet (or GDI) counts as had when the game's tracks are in a CHD,
/// which keeps no cue (docs/PLAN.md, "2b", Compressed images). An arcade DAT's BIOS sets
/// and devices aren't games (D58): [`MACHINES`] has them too.
macro_rules! games_sql {
    ($only:literal) => {
        concat!("
  rs AS (SELECT r.game_id, r.id,
                EXISTS (SELECT 1 FROM matches m WHERE m.rom_id = r.id)
                OR ((lower(r.name) LIKE '%.cue' OR lower(r.name) LIKE '%.gdi')
                    AND EXISTS (SELECT 1 FROM dat_roms o JOIN matches m ON m.rom_id = o.id JOIN files f ON f.id = m.file_id
                                JOIN files p ON p.id = f.parent_id WHERE o.game_id = r.game_id AND p.kind = 'chd')) AS have
         FROM dat_roms r WHERE r.dat_id = ?1 AND (r.status IS NULL OR r.status != 'nodump')),
  gs AS (SELECT g.id, g.name, g.title, g.regions, g.flags, g.parent_id, g.kind, COUNT(rs.id) AS roms, SUM(rs.have) AS have,
                CASE WHEN SUM(rs.have) = COUNT(rs.id) THEN 'have' WHEN SUM(rs.have) > 0 THEN 'partial' ELSE 'missing' END AS status
         FROM dat_games g JOIN rs ON rs.game_id = g.id WHERE g.dat_id = ?1", $only, " GROUP BY g.id)")
    };
}
pub(crate) const GAMES: &str = games_sql!(" AND (g.kind IS NULL OR g.kind NOT IN ('bios', 'device'))");
pub(crate) const MACHINES: &str = games_sql!("");

/// Each file matched with one of the DAT's ROMs, whether it's named as the DAT says
/// (a ZIP member: the ZIP named for the game, the member for the ROM; a track in a disc
/// image: the image named for the game, as its tracks have no names of their own), and
/// the ROM it's listed under (one it's named for, if any).
const MATCHED: &str = "
  fm AS (SELECT f.id AS file_id,
                MAX(CASE WHEN (f.name = r.name OR p.kind IN ('chd', 'cso', 'zso', 'rvz'))
                               AND (f.parent_id IS NULL OR p.name = g.name || '.' || p.kind) THEN 1 ELSE 0 END) AS named,
                COALESCE(MIN(CASE WHEN f.name = r.name AND (f.parent_id IS NULL OR p.name = g.name || '.' || p.kind) THEN r.id END), MIN(r.id)) AS rom_id
         FROM matches m JOIN dat_roms r ON r.id = m.rom_id AND r.dat_id = ?1 JOIN dat_games g ON g.id = r.game_id
         JOIN files f ON f.id = m.file_id LEFT JOIN files p ON p.id = f.parent_id
         GROUP BY f.id)";

/// The same for an arcade DAT: named when it's where its system's set type puts it
/// (`dat_places`): in the ZIP (or 7z) of the machine it belongs in, under its name there, or
/// a disk (or loose file) in a folder named for that machine.
macro_rules! placed {
    // file f (in archive p, if any) is where its system's set type puts ROM r
    () => {
        "EXISTS (SELECT 1 FROM dat_places pl WHERE pl.rom_id = r.id AND (
                      (p.id IS NOT NULL AND p.kind IN ('zip', '7z') AND p.name = pl.container || '.' || p.kind AND f.rel_path = p.rel_path || '/' || pl.member)
                      OR (p.id IS NULL AND (f.rel_path = pl.container || '/' || pl.member
                          OR substr(f.rel_path, -length(pl.container) - length(pl.member) - 2) = '/' || pl.container || '/' || pl.member))))"
    };
}
const MATCHED_ARCADE: &str = concat!("
  fm AS (SELECT f.id AS file_id,
                MAX(CASE WHEN ", placed!(), " THEN 1 ELSE 0 END) AS named,
                MIN(r.id) AS rom_id
         FROM matches m JOIN dat_roms r ON r.id = m.rom_id AND r.dat_id = ?1
         JOIN files f ON f.id = m.file_id LEFT JOIN files p ON p.id = f.parent_id
         GROUP BY f.id)");

fn matched_sql(platform: &str) -> &'static str {
    if platforms::is_arcade(platform) {
        MATCHED_ARCADE
    } else {
        MATCHED
    }
}

/// Files of the DAT's platform with no match in it (nor in another platform's DAT):
/// *bad dumps* when named as one of its ROMs, else *unknown*.
const UNMATCHED: &str = "
  um AS (SELECT f.id AS file_id, EXISTS (SELECT 1 FROM dat_roms r WHERE r.dat_id = ?1 AND r.name = f.name) AS bad
         FROM files f
         WHERE f.platform = ?2 AND f.kind NOT IN ('zip', '7z', 'rar', 'chd', 'cso', 'zso', 'rvz') AND f.hashed > 0
           AND NOT EXISTS (SELECT 1 FROM matches m JOIN dat_roms r ON r.id = m.rom_id
                           WHERE m.file_id = f.id AND (r.dat_id = ?1 OR r.dat_id IN (SELECT id FROM dats WHERE platform != ?2)))
           -- (the tracks of a CHD that is an arcade machine's disk)
           AND NOT EXISTS (SELECT 1 FROM matches m WHERE m.file_id = f.parent_id))";

/// The files of ROMs of the DAT matched by more than one file (`dm`: rom_id, file_id).
const DUPLICATES: &str = "
  dr AS (SELECT m.rom_id FROM matches m JOIN dat_roms r ON r.id = m.rom_id AND r.dat_id = ?1 GROUP BY m.rom_id HAVING COUNT(*) > 1),
  dm AS (SELECT m.rom_id, m.file_id FROM matches m WHERE m.rom_id IN (SELECT rom_id FROM dr))";

/// The same for an arcade DAT, where a ROM a clone shares with its parent is in both
/// machines' ZIPs of a non-merged set: a file is a duplicate when it's at none of its
/// ROMs' places while another file is at one, or when two files are at the same place.
const DUPLICATES_ARCADE: &str = concat!("
  pr AS (SELECT m.rom_id, m.file_id FROM matches m JOIN dat_roms r ON r.id = m.rom_id AND r.dat_id = ?1
         JOIN files f ON f.id = m.file_id LEFT JOIN files p ON p.id = f.parent_id WHERE ", placed!(), "),
  twice AS (SELECT rom_id FROM pr GROUP BY rom_id HAVING COUNT(*) > 1),
  dm AS (SELECT MIN(m.rom_id) AS rom_id, m.file_id FROM matches m JOIN dat_roms r ON r.id = m.rom_id AND r.dat_id = ?1
         WHERE (m.file_id NOT IN (SELECT file_id FROM pr) AND m.rom_id IN (SELECT rom_id FROM pr))
            OR (m.rom_id IN (SELECT rom_id FROM twice) AND EXISTS (SELECT 1 FROM pr WHERE pr.rom_id = m.rom_id AND pr.file_id = m.file_id))
         GROUP BY m.file_id)");

fn duplicates_sql(platform: &str) -> &'static str {
    if platforms::is_arcade(platform) {
        DUPLICATES_ARCADE
    } else {
        DUPLICATES
    }
}

pub struct DatRef {
    pub id: i64,
    pub platform: String,
    pub name: String,
    pub version: String,
}

pub fn dat_ref(conn: &Connection, id: i64) -> Result<DatRef> {
    conn.query_row("SELECT id, platform, name, version FROM dats WHERE id = ?1", [id], |r| {
        Ok(DatRef { id: r.get(0)?, platform: r.get(1)?, name: r.get(2)?, version: r.get(3)? })
    })
    .map_err(|_| anyhow!("There's no DAT {id}."))
}

/// The DAT a platform is audited against: the one chosen for it on the Audit page
/// (kept in library.json as `audit_dats: {platform: DAT name}`), else its current
/// No-Intro DAT, else its first current DAT by name that isn't a DAT of updates and DLC.
pub fn primary_dat(conn: &Connection, lib: &Library, platform: &str) -> Result<Option<i64>> {
    let chosen = lib.meta()["audit_dats"][platform].as_str().map(String::from);
    let preferred = platforms::by_id(platform).and_then(|p| p.dat_names.first()).map(|n| n.to_ascii_lowercase()).unwrap_or_default();
    let mut q = conn.prepare("SELECT id, name FROM dats WHERE platform = ?1 AND current = 1 ORDER BY name")?;
    let rows: Vec<(i64, String)> = q.query_map([platform], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    Ok(rows
        .iter()
        .find(|(_, n)| Some(n) == chosen.as_ref())
        .or_else(|| rows.iter().find(|(_, n)| platforms::dat_base_name(n).to_ascii_lowercase() == preferred && !crate::addons::is_addon_dat(n)))
        .or_else(|| rows.iter().find(|(_, n)| !crate::addons::is_addon_dat(n)))
        .or(rows.first())
        .map(|(id, _)| *id))
}

/// Audit `platform` against DAT `dat_id` from now on (Home and Audit use it).
pub fn choose_dat(conn: &Connection, lib: &Library, dat_id: i64) -> Result<()> {
    let d = dat_ref(conn, dat_id)?;
    let mut chosen = lib.meta()["audit_dats"].as_object().cloned().unwrap_or_default();
    if chosen.get(&d.platform).and_then(Value::as_str) != Some(d.name.as_str()) {
        chosen.insert(d.platform, json!(d.name));
        lib.update_meta(json!({ "audit_dats": chosen }))?;
    }
    Ok(())
}

/// The counts for a DAT: games have/partial/missing, 1G1R groups, and files by category.
pub fn summary(conn: &Connection, dat_id: i64) -> Result<Value> {
    let key = format!("summary:{dat_id}");
    if let Some(cached) = conn
        .query_row("SELECT value FROM index_meta WHERE key = ?1", [&key], |r| r.get::<_, String>(0))
        .optional()?
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
    {
        return Ok(cached);
    }
    let d = dat_ref(conn, dat_id)?;
    let (have, partial, missing, groups, groups_have): (i64, i64, i64, i64, i64) = conn.query_row(
        &format!(
            "WITH {GAMES} SELECT COALESCE(SUM(status = 'have'), 0), COALESCE(SUM(status = 'partial'), 0), COALESCE(SUM(status = 'missing'), 0),
                    COUNT(DISTINCT COALESCE(parent_id, id)), COUNT(DISTINCT CASE WHEN status = 'have' THEN COALESCE(parent_id, id) END) FROM gs"
        ),
        [dat_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
    )?;
    let (matched, misnamed): (i64, i64) = conn.query_row(
        &format!("WITH {} SELECT COALESCE(SUM(named), 0), COALESCE(SUM(1 - named), 0) FROM fm", matched_sql(&d.platform)),
        [dat_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let (bad, unknown): (i64, i64) = conn.query_row(
        &format!("WITH {UNMATCHED} SELECT COALESCE(SUM(bad), 0), COALESCE(SUM(1 - bad), 0) FROM um"),
        params![dat_id, d.platform],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let duplicates: i64 = conn.query_row(
        &format!("WITH {} SELECT COUNT(*) FROM dm", duplicates_sql(&d.platform)),
        [dat_id],
        |r| r.get(0),
    )?;
    let needs_tool: i64 = conn.query_row("SELECT COUNT(*) FROM files WHERE platform = ?1 AND locked = 2 AND missing_since IS NULL", [&d.platform], |r| r.get(0))?;
    let by_crc: i64 = conn.query_row(
        "SELECT COUNT(DISTINCT m.file_id) FROM matches m JOIN dat_roms r ON r.id = m.rom_id AND r.dat_id = ?1 WHERE m.method LIKE 'crc%'",
        [dat_id],
        |r| r.get(0),
    )?;
    let v = json!({
        "dat": { "id": d.id, "platform": d.platform, "platform_name": platforms::name_of(&d.platform), "name": d.name, "version": d.version },
        "games": have + partial + missing, "have": have, "partial": partial, "missing": missing,
        "groups": groups, "groups_have": groups_have,
        "matched": matched, "misnamed": misnamed, "baddump": bad, "unknown": unknown, "duplicates": duplicates, "by_crc": by_crc,
        "needs_tool": needs_tool,
    });
    let mut v = v;
    if platforms::is_arcade(&d.platform) {
        // D59: the games that run, and the BIOS sets and devices they lean on
        let a = crate::arcade::needs(conn, dat_id)?;
        v["arcade"] = json!({ "playable": a["playable"], "sets": a["sets"], "sets_have": a["sets_have"] });
    }
    conn.execute(
        "INSERT INTO index_meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, v.to_string()],
    )?;
    Ok(v)
}

/// The lists the Audit page shows.
pub const TABS: &[&str] = &["have", "partial", "missing", "matched", "misnamed", "baddump", "unknown", "duplicates", "sets"];

/// Where a file is on this computer: (the file to show in a folder, how it's shown).
fn locate(dirs: &HashMap<String, PathBuf>, root_id: &str, rel: &str, member: Option<&str>) -> (String, String) {
    let base = dirs.get(root_id).cloned().unwrap_or_default();
    let rel_fs = match member {
        Some(m) => rel.strip_suffix(&format!("/{m}")).unwrap_or(rel),
        None => rel,
    };
    let path = rel_fs.split('/').fold(base, |p, part| p.join(part));
    let shown = match member {
        Some(m) => format!("{} › {m}", path.display()),
        None => path.display().to_string(),
    };
    (path.display().to_string(), shown)
}

fn like(search: &str) -> String {
    let esc: String = search.trim().chars().flat_map(|c| match c {
        '%' | '_' | '\\' => vec!['\\', c],
        c => vec![c],
    }).collect();
    format!("%{esc}%")
}

/// One page of a list: `{ total, items }`.
pub fn list(ix: &Index, lib: &Library, dat_id: i64, tab: &str, search: &str, offset: i64, limit: i64) -> Result<Value> {
    list_by(ix, lib, dat_id, tab, search, "", "name", offset, limit)
}

/// `list`, a games list narrowed to one region and in another order (by `region` or
/// `year`); its first page also says the regions there are in it, and how many of each.
#[allow(clippy::too_many_arguments)]
pub fn list_by(ix: &Index, lib: &Library, dat_id: i64, tab: &str, search: &str, region: &str, sort: &str, offset: i64, limit: i64) -> Result<Value> {
    let conn = ix.conn();
    let d = dat_ref(conn, dat_id)?;
    let limit = limit.clamp(1, 1000);
    let pat = like(search);
    let dirs: HashMap<String, PathBuf> = roots::list(lib).into_iter().map(|r| (r.id, r.dir)).collect();
    match tab {
        "have" | "partial" | "missing" | "sets" => {
            // an arcade DAT's BIOS sets and devices are listed apart (D58)
            let (with, which) = if tab == "sets" { (MACHINES, "gs.kind IN ('bios', 'device') AND ?2 = ?2") } else { (GAMES, "gs.status = ?2") };
            let in_region = "(?4 = '' OR (', ' || COALESCE(gs.regions, '') || ', ') LIKE '%, ' || ?4 || ', %')";
            let total: i64 = conn.query_row(
                &format!("WITH {with} SELECT COUNT(*) FROM gs WHERE {which} AND name LIKE ?3 ESCAPE '\\' AND {in_region}"),
                params![dat_id, tab, pat, region],
                |r| r.get(0),
            )?;
            let order = match sort {
                "region" => "COALESCE(gs.regions, '~'), gs.name",
                "year" => "COALESCE(dg.year, '9999'), gs.name",
                _ => "gs.name",
            };
            let mut q = conn.prepare(&format!(
                "WITH {with}
                 SELECT gs.id, gs.name, gs.title, gs.regions, gs.flags, gs.roms, gs.have, pg.name,
                        (SELECT f.root_id || char(9) || f.rel_path || char(9) || COALESCE(p.name, '') || char(9) || f.name
                         FROM dat_roms r JOIN matches m ON m.rom_id = r.id JOIN files f ON f.id = m.file_id LEFT JOIN files p ON p.id = f.parent_id
                         WHERE r.game_id = gs.id LIMIT 1),
                        dg.year, dg.manufacturer, gs.kind, gs.status
                 FROM gs JOIN dat_games dg ON dg.id = gs.id LEFT JOIN dat_games pg ON pg.id = gs.parent_id
                 WHERE {which} AND gs.name LIKE ?3 ESCAPE '\\' AND {in_region} ORDER BY {order} LIMIT ?5 OFFSET ?6"
            ))?;
            let items: Vec<Value> = q
                .query_map(params![dat_id, tab, pat, region, limit, offset], |r| {
                    let file: Option<String> = r.get(8)?;
                    let (path, shown) = file
                        .map(|s| {
                            let p: Vec<&str> = s.split('\t').collect();
                            let member = (!p[2].is_empty()).then_some(p[3]);
                            let (a, b) = locate(&dirs, p[0], p[1], member);
                            (Some(a), Some(b))
                        })
                        .unwrap_or((None, None));
                    Ok(json!({
                        "id": r.get::<_, i64>(0)?, "name": r.get::<_, String>(1)?, "title": r.get::<_, Option<String>>(2)?,
                        "regions": r.get::<_, Option<String>>(3)?, "flags": r.get::<_, Option<String>>(4)?,
                        "roms": r.get::<_, i64>(5)?, "have": r.get::<_, i64>(6)?, "clone_of": r.get::<_, Option<String>>(7)?,
                        "path": path, "shown": shown, "year": r.get::<_, Option<String>>(9)?, "manufacturer": r.get::<_, Option<String>>(10)?,
                        "kind": r.get::<_, Option<String>>(11)?, "status": r.get::<_, String>(12)?,
                    }))
                })?
                .collect::<rusqlite::Result<_>>()?;
            let mut items = items;
            if platforms::is_arcade(&d.platform) && tab != "sets" {
                // what each game is waiting for to run (D59)
                let a = crate::arcade::needs(conn, dat_id)?;
                for it in &mut items {
                    let n = &a["games"][it["id"].to_string()];
                    it["needs"] = n["needs"].clone();
                    it["samples"] = n["samples"].clone();
                }
            }
            let mut out = json!({ "total": total, "items": items });
            if offset == 0 {
                // the regions in the list (searched, not narrowed), the most games first
                let mut q = conn.prepare(&format!("WITH {with} SELECT gs.regions FROM gs WHERE {which} AND name LIKE ?3 ESCAPE '\\'"))?;
                let mut n: HashMap<String, i64> = HashMap::new();
                for r in q.query_map(params![dat_id, tab, pat], |r| r.get::<_, Option<String>>(0))? {
                    for x in r?.unwrap_or_default().split(", ").filter(|x| !x.is_empty()) {
                        *n.entry(x.to_string()).or_default() += 1;
                    }
                }
                let mut v: Vec<(String, i64)> = n.into_iter().collect();
                v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
                out["regions"] = json!(v.iter().map(|(r, c)| json!({ "region": r, "count": c })).collect::<Vec<_>>());
            }
            Ok(out)
        }
        "matched" | "misnamed" | "duplicates" => {
            let (with, filter, from) = match tab {
                "matched" => (matched_sql(&d.platform), "fm.named = 1", "fm JOIN files f ON f.id = fm.file_id JOIN dat_roms r ON r.id = fm.rom_id"),
                "misnamed" => (matched_sql(&d.platform), "fm.named = 0", "fm JOIN files f ON f.id = fm.file_id JOIN dat_roms r ON r.id = fm.rom_id"),
                _ => (duplicates_sql(&d.platform), "1", "dm JOIN files f ON f.id = dm.file_id JOIN dat_roms r ON r.id = dm.rom_id"),
            };
            let search_sql = "(f.rel_path LIKE ?2 ESCAPE '\\' OR r.name LIKE ?2 ESCAPE '\\')";
            let total: i64 = conn.query_row(
                &format!("WITH {with} SELECT COUNT(*) FROM {from} WHERE {filter} AND {search_sql}"),
                params![dat_id, pat],
                |r| r.get(0),
            )?;
            let mut q = conn.prepare(&format!(
                "WITH {with}
                 SELECT f.id, f.root_id, f.rel_path, f.name, f.parent_id IS NOT NULL, p.name, r.name, g.name, r.status,
                        (SELECT method FROM matches x WHERE x.file_id = f.id AND x.rom_id = r.id), f.missing_since,
                        (SELECT pl.container || char(9) || pl.member || char(9) || r.disk FROM dat_places pl WHERE pl.rom_id = r.id LIMIT 1)
                 FROM {from} JOIN dat_games g ON g.id = r.game_id LEFT JOIN files p ON p.id = f.parent_id
                 WHERE {filter} AND {search_sql} ORDER BY r.name, f.rel_path LIMIT ?3 OFFSET ?4"
            ))?;
            let items: Vec<Value> = q
                .query_map(params![dat_id, pat, limit, offset], |r| {
                    let (root, rel, name, member): (String, String, String, bool) = (r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?);
                    let (rom, game): (String, String) = (r.get(6)?, r.get(7)?);
                    let (path, shown) = locate(&dirs, &root, &rel, member.then_some(name.as_str()));
                    // an arcade ROM is where its set type puts it
                    let place: Option<String> = r.get(11)?;
                    let expected = match place.as_deref().map(|p| p.split('\t').collect::<Vec<_>>()) {
                        Some(p) if p.len() == 3 && p[2] == "1" => format!("{}/{}", p[0], p[1]),
                        Some(p) if p.len() == 3 => format!("{}.zip › {}", p[0], p[1]),
                        _ if member => format!("{game}.zip › {rom}"),
                        _ => rom.clone(),
                    };
                    Ok(json!({
                        "id": r.get::<_, i64>(0)?, "path": path, "shown": shown, "name": name, "zip": r.get::<_, Option<String>>(5)?,
                        "rom": rom, "game": game, "expected": expected, "baddump": r.get::<_, Option<String>>(8)?.as_deref() == Some("baddump"),
                        "method": r.get::<_, Option<String>>(9)?, "offline": r.get::<_, Option<String>>(10)?.is_some(),
                    }))
                })?
                .collect::<rusqlite::Result<_>>()?;
            Ok(json!({ "total": total, "items": items }))
        }
        "baddump" | "unknown" => {
            let bad = (tab == "baddump") as i64;
            let total: i64 = conn.query_row(
                &format!("WITH {UNMATCHED} SELECT COUNT(*) FROM um JOIN files f ON f.id = um.file_id WHERE um.bad = ?3 AND f.rel_path LIKE ?4 ESCAPE '\\'"),
                params![dat_id, d.platform, bad, pat],
                |r| r.get(0),
            )?;
            let mut q = conn.prepare(&format!(
                "WITH {UNMATCHED}
                 SELECT f.id, f.root_id, f.rel_path, f.name, f.parent_id IS NOT NULL, p.name, f.size, b.crc32, b.sha1, f.missing_since
                 FROM um JOIN files f ON f.id = um.file_id LEFT JOIN files p ON p.id = f.parent_id LEFT JOIN blobs b ON b.id = f.blob_id
                 WHERE um.bad = ?3 AND f.rel_path LIKE ?4 ESCAPE '\\' ORDER BY f.rel_path LIMIT ?5 OFFSET ?6"
            ))?;
            let items: Vec<Value> = q
                .query_map(params![dat_id, d.platform, bad, pat, limit, offset], |r| {
                    let (root, rel, name, member): (String, String, String, bool) = (r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?);
                    let (path, shown) = locate(&dirs, &root, &rel, member.then_some(name.as_str()));
                    Ok(json!({
                        "id": r.get::<_, i64>(0)?, "path": path, "shown": shown, "name": name, "zip": r.get::<_, Option<String>>(5)?,
                        "size": r.get::<_, i64>(6)?, "crc32": r.get::<_, Option<String>>(7)?, "sha1": r.get::<_, Option<String>>(8)?,
                        "offline": r.get::<_, Option<String>>(9)?.is_some(),
                    }))
                })?
                .collect::<rusqlite::Result<_>>()?;
            Ok(json!({ "total": total, "items": items }))
        }
        other => bail!("unknown list {other} (one of {})", TABS.join(", ")),
    }
}

pub(crate) fn xml(s: &str) -> String {
    quick_xml::escape::escape(s).into_owned()
}

/// The games missing (and partly there) as a Logiqx DAT, listing the ROMs not yet matched.
pub fn fixdat_xml(conn: &Connection, dat_id: i64) -> Result<(String, String, usize)> {
    let d = dat_ref(conn, dat_id)?;
    let date = &now()[..10];
    let name = format!("{} (fixdat {date})", d.name);
    let mut out = format!(
        "<?xml version=\"1.0\"?>\n<datafile>\n\t<header>\n\t\t<name>{}</name>\n\t\t<description>{}</description>\n\t\t<version>{}</version>\n\t\t<author>Game Repo</author>\n\t</header>\n",
        xml(&name), xml(&format!("Missing from {} ({})", d.name, d.version)), xml(date)
    );
    let mut q = conn.prepare(&format!(
        "WITH {GAMES}
         SELECT gs.id, gs.name, g.description, pg.name, r.name, r.size, r.crc32, r.md5, r.sha1
         FROM gs JOIN dat_games g ON g.id = gs.id LEFT JOIN dat_games pg ON pg.id = gs.parent_id
         JOIN dat_roms r ON r.game_id = gs.id AND (r.status IS NULL OR r.status != 'nodump')
         JOIN rs ON rs.id = r.id
         WHERE gs.status != 'have' AND NOT rs.have
         ORDER BY gs.name, r.name"
    ))?;
    let mut rows = q.query([dat_id])?;
    let mut games = 0;
    let mut open: Option<i64> = None;
    while let Some(r) = rows.next()? {
        let gid: i64 = r.get(0)?;
        if open != Some(gid) {
            if open.is_some() {
                out.push_str("\t</game>\n");
            }
            let gname: String = r.get(1)?;
            let parent: Option<String> = r.get(3)?;
            let clone = parent.map(|p| format!(" cloneof=\"{}\"", xml(&p))).unwrap_or_default();
            out.push_str(&format!("\t<game name=\"{}\"{clone}>\n\t\t<description>{}</description>\n", xml(&gname), xml(&r.get::<_, Option<String>>(2)?.unwrap_or(gname.clone()))));
            open = Some(gid);
            games += 1;
        }
        let mut rom = format!("\t\t<rom name=\"{}\"", xml(&r.get::<_, String>(4)?));
        if let Some(s) = r.get::<_, Option<i64>>(5)? {
            rom.push_str(&format!(" size=\"{s}\""));
        }
        for (k, i) in [("crc", 6), ("md5", 7), ("sha1", 8)] {
            if let Some(v) = r.get::<_, Option<String>>(i)? {
                rom.push_str(&format!(" {k}=\"{v}\""));
            }
        }
        out.push_str(&rom);
        out.push_str("/>\n");
    }
    if open.is_some() {
        out.push_str("\t</game>\n");
    }
    out.push_str("</datafile>\n");
    Ok((name, out, games))
}

/// Write the fixdat to `_library/fixdats/` (or `to`), returning where and how many games.
pub fn fixdat(ix: &Index, lib: &Library, dat_id: i64, to: Option<PathBuf>) -> Result<Value> {
    let (name, text, games) = fixdat_xml(ix.conn(), dat_id)?;
    let path = match to {
        Some(p) => p,
        None => {
            lib.writable()?;
            let safe: String = name.chars().map(|c| if "<>:\"/\\|?*".contains(c) { '_' } else { c }).collect();
            lib.root().join(APP_DIR).join("fixdats").join(format!("{safe}.dat"))
        }
    };
    crate::config::write_atomic(&path, text.as_bytes())?;
    Ok(json!({ "path": path.display().to_string(), "rel": lib.relative(&path), "name": name, "games": games }))
}

/// A plain-text report of the audit (Copy report).
pub fn report(ix: &Index, dat_id: i64) -> Result<String> {
    let s = summary(ix.conn(), dat_id)?;
    let n = |k: &str| s[k].as_i64().unwrap_or(0);
    let mut out = format!(
        "{} ({})\n{}\n\nGames: {} of {} ({} partly there, {} missing)\n1G1R: {} of {} groups\nFiles: {} matched, {} misnamed, {} bad dumps, {} unknown, {} duplicates\n",
        s["dat"]["name"].as_str().unwrap_or(""), s["dat"]["version"].as_str().unwrap_or(""), s["dat"]["platform_name"].as_str().unwrap_or(""),
        n("have"), n("games"), n("partial"), n("missing"), n("groups_have"), n("groups"),
        n("matched"), n("misnamed"), n("baddump"), n("unknown"), n("duplicates"),
    );
    let conn = ix.conn();
    let mut q = conn.prepare(&format!("WITH {GAMES} SELECT name FROM gs WHERE status != 'have' ORDER BY name"))?;
    let missing: Vec<String> = q.query_map([dat_id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    if !missing.is_empty() {
        out.push_str("\nMissing or partly there:\n");
        for m in missing {
            out.push_str(&format!("  {m}\n"));
        }
    }
    Ok(out)
}

/// Save the audit as a report in `_library/reports/`: "txt" (the counts and what's
/// missing, as Copy report gives) or "csv" (every list, a row per game or file).
pub fn save_report(ix: &Index, lib: &Library, dat_id: i64, format: &str) -> Result<Value> {
    lib.writable()?;
    let d = dat_ref(ix.conn(), dat_id)?;
    let text = match format {
        "txt" => report(ix, dat_id)?,
        "csv" => {
            let cell = |s: &str| if s.contains([',', '"', '\n']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() };
            let mut out = String::from("list,game or file,path,expected\n");
            for tab in TABS {
                let mut offset = 0;
                loop {
                    let page = list(ix, lib, dat_id, tab, "", offset, 1000)?;
                    let items = page["items"].as_array().cloned().unwrap_or_default();
                    for it in &items {
                        let s = |k: &str| it[k].as_str().unwrap_or("").to_string();
                        out.push_str(&format!("{},{},{},{}\n", tab, cell(&s("name")), cell(&s("shown")), cell(&s("expected"))));
                    }
                    if items.len() < 1000 {
                        break;
                    }
                    offset += 1000;
                }
            }
            out
        }
        other => bail!("unknown report format {other} (txt or csv)"),
    };
    let safe: String = d.name.chars().map(|c| if "<>:\"/\\|?*".contains(c) || c.is_control() { '_' } else { c }).collect();
    let path = lib.root().join(APP_DIR).join("reports").join(format!("{safe} ({}).{format}", &now()[..10]));
    crate::config::write_atomic(&path, text.as_bytes())?;
    Ok(json!({ "path": path.display().to_string(), "rel": lib.relative(&path) }))
}

/// Every system with a DAT, audited against its DAT: the counts of each list, for the
/// Audit page's overview.
pub fn overview(ix: &Index, lib: &Library) -> Result<Value> {
    let conn = ix.conn();
    let mut q = conn.prepare("SELECT DISTINCT platform FROM dats WHERE current = 1 AND platform != 'bios' ORDER BY platform")?;
    let with_dats: Vec<String> = q.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    let mut v = vec![];
    for p in &with_dats {
        if let Some(id) = primary_dat(conn, lib, p)? {
            let mut s = summary(conn, id)?;
            s["platform"] = json!(p);
            s["platform_name"] = json!(platforms::name_of(p));
            v.push(s);
        }
    }
    v.sort_by(|a, b| a["platform_name"].as_str().cmp(&b["platform_name"].as_str()));
    Ok(json!({ "systems": v }))
}

/// What Home shows: a bar per platform with a DAT, the last scan, and what needs attention.
pub fn home(ix: &Index, lib: &Library) -> Result<Value> {
    let conn = ix.conn();
    // (BIOS files have a page of their own)
    let mut q = conn.prepare("SELECT DISTINCT platform FROM dats WHERE current = 1 AND platform != 'bios' ORDER BY platform")?;
    let with_dats: Vec<String> = q.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    let mut bars = vec![];
    for p in &with_dats {
        if let Some(id) = primary_dat(conn, lib, p)? {
            let s = summary(conn, id)?;
            bars.push(json!({
                "platform": p, "platform_name": platforms::name_of(p), "dat": id, "dat_name": s["dat"]["name"],
                "have": s["have"], "games": s["games"], "groups": s["groups"], "groups_have": s["groups_have"],
                "partial": s["partial"], "attention": s["misnamed"].as_i64().unwrap_or(0) + s["baddump"].as_i64().unwrap_or(0),
            }));
        }
    }
    bars.sort_by(|a, b| a["platform_name"].as_str().cmp(&b["platform_name"].as_str()));
    // files of platforms without a DAT, and files without a platform
    let mut q = conn.prepare(
        "SELECT COALESCE(platform, ''), COUNT(*) FROM files f WHERE kind NOT IN ('zip', '7z', 'rar', 'chd', 'cso', 'zso', 'rvz')
           AND NOT EXISTS (SELECT 1 FROM matches m WHERE m.file_id = f.id)
           AND (platform IS NULL OR platform NOT IN (SELECT platform FROM dats))
         GROUP BY platform ORDER BY platform",
    )?;
    let loose: Vec<Value> = q
        .query_map([], |r| {
            let p: String = r.get(0)?;
            Ok(json!({ "platform": (!p.is_empty()).then(|| p.clone()), "platform_name": (!p.is_empty()).then(|| platforms::name_of(&p)), "files": r.get::<_, i64>(1)? }))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let roots: Vec<Value> = roots::list(lib).iter().map(|r| r.to_json()).collect();
    let files: i64 = conn.query_row("SELECT COUNT(*) FROM files WHERE kind NOT IN ('zip', '7z', 'rar', 'chd', 'cso', 'zso', 'rvz')", [], |r| r.get(0))?;
    let unhashed: i64 = conn.query_row("SELECT COUNT(*) FROM files WHERE hashed = 0 AND locked != 2", [], |r| r.get(0))?;
    // RVZ files wait for dolphin-tool (scan.rs)
    let needs_tool: i64 = conn.query_row("SELECT COUNT(*) FROM files WHERE locked = 2 AND missing_since IS NULL", [], |r| r.get(0))?;
    let dats: i64 = conn.query_row("SELECT COUNT(*) FROM dats", [], |r| r.get(0))?;
    Ok(json!({
        "platforms": bars, "loose": loose, "roots": roots, "files": files, "unhashed": unhashed, "dats": dats, "needs_tool": needs_tool,
        "last_scan": crate::scan::last(ix)?, "next": next(ix, lib, &bars)?,
    }))
}

/// What to do next, for Home's *Next* strip: files to put right on the Audit page, games
/// whose details need a look, systems you have games of that aren't scraped yet, and
/// systems missing BIOS files. Each says what, how many, and the page it's done on.
fn next(ix: &Index, lib: &Library, bars: &[Value]) -> Result<Vec<Value>> {
    let conn = ix.conn();
    let mut v = vec![];
    for b in bars {
        let n = b["attention"].as_i64().unwrap_or(0);
        if n > 0 {
            v.push(json!({ "kind": "misnamed", "system": b["platform"], "count": n, "page": "audit",
                           "text": format!("{} in {} to put right", if n == 1 { "1 file".to_string() } else { format!("{n} files") }, b["platform_name"].as_str().unwrap_or("")) }));
        }
    }
    let look: i64 = conn.query_row("SELECT COUNT(*) FROM meta_games WHERE review = 1", [], |r| r.get(0))?;
    if look > 0 {
        v.push(json!({ "kind": "look", "count": look, "page": "library?look=1",
                       "text": format!("{} whose details need a look", if look == 1 { "1 game".to_string() } else { format!("{look} games") }) }));
    }
    for b in bars {
        let (p, have) = (b["platform"].as_str().unwrap_or(""), b["have"].as_i64().unwrap_or(0));
        let scraped: i64 = conn.query_row("SELECT COUNT(*) FROM meta_games WHERE platform = ?1 AND details > 0", [p], |r| r.get(0))?;
        if have > 0 && scraped == 0 && !platforms::is_arcade(p) {
            v.push(json!({ "kind": "scrape", "system": p, "count": have, "page": "metadata",
                           "text": format!("{} isn't scraped yet", b["platform_name"].as_str().unwrap_or("")) }));
        }
    }
    let bios_dats: i64 = conn.query_row("SELECT COUNT(*) FROM dats WHERE current = 1 AND platform = ?1", [crate::bios::ID], |r| r.get(0))?;
    if bios_dats > 0 {
        let b = crate::bios::summary(ix, lib)?;
        let short = b["systems"].as_array().map(|a| a.iter().filter(|s| s["yours"] == true && s["missing"].as_i64().unwrap_or(0) > 0).count()).unwrap_or(0);
        if short > 0 {
            v.push(json!({ "kind": "bios", "count": short, "page": "bios",
                           "text": format!("BIOS files missing for {}", if short == 1 { "1 system".to_string() } else { format!("{short} systems") }) }));
        }
    }
    Ok(v)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::hash::hash_reader;
    use crate::library::tests::temp_dir;
    use crate::scan::{self, Options};
    use serde_json::Map;
    use std::io::Write;
    use std::path::Path;
    use std::sync::atomic::AtomicBool;

    /// Bytes that look like a ROM: `n` pseudo-random bytes from `seed`.
    pub fn rom(seed: u32, n: usize) -> Vec<u8> {
        let mut x = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect()
    }

    pub fn rom_xml(name: &str, bytes: &[u8]) -> String {
        let h = hash_reader(&mut &bytes[..], bytes.len() as u64, name, &AtomicBool::new(false)).unwrap();
        format!(
            "<rom name=\"{}\" size=\"{}\" crc=\"{}\" md5=\"{}\" sha1=\"{}\"/>",
            xml(name), h.size, h.crc32, h.md5.unwrap(), h.sha1.unwrap()
        )
    }

    pub fn dat_xml(name: &str, version: &str, games: &[(&str, Option<&str>, String)]) -> String {
        let mut s = format!("<?xml version=\"1.0\"?>\n<datafile><header><name>{name}</name><description>{name}</description><version>{version}</version><homepage>No-Intro</homepage></header>\n");
        for (g, parent, roms) in games {
            let clone = parent.map(|p| format!(" cloneof=\"{}\"", xml(p))).unwrap_or_default();
            s.push_str(&format!("<game name=\"{}\"{clone}><description>{}</description>{roms}</game>\n", xml(g), xml(g)));
        }
        s.push_str("</datafile>\n");
        s
    }

    pub(crate) fn write(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    pub(crate) fn zip(path: &Path, members: &[(&str, &[u8])]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (name, bytes) in members {
            z.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(bytes).unwrap();
        }
        z.finish().unwrap();
    }

    pub(crate) struct Fixture {
        pub dir: PathBuf,
        pub lib: Library,
        pub ix: Index,
        pub gb: PathBuf,
    }

    pub(crate) fn fixture(name: &str) -> Fixture {
        let dir = temp_dir(name);
        let lib = Library::open(dir.join("Lib")).unwrap();
        let ix = Index::open(&dir.join("index.sqlite"), "lib-x").unwrap();
        let gb = dir.join("Roms/Game Boy");
        let (a, b, c, d, e) = (rom(1, 32768), rom(2, 32768), rom(3, 32768), rom(4, 65536), rom(5, 16384));
        write(&gb.join("Alpha (USA).gb"), &a);
        write(&gb.join("Copies/Alpha copy.gb"), &a);
        write(&gb.join("beta.gb"), &b);
        let mut bad = c.clone();
        bad[100] ^= 0xff;
        write(&gb.join("Gamma (Japan).gb"), &bad);
        write(&gb.join("My Hack.gb"), &rom(99, 32768));
        zip(&gb.join("Delta (World).zip"), &[("Delta (World).gb", &d)]);
        let mut ines = b"NES\x1a\x01\x01\0\0\0\0\0\0\0\0\0\0".to_vec();
        ines.extend_from_slice(&e);
        write(&dir.join("Roms/NES/Echo (USA).nes"), &ines);
        write(&gb.join(".hidden.gb"), &a);
        write(&gb.join("Thumbs.db"), b"x");
        crate::roots::add(&lib, &dir.join("Roms"), None).unwrap();
        let gb_dat = dat_xml("Nintendo - Game Boy (Parent-Clone)", "20261001-000000", &[
            ("Alpha (USA)", None, rom_xml("Alpha (USA).gb", &a)),
            ("Alpha (Europe)", Some("Alpha (USA)"), rom_xml("Alpha (Europe).gb", &rom(11, 32768))),
            ("Beta (Europe)", None, rom_xml("Beta (Europe).gb", &b)),
            ("Gamma (Japan)", None, rom_xml("Gamma (Japan).gb", &c)),
            ("Delta (World)", None, rom_xml("Delta (World).gb", &d)),
            ("Epsilon (USA)", None, rom_xml("Epsilon (USA).gb", &rom(12, 1024)) + &rom_xml("Epsilon (USA) (Track 2).gb", &b)),
            ("Zeta (USA)", None, "<rom name=\"Zeta (USA).gb\" size=\"1\" status=\"nodump\"/>".to_string()),
        ]);
        write(&dir.join("dats/gb.dat"), gb_dat.as_bytes());
        let nes_dat = dat_xml("Nintendo - Nintendo Entertainment System", "2026", &[("Echo (USA)", None, rom_xml("Echo (USA).nes", &e))]);
        zip(&dir.join("dats/nes.zip"), &[("Nintendo - Nintendo Entertainment System (2026).dat", nes_dat.as_bytes())]);
        Fixture { dir, lib, ix, gb }
    }

    pub(crate) fn run_scan(f: &Fixture, full: bool) -> Value {
        scan::scan(&f.ix, &f.lib, &Options { full, readers: 2, only: None }, &AtomicBool::new(false), &mut |_| {}).unwrap()
    }

    pub(crate) fn import(f: &Fixture, file: &str) -> Vec<Value> {
        crate::dat::import(&f.ix, &f.lib, &f.dir.join("dats").join(file), &Map::new())
            .unwrap()
            .into_iter()
            .map(|i| match i {
                crate::dat::Imported::Loaded(v) => v,
                crate::dat::Imported::NeedsPlatform { name, .. } => panic!("no platform for {name}"),
            })
            .collect()
    }

    pub(crate) fn names(f: &Fixture, dat: i64, tab: &str) -> Vec<String> {
        let l = list(&f.ix, &f.lib, dat, tab, "", 0, 100).unwrap();
        let mut v: Vec<String> = l["items"].as_array().unwrap().iter()
            .map(|i| i["name"].as_str().unwrap().to_string())
            .collect();
        v.sort();
        assert_eq!(l["total"].as_u64().unwrap() as usize, v.len());
        v
    }

    // review H7 and H8: a headered ROM in a ZIP matches with a quick scan, and a member
    // that doesn't read back leaves the archive's others in the audit
    #[test]
    fn zipped_headered_roms_match_and_a_bad_member_hides_no_others() {
        let f = fixture("audit-headered-zip");
        let nes = f.dir.join("Roms/NES");
        let echo = std::fs::read(nes.join("Echo (USA).nes")).unwrap();
        std::fs::remove_file(nes.join("Echo (USA).nes")).unwrap();
        zip(&nes.join("Echo (USA).zip"), &[("Echo (USA).nes", &echo)]);
        run_scan(&f, false);
        let id = import(&f, "nes.zip")[0]["id"].as_i64().unwrap();
        assert_eq!(names(&f, id, "have"), ["Echo (USA)"]);

        // a ZIP of two stored members, the first damaged (its bytes no longer match its CRC)
        let gb_dat = import(&f, "gb.dat")[0]["id"].as_i64().unwrap();
        let both = f.gb.join("Both.zip");
        {
            let mut z = zip::ZipWriter::new(std::fs::File::create(&both).unwrap());
            let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
            z.start_file("Gamma (Japan).gb", stored).unwrap();
            z.write_all(&rom(3, 32768)).unwrap();
            z.start_file("Delta (World).gb", stored).unwrap();
            z.write_all(&rom(4, 65536)).unwrap();
            z.finish().unwrap();
        }
        let mut bytes = std::fs::read(&both).unwrap();
        bytes[200] ^= 0xff;
        std::fs::write(&both, &bytes).unwrap();
        std::fs::remove_file(f.gb.join("Delta (World).zip")).unwrap();
        run_scan(&f, true);
        let kind: String = f.ix.conn().query_row("SELECT kind FROM files WHERE name = 'Both.zip'", [], |r| r.get(0)).unwrap();
        assert_eq!(kind, "zip", "still read as a ZIP");
        assert!(names(&f, gb_dat, "have").contains(&"Delta (World)".to_string()));
        let _ = std::fs::remove_dir_all(&f.dir);
    }

    #[test]
    fn scan_import_and_audit() {
        let f = fixture("audit");
        let s = run_scan(&f, false);
        assert_eq!(s["added"], 7, "{s}");
        assert_eq!(s["hashed"], 7);
        let gb = import(&f, "gb.dat");
        assert_eq!(gb[0]["platform"], "gb");
        let id = gb[0]["id"].as_i64().unwrap();
        assert!(f.lib.root().join(gb[0]["path"].as_str().unwrap()).is_file(), "a copy is kept in the library");
        let nes = import(&f, "nes.zip");
        assert_eq!(nes[0]["platform"], "nes");
        assert_eq!(nes[0]["matched"], 1, "the iNES file matches without its header");

        let sum = summary(f.ix.conn(), id).unwrap();
        // Zeta is a nodump: not counted
        assert_eq!((sum["games"].as_i64(), sum["have"].as_i64(), sum["partial"].as_i64(), sum["missing"].as_i64()), (Some(6), Some(3), Some(1), Some(2)), "{sum}");
        // groups: Alpha (with its clone), Beta, Gamma, Delta, Epsilon
        assert_eq!((sum["groups"].as_i64(), sum["groups_have"].as_i64()), (Some(5), Some(3)));
        assert_eq!(names(&f, id, "have"), ["Alpha (USA)", "Beta (Europe)", "Delta (World)"]);
        assert_eq!(names(&f, id, "partial"), ["Epsilon (USA)"]);
        assert_eq!(names(&f, id, "missing"), ["Alpha (Europe)", "Gamma (Japan)"]);
        // a games list narrowed to a region, in region order, saying its regions
        let by = list_by(&f.ix, &f.lib, id, "have", "", "Europe", "name", 0, 50).unwrap();
        assert_eq!(by["items"].as_array().unwrap().iter().map(|x| x["name"].as_str().unwrap()).collect::<Vec<_>>(), ["Beta (Europe)"]);
        let regions: Vec<&str> = by["regions"].as_array().unwrap().iter().map(|x| x["region"].as_str().unwrap()).collect();
        assert_eq!(regions.len(), 3, "{regions:?}");
        let by = list_by(&f.ix, &f.lib, id, "have", "", "", "region", 0, 50).unwrap();
        assert_eq!(by["items"][0]["name"], "Beta (Europe)");
        let o = overview(&f.ix, &f.lib).unwrap();
        let gbo = o["systems"].as_array().unwrap().iter().find(|x| x["platform"] == "gb").unwrap();
        assert_eq!((gbo["have"].as_i64(), gbo["misnamed"].as_i64(), gbo["platform_name"].as_str()), (Some(3), Some(2), Some("Game Boy")));
        assert_eq!(names(&f, id, "matched"), ["Alpha (USA).gb", "Delta (World).gb"]);
        assert_eq!(names(&f, id, "misnamed"), ["Alpha copy.gb", "beta.gb"]);
        assert_eq!(names(&f, id, "baddump"), ["Gamma (Japan).gb"]);
        assert_eq!(names(&f, id, "unknown"), ["My Hack.gb"]);
        assert_eq!(names(&f, id, "duplicates"), ["Alpha (USA).gb", "Alpha copy.gb"]);
        assert_eq!(sum["by_crc"], 1, "the ZIP member is matched by its CRC");
        let mis = list(&f.ix, &f.lib, id, "misnamed", "beta", 0, 10).unwrap();
        assert_eq!(mis["items"][0]["expected"], "Beta (Europe).gb");
        assert!(mis["items"][0]["path"].as_str().unwrap().ends_with("beta.gb"));
        let have = list(&f.ix, &f.lib, id, "have", "delta", 0, 10).unwrap();
        assert!(have["items"][0]["shown"].as_str().unwrap().ends_with("Delta (World).zip › Delta (World).gb"));

        // a full scan hashes the ZIP's member: its match is by SHA1 now
        let s = run_scan(&f, true);
        assert_eq!(s["hashed"], 1, "{s}");
        assert_eq!(summary(f.ix.conn(), id).unwrap()["by_crc"], 0);
        // nothing changed: nothing is read
        assert_eq!(run_scan(&f, true)["hashed"], 0);

        // a file moved to another folder keeps its hashes; a renamed one is read again; a deleted one goes
        std::fs::create_dir_all(f.gb.join("Hacks")).unwrap();
        std::fs::rename(f.gb.join("My Hack.gb"), f.gb.join("Hacks/My Hack.gb")).unwrap();
        std::fs::rename(f.gb.join("beta.gb"), f.gb.join("Beta (Europe).gb")).unwrap();
        std::fs::remove_file(f.gb.join("Copies/Alpha copy.gb")).unwrap();
        let s = run_scan(&f, false);
        assert_eq!((s["moved"].as_i64(), s["added"].as_i64(), s["removed"].as_i64(), s["hashed"].as_i64()), (Some(1), Some(1), Some(2), Some(1)), "{s}");
        assert_eq!(names(&f, id, "unknown"), ["My Hack.gb"]);
        assert_eq!(names(&f, id, "misnamed"), Vec::<String>::new());
        assert_eq!(names(&f, id, "duplicates"), Vec::<String>::new());

        // the fixdat lists what's missing, and reads back as the same games
        let fx = fixdat(&f.ix, &f.lib, id, None).unwrap();
        assert_eq!(fx["games"], 3);
        let back = crate::dat::parse(&std::fs::read(fx["path"].as_str().unwrap()).unwrap()).unwrap();
        let mut got: Vec<&str> = back.games.iter().map(|g| g.name.as_str()).collect();
        got.sort();
        assert_eq!(got, ["Alpha (Europe)", "Epsilon (USA)", "Gamma (Japan)"]);
        assert_eq!(back.games.iter().find(|g| g.name == "Epsilon (USA)").unwrap().roms.len(), 1, "only the ROM not yet matched");
        assert!(back.name.starts_with("Nintendo - Game Boy (Parent-Clone) (fixdat "));
        assert!(report(&f.ix, id).unwrap().contains("Games: 3 of 6 (1 partly there, 2 missing)"));
        let h = home(&f.ix, &f.lib).unwrap();
        assert_eq!(h["platforms"].as_array().unwrap().len(), 2);
        let csv = save_report(&f.ix, &f.lib, id, "csv").unwrap();
        let text = std::fs::read_to_string(csv["path"].as_str().unwrap()).unwrap();
        assert!(text.contains("missing,Alpha (Europe),,") && text.contains("unknown,My Hack.gb,"), "{text}");
        assert!(save_report(&f.ix, &f.lib, id, "txt").unwrap()["rel"].as_str().unwrap().starts_with("_library/reports/"));
        // another DAT for the same platform, chosen on the Audit page, is kept as the platform's
        let other = dat_xml("Nintendo - Game Boy (TOSEC)", "1", &[("Alpha", None, rom_xml("Alpha.gb", &rom(1, 32768)))]);
        write(&f.dir.join("dats/tosec.dat"), other.as_bytes());
        let mut answers = Map::new();
        answers.insert("Nintendo - Game Boy (TOSEC)".into(), json!("gb"));
        let tosec = match crate::dat::import(&f.ix, &f.lib, &f.dir.join("dats/tosec.dat"), &answers).unwrap().remove(0) {
            crate::dat::Imported::Loaded(v) => v["id"].as_i64().unwrap(),
            _ => panic!(),
        };
        assert_eq!(primary_dat(f.ix.conn(), &f.lib, "gb").unwrap(), Some(id), "No-Intro's by default");
        choose_dat(f.ix.conn(), &f.lib, tosec).unwrap();
        assert_eq!(primary_dat(f.ix.conn(), &f.lib, "gb").unwrap(), Some(tosec));
        let _ = std::fs::remove_dir_all(&f.dir);
    }

    #[test]
    fn dat_versions_and_the_library_copy() {
        let f = fixture("versions");
        run_scan(&f, false);
        let v1 = import(&f, "gb.dat")[0]["id"].as_i64().unwrap();
        // the same file again: already there
        assert_eq!(import(&f, "gb.dat")[0]["already"], true);
        let newer = dat_xml("Nintendo - Game Boy (Parent-Clone)", "20261101-000000", &[("Alpha (USA)", None, rom_xml("Alpha (USA).gb", &rom(1, 32768)))]);
        write(&f.dir.join("dats/gb2.dat"), newer.as_bytes());
        let v2 = import(&f, "gb2.dat")[0]["id"].as_i64().unwrap();
        let current = |id: i64| f.ix.conn().query_row("SELECT current FROM dats WHERE id = ?1", [id], |r| r.get::<_, i64>(0)).unwrap();
        assert_eq!((current(v1), current(v2)), (0, 1), "the newest is current");
        crate::dat::use_version(&f.ix, &f.lib, v1).unwrap();
        assert_eq!((current(v1), current(v2)), (1, 0));
        // a new index is loaded from the library's DATs
        let ix2 = Index::open(&f.dir.join("index2.sqlite"), "lib-x").unwrap();
        crate::dat::sync_library(&ix2, &f.lib).unwrap();
        let n: i64 = ix2.conn().query_row("SELECT COUNT(*) FROM dats WHERE current = 1 AND version = '20261001-000000'", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1, "the chosen version is kept in the library");
        crate::dat::remove(&f.ix, &f.lib, v1).unwrap();
        assert_eq!(current(v2), 1);
        // review M14: an undated copy is older than any dated one
        let undated = dat_xml("Nintendo - Game Boy (Parent-Clone)", "", &[("Alpha (USA)", None, rom_xml("Alpha (USA).gb", &rom(1, 32768)))]);
        write(&f.dir.join("dats/gb0.dat"), undated.as_bytes());
        let v0 = import(&f, "gb0.dat")[0]["id"].as_i64().unwrap();
        assert_eq!((current(v0), current(v2)), (0, 1), "the dated one stays current");
        // a DAT whose platform isn't known is asked about, and the answer remembered
        let odd = dat_xml("Nintendo - Satellaview", "1", &[("BS X (Japan)", None, rom_xml("BS X (Japan).bs", &rom(7, 64)))]);
        write(&f.dir.join("dats/bs.dat"), odd.as_bytes());
        let r = crate::dat::import(&f.ix, &f.lib, &f.dir.join("dats/bs.dat"), &Map::new()).unwrap();
        assert!(matches!(&r[0], crate::dat::Imported::NeedsPlatform { name, .. } if name == "Nintendo - Satellaview"));
        let mut answers = Map::new();
        answers.insert("Nintendo - Satellaview".into(), json!("satellaview"));
        crate::dat::import(&f.ix, &f.lib, &f.dir.join("dats/bs.dat"), &answers).unwrap();
        assert_eq!(f.lib.meta()["dat_platforms"]["Nintendo - Satellaview"], "satellaview");
        let _ = std::fs::remove_dir_all(&f.dir);
    }

    #[test]
    fn a_damaged_zip_and_a_stopped_scan() {
        let f = fixture("damaged");
        // a ZIP cut short: its central directory is gone, so it's read from its local headers
        let good = std::fs::read(f.gb.join("Delta (World).zip")).unwrap();
        write(&f.gb.join("Cut.zip"), &good[..good.len() - 30]);
        let stop = AtomicBool::new(true);
        let s = scan::scan(&f.ix, &f.lib, &Options::default(), &stop, &mut |_| {}).unwrap();
        assert_eq!(s["stopped"], true);
        let s = run_scan(&f, false);
        assert_eq!(s["stopped"], false);
        let cut: (String, i64) = f.ix.conn().query_row(
            "SELECT m.name, m.hashed FROM files z JOIN files m ON m.parent_id = z.id WHERE z.name = 'Cut.zip'", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!(cut, ("Delta (World).gb".to_string(), 2));
        let _ = std::fs::remove_dir_all(&f.dir);
    }

    // review M10 and M13: a folder's new system hint reaches what's scanned there, and a
    // scan that finds nothing new keeps the audit's summaries
    #[test]
    fn a_new_system_hint_reaches_scanned_files_and_a_quiet_scan_keeps_summaries() {
        let f = fixture("hint-quiet");
        run_scan(&f, false);
        let gb_id = import(&f, "gb.dat")[0]["id"].as_i64().unwrap();
        let platform = |name: &str| -> Option<String> {
            f.ix.conn().query_row("SELECT platform FROM files WHERE name = ?1", [name], |r| r.get(0)).unwrap()
        };
        assert_eq!(platform("beta.gb").as_deref(), Some("gb"));
        summary(f.ix.conn(), gb_id).unwrap();
        let cached = || f.ix.conn().query_row("SELECT COUNT(*) FROM index_meta WHERE key LIKE 'summary:%'", [], |r| r.get::<_, i64>(0)).unwrap();
        let (gen, kept) = (generation(f.ix.conn()).unwrap(), cached());
        assert!(kept > 0);
        run_scan(&f, false);
        assert_eq!((generation(f.ix.conn()).unwrap(), cached()), (gen.clone(), kept), "nothing changed");
        // the hint, as the page sets it: the index follows at once
        let id = crate::roots::list(&f.lib)[0].id.clone();
        crate::roots::update(&f.lib, &id, None, Some(Some("nes"))).unwrap();
        assert!(scan::reguess_root(&f.ix, &f.lib, &id).unwrap() > 0);
        assert_eq!(platform("beta.gb").as_deref(), Some("nes"));
        assert_eq!(platform("Delta (World).gb").as_deref(), Some("nes"), "a ZIP's member too");
        assert_ne!(generation(f.ix.conn()).unwrap(), gen);
        // and as another computer would (only library.json changed): the next scan follows
        crate::roots::update(&f.lib, &id, None, Some(None)).unwrap();
        run_scan(&f, false);
        assert_eq!(platform("beta.gb").as_deref(), Some("gb"));
        assert_eq!(platform("Echo (USA).nes").as_deref(), Some("nes"));
        let _ = std::fs::remove_dir_all(&f.dir);
    }

    // review M12: a cue sheet scanned before its tracks is matched once they're there
    #[test]
    fn a_cue_scanned_before_its_tracks_is_matched_with_them() {
        let f = fixture("cue-first");
        let src = crate::image::tests::sources("cue-first-src");
        let read = |n: &str| std::fs::read(src.join(n)).unwrap();
        let (cue, t1, t2) = (read("Disc Game (USA).cue"), read("Disc Game (USA) (Track 1).bin"), read("Disc Game (USA) (Track 2).bin"));
        let mine = f.dir.join("Roms/PSX/Mine");
        write(&mine.join("mine.cue"), b"FILE \"t1.bin\" BINARY\n  TRACK 01 MODE1/2352\n    INDEX 01 00:00:00\nFILE \"t2.bin\" BINARY\n  TRACK 02 AUDIO\n    INDEX 00 00:00:00\n    INDEX 01 00:00:10\n");
        let psx_dat = dat_xml("Sony - PlayStation", "2026.10.07", &[(
            "Disc Game (USA)", None,
            rom_xml("Disc Game (USA).cue", &cue) + &rom_xml("Disc Game (USA) (Track 1).bin", &t1) + &rom_xml("Disc Game (USA) (Track 2).bin", &t2),
        )]);
        write(&f.dir.join("dats/psx.dat"), psx_dat.as_bytes());
        let psx_id = import(&f, "psx.dat")[0]["id"].as_i64().unwrap();
        run_scan(&f, false);
        let method = || -> Option<String> {
            f.ix.conn().query_row("SELECT m.method FROM matches m JOIN files f ON f.id = m.file_id WHERE f.name = 'mine.cue'", [], |r| r.get(0)).optional().unwrap()
        };
        assert_eq!(method(), None, "its tracks aren't there yet");
        write(&mine.join("t1.bin"), &t1);
        write(&mine.join("t2.bin"), &t2);
        run_scan(&f, false);
        assert_eq!(method().as_deref(), Some("cue"));
        assert_eq!(summary(f.ix.conn(), psx_id).unwrap()["have"], 1);
        let _ = std::fs::remove_dir_all(&f.dir);
    }

    #[test]
    fn discs_in_the_audit() {
        let f = fixture("discs");
        let src = crate::image::tests::sources("discs-src");
        let read = |n: &str| std::fs::read(src.join(n)).unwrap();
        let (cue, t1, t2, iso) = (read("Disc Game (USA).cue"), read("Disc Game (USA) (Track 1).bin"), read("Disc Game (USA) (Track 2).bin"), read("Dvd Game (Europe).iso"));
        let psx = f.dir.join("Roms/PSX");
        // the game as a CHD, and again as cue/bin renamed (its cue no longer Redump's)
        std::fs::create_dir_all(&psx).unwrap();
        std::fs::copy(crate::image::tests::fixtures().join("Disc Game (USA).chd"), psx.join("Disc Game (USA).chd")).unwrap();
        write(&psx.join("Mine/t1.bin"), &t1);
        write(&psx.join("Mine/t2.bin"), &t2);
        write(&psx.join("Mine/mine.cue"), b"FILE \"t1.bin\" BINARY\n  TRACK 01 MODE1/2352\n    INDEX 01 00:00:00\nFILE \"t2.bin\" BINARY\n  TRACK 02 AUDIO\n    INDEX 00 00:00:00\n    INDEX 01 00:00:10\n");
        std::fs::create_dir_all(f.dir.join("Roms/PSP")).unwrap();
        std::fs::copy(crate::image::tests::fixtures().join("Dvd Game (Europe).cso"), f.dir.join("Roms/PSP/Dvd Game (Europe).cso")).unwrap();
        let cube = rom(77, 8192);
        let mut rvz = b"RVZ\x01".to_vec();
        rvz.resize(36, 0);
        rvz.extend_from_slice(&(cube.len() as u64).to_be_bytes());
        rvz.resize(100, 7);
        write(&f.dir.join("Roms/GameCube/Cube (USA).rvz"), &rvz);
        let psx_dat = dat_xml("Sony - PlayStation", "2026.10.07", &[(
            "Disc Game (USA)", None,
            rom_xml("Disc Game (USA).cue", &cue) + &rom_xml("Disc Game (USA) (Track 1).bin", &t1) + &rom_xml("Disc Game (USA) (Track 2).bin", &t2),
        )]);
        write(&f.dir.join("dats/psx.dat"), psx_dat.as_bytes());
        write(&f.dir.join("dats/psp.dat"), dat_xml("Sony - PlayStation Portable", "2026", &[("Dvd Game (Europe)", None, rom_xml("Dvd Game (Europe).iso", &iso))]).as_bytes());
        write(&f.dir.join("dats/gc.dat"), dat_xml("Nintendo - GameCube", "2026", &[("Cube (USA)", None, rom_xml("Cube (USA).iso", &cube))]).as_bytes());
        let _turn = crate::tools::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::tools::configure(&Default::default());
        run_scan(&f, false);
        let psx_id = import(&f, "psx.dat")[0]["id"].as_i64().unwrap();
        let psp_id = import(&f, "psp.dat")[0]["id"].as_i64().unwrap();
        let gc_id = import(&f, "gc.dat")[0]["id"].as_i64().unwrap();
        // the CHD's tracks are the game's bins; its cue isn't needed
        let s = summary(f.ix.conn(), psx_id).unwrap();
        assert_eq!((s["have"].as_i64(), s["unknown"].as_i64()), (Some(1), Some(0)), "{s}");
        assert_eq!(names(&f, psx_id, "matched"), ["Track 01.bin", "Track 02.bin"]);
        // the renamed set: its cue matched by the files it names (cue differs), all three misnamed
        assert_eq!(names(&f, psx_id, "misnamed"), ["mine.cue", "t1.bin", "t2.bin"]);
        let method: String = f.ix.conn().query_row("SELECT m.method FROM matches m JOIN files f ON f.id = m.file_id WHERE f.name = 'mine.cue'", [], |r| r.get(0)).unwrap();
        assert_eq!(method, "cue");
        assert_eq!(s["duplicates"].as_i64(), Some(4), "{s}");
        // the CSO holds the ISO
        let s = summary(f.ix.conn(), psp_id).unwrap();
        assert_eq!((s["have"].as_i64(), s["matched"].as_i64()), (Some(1), Some(1)), "{s}");
        // the RVZ waits for dolphin-tool
        let s = summary(f.ix.conn(), gc_id).unwrap();
        assert_eq!((s["have"].as_i64(), s["needs_tool"].as_i64()), (Some(0), Some(1)), "{s}");
        assert_eq!(home(&f.ix, &f.lib).unwrap()["needs_tool"], 1);
        #[cfg(unix)]
        {
            // a stand-in for dolphin-tool, printing what verify does
            use std::os::unix::fs::PermissionsExt;
            let h = hash_reader(&mut &cube[..], cube.len() as u64, "x", &AtomicBool::new(false)).unwrap();
            let tool = f.dir.join("tools/dolphin-tool");
            write(&tool, format!("#!/bin/sh\necho 'Problems Found: No'\necho 'CRC32: {}'\necho 'SHA1: {}'\n", h.crc32, h.sha1.unwrap()).as_bytes());
            std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
            crate::tools::configure(&[("dolphin-tool".to_string(), tool)].into());
            run_scan(&f, false);
            crate::tools::configure(&Default::default());
            let s = summary(f.ix.conn(), gc_id).unwrap();
            assert_eq!((s["have"].as_i64(), s["matched"].as_i64(), s["needs_tool"].as_i64()), (Some(1), Some(1), Some(0)), "{s}");
        }
        let _ = std::fs::remove_dir_all(&f.dir);
        let _ = std::fs::remove_dir_all(&src);
    }

    #[test]
    fn a_7z_in_the_audit_and_a_scan_of_only_what_changed() {
        let f = fixture("only");
        // Delta as a 7z instead of a ZIP: matched, and named like its game
        let d = rom(4, 65536);
        std::fs::remove_file(f.gb.join("Delta (World).zip")).unwrap();
        crate::archive::tests::write_7z(&f.gb.join("Delta (World).7z"), &[("Delta (World).gb", &d)]);
        run_scan(&f, false);
        let id = import(&f, "gb.dat")[0]["id"].as_i64().unwrap();
        assert_eq!(names(&f, id, "matched"), ["Alpha (USA).gb", "Delta (World).gb"]);
        assert!(!names(&f, id, "misnamed").contains(&"Delta (World).gb".to_string()));

        // changes in the Game Boy folder, and one in the NES folder that isn't reported
        std::fs::rename(f.gb.join("beta.gb"), f.gb.join("Beta (Europe).gb")).unwrap();
        std::fs::remove_file(f.gb.join("My Hack.gb")).unwrap();
        write(&f.gb.join("New/Gamma (Japan).gb"), &rom(3, 32768));
        write(&f.gb.join(".gamerepo-set-aside/x/old.gb"), &rom(7, 100));
        std::fs::remove_file(f.dir.join("Roms/NES/Echo (USA).nes")).unwrap();
        let only = vec![f.gb.join("beta.gb"), f.gb.join("Beta (Europe).gb"), f.gb.join("My Hack.gb"), f.gb.join("New"), f.gb.join("New/Gamma (Japan).gb"), f.dir.join("elsewhere/x.gb")];
        let s = scan::scan(&f.ix, &f.lib, &Options { only: Some(only), ..Options::default() }, &AtomicBool::new(false), &mut |_| {}).unwrap();
        // (a file renamed is read again: only one moved under its own name keeps its hashes)
        assert_eq!((s["added"].as_u64(), s["removed"].as_u64(), s["places"].as_u64()), (Some(2), Some(2), Some(4)), "{s}");
        assert_eq!(names(&f, id, "matched"), ["Alpha (USA).gb", "Beta (Europe).gb", "Delta (World).gb", "Gamma (Japan).gb"]);
        assert_eq!(names(&f, id, "unknown"), Vec::<String>::new());
        let nes: i64 = f.ix.conn().query_row("SELECT COUNT(*) FROM files WHERE name = 'Echo (USA).nes'", [], |r| r.get(0)).unwrap();
        assert_eq!(nes, 1, "the NES folder wasn't looked at");
        // a full scan sees the rest
        let s = run_scan(&f, false);
        assert_eq!((s["removed"].as_u64(), s["added"].as_u64()), (Some(1), Some(0)), "{s}");
        let _ = std::fs::remove_dir_all(&f.dir);
    }

    // The performance targets of docs/PLAN.md, on generated data. Run with
    // `cargo test --release -- --ignored timing` (CI does, on Windows).

    #[test]
    #[ignore]
    fn timing_rescan_of_100k_unchanged_files() {
        let dir = temp_dir("timing-scan");
        let lib = Library::open(dir.join("Lib")).unwrap();
        let ix = Index::open(&dir.join("index.sqlite"), "lib-x").unwrap();
        for d in 0..200u32 {
            let folder = dir.join(format!("Roms/Game Boy/Folder {d}"));
            std::fs::create_dir_all(&folder).unwrap();
            for i in 0..500 {
                std::fs::write(folder.join(format!("Game {d}-{i} (USA).gb")), (d * 1000 + i as u32).to_le_bytes()).unwrap();
            }
        }
        crate::roots::add(&lib, &dir.join("Roms"), None).unwrap();
        let opts = Options { full: false, readers: 4, only: None };
        let first = scan::scan(&ix, &lib, &opts, &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(first["hashed"], 100_000);
        // the better of two rescans, as a shared runner's timings swing by a third from run to run
        let mut took = std::time::Duration::MAX;
        for _ in 0..2 {
            let t = std::time::Instant::now();
            let again = scan::scan(&ix, &lib, &opts, &AtomicBool::new(false), &mut |_| {}).unwrap();
            took = took.min(t.elapsed());
            assert_eq!(again["hashed"], 0);
        }
        eprintln!("first scan {}s; rescan of 100,000 unchanged files {took:?}", first["seconds"]);
        assert!(took.as_secs_f64() < 5.0, "rescan took {took:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore]
    fn timing_import_and_audit_of_a_50k_rom_dat() {
        let dir = temp_dir("timing-dat");
        let lib = Library::open(dir.join("Lib")).unwrap();
        let ix = Index::open(&dir.join("index.sqlite"), "lib-x").unwrap();
        let mut s = String::from("<?xml version=\"1.0\"?>\n<datafile><header><name>Nintendo - Game Boy</name><version>1</version></header>\n");
        for i in 0..50_000u32 {
            let clone = if i % 3 == 0 { String::new() } else { format!(" cloneof=\"Game {} (USA)\"", i - i % 3) };
            s.push_str(&format!(
                "<game name=\"Game {i} (USA)\"{clone}><description>Game {i}</description><rom name=\"Game {i} (USA).gb\" size=\"{}\" crc=\"{:08x}\" md5=\"{:032x}\" sha1=\"{:040x}\"/></game>\n",
                32768 + i, i.wrapping_mul(2_654_435_761), i, i
            ));
        }
        s.push_str("</datafile>\n");
        std::fs::write(dir.join("big.dat"), &s).unwrap();
        let t = std::time::Instant::now();
        let r = crate::dat::import(&ix, &lib, &dir.join("big.dat"), &Map::new()).unwrap();
        let took = t.elapsed();
        let crate::dat::Imported::Loaded(v) = &r[0] else { panic!() };
        assert_eq!(v["roms"], 50_000);
        let t = std::time::Instant::now();
        let id = v["id"].as_i64().unwrap();
        let sum = summary(ix.conn(), id).unwrap();
        let page = list(&ix, &lib, id, "missing", "Game 4", 2000, 100).unwrap();
        let audit = t.elapsed();
        eprintln!("import of 50,000 ROMs {took:?}; audit {audit:?}");
        assert_eq!(sum["missing"], 50_000);
        assert_eq!(page["items"].as_array().unwrap().len(), 100);
        assert!(took.as_secs_f64() < 5.0, "import took {took:?}");
        assert!(audit.as_secs_f64() < 1.0, "audit took {audit:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
