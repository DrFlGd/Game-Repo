//! Arcade sets (docs/PLAN.md, "Phase 7 design", D57–D61): where each ROM of a MAME or
//! FinalBurn Neo DAT goes in the set type the system is kept as, which other sets a
//! machine needs to run (its BIOS set, its devices, a split DAT's parent) and so whether
//! it's playable, and its samples.
//!
//! A ROM a machine shares with its parent or BIOS set (`merge`, or the same name and
//! contents when the DAT doesn't say) *comes from* that set. The set types then say:
//! non-merged, every machine's ZIP holds all its ROMs but its BIOS's; split, only the ROMs
//! that come from it; merged, a parent's ZIP holds its clones' too. BIOS sets and devices
//! are ZIPs of their own, and a disk stays in the folder of the machine it comes from.

use crate::library::Library;
use crate::platforms;
use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};

pub const SET_TYPES: &[&str] = &["non-merged", "split", "merged"];
pub const DEFAULT_SET_TYPE: &str = "non-merged";

/// The set type an arcade system is kept as (library.json `arcade_sets`).
pub fn set_type(lib: &Library, platform: &str) -> String {
    lib.meta()["arcade_sets"][platform].as_str().filter(|t| SET_TYPES.contains(t)).unwrap_or(DEFAULT_SET_TYPE).to_string()
}

