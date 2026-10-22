//! Convert and Extract (docs/PLAN.md, "2b", Convert and Extract): a game, a selection
//! or a whole platform changed to another format (CHD for CD and DVD systems, RVZ for
//! GameCube and Wii, CSO or ZSO for PSP, ZIP for cartridges), or back to its raw
//! files. A change is a plan like Organise's (organise.rs), run with its journal and
//! Undo: each result is written beside where it goes, read back as the audit would
//! and compared with the files it came from, and only then given its name; with
//! *Convert in place* the originals are set aside after that. Cartridge conversions
//! (to and from ZIP) are Organise's own, in copy or rename mode.

use crate::archive;
use crate::audit;
use crate::cue;
use crate::hash;
use crate::image;
use crate::index::Index;
use crate::library::{now, Library};
use crate::organise::{self, Cand, GameRow, Op, Plan, RomRow, Src, Written};
use crate::platforms::{self, Media};
use crate::roots::{self, Root};
use anyhow::{anyhow, bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

/// What the Convert dialog chose.
#[derive(Clone, Debug)]
pub struct Options {
    pub dat_id: i64,
    /// chd | cso | zso | rvz | zip, or extract
    pub to: String,
    /// Only these games (by name); all the platform's when None
    pub games: Option<Vec<String>>,
    /// The folder results go to; None converts in place (originals set aside)
    pub out: Option<PathBuf>,
}

// ------------------------------------------------------------ preferred formats (D12)

/// The format a platform's games are kept in: the one chosen in Settings › Preferred
/// formats (library.json `formats`), else the first the platform can have.
pub fn preferred(lib: &Library, platform: &str) -> Option<String> {
    let can = platforms::formats(platform);
    lib.meta()["formats"][platform]
        .as_str()
        .filter(|f| can.contains(f))
        .map(String::from)
        .or_else(|| can.first().map(|f| f.to_string()))
}

pub fn set_preferred(lib: &Library, platform: &str, format: &str) -> Result<()> {
    if !platforms::formats(platform).contains(&format) {
        bail!("{} games can't be kept as {format}.", platforms::name_of(platform));
    }
    let mut all = lib.meta()["formats"].as_object().cloned().unwrap_or_default();
    all.insert(platform.to_string(), json!(format));
    lib.update_meta(json!({ "formats": all }))?;
    Ok(())
}

/// Each platform that can be converted, with its formats and the one preferred.
pub fn formats(lib: &Library) -> Value {
    json!(platforms::all()
        .filter(|p| !platforms::formats(p.id).is_empty())
        .map(|p| json!({ "platform": p.id, "name": p.name, "formats": platforms::formats(p.id), "preferred": preferred(lib, p.id) }))
        .collect::<Vec<_>>())
}

// ------------------------------------------------------------ planning

/// A file of the index as a source: a file in a folder with games, or a member of an archive or image there.
fn src_of(conn: &Connection, roots: &[Root], file_id: i64) -> Result<Option<Src>> {
    type Row = (String, String, Option<i64>, i64, Option<String>, Option<String>);
    let row: Option<Row> = conn
        .query_row(
            "SELECT f.root_id, f.rel_path, f.parent_id, f.size, b.crc32, b.sha1 FROM files f LEFT JOIN blobs b ON b.id = f.blob_id WHERE f.id = ?1",
            [file_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )
        .optional()?;
    let Some((root, rel, parent, size, crc32, sha1)) = row else { return Ok(None) };
    let Some(crc32) = crc32 else { return Ok(None) };
    let (file_id, rel, member) = match parent {
        Some(pid) => {
            let prel: String = conn.query_row("SELECT rel_path FROM files WHERE id = ?1", [pid], |r| r.get(0))?;
            let member = rel.strip_prefix(&format!("{prel}/")).unwrap_or(&rel).to_string();
            (pid, prel, Some(member))
        }
        None => (file_id, rel, None),
    };
    let base = roots.iter().find(|r| r.id == root).map(|r| r.dir.clone()).unwrap_or_default();
    let path = rel.split('/').fold(base, |p, part| p.join(part));
    Ok(Some(Src { file_id, root, rel, path: path.display().to_string(), member, size: size as u64, crc32, sha1 }))
}

/// The id of the file (or member) at `rel` in `root`, or `member` of the container `parent`.
fn file_id(conn: &Connection, root: &str, rel: &str) -> Result<Option<i64>> {
    Ok(conn.query_row("SELECT id FROM files WHERE root_id = ?1 AND rel_path = ?2", params![root, rel], |r| r.get(0)).optional()?)
}

/// A disc's cue sheet (or GDI) and the files it names, as sources: the sheet the DAT
/// matched, else one beside the tracks that names them (a DAT without the cue, such
/// as libretro's copies of Redump's).
fn disc_sources(conn: &Connection, roots: &[Root], chosen: &[(&RomRow, &Cand)]) -> Result<Option<Vec<Src>>> {
    let sheet = match chosen.iter().find(|(r, _)| cue::is_sheet(&r.name)) {
        Some((_, c)) => Some(c.src.clone()),
        None => {
            // a sheet beside the first track
            let Some((_, first)) = chosen.first() else { return Ok(None) };
            let (dir, inside) = match &first.src.member {
                Some(m) => (format!("{}/{}", first.src.rel, m.rsplit_once('/').map(|(d, _)| d).unwrap_or("")).trim_end_matches('/').to_string(), true),
                None => (first.src.rel.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default(), false),
            };
            let track = first.src.member.clone().unwrap_or_else(|| Path::new(&first.src.path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
            let track = track.rsplit('/').next().unwrap_or(&track).to_string();
            let mut q = conn.prepare(
                "SELECT id, rel_path FROM files WHERE root_id = ?1 AND (lower(name) LIKE '%.cue' OR lower(name) LIKE '%.gdi')
                   AND rel_path LIKE ?2 ESCAPE '\\' AND missing_since IS NULL",
            )?;
            let like = format!("{}%", dir.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
            let found: Vec<(i64, String)> = q.query_map(params![first.src.root, like], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
            let mut pick = None;
            for (id, rel) in found {
                let parent = rel.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default();
                if parent != dir {
                    continue;
                }
                let Some(s) = src_of(conn, roots, id)? else { continue };
                let Some(text) = read_small(&s)? else { continue };
                if cue::file_names(&text, platforms::ext_of(&rel) == "gdi").iter().any(|n| n == &track) {
                    pick = Some(s);
                    break;
                }
            }
            let _ = inside;
            pick
        }
    };
    let Some(sheet) = sheet else { return Ok(None) };
    let Some(text) = read_small(&sheet)? else { return Ok(None) };
    let gdi = platforms::ext_of(sheet.member.as_deref().unwrap_or(&sheet.rel)) == "gdi";
    let mut out = vec![sheet.clone()];
    for n in cue::file_names(&text, gdi) {
        let n = n.replace('\\', "/");
        let rel = match &sheet.member {
            Some(m) => format!("{}/{}{n}", sheet.rel, m.rsplit_once('/').map(|(d, _)| format!("{d}/")).unwrap_or_default()),
            None => format!("{}{n}", sheet.rel.rsplit_once('/').map(|(d, _)| format!("{d}/")).unwrap_or_default()),
        };
        let found = match file_id(conn, &sheet.root, &rel)? {
            Some(id) => src_of(conn, roots, id)?,
            None => None,
        };
        match found {
            Some(s) => out.push(s),
            None => return Ok(None),
        }
    }
    Ok(Some(out))
}

/// A cue sheet's text (they're small), from its file or its archive.
fn read_small(s: &Src) -> Result<Option<String>> {
    if s.size > 1 << 20 {
        return Ok(None);
    }
    match &s.member {
        None => Ok(std::fs::read(&s.path).ok().map(|b| String::from_utf8_lossy(&b).into_owned())),
        Some(m) => {
            let mut out = None;
            let _ = archive::each_wanted(Path::new(&s.path), &|n| n == m, &AtomicBool::new(false), &mut |_, _, r| {
                let mut v = vec![];
                std::io::Read::read_to_end(r, &mut v)?;
                out = Some(String::from_utf8_lossy(&v).into_owned());
                Ok(())
            });
            Ok(out)
        }
    }
}

/// An image or archive as one whole file, to be set aside.
fn whole(conn: &Connection, s: &Src) -> Result<Src> {
    let mut w = organise::whole_archive(s);
    if w.size == 0 {
        w.size = conn.query_row("SELECT size FROM files WHERE id = ?1", [s.file_id], |r| r.get::<_, i64>(0)).unwrap_or(0) as u64;
    }
    Ok(w)
}

/// The originals a conversion replaces (in place): loose files, and archives or images
/// every member of which it used. Others are reported as left.
fn originals(conn: &Connection, sources: &[Src], left: &mut Vec<Value>) -> Result<Vec<Src>> {
    let mut out = vec![];
    let mut used: BTreeMap<i64, (Src, HashSet<String>)> = BTreeMap::new();
    for s in sources {
        match &s.member {
            None => out.push(s.clone()),
            Some(m) => {
                used.entry(s.file_id).or_insert_with(|| (s.clone(), HashSet::new())).1.insert(m.clone());
            }
        }
    }
    for (id, (s, members)) in used {
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM files WHERE parent_id = ?1", [id], |r| r.get(0))?;
        if count as usize == members.len() {
            out.push(whole(conn, &s)?);
        } else {
            left.push(json!({ "path": s.path, "why": "It holds other files too, so it stays where it is." }));
        }
    }
    Ok(out)
}

/// The image kind (chd, cso, zso, rvz) a game is in, when it's in one whole.
fn image_kind(chosen: &[(&RomRow, &Cand)]) -> Option<String> {
    chosen.first().filter(|(_, c)| c.in_image()).and_then(|(_, c)| c.archive.as_ref().map(|a| a.0.clone()))
}

fn new_plan(lib: &Library, d: &audit::DatRef, kind: &str, (label, title): (String, String), to: &str, out: Option<&Path>) -> Plan {
    let _ = lib;
    Plan {
        id: organise::new_id(), created: now(), kind: kind.into(), label, title,
        dat: json!({ "id": d.id, "name": d.name, "version": d.version }), platform: d.platform.clone(),
        mode: if out.is_some() { "copy".into() } else { "in-place".into() }, format: to.into(),
        out: out.map(|o| o.display().to_string()), ops: vec![], games: 0, in_place: 0,
        conflicts: vec![], duplicates: vec![], left: vec![], space: vec![], new_root: None, new_roots: vec![], present: vec![], export: None,
    }
}

/// What a conversion did (for History) and what its preview says it will do.
fn labels(name: &str, to: &str, out: Option<&Path>) -> (String, String) {
    let place = match out {
        Some(out) => format!("into {}", organise::shown(out)),
        None => "where they are".into(),
    };
    if to == "extract" {
        (format!("Extracted {name} {place}"), format!("Extract {name} {place}"))
    } else {
        let to = to.to_uppercase();
        (format!("Converted {name} to {to} {place}"), format!("Convert {name} to {to} {place}"))
    }
}

/// Plan converting (or extracting) a platform's games, or some of them (nothing is written).
pub fn plan(ix: &Index, lib: &Library, o: &Options) -> Result<Plan> {
    let conn = ix.conn();
    let d = audit::dat_ref(conn, o.dat_id)?;
    let media = platforms::media(&d.platform);
    let name = platforms::name_of(&d.platform);
    if media == Media::AuditOnly {
        bail!("{name} games are audited only: Game Repo doesn't convert them.");
    }
    if media == Media::Arcade {
        bail!("{name} sets are ZIPs as their set type says: Organise builds them.");
    }
    if o.to != "extract" && !platforms::formats(&d.platform).contains(&o.to.as_str()) {
        bail!("{name} games can't be converted to {}.", o.to.to_uppercase());
    }
    if let Some(out) = &o.out {
        if organise::key(out).starts_with(&organise::key(&lib.root().join(crate::library::APP_DIR))) {
            bail!("The library's own _library folder can't hold games.");
        }
    }
    if media == Media::Cartridge {
        // ZIP and back are Organise's: copy into the folder, or rename where they are
        let mut plan = organise::plan(ix, lib, &organise::Options {
            dat_id: o.dat_id, mode: if o.out.is_some() { "copy".into() } else { "rename".into() }, out: o.out.clone(),
            format: if o.to == "extract" { "loose".into() } else { "zip".into() },
            partial: false, always_copy: false, only_misnamed: false, games: o.games.clone(), ..Default::default()
        })?;
        plan.kind = if o.to == "extract" { "extract".into() } else { "convert".into() };
        (plan.label, plan.title) = labels(&name, if o.to == "extract" { "extract" } else { "zip" }, o.out.as_deref());
        return Ok(plan);
    }
    if o.to == "rvz" && crate::tools::find("dolphin-tool").is_none() {
        bail!("Converting to RVZ needs dolphin-tool (Settings › Tools).");
    }
    let roots = roots::list(lib);
    let naming = organise::naming(lib, &d.platform);
    let mut games = organise::load_games(conn, &roots, o.dat_id)?;
    if let Some(only) = &o.games {
        let only: HashSet<&str> = only.iter().map(String::as_str).collect();
        games.retain(|g| only.contains(g.fields.name.as_str()));
    }
    let label = labels(&name, &o.to, o.out.as_deref());
    let mut p = new_plan(lib, &d, if o.to == "extract" { "extract" } else { "convert" }, label, &o.to, o.out.as_deref());
    let mut taken: HashSet<String> = HashSet::new();
    for g in &games {
        let in_image = g.roms.iter().any(|r| r.cands.iter().any(Cand::in_image));
        let have = g.roms.iter().filter(|r| !r.cands.is_empty() || (in_image && cue::is_sheet(&r.name))).count();
        if have == 0 {
            continue;
        }
        if have < g.roms.len() && !in_image && !g.roms.iter().filter(|r| r.cands.is_empty()).all(|r| cue::is_sheet(&r.name)) {
            p.left.push(json!({ "path": g.fields.name, "why": "Only some of its files are here." }));
            continue;
        }
        let (chosen, _) = organise::choose(g, "keep", o.out.as_deref());
        if chosen.is_empty() {
            continue;
        }
        let before = p.ops.len();
        let r = if o.to == "extract" {
            plan_extract(conn, &roots, &naming, g, &chosen, o, &mut p, &mut taken)
        } else {
            plan_convert(conn, &roots, &naming, g, &chosen, o, &d.platform, &mut p, &mut taken)
        };
        match r {
            Ok(true) => {
                p.games += 1;
                if p.ops.len() == before {
                    p.in_place += 1;
                }
            }
            Ok(false) => {
                p.ops.truncate(before);
                p.in_place += 1;
            }
            Err(e) => {
                p.ops.truncate(before);
                p.left.push(json!({ "path": g.fields.name, "why": format!("{e:#}") }));
            }
        }
    }
    // space on each drive written to: up to the size of what's converted
    let mut need: BTreeMap<String, (u64, PathBuf)> = BTreeMap::new();
    for op in &p.ops {
        if op.op == "aside" {
            continue;
        }
        let to = PathBuf::from(&op.to);
        let e = need.entry(organise::drive_name(&to)).or_insert((0, to.clone()));
        e.0 += op.size;
    }
    for (drive, (bytes, at)) in need {
        let free = fs4::available_space(organise::existing(&at)).unwrap_or(u64::MAX);
        p.space.push(json!({ "drive": drive, "needed": bytes, "free": free, "short": bytes > free }));
    }
    Ok(p)
}

/// Where a game's results go: the output folder, else beside its files.
fn dest_dir(o: &Options, first: &Src) -> PathBuf {
    match &o.out {
        Some(out) => out.clone(),
        None => Path::new(&first.path).parent().map(Path::to_path_buf).unwrap_or_default(),
    }
}

/// A target no other game of the plan has, and not already there (else it's a conflict).
fn claim(taken: &mut HashSet<String>, to: &Path) -> Result<()> {
    if !taken.insert(organise::key(to)) {
        bail!("Another game of this change goes to {} too.", to.display());
    }
    if to.exists() {
        bail!("{} is there already.", to.display());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn plan_convert(
    conn: &Connection, roots: &[Root], naming: &organise::Naming, g: &GameRow, chosen: &[(&RomRow, &Cand)], o: &Options,
    platform: &str, p: &mut Plan, taken: &mut HashSet<String>,
) -> Result<bool> {
    let current = image_kind(chosen);
    if current.as_deref() == Some(o.to.as_str()) {
        return Ok(false);
    }
    let base = organise::safe_name(&organise::render(&naming.template, &g.fields));
    // what's converted: a disc's sheet and tracks, or its one ISO (loose, in an archive, or in another image)
    let isos: Vec<&Src> = chosen.iter().filter(|(r, _)| platforms::ext_of(&r.name) == "iso").map(|(_, c)| &c.src).collect();
    let (format, sources): (&str, Vec<Src>) = match (o.to.as_str(), current.as_deref()) {
        (_, Some("rvz")) => bail!("An RVZ can only be extracted (to an ISO), not converted to {}.", o.to.to_uppercase()),
        ("chd", Some("chd")) => return Ok(false),
        ("chd", _) if isos.len() == 1 && chosen.len() == 1 => {
            // PS2 and PSP ISOs are DVDs (and UMDs); other systems' ISOs are a CD's one data track
            let dvd = matches!(platform, "ps2" | "psp") || isos[0].size > 900 << 20;
            (if dvd { "chd-dvd" } else { "chd-cd" }, vec![isos[0].clone()])
        }
        ("chd", Some(k)) => bail!("A {} holding a CD can't be converted straight to a CHD: extract it first.", k.to_uppercase()),
        ("chd", None) => match disc_sources(conn, roots, chosen)? {
            Some(s) => ("chd-cd", s),
            None => bail!("Its cue sheet isn't here, so its tracks can't be laid out on a CD."),
        },
        ("cso" | "zso" | "rvz", _) if isos.len() == 1 && chosen.len() == 1 => (o.to.as_str(), vec![isos[0].clone()]),
        (to, _) => bail!("Only a game that's one ISO can be converted to {}.", to.to_uppercase()),
    };
    if current.as_deref() == Some("chd") && format == "chd-dvd" {
        return Ok(false);
    }
    let ext = &o.to;
    let to = dest_dir(o, sources.iter().find(|s| !cue::is_sheet(s.member.as_deref().unwrap_or(&s.rel))).unwrap_or(&sources[0]));
    let to = if naming.folder && o.out.is_some() { to.join(&base) } else { to }.join(format!("{base}.{ext}"));
    claim(taken, &to)?;
    let aside = if o.out.is_none() { originals(conn, &sources, &mut p.left)? } else { vec![] };
    let size = sources.iter().map(|s| s.size).sum();
    p.ops.push(Op {
        op: "convert".into(), game: g.fields.name.clone(), to: to.display().to_string(), from: sources, names: vec![format.into()],
        aside, size, note: None,
    });
    Ok(true)
}

#[allow(clippy::too_many_arguments)]
fn plan_extract(
    conn: &Connection, roots: &[Root], naming: &organise::Naming, g: &GameRow, chosen: &[(&RomRow, &Cand)], o: &Options,
    p: &mut Plan, taken: &mut HashSet<String>,
) -> Result<bool> {
    let base = organise::safe_name(&organise::render(&naming.template, &g.fields));
    let first = &chosen[0].1.src;
    let Some((kind, _, _)) = chosen[0].1.archive.clone() else { return Ok(false) };
    let dir = dest_dir(o, first);
    // every file the image or archive holds, named as the DAT names it where it's matched
    let mut q = conn.prepare("SELECT id, name, rel_path FROM files WHERE parent_id = ?1 ORDER BY rel_path")?;
    let members: Vec<(i64, String, String)> = q.query_map([first.file_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
    let named: HashMap<&str, &str> = chosen.iter().filter_map(|(r, c)| c.src.member.as_deref().map(|m| (m, r.name.as_str()))).collect();
    let sheet_rom = g.roms.iter().find(|r| cue::is_sheet(&r.name)).map(|r| r.name.clone());
    if kind == "rvz" {
        let to = dir.join(format!("{base}.iso"));
        claim(taken, &to)?;
        let mut src = first.clone();
        src.member = None;
        let want = chosen[0].1.src.clone();
        let aside = if o.out.is_none() { vec![whole(conn, &src)?] } else { vec![] };
        p.ops.push(Op {
            op: "convert".into(), game: g.fields.name.clone(), to: to.display().to_string(), from: vec![want], names: vec!["iso".into()],
            aside, size: first.size, note: None,
        });
        return Ok(true);
    }
    let cd = kind == "chd" && members.iter().any(|(_, n, _)| n.starts_with("Track "));
    // a GD-ROM's audio tracks are .raw files, as Redump names them
    let gd_audio = if cd { image::gd_audio(Path::new(&first.path)) } else { None };
    let gd = gd_audio.is_some();
    let many = members.len() > 1 || cd;
    let folder = if many || (naming.folder && o.out.is_some()) { dir.join(&base) } else { dir.clone() };
    let mut track_names = vec![];
    for (id, mname, _) in &members {
        let Some(src) = src_of(conn, roots, *id)? else { bail!("{mname} in it hasn't been read yet: scan first.") };
        let member = src.member.clone().unwrap_or_default();
        let out_name = match named.get(member.as_str()) {
            Some(n) => n.to_string(),
            None if cd => format!("{base} (Track {}).{}", mname.trim_start_matches("Track ").trim_end_matches(".bin").trim_start_matches('0'), if gd_audio.as_ref().is_some_and(|a| a.contains(mname)) { "raw" } else { "bin" }),
            None if kind == "chd" || image::KINDS.contains(&kind.as_str()) => format!("{base}.iso"),
            None => member.rsplit('/').next().unwrap_or(&member).to_string(),
        };
        let to = folder.join(organise::safe_rel(&out_name));
        claim(taken, &to)?;
        track_names.push(to.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        let size = src.size;
        p.ops.push(Op { op: "extract".into(), game: g.fields.name.clone(), to: to.display().to_string(), from: vec![src], names: vec![], aside: vec![], size, note: None });
    }
    if cd {
        let to = folder.join(sheet_rom.map(|n| organise::safe_name(&n)).unwrap_or_else(|| format!("{base}.{}", if gd { "gdi" } else { "cue" })));
        claim(taken, &to)?;
        let mut src = first.clone();
        src.member = None;
        p.ops.push(Op { op: "sheet".into(), game: g.fields.name.clone(), to: to.display().to_string(), from: vec![src], names: track_names, aside: vec![], size: 0, note: None });
    }
    if o.out.is_none() {
        let src = whole(conn, first)?;
        p.ops.push(Op { op: "aside".into(), game: g.fields.name.clone(), to: src.path.clone(), size: 0, from: vec![src], names: vec![], aside: vec![], note: None });
    }
    Ok(true)
}

// ------------------------------------------------------------ running

/// A temporary name beside `to`, which becomes `to` once the result is checked.
fn tmp_of(to: &Path) -> PathBuf {
    to.with_file_name(format!("{}.gamerepo-tmp", to.file_name().unwrap_or_default().to_string_lossy()))
}

/// Do a `convert` or `sheet` step (organise::run calls this): write the result beside
/// its target, check it against what it came from, then give it its name.
pub(crate) fn perform(op: &Op, cancel: &AtomicBool) -> Result<Written> {
    let to = Path::new(&op.to);
    let tmp = tmp_of(to);
    let staging = to.parent().unwrap_or(Path::new(".")).join(organise::STAGING).join(format!("{}-{}", std::process::id(), to.file_name().unwrap_or_default().to_string_lossy()));
    let r = (|| -> Result<Written> {
        if to.exists() {
            bail!("{} is there already", to.display());
        }
        let w = if op.op == "sheet" {
            let text = image::cue_for(Path::new(&op.from[0].path), &op.names)?;
            std::fs::write(&tmp, text.as_bytes())?;
            let h = hash::hash_reader(&mut text.as_bytes(), text.len() as u64, &op.to, cancel)?;
            Written { members: vec![], file: Some(h), kind: "file" }
        } else {
            // (an RVZ is read by dolphin-tool itself)
            let inputs = if op.names.first().is_some_and(|f| f == "iso") {
                op.from.iter().map(|s| (s.clone(), PathBuf::from(&s.path))).collect()
            } else {
                materialise(op, &staging, cancel)?
            };
            convert(op, &inputs, &tmp, to, cancel)?
        };
        std::fs::rename(&tmp, to).with_context(|| format!("couldn't name {}", to.display()))?;
        Ok(w)
    })();
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    let _ = std::fs::remove_dir_all(&staging);
    let _ = std::fs::remove_dir(to.parent().unwrap_or(Path::new(".")).join(organise::STAGING));
    r
}

/// Each source as a file on disk: loose files as they are, members of archives or
/// images unpacked into `staging` under their own names (so a cue sheet finds its tracks).
fn materialise(op: &Op, staging: &Path, cancel: &AtomicBool) -> Result<Vec<(Src, PathBuf)>> {
    let mut wanted: BTreeMap<String, HashSet<String>> = BTreeMap::new();
    for s in &op.from {
        if let Some(m) = &s.member {
            wanted.entry(s.path.clone()).or_default().insert(m.clone());
        }
    }
    let mut out: HashMap<(String, String), PathBuf> = HashMap::new();
    for (path, members) in wanted {
        std::fs::create_dir_all(staging)?;
        archive::each_wanted(Path::new(&path), &|n| members.contains(n), cancel, &mut |name, _, r| {
            let p = staging.join(name.rsplit('/').next().unwrap_or(name));
            let mut f = std::io::BufWriter::new(std::fs::File::create(&p)?);
            std::io::copy(r, &mut f)?;
            f.into_inner().map_err(|e| e.into_error())?.sync_all()?;
            out.insert((path.clone(), name.to_string()), p);
            Ok(())
        })
        .with_context(|| format!("couldn't read {path}"))?;
    }
    op.from
        .iter()
        .map(|s| {
            let p = match &s.member {
                Some(m) => out.get(&(s.path.clone(), m.clone())).cloned().ok_or_else(|| anyhow!("{m} isn't in {} any more", s.path))?,
                None => PathBuf::from(&s.path),
            };
            Ok((s.clone(), p))
        })
        .collect()
}

/// Write the conversion into `tmp` and check it.
fn convert(op: &Op, inputs: &[(Src, PathBuf)], tmp: &Path, to: &Path, cancel: &AtomicBool) -> Result<Written> {
    let format = op.names.first().map(String::as_str).unwrap_or("");
    let stem = to.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let one = |what: &str| -> Result<&(Src, PathBuf)> {
        match inputs {
            [x] => Ok(x),
            _ => bail!("{what} is made from one ISO"),
        }
    };
    match format {
        "chd-cd" => {
            let sheet = inputs.iter().find(|(s, _)| cue::is_sheet(s.member.as_deref().unwrap_or(&s.rel)));
            let tracks = match sheet {
                Some((_, p)) => image::cd_tracks(p)?,
                None => image::iso_as_cd(&one("A CD CHD without a cue sheet")?.1)?,
            };
            image::write_cd(&tracks, tmp, cancel).context("couldn't write the CHD")?;
            // each file the CHD was made from, read back out of it
            let back = image::cd_file_hashes(tmp, &tracks, cancel)?;
            for (path, h) in &back {
                let (src, _) = inputs.iter().find(|(_, p)| p == path).ok_or_else(|| anyhow!("{} wasn't one of its files", path.display()))?;
                organise::check_hashes(h, src).with_context(|| format!("{} read back from the CHD isn't the same", src.member.as_deref().unwrap_or(&src.path)))?;
            }
            for (s, p) in inputs {
                if !cue::is_sheet(s.member.as_deref().unwrap_or(&s.rel)) && !back.iter().any(|(bp, _)| bp == p) {
                    bail!("{} isn't in the CHD", p.display());
                }
            }
            // the index's tracks: the files' hashes when each was one track, else read again
            let members = if back.len() == tracks.len() {
                back.into_iter().enumerate().map(|(i, (_, h))| (image::track_name(tracks[i].track.number), h)).collect()
            } else {
                image::read(tmp, "chd", cancel)?.members.into_iter().map(|m| (m.name, m.hashes)).collect()
            };
            Ok(Written { members, file: None, kind: "chd" })
        }
        "chd-dvd" | "cso" | "zso" | "rvz" => {
            let (src, iso) = one("It")?;
            let kind = match format {
                "chd-dvd" => "chd",
                "cso" => "cso",
                "zso" => "zso",
                _ => "rvz",
            };
            match format {
                "chd-dvd" => image::write_dvd(iso, tmp, cancel)?,
                "cso" | "zso" => image::write_cso(iso, tmp, format == "zso", cancel)?,
                _ => run_tool(&["convert", "-i"], iso, tmp, &["-f", "rvz", "-c", "zstd", "-l", "5", "-b", "131072"], cancel)?,
            }
            let back = image::read(tmp, kind, cancel).with_context(|| format!("the new {} doesn't read back", kind.to_uppercase()))?;
            let [m] = &back.members[..] else { bail!("the new {} holds {} files, not one", kind.to_uppercase(), back.members.len()) };
            organise::check_hashes(&m.hashes, src).with_context(|| format!("the ISO read back from the new {} isn't the same", kind.to_uppercase()))?;
            Ok(Written { members: vec![(format!("{stem}.iso"), m.hashes.clone())], file: None, kind })
        }
        "iso" => {
            // an RVZ back to its ISO
            let (src, _) = one("It")?;
            run_tool(&["convert", "-i"], Path::new(&src.path), tmp, &["-f", "iso"], cancel)?;
            let size = std::fs::metadata(tmp)?.len();
            let h = hash::hash_reader(&mut std::fs::File::open(tmp)?, size, &op.to, cancel)?;
            organise::check_hashes(&h, src).context("the ISO dolphin-tool wrote isn't the disc the RVZ holds")?;
            Ok(Written { members: vec![], file: Some(h), kind: "file" })
        }
        other => bail!("there's no conversion called {other}"),
    }
}

/// Run dolphin-tool: `args… <input> -o <output> more…`.
fn run_tool(args: &[&str], input: &Path, output: &Path, more: &[&str], cancel: &AtomicBool) -> Result<()> {
    let tool = crate::tools::find("dolphin-tool").ok_or_else(|| anyhow!("dolphin-tool isn't found (Settings › Tools)"))?;
    let mut child = crate::tools::command(&tool)
        .args(args)
        .arg(input)
        .arg("-o")
        .arg(output)
        .args(more)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| format!("couldn't run {}", tool.display()))?;
    let out = image::wait_stopping(&mut child, cancel)?;
    if !out.status.success() || !output.is_file() {
        bail!("dolphin-tool couldn't convert it: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::tests::{fixture, import, names, rom_xml, run_scan, write, Fixture};

    fn go(f: &Fixture, plan: &Plan) -> Value {
        organise::run(&f.ix, &f.lib, plan, &AtomicBool::new(false), &mut |_| {}).unwrap()
    }

    fn tree(dir: &Path) -> BTreeMap<String, u64> {
        let mut out = BTreeMap::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    out.insert(p.strip_prefix(dir).unwrap().to_string_lossy().replace('\\', "/"), std::fs::metadata(&p).unwrap().len());
                }
            }
        }
        out
    }

    /// The made-up CD as Redump lays it out, and the DVD as an ISO, with DATs for them.
    fn setup(name: &str) -> (Fixture, i64, i64) {
        let f = fixture(name);
        let src = image::tests::sources(&format!("{name}-src"));
        let read = |n: &str| std::fs::read(src.join(n)).unwrap();
        let cd = ["Disc Game (USA).cue", "Disc Game (USA) (Track 1).bin", "Disc Game (USA) (Track 2).bin"];
        for n in cd {
            write(&f.dir.join("Roms/PSX/Disc Game (USA)").join(n), &read(n));
        }
        write(&f.dir.join("Roms/PSP/Dvd Game (Europe).iso"), &read("Dvd Game (Europe).iso"));
        let roms: String = cd.iter().map(|n| rom_xml(n, &read(n))).collect();
        write(&f.dir.join("dats/psx.dat"), crate::audit::tests::dat_xml("Sony - PlayStation", "2026", &[("Disc Game (USA)", None, roms)]).as_bytes());
        let iso = rom_xml("Dvd Game (Europe).iso", &read("Dvd Game (Europe).iso"));
        write(&f.dir.join("dats/psp.dat"), crate::audit::tests::dat_xml("Sony - PlayStation Portable", "2026", &[("Dvd Game (Europe)", None, iso)]).as_bytes());
        run_scan(&f, false);
        let psx = import(&f, "psx.dat")[0]["id"].as_i64().unwrap();
        let psp = import(&f, "psp.dat")[0]["id"].as_i64().unwrap();
        let _ = std::fs::remove_dir_all(&src);
        (f, psx, psp)
    }

    fn opts(dat: i64, to: &str, out: Option<PathBuf>) -> Options {
        Options { dat_id: dat, to: to.into(), games: None, out }
    }

    #[test]
    fn cue_bin_to_chd_in_place_then_back_out_and_undo() {
        let (f, psx, _) = setup("convert-cd");
        let roms = f.dir.join("Roms");
        let before = tree(&roms);
        let p = plan(&f.ix, &f.lib, &opts(psx, "chd", None)).unwrap();
        assert_eq!((p.games, p.ops.len()), (1, 1), "{:#?}", p);
        assert_eq!(p.ops[0].aside.len(), 3, "the cue and both bins are set aside");
        let r = go(&f, &p);
        assert_eq!(r["done"], 1, "{r}");
        let chd = roms.join("PSX/Disc Game (USA)/Disc Game (USA).chd");
        assert!(chd.is_file() && !roms.join("PSX/Disc Game (USA)/Disc Game (USA).cue").exists());
        // the index followed: the game is had from the CHD with no rescan, and a rescan reads nothing
        let s = audit::summary(f.ix.conn(), psx).unwrap();
        assert_eq!((s["have"].as_i64(), s["matched"].as_i64()), (Some(1), Some(2)), "{s}");
        assert_eq!(run_scan(&f, false)["hashed"], 0);
        // nothing more to do for it
        let again = plan(&f.ix, &f.lib, &opts(psx, "chd", None)).unwrap();
        assert_eq!((again.ops.len(), again.in_place), (0, 1));
        // extracted into a folder: Redump's own files again, its cue an exact match
        let out = f.dir.join("Extracted");
        let x = plan(&f.ix, &f.lib, &opts(psx, "extract", Some(out.clone()))).unwrap();
        assert_eq!(x.ops.iter().map(|o| o.op.as_str()).collect::<Vec<_>>(), ["extract", "extract", "sheet"]);
        let r = go(&f, &x);
        assert_eq!(r["done"], 3, "{r}");
        for n in ["Disc Game (USA).cue", "Disc Game (USA) (Track 1).bin", "Disc Game (USA) (Track 2).bin"] {
            assert_eq!(std::fs::metadata(out.join("Disc Game (USA)").join(n)).unwrap().len(), before[&format!("PSX/Disc Game (USA)/{n}")], "{n}");
        }
        crate::roots::add(&f.lib, &out, Some("psx")).unwrap();
        run_scan(&f, false);
        let method: String = f.ix.conn().query_row("SELECT m.method FROM matches m JOIN files x ON x.id = m.file_id WHERE x.name = 'Disc Game (USA).cue'", [], |r| r.get(0)).unwrap();
        assert!(method.starts_with("sha1"), "the cue written is Redump's: {method}");
        // undo both, newest first: the folders are as they were
        organise::undo(&f.ix, &f.lib, &x.id, &AtomicBool::new(false), &mut |_| {}).unwrap();
        let u = organise::undo(&f.ix, &f.lib, &p.id, &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(u["undone"], 1, "{u}");
        assert_eq!(tree(&roms).into_iter().filter(|(k, _)| !k.starts_with(".gamerepo")).collect::<BTreeMap<_, _>>(), before);
        let _ = std::fs::remove_dir_all(&f.dir);
    }

    #[test]
    fn a_gamecube_iso_to_rvz_and_back_with_dolphin_tool() {
        let f = fixture("convert-rvz");
        let disc = crate::audit::tests::rom(21, 64 * 1024);
        write(&f.dir.join("Roms/GameCube/Cube (USA).iso"), &disc);
        write(&f.dir.join("dats/gc.dat"), crate::audit::tests::dat_xml("Nintendo - GameCube", "2026", &[("Cube (USA)", None, rom_xml("Cube (USA).iso", &disc))]).as_bytes());
        run_scan(&f, false);
        let _turn = crate::tools::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let gc = import(&f, "gc.dat")[0]["id"].as_i64().unwrap();
        crate::tools::configure(&Default::default());
        if crate::tools::find("dolphin-tool").is_none() {
            assert!(plan(&f.ix, &f.lib, &opts(gc, "rvz", None)).is_err(), "RVZ needs dolphin-tool");
        }
        // (on Windows a batch file starts the Python script: review M46)
        let tool = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures").join(if cfg!(windows) { "fake_dolphin_tool.cmd" } else { "fake_dolphin_tool.py" });
        crate::tools::configure(&[("dolphin-tool".to_string(), tool)].into());
        let p = plan(&f.ix, &f.lib, &opts(gc, "rvz", None)).unwrap();
        let r = go(&f, &p);
        assert_eq!(r["done"], 1, "{r}");
        let rvz = f.dir.join("Roms/GameCube/Cube (USA).rvz");
        assert!(rvz.is_file() && !f.dir.join("Roms/GameCube/Cube (USA).iso").exists());
        let s = audit::summary(f.ix.conn(), gc).unwrap();
        assert_eq!((s["have"].as_i64(), s["matched"].as_i64()), (Some(1), Some(1)), "{s}");
        // and back to the ISO
        let x = plan(&f.ix, &f.lib, &opts(gc, "extract", None)).unwrap();
        let r = go(&f, &x);
        assert_eq!(r["done"], 1, "{r}");
        assert_eq!(std::fs::read(f.dir.join("Roms/GameCube/Cube (USA).iso")).unwrap(), disc);
        assert!(!rvz.exists());
        crate::tools::configure(&Default::default());
        let _ = std::fs::remove_dir_all(&f.dir);
    }

    #[test]
    fn an_iso_to_cso_zso_and_chd_and_a_bad_conversion_refused() {
        let (f, _, psp) = setup("convert-dvd");
        let out = f.dir.join("Out");
        for to in ["cso", "zso", "chd"] {
            let p = plan(&f.ix, &f.lib, &opts(psp, to, Some(out.join(to)))).unwrap();
            assert_eq!(p.ops.len(), 1, "{to}");
            let r = go(&f, &p);
            assert_eq!(r["done"], 1, "{to}: {r}");
            let made = out.join(to).join(format!("Dvd Game (Europe).{to}"));
            let l = image::read(&made, to, &AtomicBool::new(false)).unwrap();
            assert_eq!(l.members[0].hashes.size, 70 * 2048, "{to}");
        }
        assert_eq!(preferred(&f.lib, "psp").as_deref(), Some("cso"));
        set_preferred(&f.lib, "psp", "chd").unwrap();
        assert_eq!(preferred(&f.lib, "psp").as_deref(), Some("chd"));
        assert!(set_preferred(&f.lib, "psp", "rvz").is_err());
        // a source that changed after it was hashed: the result doesn't match, nothing is placed
        let p = plan(&f.ix, &f.lib, &opts(psp, "cso", None)).unwrap();
        let iso = f.dir.join("Roms/PSP/Dvd Game (Europe).iso");
        let mut bytes = std::fs::read(&iso).unwrap();
        bytes[5000] ^= 1;
        std::fs::write(&iso, &bytes).unwrap();
        let r = go(&f, &p);
        assert_eq!(r["done"], 0, "{r}");
        assert!(r["failed"][0]["error"].as_str().unwrap().contains("isn't the same"), "{r}");
        assert!(iso.is_file() && !f.dir.join("Roms/PSP/Dvd Game (Europe).cso").exists());
        assert!(!f.dir.join("Roms/PSP/Dvd Game (Europe).cso.gamerepo-tmp").exists());
        let _ = names;
        let _ = std::fs::remove_dir_all(&f.dir);
    }
}
