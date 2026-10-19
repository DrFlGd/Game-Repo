//! Disc images read as the files they hold (docs/PLAN.md, "2b", Compressed images),
//! so a scan lists them like archives and the audit matches what's inside:
//!
//! - **CHD**: a CD's tracks, each as the `.bin` Redump lists (2,352-byte sectors, or
//!   2,048 for a MODE1/2048 track, without CHD's subcode and padding, audio in the
//!   bin's byte order); a DVD as its `.iso`. Read with the `chd` crate.
//! - **CSO / ZSO** (PSP): the `.iso`, unpacked as it's read (deflate or LZ4 blocks).
//! - **RVZ** (GameCube, Wii): the `.iso`'s hashes as `dolphin-tool verify` reports
//!   them; without the tool it's left to be checked when it's found.
//!
//! It also writes CHD (CD from a cue sheet, GD-ROM from a GDI, DVD from an ISO) and
//! CSO/ZSO, for Convert.

use crate::archive::{Listing, Member};
use crate::cue;
use crate::hash::{self, Hashes};
use chd::Chd;
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// The kinds of image, as `files.kind` names them (the extension, lower case).
pub const KINDS: &[&str] = &["chd", "cso", "zso", "rvz"];

/// The image kind of a file name, if it's one.
pub fn kind_of(name: &str) -> Option<&'static str> {
    let ext = crate::platforms::ext_of(name);
    KINDS.iter().copied().find(|k| *k == ext)
}

fn other(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}

fn stopped() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "stopped")
}

/// The image's name without its extension, for the single file it holds.
fn stem(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "image".into())
}

/// What's inside the image at `path` of `kind`, hashed. An RVZ when `dolphin-tool`
/// isn't found is an error of kind `NotFound` (the scan leaves it to be checked later).
pub fn read(path: &Path, kind: &str, cancel: &AtomicBool) -> io::Result<Listing> {
    if kind == "rvz" {
        return read_rvz(path, cancel);
    }
    // a hard disk or laserdisc (an arcade machine's) is matched by its own SHA1, not unpacked
    if kind == "chd" && matches!(ChdImage::open(path)?.layout, Layout::Raw) {
        return Ok(Listing { members: vec![], locked: false });
    }
    let mut members = vec![];
    each(path, kind, cancel, &mut |name, size, r| {
        let hashes = hash::hash_reader(r, size, name, cancel)?;
        members.push(Member { name: name.to_string(), hashes, full: true });
        Ok(())
    })?;
    Ok(Listing { members, locked: false })
}

/// Each file the image holds in turn, with its bytes: `f(name, size, reader)`.
pub fn each(path: &Path, kind: &str, cancel: &AtomicBool, f: &mut dyn FnMut(&str, u64, &mut dyn Read) -> io::Result<()>) -> io::Result<()> {
    each_wanted(path, kind, &|_| true, cancel, f)
}

/// The same for the files `want` names only (a CHD's other tracks aren't unpacked).
pub fn each_wanted(path: &Path, kind: &str, want: &dyn Fn(&str) -> bool, cancel: &AtomicBool, f: &mut dyn FnMut(&str, u64, &mut dyn Read) -> io::Result<()>) -> io::Result<()> {
    match kind {
        "chd" => {
            let mut c = ChdImage::open(path)?;
            match c.layout.clone() {
                Layout::Cd(tracks) => {
                    let mut frame = 0u64;
                    for t in &tracks {
                        if cancel.load(Ordering::Relaxed) {
                            return Err(stopped());
                        }
                        let name = track_name(t.number);
                        if want(&name) {
                            let size = t.frames * t.data_size() as u64;
                            let mut r = TrackReader::new(&mut c, frame, t);
                            f(&name, size, &mut r)?;
                        }
                        frame += t.frames + t.pad;
                    }
                    Ok(())
                }
                Layout::Dvd | Layout::Raw => {
                    let ext = if matches!(c.layout, Layout::Dvd) { "iso" } else { "img" };
                    let size = c.chd.header().logical_bytes();
                    let mut r = ChdBytes { c: &mut c, at: 0, end: size };
                    let name = format!("{}.{ext}", stem(path));
                    if !want(&name) {
                        return Ok(());
                    }
                    f(&name, size, &mut r)
                }
            }
        }
        "cso" | "zso" => {
            let mut r = CsoReader::open(path)?;
            let size = r.total;
            let name = format!("{}.iso", stem(path));
            if !want(&name) {
                return Ok(());
            }
            f(&name, size, &mut r)
        }
        _ => Err(other(format!("{kind} images can't be read without a tool"))),
    }
}

// ------------------------------------------------------------ CHD

/// A CHD's own SHA1 (of its data and metadata), as MAME's DATs list a disk.
pub fn chd_sha1(path: &Path) -> Option<String> {
    ChdImage::open(path).ok()?.sha1()
}

/// The name a CD track has as a member of its CHD.
pub fn track_name(number: u32) -> String {
    format!("Track {number:02}.bin")
}

/// A CD track as CHD's metadata describes it.
#[derive(Clone, Debug, PartialEq)]
pub struct ChdTrack {
    pub number: u32,
    /// MODE1, MODE1_RAW, MODE2, MODE2_FORM1, MODE2_FORM2, MODE2_FORM_MIX, MODE2_RAW, AUDIO
    pub kind: String,
    /// Frames stored, the pregap included when it's in the data (PGTYPE starting with V)
    pub frames: u64,
    pub pregap: u64,
    pub pgtype: String,
    pub postgap: u64,
    /// Empty frames after it (CHD pads each track to 4 frames; a GD-ROM's also hold the
    /// gap before the next track)
    pub pad: u64,
    /// The disc is a GD-ROM (CHGD metadata, from a GDI): `lba` is where the track starts
    pub gd: bool,
    pub lba: u64,
}

impl ChdTrack {
    /// Bytes of each frame that are the track's (what its `.bin` holds per sector).
    pub fn data_size(&self) -> usize {
        match self.kind.as_str() {
            "MODE1" | "MODE2_FORM1" => 2048,
            "MODE2" | "MODE2_FORM_MIX" => 2336,
            "MODE2_FORM2" => 2324,
            _ => 2352,
        }
    }

