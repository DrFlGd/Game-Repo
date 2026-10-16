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
use std::collections::HashMap;
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

/// Something matched changed: summaries worked out before are stale.
pub fn changed(conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM index_meta WHERE key LIKE 'summary:%'", [])?;
    Ok(())
}

/// Match the files hashed since they were last matched. Returns how many matches were made.
pub fn match_files(conn: &Connection) -> Result<usize> {
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM matches WHERE file_id IN (SELECT id FROM files WHERE matched = 0)", [])?;
    let scope = "f.matched = 0";
    let n = tx.execute(&match_sql("", scope), [])? + tx.execute(&match_sql("hl_", &format!("{scope} AND b.hl_crc32 IS NOT NULL")), [])?;
    let touched = tx.execute("UPDATE files SET matched = 1 WHERE matched = 0 AND (blob_id IS NOT NULL OR kind IN ('zip', '7z', 'rar'))", [])?;
    if touched > 0 {
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
    changed(&tx)?;
    tx.commit()?;
    Ok(n)
}

// ------------------------------------------------------------ the audit

/// Each game of the DAT with how many of its ROMs (nodumps aside) are matched.
const GAMES: &str = "
  rs AS (SELECT r.game_id, r.id, EXISTS (SELECT 1 FROM matches m WHERE m.rom_id = r.id) AS have
         FROM dat_roms r WHERE r.dat_id = ?1 AND (r.status IS NULL OR r.status != 'nodump')),
  gs AS (SELECT g.id, g.name, g.title, g.regions, g.flags, g.parent_id, COUNT(rs.id) AS roms, SUM(rs.have) AS have,
                CASE WHEN SUM(rs.have) = COUNT(rs.id) THEN 'have' WHEN SUM(rs.have) > 0 THEN 'partial' ELSE 'missing' END AS status
         FROM dat_games g JOIN rs ON rs.game_id = g.id WHERE g.dat_id = ?1 GROUP BY g.id)";

/// Each file matched with one of the DAT's ROMs, whether it's named as the DAT says
/// (a ZIP member: the ZIP named for the game, the member for the ROM), and the ROM it's
/// listed under (one it's named for, if any).
const MATCHED: &str = "
  fm AS (SELECT f.id AS file_id,
                MAX(CASE WHEN f.name = r.name AND (f.parent_id IS NULL OR p.name = g.name || '.' || p.kind) THEN 1 ELSE 0 END) AS named,
                COALESCE(MIN(CASE WHEN f.name = r.name AND (f.parent_id IS NULL OR p.name = g.name || '.' || p.kind) THEN r.id END), MIN(r.id)) AS rom_id
         FROM matches m JOIN dat_roms r ON r.id = m.rom_id AND r.dat_id = ?1 JOIN dat_games g ON g.id = r.game_id
         JOIN files f ON f.id = m.file_id LEFT JOIN files p ON p.id = f.parent_id
         GROUP BY f.id)";

/// Files of the DAT's platform with no match in it (nor in another platform's DAT):
/// *bad dumps* when named as one of its ROMs, else *unknown*.
const UNMATCHED: &str = "
  um AS (SELECT f.id AS file_id, EXISTS (SELECT 1 FROM dat_roms r WHERE r.dat_id = ?1 AND r.name = f.name) AS bad
         FROM files f
         WHERE f.platform = ?2 AND f.kind NOT IN ('zip', '7z', 'rar') AND f.hashed > 0
           AND NOT EXISTS (SELECT 1 FROM matches m JOIN dat_roms r ON r.id = m.rom_id
                           WHERE m.file_id = f.id AND (r.dat_id = ?1 OR r.dat_id IN (SELECT id FROM dats WHERE platform != ?2))))";

/// ROMs of the DAT matched by more than one file.
const DUPLICATES: &str = "
  dr AS (SELECT m.rom_id FROM matches m JOIN dat_roms r ON r.id = m.rom_id AND r.dat_id = ?1 GROUP BY m.rom_id HAVING COUNT(*) > 1)";

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
/// No-Intro DAT, else its first current DAT by name.
pub fn primary_dat(conn: &Connection, lib: &Library, platform: &str) -> Result<Option<i64>> {
    let chosen = lib.meta()["audit_dats"][platform].as_str().map(String::from);
    let preferred = platforms::by_id(platform).map(|p| p.dat_names[0].to_ascii_lowercase()).unwrap_or_default();
    let mut q = conn.prepare("SELECT id, name FROM dats WHERE platform = ?1 AND current = 1 ORDER BY name")?;
    let rows: Vec<(i64, String)> = q.query_map([platform], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    Ok(rows
        .iter()
        .find(|(_, n)| Some(n) == chosen.as_ref())
        .or_else(|| rows.iter().find(|(_, n)| platforms::dat_base_name(n).to_ascii_lowercase() == preferred))
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
        &format!("WITH {MATCHED} SELECT COALESCE(SUM(named), 0), COALESCE(SUM(1 - named), 0) FROM fm"),
        [dat_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let (bad, unknown): (i64, i64) = conn.query_row(
        &format!("WITH {UNMATCHED} SELECT COALESCE(SUM(bad), 0), COALESCE(SUM(1 - bad), 0) FROM um"),
        params![dat_id, d.platform],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let duplicates: i64 = conn.query_row(
        &format!("WITH {DUPLICATES} SELECT COUNT(*) FROM matches m WHERE m.rom_id IN (SELECT rom_id FROM dr)"),
        [dat_id],
        |r| r.get(0),
    )?;
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
    });
    conn.execute(
        "INSERT INTO index_meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, v.to_string()],
    )?;
    Ok(v)
}

/// The lists the Audit page shows.
pub const TABS: &[&str] = &["have", "partial", "missing", "matched", "misnamed", "baddump", "unknown", "duplicates"];

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
    let conn = ix.conn();
    let d = dat_ref(conn, dat_id)?;
    let limit = limit.clamp(1, 1000);
    let pat = like(search);
    let dirs: HashMap<String, PathBuf> = roots::list(lib).into_iter().map(|r| (r.id, r.dir)).collect();
    match tab {
        "have" | "partial" | "missing" => {
            let total: i64 = conn.query_row(
                &format!("WITH {GAMES} SELECT COUNT(*) FROM gs WHERE status = ?2 AND name LIKE ?3 ESCAPE '\\'"),
                params![dat_id, tab, pat],
                |r| r.get(0),
            )?;
            let mut q = conn.prepare(&format!(
                "WITH {GAMES}
                 SELECT gs.id, gs.name, gs.title, gs.regions, gs.flags, gs.roms, gs.have, pg.name,
                        (SELECT f.root_id || char(9) || f.rel_path || char(9) || COALESCE(p.name, '') || char(9) || f.name
                         FROM dat_roms r JOIN matches m ON m.rom_id = r.id JOIN files f ON f.id = m.file_id LEFT JOIN files p ON p.id = f.parent_id
                         WHERE r.game_id = gs.id LIMIT 1)
                 FROM gs LEFT JOIN dat_games pg ON pg.id = gs.parent_id
                 WHERE gs.status = ?2 AND gs.name LIKE ?3 ESCAPE '\\' ORDER BY gs.name LIMIT ?4 OFFSET ?5"
            ))?;
            let items: Vec<Value> = q
                .query_map(params![dat_id, tab, pat, limit, offset], |r| {
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
                        "path": path, "shown": shown,
                    }))
                })?
                .collect::<rusqlite::Result<_>>()?;
            Ok(json!({ "total": total, "items": items }))
        }
        "matched" | "misnamed" | "duplicates" => {
            let (with, filter, from) = match tab {
                "matched" => (MATCHED, "fm.named = 1", "fm JOIN files f ON f.id = fm.file_id JOIN dat_roms r ON r.id = fm.rom_id"),
                "misnamed" => (MATCHED, "fm.named = 0", "fm JOIN files f ON f.id = fm.file_id JOIN dat_roms r ON r.id = fm.rom_id"),
                _ => (DUPLICATES, "1", "dr JOIN matches mm ON mm.rom_id = dr.rom_id JOIN files f ON f.id = mm.file_id JOIN dat_roms r ON r.id = dr.rom_id"),
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
                        (SELECT method FROM matches x WHERE x.file_id = f.id AND x.rom_id = r.id), f.missing_since
                 FROM {from} JOIN dat_games g ON g.id = r.game_id LEFT JOIN files p ON p.id = f.parent_id
                 WHERE {filter} AND {search_sql} ORDER BY r.name, f.rel_path LIMIT ?3 OFFSET ?4"
            ))?;
            let items: Vec<Value> = q
                .query_map(params![dat_id, pat, limit, offset], |r| {
                    let (root, rel, name, member): (String, String, String, bool) = (r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?);
                    let (rom, game): (String, String) = (r.get(6)?, r.get(7)?);
                    let (path, shown) = locate(&dirs, &root, &rel, member.then_some(name.as_str()));
                    let expected = if member { format!("{game}.zip › {rom}") } else { rom.clone() };
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
         WHERE gs.status != 'have' AND NOT EXISTS (SELECT 1 FROM matches m WHERE m.rom_id = r.id)
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

/// What Home shows: a bar per platform with a DAT, the last scan, and what needs attention.
pub fn home(ix: &Index, lib: &Library) -> Result<Value> {
    let conn = ix.conn();
    let mut q = conn.prepare("SELECT DISTINCT platform FROM dats WHERE current = 1 ORDER BY platform")?;
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
        "SELECT COALESCE(platform, ''), COUNT(*) FROM files f WHERE kind NOT IN ('zip', '7z', 'rar')
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
    let files: i64 = conn.query_row("SELECT COUNT(*) FROM files WHERE kind NOT IN ('zip', '7z', 'rar')", [], |r| r.get(0))?;
    let unhashed: i64 = conn.query_row("SELECT COUNT(*) FROM files WHERE hashed = 0", [], |r| r.get(0))?;
    let dats: i64 = conn.query_row("SELECT COUNT(*) FROM dats", [], |r| r.get(0))?;
    Ok(json!({
        "platforms": bars, "loose": loose, "roots": roots, "files": files, "unhashed": unhashed, "dats": dats,
        "last_scan": crate::scan::last(ix)?,
    }))
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
