//! Making a DAT from a set: the files scanned in a folder with games (or a folder
//! inside one) written as a Logiqx DAT, one game per file or ZIP. ZIP members the
//! quick scan only knows by CRC are read now, so every ROM has its SHA1.

use crate::audit::xml;
use crate::hash::Hashes;
use crate::index::Index;
use crate::library::{now, Library, APP_DIR};
use crate::roots;
use anyhow::{bail, Result};
use rusqlite::params;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

struct Entry {
    game: String,
    roms: Vec<(String, Hashes)>,
}

fn stem(name: &str) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 => name[..i].to_string(),
        _ => name.to_string(),
    }
}

fn rom_xml(name: &str, h: &Hashes) -> String {
    let mut s = format!("\t\t<rom name=\"{}\" size=\"{}\" crc=\"{}\"", xml(name), h.size, h.crc32);
    if let Some(m) = &h.md5 {
        s.push_str(&format!(" md5=\"{m}\""));
    }
    if let Some(x) = &h.sha1 {
        s.push_str(&format!(" sha1=\"{x}\""));
    }
    s.push_str("/>\n");
    s
}

/// Make a DAT named `name` of the files under `folder` (relative to the folder with
/// games `root_id`; empty for all of it). With `platform`, it's also imported to audit against.
pub fn make(
    ix: &Index,
    lib: &Library,
    root_id: &str,
    folder: &str,
    name: &str,
    platform: Option<&str>,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Value),
) -> Result<Value> {
    lib.writable()?;
    let name = name.trim();
    if name.is_empty() {
        bail!("A DAT needs a name.");
    }
    let Some(root) = roots::list(lib).into_iter().find(|r| r.id == root_id) else {
        bail!("There's no folder with games {root_id}.");
    };
    let folder = folder.trim_matches('/');
    let conn = ix.conn();
    let mut q = conn.prepare(
        "SELECT f.id, f.rel_path, f.name, f.kind, f.hashed, f.platform, b.size, b.crc32, b.md5, b.sha1
         FROM files f LEFT JOIN blobs b ON b.id = f.blob_id
         WHERE f.root_id = ?1 AND f.parent_id IS NULL AND (?2 = '' OR substr(f.rel_path, 1, length(?2) + 1) = ?2 || '/')
         ORDER BY f.rel_path",
    )?;
    struct Row {
        id: i64,
        rel: String,
        name: String,
        kind: String,
        hashed: i64,
        platform: Option<String>,
        hashes: Option<Hashes>,
    }
    let rows: Vec<Row> = q
        .query_map(params![root.id, folder], |r| {
            let size: Option<i64> = r.get(6)?;
            Ok(Row {
                id: r.get(0)?, rel: r.get(1)?, name: r.get(2)?, kind: r.get(3)?, hashed: r.get(4)?, platform: r.get(5)?,
                hashes: size.map(|s| -> rusqlite::Result<Hashes> {
                    Ok(Hashes { size: s as u64, crc32: r.get(7)?, md5: r.get(8)?, sha1: r.get(9)?, ..Default::default() })
                }).transpose()?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    if rows.is_empty() {
        bail!("Nothing has been scanned in {}{}{folder}: scan it first.", root.dir.display(), if folder.is_empty() { "" } else { "/" });
    }
    let mut entries = vec![];
    let mut skipped = 0;
    let mut platforms: HashMap<String, usize> = HashMap::new();
    let mut members = conn.prepare(
        "SELECT f.name, f.hashed, f.platform, b.size, b.crc32, b.md5, b.sha1 FROM files f JOIN blobs b ON b.id = f.blob_id WHERE f.parent_id = ?1 ORDER BY f.name",
    )?;
    for (i, row) in rows.iter().enumerate() {
        progress(json!({ "item": i + 1, "items": rows.len(), "name": row.rel }));
        if row.hashed == 0 {
            skipped += 1;
            continue;
        }
        if let Some(p) = &row.platform {
            *platforms.entry(p.clone()).or_default() += 1;
        }
        if row.kind == "zip" {
            let ms: Vec<(String, i64, Option<String>, Hashes)> = members
                .query_map([row.id], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, Hashes {
                        size: r.get::<_, i64>(3)? as u64, crc32: r.get(4)?, md5: r.get(5)?, sha1: r.get(6)?, ..Default::default()
                    }))
                })?
                .collect::<rusqlite::Result<_>>()?;
            for (_, _, p, _) in &ms {
                if let Some(p) = p {
                    *platforms.entry(p.clone()).or_default() += 1;
                }
            }
            let roms = if ms.iter().any(|m| m.1 < 2) {
                // only known by the CRCs the ZIP records: read it now
                let path = row.rel.split('/').fold(root.dir.clone(), |p, part| p.join(part));
                match crate::scan::zip_members(&path, cancel) {
                    Ok(v) => v,
                    Err(_) => {
                        skipped += 1;
                        continue;
                    }
                }
            } else {
                ms.into_iter().map(|(n, _, _, h)| (n, h)).collect()
            };
            entries.push(Entry { game: stem(&row.name), roms });
        } else if let Some(h) = &row.hashes {
            entries.push(Entry { game: stem(&row.name), roms: vec![(row.name.clone(), h.clone())] });
        }
    }
    let date = now()[..10].to_string();
    let source = if folder.is_empty() { root.dir.display().to_string() } else { root.dir.join(folder).display().to_string() };
    let mut out = format!(
        "<?xml version=\"1.0\"?>\n<datafile>\n\t<header>\n\t\t<name>{}</name>\n\t\t<description>{}</description>\n\t\t<version>{date}</version>\n\t\t<author>Game Repo</author>\n\t\t<comment>{}</comment>\n\t</header>\n",
        xml(name), xml(name), xml(&format!("Made from {source}"))
    );
    let mut roms = 0;
    for e in &entries {
        out.push_str(&format!("\t<game name=\"{}\">\n\t\t<description>{}</description>\n", xml(&e.game), xml(&e.game)));
        for (n, h) in &e.roms {
            out.push_str(&rom_xml(n, h));
            roms += 1;
        }
        out.push_str("\t</game>\n");
    }
    out.push_str("</datafile>\n");
    let safe: String = name.chars().map(|c| if "<>:\"/\\|?*".contains(c) || c.is_control() { '_' } else { c }).collect();
    let path: PathBuf = lib.root().join(APP_DIR).join("made-dats").join(format!("{safe} ({date}).dat"));
    crate::config::write_atomic(&path, out.as_bytes())?;
    let suggested = platforms.into_iter().max_by_key(|(_, n)| *n).map(|(p, _)| p);
    let mut imported = Value::Null;
    if let Some(p) = platform.filter(|p| !p.is_empty()) {
        let mut answers = Map::new();
        answers.insert(name.to_string(), json!(p));
        if let Some(crate::dat::Imported::Loaded(v)) = crate::dat::import(ix, lib, &path, &answers)?.into_iter().next() {
            imported = v;
        }
    }
    Ok(json!({
        "path": path.display().to_string(), "rel": lib.relative(&path), "name": name, "games": entries.len(), "roms": roms,
        "skipped": skipped, "platform": suggested, "imported": imported,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::tests::rom;
    use crate::library::tests::temp_dir;
    use std::io::Write;

    #[test]
    fn a_dat_made_from_a_set_audits_it_as_complete() {
        let dir = temp_dir("makedat");
        let lib = Library::open(dir.join("Lib")).unwrap();
        let ix = Index::open(&dir.join("index.sqlite"), "lib-x").unwrap();
        let set = dir.join("Roms/GBA");
        std::fs::create_dir_all(set.join("Hacks")).unwrap();
        std::fs::write(set.join("One (USA).gba"), rom(1, 4096)).unwrap();
        std::fs::write(set.join("Hacks/Two (Hack).gba"), rom(2, 4096)).unwrap();
        let mut z = zip::ZipWriter::new(std::fs::File::create(set.join("Three (Europe).zip")).unwrap());
        z.start_file("Three (Europe).gba", zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(&rom(3, 8192)).unwrap();
        z.finish().unwrap();
        let root = roots::add(&lib, &dir.join("Roms"), None).unwrap();
        crate::scan::scan(&ix, &lib, &crate::scan::Options::default(), &AtomicBool::new(false), &mut |_| {}).unwrap();
        let made = make(&ix, &lib, &root.id, "GBA", "My GBA set", Some("gba"), &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!((made["games"].as_i64(), made["roms"].as_i64(), made["skipped"].as_i64()), (Some(3), Some(3), Some(0)), "{made}");
        assert_eq!(made["platform"], "gba");
        let dat = crate::dat::parse(&std::fs::read(made["path"].as_str().unwrap()).unwrap()).unwrap();
        assert!(dat.games.iter().all(|g| g.roms.iter().all(|r| r.sha1.is_some())), "ZIP members are read for their SHA1");
        let id = made["imported"]["id"].as_i64().unwrap();
        let s = crate::audit::summary(ix.conn(), id).unwrap();
        assert_eq!((s["have"].as_i64(), s["games"].as_i64(), s["misnamed"].as_i64()), (Some(3), Some(3), Some(0)), "{s}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