    pub fn is_audio(&self) -> bool {
        self.kind == "AUDIO"
    }
}

#[derive(Clone, Debug)]
pub enum Layout {
    Cd(Vec<ChdTrack>),
    Dvd,
    Raw,
}

const fn tag(t: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*t)
}

const CHT2: u32 = tag(b"CHT2");
const CHTR: u32 = tag(b"CHTR");
const CHGD: u32 = tag(b"CHGD");
const DVD: u32 = tag(b"DVD ");
/// A CD frame in a CHD: the sector and its subcode.
pub const FRAME: u64 = 2352 + 96;

/// A track from metadata text: `TRACK:1 TYPE:MODE1_RAW SUBTYPE:NONE FRAMES:45 …`.
fn parse_track(text: &str, gd: bool) -> Option<ChdTrack> {
    let get = |k: &str| {
        text.trim_end_matches('\0')
            .split_whitespace()
            .find_map(|w| w.strip_prefix(k).and_then(|v| v.strip_prefix(':')))
            .map(str::to_string)
    };
    let num = |k: &str| get(k).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    // a GD-ROM's FRAMES runs to the next track's LBA: its file's frames, then PAD empty ones
    let all = num("FRAMES");
    let frames = if gd { all.saturating_sub(num("PAD")) } else { all };
    Some(ChdTrack {
        number: get("TRACK")?.parse().ok()?,
        kind: get("TYPE")?,
        frames,
        pregap: num("PREGAP"),
        pgtype: get("PGTYPE").unwrap_or_default(),
        postgap: num("POSTGAP"),
        pad: all.div_ceil(4) * 4 - frames,
        gd,
        lba: if gd { all } else { 0 },
    })
}

/// A CHD open for reading by byte offset, a hunk at a time.
pub struct ChdImage {
    chd: Chd<BufReader<std::fs::File>>,
    pub layout: Layout,
    cmp: Vec<u8>,
    hunk: Vec<u8>,
    loaded: Option<u32>,
}

impl ChdImage {
    pub fn open(path: &Path) -> io::Result<Self> {
        let f = BufReader::new(std::fs::File::open(path)?);
        let mut chd = Chd::open(f, None).map_err(|e| other(format!("not a CHD that can be read ({e})")))?;
        let metas: Vec<chd::metadata::Metadata> = chd.metadata_refs().try_into().map_err(other)?;
        let mut tracks = vec![];
        let mut dvd = false;
        for m in &metas {
            let text = String::from_utf8_lossy(&m.value);
            match m.metatag {
                CHT2 | CHTR => tracks.extend(parse_track(&text, false)),
                CHGD => tracks.extend(parse_track(&text, true)),
                DVD => dvd = true,
                _ => {}
            }
        }
        tracks.sort_by_key(|t| t.number);
        // each GD-ROM track starts where the ones before it end (parse_track left its length in lba)
        let mut lba = 0;
        for t in tracks.iter_mut().filter(|t| t.gd) {
            (t.lba, lba) = (lba, lba + t.lba);
        }
        let layout = if !tracks.is_empty() {
            Layout::Cd(tracks)
        } else if dvd {
            Layout::Dvd
        } else {
            Layout::Raw
        };
        // a damaged header's hunk size would divide by nought, or ask for any amount of memory
        let hs = chd.header().hunk_size();
        if hs == 0 || hs > 1 << 26 {
            return Err(other(format!("its hunk size ({hs} bytes) isn't one a CHD has")));
        }
        let hunk = chd.get_hunksized_buffer();
        Ok(Self { chd, layout, cmp: vec![], hunk, loaded: None })
    }

    /// The CHD's own SHA1 of its data and metadata, as `chdman info` shows it.
    pub fn sha1(&self) -> Option<String> {
        self.chd.header().sha1().map(hex::encode)
    }

    /// Bytes from `offset` of the CHD's data into `buf` (within one hunk or across several).
    fn read_at(&mut self, mut offset: u64, mut buf: &mut [u8]) -> io::Result<()> {
        let hs = self.hunk.len() as u64;
        while !buf.is_empty() {
            let n = (offset / hs) as u32;
            if self.loaded != Some(n) {
                self.loaded = None;
                let mut h = self.chd.hunk(n).map_err(other)?;
                h.read_hunk_in(&mut self.cmp, &mut self.hunk).map_err(|e| other(format!("hunk {n} can't be read ({e})")))?;
                self.loaded = Some(n);
            }
            let at = (offset % hs) as usize;
            let take = buf.len().min(self.hunk.len() - at);
            buf[..take].copy_from_slice(&self.hunk[at..at + take]);
            buf = &mut buf[take..];
            offset += take as u64;
        }
        Ok(())
    }
}

/// A CD track's bytes as its `.bin` holds them (or several tracks', one after another).
pub struct TrackReader<'a> {
    c: &'a mut ChdImage,
    /// (first frame, frames, bytes a frame, audio) of each track still to read
    spans: std::collections::VecDeque<(u64, u64, usize, bool)>,
    frame: u64,
    end: u64,
    size: usize,
    audio: bool,
    buf: Vec<u8>,
    pos: usize,
}

impl<'a> TrackReader<'a> {
    pub fn new(c: &'a mut ChdImage, first: u64, t: &ChdTrack) -> Self {
        Self::spans(c, vec![(first, t)])
    }

    pub fn spans(c: &'a mut ChdImage, tracks: Vec<(u64, &ChdTrack)>) -> Self {
        let spans = tracks.into_iter().map(|(first, t)| (first, t.frames, t.data_size(), t.is_audio())).collect();
        Self { c, spans, frame: 0, end: 0, size: 0, audio: false, buf: vec![], pos: 0 }
    }
}

