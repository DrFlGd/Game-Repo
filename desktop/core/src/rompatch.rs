//! ROM patches (docs/PLAN.md, "Cheats and patches", D201–D202): the patch formats that
//! translations, hacks and fixes come in, read and applied here so no tool is needed.
//!
//! - IPS (`PATCH` … `EOF`, with the size to cut to after it) and IPS32 (`IPS32` … `EEOF`).
//! - UPS (`UPS1`): XOR runs, with the source's, the result's and its own CRC32 at the end.
//! - BPS (`BPS1`): copies from the source, the patch and the result so far, CRC32s as UPS.
//! - PPF 1, 2 and 3 (PlayStation images): bytes written at offsets, PPF 2 and 3 with a
//!   block of the image to check it's the right one, and a `FILE_ID.DIZ` at the end.
//! - xdelta (VCDIFF, RFC 3284): windows of ADD, RUN and COPY with the default code table
//!   and address cache. A patch made with secondary compression (xdelta3's `-S djw`,
//!   `lzma`…) or its own code table can't be read here, and says it needs `xdelta3`.
//!
//! `info` reads what a patch says about itself (the sizes and CRC32s it's for, the block it
//! checks), which is how a patch is paired with the very file it's for; `apply` makes the
//! patched bytes and never changes the source; `test` applies it and says whether it fits.

use anyhow::{anyhow, bail, ensure, Result};
use serde::Serialize;

/// The formats, as `patches.json` and the page name them.
pub const FORMATS: &[(&str, &str)] = &[
    ("ips", "IPS"),
    ("ips32", "IPS32"),
    ("ups", "UPS"),
    ("bps", "BPS"),
    ("ppf", "PPF"),
    ("xdelta", "xdelta"),
];

/// The file names patches come with.
pub const EXTENSIONS: &[&str] = &["ips", "ups", "bps", "ppf", "xdelta", "xdelta3", "vcdiff", "delta", "dat"];

/// What a patch says about itself.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Info {
    /// One of `FORMATS`' ids
    pub format: String,
    /// The source's size and CRC32 (UPS and BPS; PPF 2 the size only)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_crc: Option<u32>,
    /// The result's (UPS and BPS)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_crc: Option<u32>,
    /// PPF 2 and 3: the bytes of the image at this offset it was made for
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_at: Option<u64>,
    #[serde(skip_serializing)]
    pub block: Vec<u8>,
    /// Its own words: PPF's description and FILE_ID.DIZ, BPS's metadata
    #[serde(skip_serializing_if = "String::is_empty")]
    pub about: String,
    /// It can't be applied here (xdelta's secondary compression): it needs `xdelta3`
    #[serde(skip_serializing_if = "String::is_empty")]
    pub needs: String,
}

impl Info {
    /// Whether a file of this size and CRC32 is what the patch says it's for: `Some(true)`
    /// sure, `Some(false)` sure it isn't, `None` the patch doesn't say.
    pub fn fits(&self, size: u64, crc: u32) -> Option<bool> {
        match (self.source_size, self.source_crc) {
            (Some(s), Some(c)) => Some(s == size && c == crc),
            _ => None,
        }
    }

    /// Whether these bytes are the image a PPF's block check was made from.
    pub fn block_fits(&self, source: &[u8]) -> Option<bool> {
        let at = self.block_at? as usize;
        Some(source.len() >= at + self.block.len() && source[at..at + self.block.len()] == self.block[..])
    }
}

/// The format of a patch, from its first bytes.
pub fn detect(patch: &[u8]) -> Option<&'static str> {
    if patch.starts_with(b"IPS32") {
        Some("ips32")
    } else if patch.starts_with(b"PATCH") {
        Some("ips")
    } else if patch.starts_with(b"UPS1") {
        Some("ups")
    } else if patch.starts_with(b"BPS1") {
        Some("bps")
    } else if patch.len() >= 5 && &patch[..3] == b"PPF" && [&b"10"[..], b"20", b"30"].contains(&&patch[3..5]) {
        Some("ppf")
    } else if patch.starts_with(&[0xD6, 0xC3, 0xC4, 0x00]) {
        Some("xdelta")
    } else {
        None
    }
}

/// What a patch says about itself, without a source.
pub fn info(patch: &[u8]) -> Result<Info> {
    let format = detect(patch).ok_or_else(|| anyhow!("This isn't a patch Game Repo knows (IPS, UPS, BPS, PPF or xdelta)."))?;
    let mut i = Info { format: format.into(), ..Default::default() };
    match format {
        "ips" | "ips32" => {}
        "ups" => {
            ensure!(patch.len() >= 18, "The UPS patch is cut short.");
            let mut r = Reader::new(patch, 4);
            i.source_size = Some(r.beat()?);
            i.target_size = Some(r.beat()?);
            let (s, t) = footer(patch);
            i.source_crc = Some(s);
            i.target_crc = Some(t);
        }
        "bps" => {
            ensure!(patch.len() >= 19, "The BPS patch is cut short.");
            let mut r = Reader::new(patch, 4);
            i.source_size = Some(r.beat()?);
            i.target_size = Some(r.beat()?);
            let n = r.beat()? as usize;
            i.about = String::from_utf8_lossy(r.take(n)?).trim().to_string();
            let (s, t) = footer(patch);
            i.source_crc = Some(s);
            i.target_crc = Some(t);
        }
        "ppf" => {
            let p = ppf(patch)?;
            i.source_size = p.size;
            i.block_at = p.block.as_ref().map(|b| b.0);
            i.block = p.block.map(|b| b.1).unwrap_or_default();
            i.about = [p.description, p.diz].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n\n");
        }
        _ => {
            let h = vcdiff_header(patch)?;
            i.needs = h.needs;
        }
    }
    Ok(i)
}