/// Keep `platform` as `kind` from now on: where its ROMs belong is worked out again.
pub fn set_set_type(conn: &Connection, lib: &Library, platform: &str, kind: &str) -> Result<()> {
    if !platforms::is_arcade(platform) {
        bail!("{} isn't an arcade system.", platforms::name_of(platform));
    }
    if !SET_TYPES.contains(&kind) {
        bail!("There's no set type called {kind}.");
    }
    let mut all = lib.meta()["arcade_sets"].as_object().cloned().unwrap_or_default();
    all.insert(platform.to_string(), json!(kind));
    lib.update_meta(json!({ "arcade_sets": all }))?;
    let dats: Vec<i64> = conn.prepare("SELECT id FROM dats WHERE platform = ?1")?.query_map([platform], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    for d in dats {
        write_places(conn, d, kind)?;
    }
    crate::audit::changed(conn)
}

/// Each arcade system's set type, for the page.
pub fn set_types(lib: &Library) -> Value {
    let m: Map<String, Value> = platforms::all().filter(|p| platforms::is_arcade(p.id)).map(|p| (p.id.to_string(), json!(set_type(lib, p.id)))).collect();
    json!({ "types": SET_TYPES, "systems": m })
}

// ------------------------------------------------------------ the machines

#[derive(Clone, Debug)]
pub struct Rom {
    pub id: i64,
    pub name: String,
    crc32: Option<String>,
    sha1: Option<String>,
    merge: Option<String>,
    pub disk: bool,
}

#[derive(Clone, Debug)]
pub struct Machine {
    pub id: i64,
    pub name: String,
    /// "bios", "device", "mechanical" or none
    pub kind: Option<String>,
    romof: Option<String>,
    /// The parent's index (cloneof)
    parent: Option<usize>,
    pub roms: Vec<Rom>,
    devices: Vec<String>,
}

impl Machine {
    pub fn is_support(&self) -> bool {
        matches!(self.kind.as_deref(), Some("bios" | "device"))
    }
}

/// A DAT's machines, in the DAT's order, with their ROMs (nodumps left out).
pub struct Machines {
    pub list: Vec<Machine>,
    pub by_name: HashMap<String, usize>,
}

impl Machines {
    pub fn load(conn: &Connection, dat_id: i64) -> Result<Self> {
        let mut list: Vec<Machine> = vec![];
        let mut by_id: HashMap<i64, usize> = HashMap::new();
        let mut parents: Vec<Option<i64>> = vec![];
        {
            let mut q = conn.prepare("SELECT id, name, kind, romof, parent_id FROM dat_games WHERE dat_id = ?1 ORDER BY id")?;
            let mut rows = q.query([dat_id])?;
            while let Some(r) = rows.next()? {
                by_id.insert(r.get(0)?, list.len());
                parents.push(r.get(4)?);
                list.push(Machine { id: r.get(0)?, name: r.get(1)?, kind: r.get(2)?, romof: r.get(3)?, parent: None, roms: vec![], devices: vec![] });
            }
        }
        for (i, p) in parents.into_iter().enumerate() {
            list[i].parent = p.and_then(|p| by_id.get(&p).copied());
        }
        {
            let mut q = conn.prepare(
                "SELECT game_id, id, name, crc32, sha1, merge, disk FROM dat_roms WHERE dat_id = ?1 AND (status IS NULL OR status != 'nodump') ORDER BY id",
            )?;
            let mut rows = q.query([dat_id])?;
            while let Some(r) = rows.next()? {
                if let Some(&i) = by_id.get(&r.get::<_, i64>(0)?) {
                    list[i].roms.push(Rom { id: r.get(1)?, name: r.get(2)?, crc32: r.get(3)?, sha1: r.get(4)?, merge: r.get(5)?, disk: r.get::<_, i64>(6)? == 1 });
                }
            }
        }
        {
            let mut q = conn.prepare("SELECT l.game_id, l.name FROM dat_links l JOIN dat_games g ON g.id = l.game_id WHERE g.dat_id = ?1 AND l.kind = 'device'")?;
            let mut rows = q.query([dat_id])?;
            while let Some(r) = rows.next()? {
                if let Some(&i) = by_id.get(&r.get::<_, i64>(0)?) {
                    list[i].devices.push(r.get(1)?);
                }
            }
        }
        let by_name = list.iter().enumerate().map(|(i, m)| (m.name.clone(), i)).collect();
        Ok(Self { list, by_name })
    }

    fn romof(&self, i: usize) -> Option<usize> {
        self.list[i].romof.as_ref().and_then(|n| self.by_name.get(n)).copied().filter(|&j| j != i)
    }

    /// The machine (and its ROM) ROM `ri` of machine `i` comes from: itself, or the
    /// parent or BIOS set it's shared with.
    pub fn origin(&self, i: usize, ri: usize) -> (usize, usize) {
        let (mut at, mut rom) = (i, ri);
        for _ in 0..8 {
            let Some(up) = self.romof(at) else { break };
            let r = &self.list[at].roms[rom];
            let same = |q: &Rom| match (&r.sha1, &q.sha1, &r.crc32, &q.crc32) {
                (Some(a), Some(b), _, _) => a == b,
                (_, _, Some(a), Some(b)) => a == b,
                _ => r.disk && q.disk,
            };
            let found = self.list[up].roms.iter().position(|q| q.disk == r.disk && same(q) && match &r.merge {
                Some(m) => q.name.eq_ignore_ascii_case(m),
                None => q.name.eq_ignore_ascii_case(&r.name),
            });
            match found {
                Some(q) => (at, rom) = (up, q),
                None => break,
            }
        }
        (at, rom)
    }

    /// The machine a merged set keeps `i` in: its top parent (not a BIOS set).
    fn top(&self, i: usize) -> usize {
        let mut at = i;
        for _ in 0..8 {
            match self.list[at].parent.filter(|&p| !self.list[p].is_support()) {
                Some(p) if p != at => at = p,
                _ => break,
            }
        }
        at
    }

    /// The other sets machine `i` needs to run: its BIOS sets, the devices it uses (and
    /// theirs), and, when the DAT lists none of the ROMs it shares with its parent (a split
    /// DAT), its parent. Only sets with ROMs are listed.
    pub fn needs(&self, i: usize) -> Vec<(usize, &'static str)> {
        let mut out: Vec<(usize, &'static str)> = vec![];
        let mut seen: HashSet<usize> = HashSet::from([i]);
        let mut at = i;
        for _ in 0..8 {
            let Some(up) = self.romof(at) else { break };
            if self.list[up].kind.as_deref() == Some("bios") && seen.insert(up) && !self.list[up].roms.is_empty() {
                out.push((up, "bios"));
            }
            at = up;
        }
        if let Some(p) = self.list[i].parent.filter(|p| !self.list[*p].is_support() && !self.list[*p].roms.is_empty()) {
            let leans = (0..self.list[i].roms.len()).any(|ri| self.origin(i, ri).0 != i);
            if !leans && seen.insert(p) {
                out.push((p, "parent"));
            }
        }
        let mut queue: Vec<usize> = vec![i];
        while let Some(m) = queue.pop() {
            for d in &self.list[m].devices {
                if let Some(&j) = self.by_name.get(d) {
                    if seen.insert(j) {
                        queue.push(j);
                        if !self.list[j].roms.is_empty() {
                            out.push((j, "device"));
                        }
                    }
                }
            }
        }
        out
    }
}

// ------------------------------------------------------------ where each ROM goes

/// Where a ROM is kept: `<container>.zip` › `member`, or a disk at `<container>/<member>`.
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub rom_id: i64,
    /// The machine whose ZIP (or folder) it's in
    pub container: String,
    pub member: String,
    pub disk: bool,
}

/// Where every ROM of the DAT goes in a set of `kind`.
pub fn places(ms: &Machines, kind: &str) -> Vec<Place> {
    let mut out = vec![];
    // merged: each parent's members by name, with their contents, so a clone's ROM of the
    // same name but other contents goes in as <clone>/<rom>
    let mut used: HashMap<(usize, String), (Option<String>, Option<String>)> = HashMap::new();
    let mut order: Vec<usize> = (0..ms.list.len()).collect();
    order.sort_by_key(|&i| ms.list[i].parent.is_some());
    for i in order {
        let m = &ms.list[i];
        for (ri, r) in m.roms.iter().enumerate() {
            let (o, oi) = ms.origin(i, ri);
            let from_bios = o != i && ms.list[o].kind.as_deref() == Some("bios");
            if from_bios || (r.disk && o != i) {
                // in the BIOS set's ZIP, or the folder of the machine whose disk it is
                place_as(&mut out, r.id, &ms.list[o].name, &ms.list[o].roms[oi].name, r.disk);
                continue;
            }
            match kind {
                "split" if o != i => place_as(&mut out, r.id, &ms.list[o].name, &ms.list[o].roms[oi].name, r.disk),
                "merged" if !r.disk && !m.is_support() => {
                    let top = ms.top(i);
                    if o != i {
                        let at = if ms.top(o) == top { top } else { o };
                        place_as(&mut out, r.id, &ms.list[at].name, &ms.list[o].roms[oi].name, false);
                        continue;
                    }
                    let key = (top, r.name.to_ascii_lowercase());
                    let member = match used.get(&key) {
                        Some((c, s)) if (c, s) != (&r.crc32, &r.sha1) => format!("{}/{}", m.name, r.name),
                        _ => {
                            used.insert(key, (r.crc32.clone(), r.sha1.clone()));
                            r.name.clone()
                        }
                    };
                    place_as(&mut out, r.id, &ms.list[top].name, &member, false);
                }
                _ => place_as(&mut out, r.id, &m.name, &r.name, r.disk),
            }
        }
    }
    out
}

fn place_as(out: &mut Vec<Place>, rom_id: i64, container: &str, member: &str, disk: bool) {
    out.push(Place { rom_id, container: container.to_string(), member: member.replace('\\', "/"), disk });
}

/// Keep the index's copy of where a DAT's ROMs go (the audit's *named* files).
pub fn write_places(conn: &Connection, dat_id: i64, kind: &str) -> Result<()> {
    let ms = Machines::load(conn, dat_id)?;
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM dat_places WHERE rom_id IN (SELECT id FROM dat_roms WHERE dat_id = ?1)", [dat_id])?;
    {
        let mut ins = tx.prepare("INSERT INTO dat_places (rom_id, container, member) VALUES (?1, ?2, ?3)")?;
        for p in places(&ms, kind) {
            ins.execute(params![p.rom_id, p.container, p.member])?;
        }
    }
    // the set type they were written for, so a change made elsewhere is noticed (review M11)
    tx.execute(
        "INSERT INTO index_meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = ?2",
        params![format!("places:{dat_id}"), kind],
    )?;
    tx.commit()?;
    Ok(())
}

/// Bring each DAT's places in line with its system as it is now (review M11): an arcade
/// DAT with none (loaded before arcade systems had them, or for a system made arcade
/// since) or with another set type's (changed on another computer) gets them written
/// again; a DAT whose system is no longer arcade loses them. Returns whether any changed.
pub fn refresh_places(conn: &Connection, lib: &Library) -> Result<bool> {
    let dats: Vec<(i64, String, Option<String>)> = conn
        .prepare("SELECT d.id, d.platform, m.value FROM dats d LEFT JOIN index_meta m ON m.key = 'places:' || d.id")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut changed = false;
    for (id, platform, written) in dats {
        if platforms::is_arcade(&platform) {
            let kind = set_type(lib, &platform);
            if written.as_deref() != Some(kind.as_str()) {
                write_places(conn, id, &kind)?;
                changed = true;
            }
        } else if written.is_some() {
            let tx = conn.unchecked_transaction()?;
            tx.execute("DELETE FROM dat_places WHERE rom_id IN (SELECT id FROM dat_roms WHERE dat_id = ?1)", [id])?;
            tx.execute("DELETE FROM index_meta WHERE key = ?1", [format!("places:{id}")])?;
            tx.commit()?;
            changed = true;
        }
    }
    conn.execute("DELETE FROM index_meta WHERE key LIKE 'places:%' AND CAST(substr(key, 8) AS INTEGER) NOT IN (SELECT id FROM dats)", [])?;
    Ok(changed)
}

/// After a DAT is loaded: an arcade DAT's places, in its system's set type.
pub fn loaded(conn: &Connection, lib: &Library, dat_id: i64, platform: &str) -> Result<()> {
    if platforms::is_arcade(platform) {
        write_places(conn, dat_id, &set_type(lib, platform))?;
    }
    Ok(())
}

/// The machines' containers a set of `kind` needs for `games` (by name) to run: their
/// own ZIPs (or their parents', merged), their parents' in a split or merged set, their
/// BIOS sets, devices, and the folders of the disks they use.
pub fn containers_for(ms: &Machines, games: &[String], kind: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    for g in games {
        let Some(&i) = ms.by_name.get(g) else { continue };
        let mut with = vec![i];
        if kind != "non-merged" {
            let mut at = i;
            while let Some(p) = ms.list[at].parent.filter(|p| !ms.list[*p].is_support() && *p != at) {
                with.push(p);
                at = p;
                if with.len() > 8 {
                    break;
                }
            }
        }
        with.extend(ms.needs(i).into_iter().map(|(j, _)| j));
        for &j in &with {
            out.insert(if kind == "merged" && !ms.list[j].is_support() { ms.list[ms.top(j)].name.clone() } else { ms.list[j].name.clone() });
            for (ri, _) in ms.list[j].roms.iter().enumerate().filter(|(_, r)| r.disk) {
                out.insert(ms.list[ms.origin(j, ri).0].name.clone());
            }
        }
    }
    out
}

// ------------------------------------------------------------ playable

/// What each game of an arcade DAT is waiting for: `{ game id: { needs: [{name, why}],
/// samples: [names] } }` for those missing something, and the playable count. Kept with
/// the summaries (cleared when anything matched changes).
pub fn needs(conn: &Connection, dat_id: i64) -> Result<Value> {
    let key = format!("summary:arcade:{dat_id}");
    if let Some(v) = conn
        .query_row("SELECT value FROM index_meta WHERE key = ?1", [&key], |r| r.get::<_, String>(0))
        .optional()?
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
    {
        return Ok(v);
    }
    let ms = Machines::load(conn, dat_id)?;
    let mut status: HashMap<i64, String> = HashMap::new();
    {
        let mut q = conn.prepare(&format!("WITH {} SELECT id, status FROM gs", crate::audit::MACHINES))?;
        let mut rows = q.query([dat_id])?;
        while let Some(r) = rows.next()? {
            status.insert(r.get(0)?, r.get(1)?);
        }
    }
    let had = |j: usize| status.get(&ms.list[j].id).is_some_and(|s| s == "have");
    // samples: (the set's name, the sound's name) of every .wav found in a ZIP
    let mut wavs: HashSet<(String, String)> = HashSet::new();
    let mut wanted: Vec<(i64, String, String)> = vec![];
    {
        let mut q = conn.prepare(
            "SELECT g.id, COALESCE(g.sampleof, g.name), l.name FROM dat_links l JOIN dat_games g ON g.id = l.game_id WHERE g.dat_id = ?1 AND l.kind = 'sample' ORDER BY g.id, l.rowid",
        )?;
        wanted = q.query_map([dat_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
    }
    if !wanted.is_empty() {
        let mut q = conn.prepare(
            "SELECT lower(p.name), lower(f.name) FROM files f JOIN files p ON p.id = f.parent_id
             WHERE p.kind IN ('zip', '7z') AND lower(f.name) LIKE '%.wav' AND f.missing_since IS NULL",
        )?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            let (zip, wav): (String, String) = (r.get(0)?, r.get(1)?);
            let set = zip.rsplit_once('.').map(|(s, _)| s.to_string()).unwrap_or(zip);
            let wav = wav.rsplit('/').next().unwrap_or(&wav).to_string();
            wavs.insert((set, wav));
        }
    }
    let mut missing_samples: HashMap<i64, Vec<String>> = HashMap::new();
    for (g, set, name) in wanted {
        if !wavs.contains(&(set.to_ascii_lowercase(), format!("{}.wav", name.to_ascii_lowercase()))) {
            missing_samples.entry(g).or_default().push(name);
        }
    }
    let mut games = Map::new();
    let mut playable = 0i64;
    for (i, m) in ms.list.iter().enumerate() {
        if m.is_support() || !status.contains_key(&m.id) {
            continue;
        }
        let needs: Vec<Value> = ms.needs(i).into_iter().filter(|(j, _)| !had(*j)).map(|(j, why)| json!({ "name": ms.list[j].name, "why": why })).collect();
        if had(i) && needs.is_empty() {
            playable += 1;
        }
        let samples = missing_samples.remove(&m.id).unwrap_or_default();
        if !needs.is_empty() || !samples.is_empty() {
            games.insert(m.id.to_string(), json!({ "needs": needs, "samples": samples }));
        }
    }
    let support: Vec<&Machine> = ms.list.iter().filter(|m| m.is_support() && status.contains_key(&m.id)).collect();
    let v = json!({
        "playable": playable, "games": games,
        "sets": support.len(), "sets_have": support.iter().filter(|m| status.get(&m.id).is_some_and(|s| s == "have")).count(),
    });
    conn.execute(
        "INSERT INTO index_meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, v.to_string()],
    )?;
    Ok(v)
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A small arcade DAT: a BIOS set, a parent and its clone (sharing a ROM, one of their
    /// own each, a ROM of the same name with other contents), a game on the BIOS with a
    /// device, a disk game and its clone, and a game with samples.
    pub const DAT: &str = r#"<?xml version="1.0"?>
<mame build="0.270 (mame0270)">
  <machine name="bios1" isbios="yes"><description>Test BIOS</description>
    <rom name="bios.bin" size="16" crc="0b0b0b0b" sha1="0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b"/></machine>
  <machine name="dev1" isdevice="yes" runnable="no"><description>Test device</description>
    <rom name="dev.bin" size="16" crc="0d0d0d0d" sha1="0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d"/></machine>
  <machine name="z80" isdevice="yes" runnable="no"><description>Z80</description></machine>
  <machine name="alpha"><description>Alpha Fighter (World)</description><year>1991</year><manufacturer>Acme</manufacturer>
    <rom name="a.1" size="16" crc="0a0a0a01" sha1="0a0a0a010a0a0a010a0a0a010a0a0a010a0a0a01"/>
    <rom name="shared.2" size="16" crc="0a0a0a02" sha1="0a0a0a020a0a0a020a0a0a020a0a0a020a0a0a02"/>
    <rom name="same.3" size="16" crc="0a0a0a03" sha1="0a0a0a030a0a0a030a0a0a030a0a0a030a0a0a03"/>
    <device_ref name="z80"/></machine>
  <machine name="alphaj" cloneof="alpha" romof="alpha"><description>Alpha Fighter (Japan)</description>
    <rom name="aj.1" size="16" crc="0a0b0b01" sha1="0a0b0b010a0b0b010a0b0b010a0b0b010a0b0b01"/>
    <rom name="shared.2" merge="shared.2" size="16" crc="0a0a0a02" sha1="0a0a0a020a0a0a020a0a0a020a0a0a020a0a0a02"/>
    <rom name="same.3" size="16" crc="0a0b0b03" sha1="0a0b0b030a0b0b030a0b0b030a0b0b030a0b0b03"/></machine>
  <machine name="beta" romof="bios1"><description>Beta Quest</description>
    <rom name="bios.bin" merge="bios.bin" bios="x" size="16" crc="0b0b0b0b" sha1="0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b"/>
    <rom name="b.1" size="16" crc="0bbb0001" sha1="0bbb00010bbb00010bbb00010bbb00010bbb0001"/>
    <device_ref name="dev1"/></machine>
  <machine name="gamma"><description>Gamma Disk</description>
    <rom name="g.1" size="16" crc="0c0c0c01" sha1="0c0c0c010c0c0c010c0c0c010c0c0c010c0c0c01"/>
    <disk name="gamma" sha1="1111111111111111111111111111111111111111"/></machine>
  <machine name="gammab" cloneof="gamma" romof="gamma"><description>Gamma Disk (bootleg)</description>
    <rom name="gb.1" size="16" crc="0c0c0c02" sha1="0c0c0c020c0c0c020c0c0c020c0c0c020c0c0c02"/>
    <disk name="gamma" merge="gamma" sha1="1111111111111111111111111111111111111111"/></machine>
  <machine name="delta" sampleof="delta"><description>Delta Sounds</description>
    <rom name="d.1" size="16" crc="0d0e0d01" sha1="0d0e0d010d0e0d010d0e0d010d0e0d010d0e0d01"/>
    <sample name="boom"/><sample name="zap"/></machine>
</mame>
"#;

    fn load_dat() -> (crate::index::Index, i64) {
        let d = crate::library::tests::temp_dir("arcade-places");
        let ix = crate::index::Index::open(&d.join("ix.sqlite"), "lib-test").unwrap();
        let dat = crate::dat::parse(DAT.as_bytes()).unwrap();
        let id = crate::dat::tests::load_for_test(ix.conn(), "mame", &dat);
        (ix, id)
    }

    fn set(ms: &Machines, kind: &str) -> Vec<(String, String)> {
        let names: HashMap<i64, (String, String)> = ms.list.iter().flat_map(|m| m.roms.iter().map(move |r| (r.id, (m.name.clone(), r.name.clone())))).collect();
        let mut v: Vec<(String, String)> = places(ms, kind).into_iter().map(|p| (format!("{} {}", names[&p.rom_id].0, names[&p.rom_id].1), format!("{}:{}", p.container, p.member))).collect();
        v.sort();
        v
    }

    #[test]
    fn the_dat_reads_as_mames_with_its_bios_devices_disks_and_samples() {
        let d = crate::dat::parse(DAT.as_bytes()).unwrap();
        assert_eq!((d.name.as_str(), d.version.as_str()), ("MAME", "0.270"));
        let beta = d.games.iter().find(|g| g.name == "beta").unwrap();
        assert_eq!(beta.romof.as_deref(), Some("bios1"));
        assert_eq!(beta.devices, ["dev1"]);
        assert_eq!(beta.roms[0].merge.as_deref(), Some("bios.bin"));
        let gamma = d.games.iter().find(|g| g.name == "gamma").unwrap();
        assert!(gamma.roms[1].disk && gamma.roms[1].name == "gamma.chd" && gamma.roms[1].sha1.is_some());
        assert_eq!(d.games.iter().find(|g| g.name == "delta").unwrap().samples, ["boom", "zap"]);
        assert_eq!(d.games[0].kind.as_deref(), Some("bios"));
        assert_eq!(d.games.iter().find(|g| g.name == "alpha").unwrap().year.as_deref(), Some("1991"));
    }

    #[test]
    fn each_set_type_puts_the_roms_where_mame_looks() {
        let (ix, id) = load_dat();
        let ms = Machines::load(ix.conn(), id).unwrap();
        let non = set(&ms, "non-merged");
        let has = |v: &[(String, String)], a: &str, b: &str| v.iter().any(|(x, y)| x == a && y == b);
        // non-merged: the clone has the shared ROM; the BIOS's ROM stays in the BIOS set
        assert!(has(&non, "alphaj shared.2", "alphaj:shared.2"));
        assert!(has(&non, "beta bios.bin", "bios1:bios.bin"));
        assert!(has(&non, "gammab gamma.chd", "gamma:gamma.chd"), "a clone's disk is in its parent's folder");
        let split = set(&ms, "split");
        assert!(has(&split, "alphaj shared.2", "alpha:shared.2"));
        assert!(has(&split, "alphaj aj.1", "alphaj:aj.1"));
        assert!(has(&split, "alphaj same.3", "alphaj:same.3"));
        let merged = set(&ms, "merged");
        assert!(has(&merged, "alphaj aj.1", "alpha:aj.1"));
        assert!(has(&merged, "alphaj same.3", "alpha:alphaj/same.3"), "a name the parent uses for other contents");
        assert!(has(&merged, "alpha same.3", "alpha:same.3"));
        assert!(has(&merged, "bios1 bios.bin", "bios1:bios.bin"));
        assert!(has(&merged, "dev1 dev.bin", "dev1:dev.bin"));
        // what a game needs
        let need = |g: &str| { let mut v: Vec<String> = ms.needs(ms.by_name[g]).into_iter().map(|(j, w)| format!("{w} {}", ms.list[j].name)).collect(); v.sort(); v };
        assert_eq!(need("beta"), ["bios bios1", "device dev1"]);
        assert!(need("alpha").is_empty(), "z80 has no ROMs");
        assert!(need("alphaj").is_empty(), "the DAT lists the ROMs it shares with its parent");
        let mut c: Vec<String> = containers_for(&ms, &["beta".into(), "gammab".into()], "non-merged").into_iter().collect();
        c.sort();
        assert_eq!(c, ["beta", "bios1", "dev1", "gamma", "gammab"]);
        let mut c: Vec<String> = containers_for(&ms, &["alphaj".into()], "merged").into_iter().collect();
        c.sort();
        assert_eq!(c, ["alpha"]);
    }

    /// A set on disk, scanned, audited, then organised as each set type.
    #[test]
    fn an_arcade_set_is_audited_playable_and_organised_as_its_set_type() {
        use crate::audit::tests::{rom, rom_xml, write, zip};
        use std::sync::atomic::AtomicBool;
        let dir = crate::library::tests::temp_dir("arcade-e2e");
        let lib = Library::open(dir.join("Lib")).unwrap();
        let ix = crate::index::Index::open(&dir.join("index.sqlite"), "lib-x").unwrap();
        let roms = dir.join("Roms/MAME");
        let b = |n: u32| rom(1000 + n, 64);
        let chd = std::fs::read(crate::image::tests::fixtures().join("Disc Game (USA).chd")).unwrap();
        write(&roms.join("gamma/gamma.chd"), &chd);
        let disk_sha1 = crate::image::chd_sha1(&roms.join("gamma/gamma.chd")).unwrap();
        zip(&roms.join("alpha.zip"), &[("a.1", &b(1)), ("shared.2", &b(2)), ("same.3", &b(3))]);
        zip(&roms.join("alphaj.zip"), &[("aj.1", &b(4)), ("shared.2", &b(2)), ("same.3", &b(5))]);
        zip(&roms.join("bios1.zip"), &[("bios.bin", &b(6))]);
        zip(&roms.join("beta.zip"), &[("b.1", &b(7))]);
        zip(&roms.join("gamma.zip"), &[("g.1", &b(8))]);
        zip(&roms.join("delta.zip"), &[("d.1", &b(9))]);
        zip(&roms.join("samples/delta.zip"), &[("boom.wav", b"RIFF")]);
        let r = |n: &str, x: u32| rom_xml(n, &b(x));
        let merge = |n: &str, x: u32| rom_xml(n, &b(x)).replace("<rom ", &format!("<rom merge=\"{n}\" "));
        let dat = format!(
            r#"<?xml version="1.0"?><mame build="0.271 (mame0271)">
<machine name="bios1" isbios="yes"><description>Test BIOS</description>{}</machine>
<machine name="dev1" isdevice="yes"><description>Device</description>{}</machine>
<machine name="alpha"><description>Alpha Fighter (World)</description><year>1991</year>{}{}{}</machine>
<machine name="alphaj" cloneof="alpha" romof="alpha"><description>Alpha Fighter (Japan)</description>{}{}{}</machine>
<machine name="beta" romof="bios1"><description>Beta Quest</description>{}{}<device_ref name="dev1"/></machine>
<machine name="gamma"><description>Gamma Disk</description>{}<disk name="gamma" sha1="{disk_sha1}"/></machine>
<machine name="delta" sampleof="delta"><description>Delta Sounds</description>{}<sample name="boom"/><sample name="zap"/></machine>
</mame>"#,
            r("bios.bin", 6), r("dev.bin", 10), r("a.1", 1), r("shared.2", 2), r("same.3", 3),
            r("aj.1", 4), merge("shared.2", 2), r("same.3", 5), merge("bios.bin", 6), r("b.1", 7), r("g.1", 8), r("d.1", 9)
        );
        write(&dir.join("dats/mame.xml"), dat.as_bytes());
        crate::roots::add(&lib, &dir.join("Roms"), None).unwrap();
        crate::scan::scan(&ix, &lib, &crate::scan::Options { full: false, readers: 2, only: None }, &AtomicBool::new(false), &mut |_| {}).unwrap();
        let imported = crate::dat::import(&ix, &lib, &dir.join("dats/mame.xml"), &Map::new()).unwrap();
        let id = match &imported[0] {
            crate::dat::Imported::Loaded(v) => {
                assert_eq!(v["platform"], "mame");
                v["id"].as_i64().unwrap()
            }
            _ => panic!("MAME's DAT is found by its name"),
        };
        let sum = crate::audit::summary(ix.conn(), id).unwrap();
        // the BIOS set and the device aren't games; every game is had, and all but beta (no dev1) play
        assert_eq!((sum["games"].as_i64(), sum["have"].as_i64()), (Some(5), Some(5)), "{sum}");
        assert_eq!(sum["arcade"]["playable"], 4, "{sum}");
        assert_eq!((sum["arcade"]["sets"].as_i64(), sum["arcade"]["sets_have"].as_i64()), (Some(2), Some(1)));
        assert_eq!(sum["misnamed"], 0, "a non-merged set is named as it should be: {sum}");
        // review H11: the clone's own copy of its parent's ROM isn't a duplicate
        assert_eq!(sum["duplicates"], 0, "{sum}");
        assert_eq!(sum["unknown"], 1, "the sample (samples aren't in the DAT's ROMs): {sum}");
        let have = crate::audit::list(&ix, &lib, id, "have", "", 0, 50).unwrap();
        let item = |n: &str| have["items"].as_array().unwrap().iter().find(|i| i["name"] == n).cloned().unwrap();
        assert_eq!(item("beta")["needs"], json!([{ "name": "dev1", "why": "device" }]));
        assert_eq!(item("delta")["samples"], json!(["zap"]));
        assert_eq!(item("alpha")["title"], "Alpha Fighter");
        assert_eq!(item("alpha")["year"], "1991");
        let sets = crate::audit::list(&ix, &lib, id, "sets", "", 0, 50).unwrap();
        assert_eq!(sets["total"], 2);

        // review M11: a DAT loaded before arcade systems had places gets them on the next scan
        let places = |ix: &crate::index::Index| ix.conn().query_row("SELECT COUNT(*) FROM dat_places", [], |r| r.get::<_, i64>(0)).unwrap();
        let had = places(&ix);
        ix.conn().execute("DELETE FROM dat_places", []).unwrap();
        ix.conn().execute("DELETE FROM index_meta WHERE key LIKE 'places:%'", []).unwrap();
        crate::dat::sync_library(&ix, &lib).unwrap();
        assert_eq!(places(&ix), had);
        assert_eq!(crate::audit::summary(ix.conn(), id).unwrap()["misnamed"], 0);
        // and a set type changed on another computer is noticed (split: below)
        lib.update_meta(json!({ "arcade_sets": { "mame": "split" } })).unwrap();
        crate::dat::sync_library(&ix, &lib).unwrap();
        assert_eq!(crate::audit::summary(ix.conn(), id).unwrap()["misnamed"], 1);
        lib.update_meta(json!({ "arcade_sets": { "mame": "non-merged" } })).unwrap();
        crate::dat::sync_library(&ix, &lib).unwrap();
        assert_eq!(crate::audit::summary(ix.conn(), id).unwrap()["misnamed"], 0);

        // split: the clone's copy of its parent's ROM is in the wrong ZIP now
        set_set_type(ix.conn(), &lib, "mame", "split").unwrap();
        let sum = crate::audit::summary(ix.conn(), id).unwrap();
        assert_eq!(sum["misnamed"], 1, "{sum}");
        let mis = crate::audit::list(&ix, &lib, id, "misnamed", "", 0, 50).unwrap();
        assert_eq!(mis["items"][0]["expected"], "alpha.zip › shared.2");

        // merged, organised into a folder: the clone in its parent's ZIP, the disk beside it
        set_set_type(ix.conn(), &lib, "mame", "merged").unwrap();
        let out = dir.join("Sorted");
        let plan = crate::organise::plan(&ix, &lib, &crate::organise::Options {
            dat_id: id, mode: "copy".into(), out: Some(out.clone()), format: "zip".into(), ..Default::default()
        }).unwrap();
        crate::organise::run(&ix, &lib, &plan, &AtomicBool::new(false), &mut |_| {}).unwrap();
        let members = |z: &str| {
            let l = crate::archive::read(&out.join(z), "zip", false, &AtomicBool::new(false)).unwrap();
            let mut v: Vec<String> = l.members.iter().map(|m| m.name.clone()).collect();
            v.sort();
            v
        };
        assert_eq!(members("alpha.zip"), ["a.1", "aj.1", "alphaj/same.3", "same.3", "shared.2"]);
        assert_eq!(members("beta.zip"), ["b.1"], "the BIOS's ROM stays in its own set");
        assert_eq!(members("bios1.zip"), ["bios.bin"]);
        assert!(!out.join("alphaj.zip").exists());
        assert!(out.join("gamma/gamma.chd").is_file());

        // an export's games are non-merged, with the sets they need
        let only = crate::organise::plan(&ix, &lib, &crate::organise::Options {
            dat_id: id, mode: "copy".into(), out: Some(dir.join("Card")), format: "keep".into(),
            games: Some(vec!["alphaj".into(), "beta".into()]), set_type: Some("non-merged".into()), ..Default::default()
        }).unwrap();
        let mut to: Vec<String> = only.ops.iter().map(|o| o.to.rsplit(['/', '\\']).next().unwrap().to_string()).collect();
        to.sort();
        assert_eq!(to, ["alphaj.zip", "beta.zip", "bios1.zip"]);
    }
}