impl Read for TrackReader<'_> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.pos == self.buf.len() {
            while self.frame >= self.end {
                let Some((first, frames, size, audio)) = self.spans.pop_front() else { return Ok(0) };
                (self.frame, self.end, self.size, self.audio) = (first, first + frames, size, audio);
            }
            // a hunk's worth of frames at a time
            let n = (self.end - self.frame).min(64) as usize;
            self.buf.clear();
            let mut frame = vec![0u8; FRAME as usize];
            for i in 0..n {
                self.c.read_at((self.frame + i as u64) * FRAME, &mut frame)?;
                let data = &mut frame[..self.size];
                if self.audio {
                    // CHD keeps CD audio big-endian; a .bin has it little-endian
                    for pair in data.chunks_exact_mut(2) {
                        pair.swap(0, 1);
                    }
                }
                self.buf.extend_from_slice(data);
            }
            self.frame += n as u64;
            self.pos = 0;
        }
        let n = out.len().min(self.buf.len() - self.pos);
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

/// A DVD (or other) CHD's bytes in order.
struct ChdBytes<'a> {
    c: &'a mut ChdImage,
    at: u64,
    end: u64,
}

impl Read for ChdBytes<'_> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let n = (out.len() as u64).min(self.end - self.at) as usize;
        if n > 0 {
            self.c.read_at(self.at, &mut out[..n])?;
            self.at += n as u64;
        }
        Ok(n)
    }
}

/// The hashes of each file a CD CHD was made from, read back out of it: each run of
/// tracks that came from one file (as `tracks` says) hashed together.
pub fn cd_file_hashes(chd: &Path, tracks: &[CdTrackSource], cancel: &AtomicBool) -> io::Result<Vec<(std::path::PathBuf, Hashes)>> {
    let mut c = ChdImage::open(chd)?;
    let Layout::Cd(stored) = c.layout.clone() else {
        return Err(other("it isn't a CD image"));
    };
    if stored.len() != tracks.len() {
        return Err(other(format!("it holds {} tracks, not {}", stored.len(), tracks.len())));
    }
    let mut firsts = vec![];
    let mut frame = 0u64;
    for t in &stored {
        firsts.push(frame);
        frame += t.frames + t.pad;
    }
    let mut out = vec![];
    let mut i = 0;
    while i < tracks.len() {
        let mut j = i;
        while j + 1 < tracks.len() && tracks[j + 1].path == tracks[i].path {
            j += 1;
        }
        let group: Vec<(u64, &ChdTrack)> = (i..=j).map(|k| (firsts[k], &stored[k])).collect();
        let size: u64 = group.iter().map(|(_, t)| t.frames * t.data_size() as u64).sum();
        let mut r = TrackReader::spans(&mut c, group);
        out.push((tracks[i].path.clone(), hash::hash_reader(&mut r, size, "", cancel)?));
        i = j + 1;
    }
    Ok(out)
}

/// The tracks of an ISO written as a CD: one MODE1/2048 data track.
pub fn iso_as_cd(iso: &Path) -> io::Result<Vec<CdTrackSource>> {
    let len = std::fs::metadata(iso)?.len();
    if len % 2048 != 0 {
        return Err(other("its size isn't a whole number of 2,048-byte sectors"));
    }
    let track = cue::Track { number: 1, mode: "MODE1/2048".into(), index0: None, index1: 0, pregap: 0, postgap: 0 };
    Ok(vec![CdTrackSource { path: iso.to_path_buf(), lba: None, offset: 0, frames: len / 2048, track, pregap_in_file: 0 }])
}

/// The cue sheet for a CD CHD's tracks, naming them `names` (one per track, in order);
/// for a GD-ROM, its GDI.
pub fn cue_for(chd: &Path, names: &[String]) -> io::Result<String> {
    let c = ChdImage::open(chd)?;
    let Layout::Cd(tracks) = &c.layout else {
        return Err(other("it isn't a CD image"));
    };
    if names.len() != tracks.len() {
        return Err(other(format!("it holds {} tracks, not {}", tracks.len(), names.len())));
    }
    if is_gd(tracks) {
        return Ok(gdi_text(tracks, names));
    }
    Ok(tracks.iter().zip(names).map(|(t, n)| cue_lines(t, n)).collect())
}

/// Whether a CHD's tracks are a GD-ROM's (its sheet is a GDI).
pub fn is_gd(tracks: &[ChdTrack]) -> bool {
    tracks.first().is_some_and(|t| t.gd)
}

/// When the CHD at `path` holds a GD-ROM: the members that are its audio tracks.
pub fn gd_audio(path: &Path) -> Option<std::collections::HashSet<String>> {
    let c = ChdImage::open(path).ok()?;
    match &c.layout {
        Layout::Cd(t) if is_gd(t) => Some(t.iter().filter(|t| t.is_audio()).map(|t| track_name(t.number)).collect()),
        _ => None,
    }
}

/// A GDI for a GD-ROM's tracks, as Redump writes them (names with spaces quoted, CRLF).
fn gdi_text(tracks: &[ChdTrack], names: &[String]) -> String {
    let mut s = format!("{}\r\n", tracks.len());
    for (t, n) in tracks.iter().zip(names) {
        let name = if n.contains(' ') { format!("\"{n}\"") } else { n.clone() };
        let kind = if t.is_audio() { 0 } else { 4 };
        s.push_str(&format!("{} {} {kind} {} {name} 0\r\n", t.number, t.lba, t.data_size()));
    }
    s
}

/// mm:ss:ff of a number of frames.
fn msf(f: u64) -> String {
    format!("{:02}:{:02}:{:02}", f / 4500, f / 75 % 60, f % 75)
}