/// The patched bytes. A patch that says which source it's for (UPS, BPS) is refused for
/// another, and its result checked against the CRC32 it gives; PPF 2's and 3's block too.
pub fn apply(patch: &[u8], source: &[u8]) -> Result<Vec<u8>> {
    match detect(patch) {
        Some("ips") => ips(patch, source, false),
        Some("ips32") => ips(patch, source, true),
        Some("ups") => ups(patch, source),
        Some("bps") => bps(patch, source),
        Some("ppf") => apply_ppf(patch, source),
        Some("xdelta") => vcdiff(patch, source),
        _ => bail!("This isn't a patch Game Repo knows (IPS, UPS, BPS, PPF or xdelta)."),
    }
}

/// Applying it in memory: whether it fits, and what came of it.
#[derive(Debug, Clone, Serialize)]
pub struct Test {
    pub fits: bool,
    /// How sure: `checked` (the result's CRC32 matched, or PPF's block), `applied` (the
    /// format can't say: it only didn't run past the end)
    pub how: String,
    pub size: u64,
    pub crc: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub why: String,
}

pub fn test(patch: &[u8], source: &[u8]) -> Test {
    let checks = match info(patch) {
        Ok(i) => i.target_crc.is_some() || i.block_at.is_some() || i.format == "xdelta",
        Err(_) => false,
    };
    match apply(patch, source) {
        Ok(out) => Test {
            fits: true,
            how: if checks { "checked" } else { "applied" }.into(),
            size: out.len() as u64,
            crc: format!("{:08x}", crc32fast::hash(&out)),
            why: String::new(),
        },
        Err(e) => Test { fits: false, how: String::new(), size: 0, crc: String::new(), why: e.to_string() },
    }
}

// ---- reading ----

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
    end: usize,
}

impl<'a> Reader<'a> {
    fn new(b: &'a [u8], at: usize) -> Self {
        Reader { b, at, end: b.len() }
    }

    fn byte(&mut self) -> Result<u8> {
        ensure!(self.at < self.end, "The patch is cut short.");
        self.at += 1;
        Ok(self.b[self.at - 1])
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        ensure!(n <= self.end - self.at, "The patch is cut short.");
        self.at += n;
        Ok(&self.b[self.at - n..self.at])
    }

    fn be(&mut self, n: usize) -> Result<u64> {
        Ok(self.take(n)?.iter().fold(0u64, |v, &x| v << 8 | x as u64))
    }

    fn le(&mut self, n: usize) -> Result<u64> {
        Ok(self.take(n)?.iter().rev().fold(0u64, |v, &x| v << 8 | x as u64))
    }

    /// UPS's and BPS's numbers (byuu's: seven bits a byte, the last with its top bit set)
    fn beat(&mut self) -> Result<u64> {
        let (mut data, mut shift) = (0u64, 1u64);
        loop {
            let x = self.byte()?;
            data = data.checked_add((x & 0x7f) as u64 * shift).ok_or_else(|| anyhow!("The patch has a number too big."))?;
            if x & 0x80 != 0 {
                return Ok(data);
            }
            shift = shift.checked_shl(7).filter(|s| *s < 1 << 56).ok_or_else(|| anyhow!("The patch has a number too big."))?;
            data += shift;
        }
    }

    /// VCDIFF's numbers (seven bits a byte, big end first, the top bit set on all but the last)
    fn var(&mut self) -> Result<u64> {
        let mut v = 0u64;
        for _ in 0..10 {
            let x = self.byte()?;
            v = v.checked_mul(128).ok_or_else(|| anyhow!("The patch has a number too big."))? | (x & 0x7f) as u64;
            if x & 0x80 == 0 {
                return Ok(v);
            }
        }
        bail!("The patch has a number too big.")
    }
}

/// UPS's and BPS's source and result CRC32s (the last twelve bytes are those and the patch's own).
fn footer(patch: &[u8]) -> (u32, u32) {
    let n = patch.len();
    let le = |b: &[u8]| u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    (le(&patch[n - 12..n - 8]), le(&patch[n - 8..n - 4]))
}

/// Its own CRC32 (the last four bytes, of everything before them), so a damaged patch says so.
fn own_crc(patch: &[u8]) -> Result<()> {
    let n = patch.len();
    let said = u32::from_le_bytes([patch[n - 4], patch[n - 3], patch[n - 2], patch[n - 1]]);
    ensure!(crc32fast::hash(&patch[..n - 4]) == said, "The patch is damaged: its own checksum doesn't match.");
    Ok(())
}