/// A track's lines in a cue sheet, as Redump writes them.
fn cue_lines(t: &ChdTrack, file: &str) -> String {
    let mode = match t.kind.as_str() {
        "AUDIO" => "AUDIO",
        "MODE1" => "MODE1/2048",
        "MODE1_RAW" => "MODE1/2352",
        "MODE2" | "MODE2_FORM_MIX" => "MODE2/2336",
        "MODE2_FORM1" => "MODE2/2048",
        "MODE2_FORM2" => "MODE2/2324",
        _ => "MODE2/2352",
    };
    let mut s = format!("FILE \"{file}\" BINARY\r\n  TRACK {:02} {mode}\r\n", t.number);
    if t.pregap > 0 && t.pgtype.starts_with('V') {
        s.push_str(&format!("    INDEX 00 00:00:00\r\n    INDEX 01 {}\r\n", msf(t.pregap)));
    } else {
        if t.pregap > 0 {
            s.push_str(&format!("    PREGAP {}\r\n", msf(t.pregap)));
        }
        s.push_str("    INDEX 01 00:00:00\r\n");
    }
    if t.postgap > 0 {
        s.push_str(&format!("    POSTGAP {}\r\n", msf(t.postgap)));
    }
    s
}

// ------------------------------------------------------------ writing CHD

/// A track to write into a CD CHD: where its bytes are, and how the cue describes it.
pub struct CdTrackSource {
    pub path: std::path::PathBuf,
    /// From a GDI: where the track starts on the GD-ROM
    pub lba: Option<u64>,
    /// Where its bytes start in that file, and how many frames it has there
    pub offset: u64,
    pub frames: u64,
    pub track: cue::Track,
    /// Its pregap is in the file (INDEX 00 to INDEX 01)
    pub pregap_in_file: u64,
}

/// The tracks of the disc a cue sheet (or GDI) describes, with the files beside it.
pub fn cd_tracks(sheet: &Path) -> io::Result<Vec<CdTrackSource>> {
    let text = String::from_utf8_lossy(&std::fs::read(sheet)?).into_owned();
    let dir = sheet.parent().unwrap_or(Path::new("."));
    let mut out = vec![];
    if crate::platforms::ext_of(&sheet.to_string_lossy()) == "gdi" {
        for t in cue::gdi_tracks(&text) {
            let path = dir.join(&t.file);
            let len = std::fs::metadata(&path)?.len();
            let mode = match (t.kind, t.sector) {
                (0, _) => "AUDIO",
                (_, 2048) => "MODE1/2048",
                _ => "MODE1/2352",
            };
            let track = cue::Track { number: t.number, mode: mode.into(), index0: None, index1: 0, pregap: 0, postgap: 0 };
            out.push(CdTrackSource { path, lba: Some(t.lba), offset: 0, frames: len / t.sector, track, pregap_in_file: 0 });
        }
        return Ok(out);
    }
    for f in cue::parse(&text) {
        if f.kind != "BINARY" && f.kind != "MOTOROLA" {
            return Err(other(format!("{} is a {} file; only BINARY tracks can be converted", f.name, f.kind)));
        }
        let path = dir.join(&f.name);
        let len = std::fs::metadata(&path).map_err(|e| other(format!("{}: {e}", path.display())))?.len();
        for (i, t) in f.tracks.iter().enumerate() {
            let size = t.sector_size();
            let start = t.start();
            let end = f.tracks.get(i + 1).map(|n| n.start() * size).unwrap_or(len);
            let frames = (end.saturating_sub(start * size)) / size;
            let pregap_in_file = t.index0.map(|z| t.index1.saturating_sub(z)).unwrap_or(0);
            out.push(CdTrackSource { path: path.clone(), lba: None, offset: start * size, frames, track: t.clone(), pregap_in_file });
        }
    }
    Ok(out)
}

/// Write a CD CHD of these tracks (as `chdman createcd` does: 2,448-byte frames, each
/// track padded to 4 frames, audio big-endian, CHT2 metadata per track; from a GDI,
/// CHGD metadata, each track followed by empty frames up to the next one's LBA).
pub fn write_cd(tracks: &[CdTrackSource], to: &Path, cancel: &AtomicBool) -> io::Result<()> {
    use chd::write::{ChdWriteOptions, ChdWriter};
    let file = std::fs::File::create(to)?;
    let mut w = ChdWriter::new(io::BufWriter::new(file), ChdWriteOptions::cd()).map_err(other)?;
    // a GD-ROM's tracks: the empty frames after each, up to the next track
    let gaps: Option<Vec<u64>> = tracks.iter().all(|t| t.lba.is_some()).then(|| {
        tracks
            .iter()
            .enumerate()
            .map(|(i, t)| match (t.lba, tracks.get(i + 1).and_then(|n| n.lba)) {
                (Some(a), Some(b)) => b.saturating_sub(a).saturating_sub(t.frames),
                _ => 0,
            })
            .collect()
    });
    for (i, t) in tracks.iter().enumerate() {
        let kind = t.track.chd_type();
        if let Some(gaps) = &gaps {
            let meta = format!(
                "TRACK:{} TYPE:{kind} SUBTYPE:NONE FRAMES:{} PAD:{} PREGAP:0 PGTYPE:MODE1 PGSUB:NONE POSTGAP:0\0",
                t.track.number,
                t.frames + gaps[i],
                gaps[i]
            );
            w.add_metadata(CHGD, meta.as_bytes(), true).map_err(other)?;
            continue;
        }
        let (pregap, pgtype) = if t.pregap_in_file > 0 {
            (t.pregap_in_file, format!("V{kind}"))
        } else {
            (t.track.pregap, "MODE1".to_string())
        };
        let meta = format!(
            "TRACK:{} TYPE:{kind} SUBTYPE:NONE FRAMES:{} PREGAP:{pregap} PGTYPE:{pgtype} PGSUB:NONE POSTGAP:{}\0",
            t.track.number, t.frames, t.track.postgap
        );
        w.add_metadata(CHT2, meta.as_bytes(), true).map_err(other)?;
    }
    let mut frame = vec![0u8; FRAME as usize];
    for (i, t) in tracks.iter().enumerate() {
        let size = t.track.sector_size() as usize;
        let all = t.frames + gaps.as_ref().map_or(0, |g| g[i]);
        let mut f = BufReader::new(std::fs::File::open(&t.path)?);
        f.seek(SeekFrom::Start(t.offset))?;
        for _ in 0..t.frames {
            if cancel.load(Ordering::Relaxed) {
                return Err(stopped());
            }
            frame.fill(0);
            f.read_exact(&mut frame[..size])?;
            if t.track.is_audio() {
                for pair in frame[..size].chunks_exact_mut(2) {
                    pair.swap(0, 1);
                }
            }
            w.write_all(&frame)?;
        }
        frame.fill(0);
        for n in t.frames..all.div_ceil(4) * 4 {
            if n % 4096 == 0 && cancel.load(Ordering::Relaxed) {
                return Err(stopped());
            }
            w.write_all(&frame)?;
        }
    }
    let mut inner = w.finish().map_err(other)?;
    inner.flush()?;
    inner.into_inner().map_err(|e| e.into_error())?.sync_all()
}

/// Write a DVD CHD of an ISO (as `chdman createdvd` does).
pub fn write_dvd(iso: &Path, to: &Path, cancel: &AtomicBool) -> io::Result<()> {
    use chd::write::{ChdWriteOptions, ChdWriter};
    let file = std::fs::File::create(to)?;
    let mut w = ChdWriter::new(io::BufWriter::new(file), ChdWriteOptions::dvd()).map_err(other)?;
    w.add_metadata(DVD, &b"\0"[..], true).map_err(other)?;
    copy_stopping(&mut std::fs::File::open(iso)?, &mut w, cancel)?;
    let mut inner = w.finish().map_err(other)?;
    inner.flush()?;
    inner.into_inner().map_err(|e| e.into_error())?.sync_all()
}

fn copy_stopping(r: &mut dyn Read, w: &mut dyn Write, cancel: &AtomicBool) -> io::Result<u64> {
    let mut buf = vec![0u8; hash::BLOCK];
    let mut n = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(stopped());
        }
        let got = r.read(&mut buf)?;
        if got == 0 {
            return Ok(n);
        }
        w.write_all(&buf[..got])?;
        n += got as u64;
    }
}

/// Write a CHD's CD tracks back out as `<base> (Track N).bin` files and a cue sheet
/// naming them (a GD-ROM's as Redump does, with `.raw` audio and a GDI), into `dir`. Returns the files written (the cue first).
pub fn extract_cd(chd: &Path, dir: &Path, base: &str, cancel: &AtomicBool) -> io::Result<Vec<std::path::PathBuf>> {
    let mut c = ChdImage::open(chd)?;
    let Layout::Cd(tracks) = c.layout.clone() else {
        return Err(other("it isn't a CD image"));
    };
    let many = tracks.len() > 1;
    let gd = is_gd(&tracks);
    let mut cue_text = String::new();
    let mut names = vec![];
    let mut written = vec![];
    let mut frame = 0u64;
    for t in &tracks {
        let ext = if gd && t.is_audio() { "raw" } else { "bin" };
        let name = if many { format!("{base} (Track {}).{ext}", t.number) } else { format!("{base}.{ext}") };
        cue_text.push_str(&cue_lines(t, &name));
        names.push(name.clone());
        let p = dir.join(&name);
        let mut out = io::BufWriter::new(std::fs::File::create(&p)?);
        copy_stopping(&mut TrackReader::new(&mut c, frame, t), &mut out, cancel)?;
        out.into_inner().map_err(|e| e.into_error())?.sync_all()?;
        written.push(p);
        frame += t.frames + t.pad;
    }
    let cue_path = dir.join(format!("{base}.{}", if gd { "gdi" } else { "cue" }));
    std::fs::write(&cue_path, if gd { gdi_text(&tracks, &names) } else { cue_text })?;
    written.insert(0, cue_path);
    Ok(written)
}

/// Write the single file an image holds (a DVD CHD's, a CSO's or a ZSO's ISO) to `to`.
pub fn extract_whole(image: &Path, kind: &str, to: &Path, cancel: &AtomicBool) -> io::Result<()> {
    let mut done = false;
    each(image, kind, cancel, &mut |_, _, r| {
        let mut out = io::BufWriter::new(std::fs::File::create(to)?);
        copy_stopping(r, &mut out, cancel)?;
        out.into_inner().map_err(|e| e.into_error())?.sync_all()?;
        done = true;
        Ok(())
    })?;
    if !done {
        return Err(other("it holds nothing"));
    }
    Ok(())
}

// ------------------------------------------------------------ CSO / ZSO

/// A CSO (v1 or v2) or ZSO, unpacked block by block as it's read.
struct CsoReader {
    f: BufReader<std::fs::File>,
    total: u64,
    block: usize,
    align: u8,
    version: u8,
    zso: bool,
    index: Vec<u32>,
    at: u64,
    buf: Vec<u8>,
    pos: usize,
    raw: Vec<u8>,
}

impl CsoReader {
    fn open(path: &Path) -> io::Result<Self> {
        let mut f = BufReader::new(std::fs::File::open(path)?);
        let mut h = [0u8; 24];
        f.read_exact(&mut h)?;
        let zso = match &h[..4] {
            b"CISO" => false,
            b"ZISO" => true,
            _ => return Err(other("not a CSO or ZSO")),
        };
        let header = u32::from_le_bytes(h[4..8].try_into().unwrap()) as u64;
        let total = u64::from_le_bytes(h[8..16].try_into().unwrap());
        let block = u32::from_le_bytes(h[16..20].try_into().unwrap()) as usize;
        let (version, align) = (h[20], h[21]);
        if block == 0 || block > 1 << 24 {
            return Err(other("its block size isn't one a CSO has"));
        }
        let blocks = total.div_ceil(block as u64);
        // the index is in the file: a damaged header's total can't ask for more than that
        let len = f.get_ref().metadata()?.len();
        if (blocks + 1).saturating_mul(4).saturating_add(header.max(24)) > len {
            return Err(other("its header says it's bigger than the file can hold"));
        }
        let blocks = blocks as usize;
        f.seek(SeekFrom::Start(header.max(24)))?;
        let mut idx = vec![0u8; (blocks + 1) * 4];
        f.read_exact(&mut idx)?;
        let index = idx.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
        Ok(Self { f, total, block, align, version, zso, index, at: 0, buf: vec![], pos: 0, raw: vec![] })
    }