fn too_big(n: u64) -> Result<usize> {
    // no ROM or disc image a patch is for is this big; a damaged patch could say so
    ensure!(n <= 16 << 30, "The patch says the result is {n} bytes, which can't be right.");
    Ok(n as usize)
}

// ---- IPS ----

fn ips(patch: &[u8], source: &[u8], wide: bool) -> Result<Vec<u8>> {
    let (head, end, off) = if wide { (5, &b"EEOF"[..], 4) } else { (5, &b"EOF"[..], 3) };
    let mut out = source.to_vec();
    let mut r = Reader::new(patch, head);
    loop {
        if r.end - r.at >= end.len() && &patch[r.at..r.at + end.len()] == end {
            // "EOF" can be a real offset (0x454F46): it's the end only if what's left is
            // nothing, or the three (or four) bytes of the size to cut to
            let left = r.end - r.at - end.len();
            if left == 0 || left == off {
                r.at += end.len();
                if left == off {
                    let cut = r.be(off)? as usize;
                    out.resize(cut, 0);
                }
                return Ok(out);
            }
        }
        if r.at >= r.end {
            bail!("The IPS patch has no end mark: it's cut short.");
        }
        let at = r.be(off)? as usize;
        let n = r.be(2)? as usize;
        let (n, run) = if n == 0 { (r.be(2)? as usize, Some(r.byte()?)) } else { (n, None) };
        ensure!(at + n <= 1 << 32, "The IPS patch writes past what any ROM could be.");
        if out.len() < at + n {
            out.resize(at + n, 0);
        }
        match run {
            Some(b) => out[at..at + n].fill(b),
            None => out[at..at + n].copy_from_slice(r.take(n)?),
        }
    }
}

// ---- UPS ----

fn ups(patch: &[u8], source: &[u8]) -> Result<Vec<u8>> {
    ensure!(patch.len() >= 18, "The UPS patch is cut short.");
    own_crc(patch)?;
    let mut r = Reader::new(patch, 4);
    let (a, b) = (r.beat()?, r.beat()?);
    let (scrc, tcrc) = footer(patch);
    let crc = crc32fast::hash(source);
    // UPS goes both ways: given the result, it gives back the source
    let (from, to, want) = if source.len() as u64 == a && crc == scrc {
        (a, b, tcrc)
    } else if source.len() as u64 == b && crc == tcrc {
        (b, a, scrc)
    } else {
        bail!("This patch is for another file: it wants one of {a} bytes with CRC32 {scrc:08x}, and this one is {} bytes, {crc:08x}.", source.len());
    };
    let _ = from;
    let to = too_big(to)?;
    let mut out = source.to_vec();
    out.resize(to, 0);
    r.end = patch.len() - 12;
    let mut at = 0usize;
    while r.at < r.end {
        at = at.checked_add(r.beat()? as usize).ok_or_else(|| anyhow!("The UPS patch is damaged."))?;
        loop {
            let x = r.byte()?;
            if at < out.len() {
                out[at] ^= x;
            }
            at += 1;
            if x == 0 {
                break;
            }
        }
    }
    ensure!(crc32fast::hash(&out) == want, "The patched file isn't what the patch says it makes (its CRC32 doesn't match).");
    Ok(out)
}

// ---- BPS ----

fn bps(patch: &[u8], source: &[u8]) -> Result<Vec<u8>> {
    ensure!(patch.len() >= 19, "The BPS patch is cut short.");
    own_crc(patch)?;
    let mut r = Reader::new(patch, 4);
    let (ssize, tsize) = (r.beat()?, r.beat()?);
    let meta = r.beat()? as usize;
    r.take(meta)?;
    let (scrc, tcrc) = footer(patch);
    let crc = crc32fast::hash(source);
    ensure!(source.len() as u64 == ssize && crc == scrc,
        "This patch is for another file: it wants one of {ssize} bytes with CRC32 {scrc:08x}, and this one is {} bytes, {crc:08x}.", source.len());
    let tsize = too_big(tsize)?;
    let mut out: Vec<u8> = Vec::with_capacity(tsize);
    r.end = patch.len() - 12;
    let (mut src_at, mut out_at) = (0i64, 0i64);
    let damaged = || anyhow!("The BPS patch is damaged: it reaches outside the file.");
    while r.at < r.end {
        let d = r.beat()?;
        let n = (d >> 2) as usize + 1;
        ensure!(out.len() + n <= tsize, "The BPS patch writes past the size it gives.");
        match d & 3 {
            0 => {
                let at = out.len();
                out.extend_from_slice(source.get(at..at + n).ok_or_else(damaged)?);
            }
            1 => out.extend_from_slice(r.take(n)?),
            2 => {
                let o = r.beat()?;
                src_at += if o & 1 == 1 { -((o >> 1) as i64) } else { (o >> 1) as i64 };
                ensure!(src_at >= 0, "The BPS patch is damaged: it reaches outside the file.");
                out.extend_from_slice(source.get(src_at as usize..src_at as usize + n).ok_or_else(damaged)?);
                src_at += n as i64;
            }
            _ => {
                let o = r.beat()?;
                out_at += if o & 1 == 1 { -((o >> 1) as i64) } else { (o >> 1) as i64 };
                ensure!(out_at >= 0 && (out_at as usize) < out.len(), "The BPS patch is damaged: it reaches outside the file.");
                // byte by byte: the copy may run into what it's writing (a repeat)
                for _ in 0..n {
                    let b = out[out_at as usize];
                    out.push(b);
                    out_at += 1;
                }
            }
        }
    }
    ensure!(out.len() == tsize, "The BPS patch makes a file of {} bytes, not the {tsize} it gives.", out.len());
    ensure!(crc32fast::hash(&out) == tcrc, "The patched file isn't what the patch says it makes (its CRC32 doesn't match).");
    Ok(out)
}