    fn load(&mut self, n: usize) -> io::Result<()> {
        let (a, b) = (self.index[n], self.index[n + 1]);
        let start = ((a & 0x7FFF_FFFF) as u64) << self.align;
        let end = ((b & 0x7FFF_FFFF) as u64) << self.align;
        let len = end.saturating_sub(start) as usize;
        let want = (self.total - n as u64 * self.block as u64).min(self.block as u64) as usize;
        self.raw.resize(len, 0);
        self.f.seek(SeekFrom::Start(start))?;
        self.f.read_exact(&mut self.raw)?;
        let flag = a & 0x8000_0000 != 0;
        // v1 CSO and ZSO: the flag means stored as it is; v2: stored when as big as a block, the flag means LZ4
        let (plain, lz4) = match (self.zso, self.version >= 2) {
            (true, _) => (flag, true),
            (false, false) => (flag, false),
            (false, true) => (len >= self.block, flag),
        };
        self.buf.clear();
        if plain {
            self.buf.extend_from_slice(&self.raw[..want.min(len)]);
        } else if lz4 {
            self.buf.resize(self.block, 0);
            let n = lz4_flex::block::decompress_into(&self.raw, &mut self.buf).map_err(|e| other(format!("block {n} doesn't unpack ({e})")))?;
            self.buf.truncate(n);
        } else {
            let mut d = flate2::read::DeflateDecoder::new(&self.raw[..]);
            d.by_ref().take(self.block as u64).read_to_end(&mut self.buf)?;
        }
        if self.buf.len() < want {
            return Err(other(format!("block {n} is short")));
        }
        self.buf.truncate(want);
        self.pos = 0;
        Ok(())
    }
}

impl Read for CsoReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.pos == self.buf.len() {
            if self.at >= self.total {
                return Ok(0);
            }
            self.load((self.at / self.block as u64) as usize)?;
        }
        let n = out.len().min(self.buf.len() - self.pos);
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        self.at += n as u64;
        Ok(n)
    }
}

/// Write an ISO as a CSO (v1, deflate) or ZSO (LZ4), 2,048-byte blocks, each kept
/// as it is when packing doesn't make it smaller (as maxcso does).
pub fn write_cso(iso: &Path, to: &Path, zso: bool, cancel: &AtomicBool) -> io::Result<()> {
    const BLOCK: usize = 2048;
    let total = std::fs::metadata(iso)?.len();
    let blocks = total.div_ceil(BLOCK as u64) as usize;
    let mut r = BufReader::new(std::fs::File::open(iso)?);
    let mut w = io::BufWriter::new(std::fs::File::create(to)?);
    let mut header = Vec::with_capacity(24);
    header.extend_from_slice(if zso { b"ZISO" } else { b"CISO" });
    header.extend_from_slice(&24u32.to_le_bytes());
    header.extend_from_slice(&total.to_le_bytes());
    header.extend_from_slice(&(BLOCK as u32).to_le_bytes());
    header.extend_from_slice(&[1, 0, 0, 0]);
    w.write_all(&header)?;
    let index_at = 24u64;
    w.write_all(&vec![0u8; (blocks + 1) * 4])?;
    let mut pos = index_at + (blocks as u64 + 1) * 4;
    let mut index = Vec::with_capacity(blocks + 1);
    let mut buf = vec![0u8; BLOCK];
    for n in 0..blocks {
        if cancel.load(Ordering::Relaxed) {
            return Err(stopped());
        }
        let want = (total - n as u64 * BLOCK as u64).min(BLOCK as u64) as usize;
        r.read_exact(&mut buf[..want])?;
        let packed = if zso {
            lz4_flex::block::compress(&buf[..want])
        } else {
            let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
            e.write_all(&buf[..want])?;
            e.finish()?
        };
        if pos > 0x7FFF_FFFF {
            return Err(other("it's too big for a CSO of this kind (over 2 GB packed)"));
        }
        if packed.len() < want {
            index.push(pos as u32);
            w.write_all(&packed)?;
            pos += packed.len() as u64;
        } else {
            index.push(pos as u32 | 0x8000_0000);
            w.write_all(&buf[..want])?;
            pos += want as u64;
        }
    }
    index.push(pos as u32);
    w.seek(SeekFrom::Start(index_at))?;
    for i in &index {
        w.write_all(&i.to_le_bytes())?;
    }
    w.into_inner().map_err(|e| e.into_error())?.sync_all()
}

// ------------------------------------------------------------ RVZ

/// The size of the disc an RVZ (or WIA) holds, from its header.
pub fn rvz_size(path: &Path) -> io::Result<u64> {
    let mut h = [0u8; 44];
    std::fs::File::open(path)?.read_exact(&mut h)?;
    if &h[..3] != b"RVZ" && &h[..3] != b"WIA" {
        return Err(other("not an RVZ"));
    }
    Ok(u64::from_be_bytes(h[36..44].try_into().unwrap()))
}

/// The hashes `dolphin-tool verify` prints: `CRC32: …`, `MD5: …`, `SHA1: …`.
pub fn parse_verify(out: &str) -> (Option<String>, Option<String>, Option<String>) {
    let mut found = (None, None, None);
    for line in out.lines() {
        let Some((k, v)) = line.split_once(':') else { continue };
        let v = v.trim().trim_start_matches("0x").to_ascii_lowercase();
        if v.is_empty() || !v.chars().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        match (k.trim().to_ascii_uppercase().as_str(), v.len()) {
            ("CRC32", 8) => found.0 = Some(v),
            ("MD5", 32) => found.1 = Some(v),
            ("SHA1" | "SHA-1", 40) => found.2 = Some(v),
            _ => {}
        }
    }
    found
}