// ---- PPF ----

struct Ppf {
    version: u8,
    description: String,
    diz: String,
    /// PPF 2: the original's size
    size: Option<u64>,
    block: Option<(u64, Vec<u8>)>,
    data: (usize, usize),
    undo: bool,
}

fn ppf(patch: &[u8]) -> Result<Ppf> {
    ensure!(patch.len() >= 56, "The PPF patch is cut short.");
    let version = patch[3] - b'0';
    let description = String::from_utf8_lossy(&patch[6..56]).trim_end_matches(['\0', ' ']).trim().to_string();
    let mut r = Reader::new(patch, 56);
    let (mut size, mut block, mut undo) = (None, None, false);
    match version {
        1 => {}
        2 => {
            size = Some(r.le(4)?);
            block = Some((0x9320, r.take(1024)?.to_vec()));
        }
        _ => {
            let kind = r.byte()?;
            let check = r.byte()?;
            undo = r.byte()? != 0;
            r.byte()?;
            if check != 0 {
                block = Some((if kind == 1 { 0x80A0 } else { 0x9320 }, r.take(1024)?.to_vec()));
            }
        }
    }
    // FILE_ID.DIZ at the end, between its two marks (then its length: PPF 2 four bytes, PPF 3 two)
    let (mut end, mut diz) = (patch.len(), String::new());
    if version >= 2 {
        if let Some(b) = find_last(&patch[r.at..], b"@BEGIN_FILE_ID.DIZ") {
            let b = r.at + b;
            if let Some(e) = find_last(&patch[b..], b"@END_FILE_ID.DIZ") {
                diz = String::from_utf8_lossy(&patch[b + 18..b + e]).trim().to_string();
                end = b;
            }
        }
    }
    Ok(Ppf { version, description, diz, size, block, data: (r.at, end), undo })
}

fn find_last(hay: &[u8], needle: &[u8]) -> Option<usize> {
    (0..=hay.len().checked_sub(needle.len())?).rev().find(|&i| &hay[i..i + needle.len()] == needle)
}

fn apply_ppf(patch: &[u8], source: &[u8]) -> Result<Vec<u8>> {
    let p = ppf(patch)?;
    if let Some((at, b)) = &p.block {
        let at = *at as usize;
        ensure!(source.len() >= at + b.len() && source[at..at + b.len()] == b[..],
            "This patch is for another image: the part it checks (at {at:#x}) isn't the same.");
    }
    if let Some(s) = p.size {
        ensure!(source.len() as u64 == s, "This patch is for an image of {s} bytes, and this one is {}.", source.len());
    }
    let mut out = source.to_vec();
    let mut r = Reader::new(patch, p.data.0);
    r.end = p.data.1;
    let wide = if p.version == 3 { 8 } else { 4 };
    while r.at < r.end {
        let at = r.le(wide)? as usize;
        let n = r.byte()? as usize;
        let bytes = r.take(n)?;
        if p.undo {
            r.take(n)?;
        }
        ensure!(at + n <= 1 << 36, "The PPF patch writes past what any image could be.");
        if out.len() < at + n {
            out.resize(at + n, 0);
        }
        out[at..at + n].copy_from_slice(bytes);
    }
    Ok(out)
}

// ---- VCDIFF (xdelta) ----

struct VcdHeader {
    /// where the first window starts
    at: usize,
    needs: String,
}

fn vcdiff_header(patch: &[u8]) -> Result<VcdHeader> {
    ensure!(patch.len() >= 5, "The xdelta patch is cut short.");
    let mut r = Reader::new(patch, 4);
    let ind = r.byte()?;
    let mut needs = String::new();
    if ind & 0x01 != 0 {
        r.byte()?;
        needs = "xdelta3".into();
    }
    if ind & 0x02 != 0 {
        needs = "xdelta3".into();
        // its own code table: its length then the table; not read here
        let n = r.var()? as usize;
        r.take(n)?;
    }
    if ind & 0x04 != 0 {
        // xdelta3's application header (the file names it was made from)
        let n = r.var()? as usize;
        r.take(n)?;
    }
    Ok(VcdHeader { at: r.at, needs })
}

#[derive(Clone, Copy, PartialEq)]
enum Ins {
    Noop,
    Add,
    Run,
    Copy,
}

/// RFC 3284's default code table: each code's two instructions (kind, size, mode).
fn code_table() -> Vec<[(Ins, u8, u8); 2]> {
    let none = (Ins::Noop, 0, 0);
    let mut t = vec![[(Ins::Run, 0, 0), none]];
    for size in 0..=17 {
        t.push([(Ins::Add, size, 0), none]);
    }
    for mode in 0..9 {
        t.push([(Ins::Copy, 0, mode), none]);
        for size in 4..=18 {
            t.push([(Ins::Copy, size, mode), none]);
        }
    }
    for mode in 0..6 {
        for add in 1..=4 {
            for copy in 4..=6 {
                t.push([(Ins::Add, add, 0), (Ins::Copy, copy, mode)]);
            }
        }
    }
    for mode in 6..9 {
        for add in 1..=4 {
            t.push([(Ins::Add, add, 0), (Ins::Copy, 4, mode)]);
        }
    }
    for mode in 0..9 {
        t.push([(Ins::Copy, 4, mode), (Ins::Add, 1, 0)]);
    }
    debug_assert_eq!(t.len(), 256);
    t
}

fn adler32(b: &[u8]) -> u32 {
    let (mut a, mut s) = (1u32, 0u32);
    for chunk in b.chunks(5552) {
        for &x in chunk {
            a += x as u32;
            s += a;
        }
        a %= 65521;
        s %= 65521;
    }
    s << 16 | a
}

fn vcdiff(patch: &[u8], source: &[u8]) -> Result<Vec<u8>> {
    let h = vcdiff_header(patch)?;
    ensure!(h.needs.is_empty(), "This xdelta patch was made with xdelta3's own compression, which Game Repo can't read: it needs xdelta3 (Settings › Tools).");
    let table = code_table();
    let mut out: Vec<u8> = Vec::new();
    let mut r = Reader::new(patch, h.at);
    let damaged = || anyhow!("The xdelta patch is damaged: it reaches outside the file.");
    while r.at < r.end {
        let win = r.byte()?;
        ensure!(win & !0x07 == 0, "The xdelta patch is damaged (a window's flags).");
        let seg: Vec<u8> = if win & 0x03 != 0 {
            ensure!(win & 0x03 != 0x03, "The xdelta patch is damaged (a window's flags).");
            let n = r.var()? as usize;
            let at = r.var()? as usize;
            let from: &[u8] = if win & 0x01 != 0 { source } else { &out };
            from.get(at..at.checked_add(n).ok_or_else(damaged)?)
                .ok_or_else(|| anyhow!("This patch is for another file: it reads past the end of it."))?
                .to_vec()
        } else {
            Vec::new()
        };
        let _delta_len = r.var()?;
        let tlen = too_big(r.var()?)?;
        let delta = r.byte()?;
        ensure!(delta == 0, "This xdelta patch was made with xdelta3's own compression, which Game Repo can't read: it needs xdelta3 (Settings › Tools).");
        let (dn, inn, an) = (r.var()? as usize, r.var()? as usize, r.var()? as usize);
        let sum = if win & 0x04 != 0 { Some(r.be(4)? as u32) } else { None };
        let data = r.take(dn)?;
        let inst = r.take(inn)?;
        let addr = r.take(an)?;
        let (mut d, mut i, mut a) = (Reader::new(data, 0), Reader::new(inst, 0), Reader::new(addr, 0));
        let mut t: Vec<u8> = Vec::with_capacity(tlen);
        let (mut near, mut next, mut same) = ([0usize; 4], 0usize, vec![0usize; 3 * 256]);
        let slen = seg.len();
        while i.at < i.end {
            let code = i.byte()? as usize;
            for (kind, size, mode) in table[code] {
                if kind == Ins::Noop {
                    continue;
                }
                let n = if size == 0 { i.var()? as usize } else { size as usize };
                ensure!(t.len() + n <= tlen, "The xdelta patch writes past a window's size.");
                match kind {
                    Ins::Add => t.extend_from_slice(d.take(n)?),
                    Ins::Run => {
                        let b = d.byte()?;
                        t.resize(t.len() + n, b);
                    }
                    _ => {
                        let here = slen + t.len();
                        let at = match mode {
                            0 => a.var()? as usize,
                            1 => here.checked_sub(a.var()? as usize).ok_or_else(damaged)?,
                            2..=5 => near[mode as usize - 2] + a.var()? as usize,
                            _ => same[(mode as usize - 6) * 256 + a.byte()? as usize],
                        };
                        ensure!(at < here, "The xdelta patch is damaged: it reaches outside the file.");
                        near[next] = at;
                        next = (next + 1) % 4;
                        same[at % (3 * 256)] = at;
                        // byte by byte: a copy from the window may run into what it's writing
                        for k in 0..n {
                            let p = at + k;
                            let b = if p < slen { seg[p] } else { *t.get(p - slen).ok_or_else(damaged)? };
                            t.push(b);
                        }
                    }
                }
            }
        }
        ensure!(t.len() == tlen, "The xdelta patch is damaged: a window came out the wrong size.");
        if let Some(s) = sum {
            ensure!(adler32(&t) == s, "The patched file isn't what the patch says it makes (its checksum doesn't match): it's probably for another file.");
        }
        out.extend_from_slice(&t);
    }
    Ok(out)
}

#[cfg(test)]
pub mod tests {
    use super::*;