fn read_rvz(path: &Path, cancel: &AtomicBool) -> io::Result<Listing> {
    let size = rvz_size(path)?;
    let tool = crate::tools::find("dolphin-tool").ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "dolphin-tool isn't found"))?;
    let mut child = crate::tools::command(&tool)
        .args(["verify", "-i"])
        .arg(path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    let out = wait_stopping(&mut child, cancel)?;
    let text = String::from_utf8_lossy(&out.stdout);
    let (crc32, md5, sha1) = parse_verify(&text);
    let Some(crc32) = crc32 else {
        return Err(other(format!("dolphin-tool didn't report its hashes: {}", String::from_utf8_lossy(&out.stderr).trim())));
    };
    let hashes = Hashes { size, crc32, md5, sha1, header: None, hl: None };
    Ok(Listing { members: vec![Member { name: format!("{}.iso", stem(path)), hashes, full: true }], locked: false })
}

/// Wait for a tool, killing it when the job is stopped.
pub fn wait_stopping(child: &mut std::process::Child, cancel: &AtomicBool) -> io::Result<std::process::Output> {
    use std::thread;
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let o = thread::spawn(move || {
        let mut v = vec![];
        if let Some(s) = stdout.as_mut() {
            let _ = s.read_to_end(&mut v);
        }
        v
    });
    let e = thread::spawn(move || {
        let mut v = vec![];
        if let Some(s) = stderr.as_mut() {
            let _ = s.read_to_end(&mut v);
        }
        v
    });
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(stopped());
        }
        thread::sleep(std::time::Duration::from_millis(50));
    };
    Ok(std::process::Output { status, stdout: o.join().unwrap_or_default(), stderr: e.join().unwrap_or_default() })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::library::tests::temp_dir;
    use serde_json::Value;
    use std::path::PathBuf;

    pub fn fixtures() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/discs")
    }

    /// The hashes make_discs.py wrote down for each file.
    pub fn expected() -> Value {
        serde_json::from_slice(&std::fs::read(fixtures().join("expected.json")).unwrap()).unwrap()
    }

    /// The source files, written by make_discs.py into a new folder.
    pub fn sources(name: &str) -> PathBuf {
        let dir = temp_dir(name);
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/make_discs.py");
        let ok = std::process::Command::new(if cfg!(windows) { "python" } else { "python3" })
            .arg(&script)
            .arg(&dir)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap()
            .success();
        assert!(ok, "make_discs.py failed");
        dir
    }

    fn check(listing: &Listing, want: &[(&str, &str)]) {
        let e = expected();
        assert_eq!(listing.members.len(), want.len());
        for (m, (name, file)) in listing.members.iter().zip(want) {
            assert_eq!(&m.name, name);
            assert_eq!(m.hashes.size, e[file]["size"].as_u64().unwrap(), "{file}");
            assert_eq!(m.hashes.crc32, e[file]["crc32"].as_str().unwrap(), "{file}");
            assert_eq!(m.hashes.sha1.as_deref(), e[file]["sha1"].as_str(), "{file}");
        }
    }

    // review H10: a damaged header is an error, never a crash
    #[test]
    fn a_damaged_cso_header_is_refused() {
        let dir = crate::library::tests::temp_dir("cso-damaged");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("bad.cso");
        let mut h = b"CISO".to_vec();
        h.extend_from_slice(&24u32.to_le_bytes());
        h.extend_from_slice(&(1u64 << 49).to_le_bytes());
        h.extend_from_slice(&2048u32.to_le_bytes());
        h.extend_from_slice(&[1, 0, 0, 0]);
        h.extend_from_slice(&[0u8; 64]);
        std::fs::write(&p, &h).unwrap();
        assert!(read(&p, "cso", &AtomicBool::new(false)).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_cd_chd_made_by_chdman_reads_as_its_bins() {
        let stop = AtomicBool::new(false);
        let l = read(&fixtures().join("Disc Game (USA).chd"), "chd", &stop).unwrap();
        check(&l, &[("Track 01.bin", "Disc Game (USA) (Track 1).bin"), ("Track 02.bin", "Disc Game (USA) (Track 2).bin")]);
    }

    #[test]
    fn a_dvd_chd_and_a_cso_read_as_the_iso() {
        let stop = AtomicBool::new(false);
        let l = read(&fixtures().join("Dvd Game (Europe).chd"), "chd", &stop).unwrap();
        check(&l, &[("Dvd Game (Europe).iso", "Dvd Game (Europe).iso")]);
        let l = read(&fixtures().join("Dvd Game (Europe).cso"), "cso", &stop).unwrap();
        check(&l, &[("Dvd Game (Europe).iso", "Dvd Game (Europe).iso")]);
    }

    #[test]
    fn chd_cso_and_zso_written_here_read_back() {
        let dir = sources("image-write");
        let stop = AtomicBool::new(false);
        let cue = dir.join("Disc Game (USA).cue");
        let tracks = cd_tracks(&cue).unwrap();
        assert_eq!(tracks.len(), 2);
        assert_eq!((tracks[1].frames, tracks[1].pregap_in_file), (31, 10));
        let chd = dir.join("cd.chd");
        write_cd(&tracks, &chd, &stop).unwrap();
        let l = read(&chd, "chd", &stop).unwrap();
        check(&l, &[("Track 01.bin", "Disc Game (USA) (Track 1).bin"), ("Track 02.bin", "Disc Game (USA) (Track 2).bin")]);
        // the same metadata chdman writes
        let c = ChdImage::open(&chd).unwrap();
        let theirs = ChdImage::open(&fixtures().join("Disc Game (USA).chd")).unwrap();
        match (&c.layout, &theirs.layout) {
            (Layout::Cd(a), Layout::Cd(b)) => assert_eq!(a, b),
            _ => panic!("not CDs"),
        }
        assert_eq!(c.chd.header().raw_sha1(), theirs.chd.header().raw_sha1(), "the same data as chdman's");
        assert_eq!(c.sha1(), theirs.sha1(), "and the same SHA1 with the metadata");
        // and back out to a cue and bins with the same bytes
        let out = dir.join("out");
        std::fs::create_dir_all(&out).unwrap();
        let files = extract_cd(&chd, &out, "Disc Game (USA)", &stop).unwrap();
        assert_eq!(files.len(), 3);
        for n in ["Disc Game (USA) (Track 1).bin", "Disc Game (USA) (Track 2).bin"] {
            assert_eq!(std::fs::read(out.join(n)).unwrap(), std::fs::read(dir.join(n)).unwrap(), "{n}");
        }
        assert_eq!(cue::parse(&std::fs::read_to_string(&files[0]).unwrap()), cue::parse(&std::fs::read_to_string(&cue).unwrap()));

        let iso = dir.join("Dvd Game (Europe).iso");
        let dvd = dir.join("dvd.chd");
        write_dvd(&iso, &dvd, &stop).unwrap();
        let theirs = ChdImage::open(&fixtures().join("Dvd Game (Europe).chd")).unwrap();
        assert_eq!(ChdImage::open(&dvd).unwrap().sha1(), theirs.sha1());
        let back = dir.join("back.iso");
        extract_whole(&dvd, "chd", &back, &stop).unwrap();
        assert_eq!(std::fs::read(&back).unwrap(), std::fs::read(&iso).unwrap());
        for (kind, zso) in [("cso", false), ("zso", true)] {
            let p = dir.join(format!("x.{kind}"));
            write_cso(&iso, &p, zso, &stop).unwrap();
            assert!(std::fs::metadata(&p).unwrap().len() < std::fs::metadata(&iso).unwrap().len());
            extract_whole(&p, kind, &back, &stop).unwrap();
            assert_eq!(std::fs::read(&back).unwrap(), std::fs::read(&iso).unwrap(), "{kind}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_gd_rom_from_a_gdi_is_written_as_chdman_writes_it_and_read_back() {
        let dir = sources("image-gd");
        let stop = AtomicBool::new(false);
        let names = ["Gd Game (Japan) (Track 1).bin", "Gd Game (Japan) (Track 2).raw", "Gd Game (Japan) (Track 3).bin"];
        // chdman's GD CHD reads as Redump's files, its gaps left out
        let theirs = fixtures().join("Gd Game (Japan).chd");
        let l = read(&theirs, "chd", &stop).unwrap();
        check(&l, &[("Track 01.bin", names[0]), ("Track 02.bin", names[1]), ("Track 03.bin", names[2])]);
        // ours, from the GDI, is the same CHD
        let gdi = dir.join("Gd Game (Japan).gdi");
        let chd = dir.join("gd.chd");
        write_cd(&cd_tracks(&gdi).unwrap(), &chd, &stop).unwrap();
        let (a, b) = (ChdImage::open(&chd).unwrap(), ChdImage::open(&theirs).unwrap());
        match (&a.layout, &b.layout) {
            (Layout::Cd(x), Layout::Cd(y)) => {
                assert_eq!(x, y);
                assert_eq!(x.iter().map(|t| t.lba).collect::<Vec<_>>(), [0, 180, 45000]);
            }
            _ => panic!("not CDs"),
        }
        assert_eq!(a.chd.header().raw_sha1(), b.chd.header().raw_sha1(), "the same data as chdman's");
        assert_eq!(a.sha1(), b.sha1(), "and the same SHA1 with the metadata");
        let hashes = cd_file_hashes(&chd, &cd_tracks(&gdi).unwrap(), &stop).unwrap();
        let e = expected();
        for ((p, h), n) in hashes.iter().zip(names) {
            assert_eq!((p.file_name().unwrap().to_str().unwrap(), h.sha1.as_deref()), (n, e[n]["sha1"].as_str()));
        }
        // and back out to Redump's GDI and tracks, byte for byte
        let names: Vec<String> = names.iter().map(|n| n.to_string()).collect();
        assert_eq!(cue_for(&chd, &names).unwrap().as_bytes(), std::fs::read(&gdi).unwrap());
        let out = dir.join("out");
        std::fs::create_dir_all(&out).unwrap();
        let files = extract_cd(&chd, &out, "Gd Game (Japan)", &stop).unwrap();
        assert_eq!(files.len(), 4);
        for n in names.iter().chain([&"Gd Game (Japan).gdi".to_string()]) {
            assert_eq!(std::fs::read(out.join(n)).unwrap(), std::fs::read(dir.join(n)).unwrap(), "{n}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn chds_written_here_pass_chdman_verify() {
        // only where chdman is installed; CI fetches it and sets GAMEREPO_TEST_CHDMAN, so
        // there the test can't pass without running (review M46)
        let Some(chdman) = crate::tools::which(&["chdman"]) else {
            assert!(std::env::var_os("GAMEREPO_TEST_CHDMAN").is_none(), "GAMEREPO_TEST_CHDMAN is set, but chdman isn't on the PATH");
            return;
        };
        let dir = sources("image-chdman");
        let stop = AtomicBool::new(false);
        let chd = dir.join("cd.chd");
        write_cd(&cd_tracks(&dir.join("Disc Game (USA).cue")).unwrap(), &chd, &stop).unwrap();
        let gd = dir.join("gd.chd");
        write_cd(&cd_tracks(&dir.join("Gd Game (Japan).gdi")).unwrap(), &gd, &stop).unwrap();
        let dvd = dir.join("dvd.chd");
        write_dvd(&dir.join("Dvd Game (Europe).iso"), &dvd, &stop).unwrap();
        for p in [&chd, &gd, &dvd] {
            let out = std::process::Command::new(&chdman).args(["verify", "-i"]).arg(p).output().unwrap();
            assert!(out.status.success(), "chdman verify {}: {}", p.display(), String::from_utf8_lossy(&out.stderr));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dolphin_tools_hashes_are_read() {
        let out = "Problems Found: No\nCRC32: 0x1a2b3c4d\nMD5: 0123456789abcdef0123456789ABCDEF\nSHA1: 0123456789abcdef0123456789abcdef01234567\n";
        let (c, m, s) = parse_verify(out);
        assert_eq!(c.as_deref(), Some("1a2b3c4d"));
        assert_eq!(m.as_deref(), Some("0123456789abcdef0123456789abcdef"));
        assert_eq!(s.as_deref(), Some("0123456789abcdef0123456789abcdef01234567"));
    }
}