    fn rom(seed: u8, n: usize) -> Vec<u8> {
        (0..n).map(|i| (i as u32).wrapping_mul(2654435761).wrapping_add(seed as u32).rotate_left(7) as u8).collect()
    }

    fn beat(mut v: u64, out: &mut Vec<u8>) {
        loop {
            let x = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(0x80 | x);
                return;
            }
            out.push(x);
            v -= 1;
        }
    }

    fn sign(mut p: Vec<u8>, a: &[u8], b: &[u8]) -> Vec<u8> {
        p.extend(crc32fast::hash(a).to_le_bytes());
        p.extend(crc32fast::hash(b).to_le_bytes());
        let c = crc32fast::hash(&p);
        p.extend(c.to_le_bytes());
        p
    }

    /// A UPS patch from `a` to `b` (each changed run XORed, as beat's tool makes them).
    pub fn make_ups(a: &[u8], b: &[u8]) -> Vec<u8> {
        let mut p = b"UPS1".to_vec();
        beat(a.len() as u64, &mut p);
        beat(b.len() as u64, &mut p);
        let n = a.len().max(b.len());
        let at = |v: &[u8], i: usize| v.get(i).copied().unwrap_or(0);
        let (mut i, mut last) = (0, 0);
        while i < n {
            if at(a, i) == at(b, i) {
                i += 1;
                continue;
            }
            beat((i - last) as u64, &mut p);
            while i < n && at(a, i) != at(b, i) {
                p.push(at(a, i) ^ at(b, i));
                i += 1;
            }
            p.push(0);
            i += 1;
            last = i;
        }
        sign(p, a, b)
    }

    /// A BPS patch using every action: the source as it is, bytes from the patch, a copy
    /// from elsewhere in the source, and a repeat of what's been written.
    fn make_bps(a: &[u8]) -> (Vec<u8>, Vec<u8>) {
        // the result: a[..100], "HELLO", "HELLO" three more times (a copy of what's just been
        // written, overlapping itself), a[300..340], then a[200..250] (a source copy going back)
        let mut b = a[..100].to_vec();
        b.extend(b"HELLO");
        for k in 0..15 {
            b.push(b"HELLO"[k % 5]);
        }
        b.extend(&a[300..340]);
        b.extend(&a[200..250]);
        let mut p = b"BPS1".to_vec();
        beat(a.len() as u64, &mut p);
        beat(b.len() as u64, &mut p);
        let meta = b"<patch>Test</patch>";
        beat(meta.len() as u64, &mut p);
        p.extend(meta);
        let act = |p: &mut Vec<u8>, kind: u64, n: usize| beat(((n as u64 - 1) << 2) | kind, p);
        let rel = |p: &mut Vec<u8>, d: i64| beat((d.unsigned_abs() << 1) | (d < 0) as u64, p);
        act(&mut p, 0, 100); // SourceRead
        act(&mut p, 1, 5); // TargetRead
        p.extend(b"HELLO");
        act(&mut p, 3, 15); // TargetCopy from 100, overlapping what it writes
        rel(&mut p, 100);
        act(&mut p, 2, 40); // SourceCopy from 300
        rel(&mut p, 300);
        act(&mut p, 2, 50); // SourceCopy from 200: from 340, 140 back
        rel(&mut p, 200 - 340);
        (sign(p, a, &b), b)
    }

    #[test]
    fn ips_writes_runs_extends_and_cuts() {
        let a = rom(1, 64);
        let mut p = b"PATCH".to_vec();
        p.extend([0, 0, 4, 0, 3, b'a', b'b', b'c']); // 3 bytes at 4
        p.extend([0, 0, 10, 0, 0, 0, 5, 0xEE]); // a run of 5 at 10
        p.extend([0, 0, 70, 0, 2, 1, 2]); // past the end: it grows (zeros between)
        p.extend(b"EOF");
        let out = apply(&p, &a).unwrap();
        assert_eq!(out.len(), 72);
        assert_eq!(&out[4..7], b"abc");
        assert_eq!(&out[10..15], &[0xEE; 5]);
        assert_eq!(&out[64..70], &[0; 6]);
        assert_eq!(&out[70..72], &[1, 2]);
        assert_eq!(&out[..4], &a[..4]);
        // with a size to cut to after EOF
        let mut cut = p.clone();
        cut.extend([0, 0, 32]);
        assert_eq!(apply(&cut, &a).unwrap().len(), 32);
        // IPS32
        let mut w = b"IPS32".to_vec();
        w.extend([0, 0, 0, 2, 0, 1, 9]);
        w.extend(b"EEOF");
        assert_eq!(apply(&w, &a).unwrap()[2], 9);
        // "EOF" as a real offset (0x454F46) isn't the end
        let mut e = b"PATCH".to_vec();
        e.extend(b"EOF");
        e.extend([0, 1, 7]);
        e.extend(b"EOF");
        let out = apply(&e, &a).unwrap();
        assert_eq!(out.len(), 0x454F47);
        assert_eq!(out[0x454F46], 7);
        // cut short
        assert!(apply(&p[..p.len() - 3], &a).is_err());
        assert_eq!(info(&p).unwrap().format, "ips");
    }

    #[test]
    fn ups_and_bps_apply_check_both_crcs_and_refuse_another_file() {
        let a = rom(2, 4096);
        let mut b = a.clone();
        b[10] ^= 0x55;
        b[11] = 0;
        b[2000..2010].copy_from_slice(b"TRANSLATED");
        b.extend(b"MORE AT THE END");
        let p = make_ups(&a, &b);
        let i = info(&p).unwrap();
        assert_eq!((i.source_size, i.target_size), (Some(4096), Some(b.len() as u64)));
        assert_eq!(i.fits(4096, crc32fast::hash(&a)), Some(true));
        assert_eq!(apply(&p, &a).unwrap(), b);
        // UPS goes back the other way too
        assert_eq!(apply(&p, &b).unwrap(), a);
        let other = rom(3, 4096);
        let e = apply(&p, &other).unwrap_err().to_string();
        assert!(e.contains("another file"), "{e}");
        // a damaged patch says so
        let mut bad = p.clone();
        bad[6] ^= 1;
        assert!(apply(&bad, &a).unwrap_err().to_string().contains("damaged"));

        let (p, want) = make_bps(&a);
        let i = info(&p).unwrap();
        assert_eq!(i.format, "bps");
        assert_eq!(i.about, "<patch>Test</patch>");
        assert_eq!(i.target_crc, Some(crc32fast::hash(&want)));
        assert_eq!(apply(&p, &a).unwrap(), want);
        assert!(apply(&p, &other).unwrap_err().to_string().contains("another file"));
        let t = test(&p, &a);
        assert!(t.fits && t.how == "checked", "{t:?}");
        assert!(!test(&p, &other).fits);
    }

    fn ppf3(a: &[u8], check: bool, undo: bool) -> Vec<u8> {
        let mut p = b"PPF30\x02".to_vec();
        let mut d = b"A test patch".to_vec();
        d.resize(50, b' ');
        p.extend(d);
        p.extend([0, check as u8, undo as u8, 0]);
        if check {
            p.extend(&a[0x9320..0x9320 + 1024]);
        }
        p.extend(5u64.to_le_bytes());
        p.extend([3, b'x', b'y', b'z']);
        if undo {
            p.extend(&a[5..8]);
        }
        p.extend(0x9400u64.to_le_bytes());
        p.extend([1, 0xAA]);
        if undo {
            p.push(a[0x9400]);
        }
        p.extend(b"@BEGIN_FILE_ID.DIZ");
        p.extend(b"By someone, 2026");
        p.extend(b"@END_FILE_ID.DIZ");
        p.extend(16u16.to_le_bytes());
        p
    }

    #[test]
    fn ppf_checks_its_block_and_keeps_its_description() {
        let a = rom(4, 0x10000);
        for (check, undo) in [(true, false), (true, true), (false, false)] {
            let p = ppf3(&a, check, undo);
            let i = info(&p).unwrap();
            assert_eq!(i.format, "ppf");
            assert!(i.about.contains("A test patch") && i.about.contains("By someone, 2026"), "{}", i.about);
            assert_eq!(i.block_fits(&a), if check { Some(true) } else { None });
            let out = apply(&p, &a).unwrap();
            assert_eq!(&out[5..8], b"xyz");
            assert_eq!(out[0x9400], 0xAA);
            assert_eq!(out.len(), a.len());
        }
        let other = rom(5, 0x10000);
        assert!(apply(&ppf3(&a, true, false), &other).unwrap_err().to_string().contains("another image"));
        // PPF 2: the size and block
        let mut p = b"PPF20\x01".to_vec();
        p.extend([b' '; 50]);
        p.extend((a.len() as u32).to_le_bytes());
        p.extend(&a[0x9320..0x9320 + 1024]);
        p.extend(1u32.to_le_bytes());
        p.extend([2, 7, 7]);
        let out = apply(&p, &a).unwrap();
        assert_eq!(&out[1..3], &[7, 7]);
        assert!(apply(&p, &a[..0x9800]).is_err());
        // PPF 1
        let mut p = b"PPF10\x00".to_vec();
        p.extend([b' '; 50]);
        p.extend(0u32.to_le_bytes());
        p.extend([1, 9]);
        assert_eq!(apply(&p, &a).unwrap()[0], 9);
    }

    fn var(mut v: u64, out: &mut Vec<u8>) {
        let mut b = vec![(v & 0x7f) as u8];
        v >>= 7;
        while v > 0 {
            b.push(0x80 | (v & 0x7f) as u8);
            v >>= 7;
        }
        b.reverse();
        out.extend(b);
    }

    /// One VCDIFF window from the source: (instruction codes and sizes, data, addresses)
    fn window(out: &mut Vec<u8>, seg: (usize, usize), tlen: usize, data: &[u8], inst: &[u8], addr: &[u8], sum: Option<u32>) {
        out.push(0x01 | if sum.is_some() { 0x04 } else { 0 });
        var(seg.1 as u64, out);
        var(seg.0 as u64, out);
        let mut d = Vec::new();
        var(tlen as u64, &mut d);
        d.push(0);
        var(data.len() as u64, &mut d);
        var(inst.len() as u64, &mut d);
        var(addr.len() as u64, &mut d);
        if let Some(s) = sum {
            d.extend(s.to_be_bytes());
        }
        d.extend(data);
        d.extend(inst);
        d.extend(addr);
        var(d.len() as u64, out);
        out.extend(d);
    }

    #[test]
    fn the_default_code_table_is_rfc_3284s() {
        let t = code_table();
        assert_eq!(t.len(), 256);
        assert!(t[0][0] == (Ins::Run, 0, 0));
        assert!(t[1][0] == (Ins::Add, 0, 0) && t[18][0] == (Ins::Add, 17, 0));
        assert!(t[19][0] == (Ins::Copy, 0, 0) && t[20][0] == (Ins::Copy, 4, 0) && t[34][0] == (Ins::Copy, 18, 0));
        assert!(t[35][0] == (Ins::Copy, 0, 1) && t[162][0] == (Ins::Copy, 18, 8));
        assert!(t[163] == [(Ins::Add, 1, 0), (Ins::Copy, 4, 0)] && t[234] == [(Ins::Add, 4, 0), (Ins::Copy, 6, 5)]);
        assert!(t[235] == [(Ins::Add, 1, 0), (Ins::Copy, 4, 6)] && t[246] == [(Ins::Add, 4, 0), (Ins::Copy, 4, 8)]);
        assert!(t[247] == [(Ins::Copy, 4, 0), (Ins::Add, 1, 0)] && t[255] == [(Ins::Copy, 4, 8), (Ins::Add, 1, 0)]);
        assert_eq!(adler32(b"Wikipedia"), 0x11E60398);
    }

    #[test]
    fn xdelta_windows_add_run_and_copy_through_the_address_cache() {
        let a = rom(6, 2000);
        // window 1, from source[1000..1500] (500 bytes, addresses 0..500 in it):
        //   COPY 10 from 0 (mode SELF), ADD "hi", RUN 4 of 'z', COPY 6 from 100 (near 0: 0 + 100),
        //   COPY 5 from 100 again (same: its byte 100), COPY 8 HERE back to the window's start
        let seg = &a[1000..1500];
        let mut want = seg[0..10].to_vec();
        want.extend(b"hi");
        want.extend(b"zzzz");
        want.extend(&seg[100..106]);
        want.extend(&seg[100..105]);
        let start = want.len();
        want.extend(&seg[..0]);
        // HERE back to the window's first byte (address 500, the target's start): an
        // overlapping copy is fine too
        for k in 0..8 {
            let b = want[k];
            want.push(b);
        }
        let _ = start;
        let mut inst = Vec::new();
        let mut addr = Vec::new();
        // COPY size 0 mode 0 (code 19) with its size 10, address 0 (SELF)
        inst.push(19);
        var(10, &mut inst);
        var(0, &mut addr);
        // ADD size 2 (code 3)
        inst.push(3);
        // RUN (code 0) size 4
        inst.push(0);
        var(4, &mut inst);
        // COPY size 6 mode 2 (near 0): code 19 + 2*16 + (6-3) = 54
        inst.push(19 + 2 * 16 + 3);
        var(100, &mut addr);
        // COPY size 5 mode 6 (same 0): code 19 + 6*16 + 2 = 117; the address 100 is in
        // same[100 % 768] = 100, so its byte is 100
        inst.push(19 + 6 * 16 + 2);
        addr.push(100);
        // COPY size 8 mode 1 (HERE): here = 500 + 27, back 27 to 500
        inst.push(19 + 16 + 5);
        var(27, &mut addr);
        let mut p = vec![0xD6, 0xC3, 0xC4, 0x00, 0x04];
        var(3, &mut p);
        p.extend(b"a/b");
        window(&mut p, (1000, 500), want.len(), b"hiz", &inst, &addr, Some(adler32(&want)));
        // window 2: all new bytes, no source
        let mut w2 = Vec::new();
        w2.push(0);
        let mut d = Vec::new();
        var(3, &mut d);
        d.push(0);
        var(3, &mut d);
        var(1, &mut d);
        var(0, &mut d);
        d.extend(b"end");
        d.push(4); // ADD 3
        var(d.len() as u64, &mut w2);
        w2.extend(d);
        p.extend(w2);
        assert_eq!(info(&p).unwrap().format, "xdelta");
        assert_eq!(info(&p).unwrap().needs, "");
        let mut all = want.clone();
        all.extend(b"end");
        assert_eq!(apply(&p, &a).unwrap(), all);
        // the window's checksum catches another file
        assert!(apply(&p, &rom(7, 2000)).unwrap_err().to_string().contains("checksum"));
        // a source too short
        assert!(apply(&p, &a[..1200]).unwrap_err().to_string().contains("another file"));
        // secondary compression asks for xdelta3
        let mut s = vec![0xD6, 0xC3, 0xC4, 0x00, 0x01, 0x02];
        s.extend(&p[9..]);
        assert_eq!(info(&s).unwrap().needs, "xdelta3");
        assert!(apply(&s, &a).unwrap_err().to_string().contains("xdelta3"));
    }

    #[test]
    fn what_isnt_a_patch_says_so() {
        assert!(detect(b"hello").is_none());
        assert!(apply(b"hello", b"x").is_err());
        assert!(info(b"PPF40").is_err());
    }
}
