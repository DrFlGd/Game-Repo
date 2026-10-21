//! Hashing a file in one read: CRC32, MD5 and SHA1 together, and, for a file with a
//! header DATs leave out (iNES, FDS, A78, Lynx, SNES copier) or an N64 ROM in another
//! byte order, a second set for the data as the DATs list it (docs/PLAN.md, "Phase 1
//! design", Headers). A big file's MD5 and SHA1 are worked out on threads of their own
//! while it's read, and a file only copied or compared (an import's, D133) is read for
//! its CRC32 and SHA1 alone, as it's copied.

use md5::{Digest, Md5};
use sha1::Sha1;
use std::collections::VecDeque;
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

/// Files are read this much at a time (a multiple of 4, for N64 byte orders).
pub const BLOCK: usize = 1 << 20;

/// Files this big or bigger have their MD5 and SHA1 worked out on threads of their own,
/// each as fast as the other and the drive allow, instead of one after the other (D133).
const THREADED: u64 = 16 << 20;

/// What a file's bytes hash to, as lowercase hex (as DATs write them).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hashes {
    pub size: u64,
    pub crc32: String,
    pub md5: Option<String>,
    pub sha1: Option<String>,
    /// The header found ("ines", "fds", "a78", "lynx", "copier", "n64-v64", "n64-le"), if any
    pub header: Option<String>,
    /// The hashes without the header (or in N64's big-endian order)
    pub hl: Option<Box<Hashes>>,
}

impl Hashes {
    /// Only the CRC32 and size a ZIP records for a member (the quick scan).
    pub fn quick(size: u64, crc32: u32) -> Self {
        Self {
            size,
            crc32: format!("{crc32:08x}"),
            ..Default::default()
        }
    }
}

/// How a header changes what is hashed the second time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Header {
    /// The first n bytes aren't part of the ROM.
    Skip(&'static str, usize),
    /// An N64 ROM in byte-swapped (v64) order.
    N64Swapped,
    /// An N64 ROM in little-endian (n64) order.
    N64Little,
}

impl Header {
    fn name(self) -> &'static str {
        match self {
            Header::Skip(n, _) => n,
            Header::N64Swapped => "n64-v64",
            Header::N64Little => "n64-le",
        }
    }
}

/// Extensions a SNES copier header is looked for on (the rule is by size, so it's kept to SNES files).
const SNES_EXTS: &[&str] = &["sfc", "smc", "swc", "fig"];

/// The header the start of a file shows, given its size and extension.
/// Whether a file of this name can carry a header (or a byte order) `detect` finds:
/// a quick scan reads such a member of an archive in full, as its CRC32 alone can't
/// give the headerless hashes the DATs list.
pub fn may_have_header(name: &str) -> bool {
    let ext = crate::platforms::ext_of(name);
    ["nes", "fds", "a78", "lnx", "v64", "n64", "z64"].contains(&ext.as_str()) || SNES_EXTS.contains(&ext.as_str())
}

pub fn detect(first: &[u8], size: u64, ext: &str) -> Option<Header> {
    let at = |off: usize, magic: &[u8]| first.len() >= off + magic.len() && &first[off..off + magic.len()] == magic;
    let bigger = |n: u64| size > n;
    if at(0, b"NES\x1a") && bigger(16) {
        return Some(Header::Skip("ines", 16));
    }
    if at(0, b"FDS\x1a") && bigger(16) {
        return Some(Header::Skip("fds", 16));
    }
    if at(1, b"ATARI7800") && bigger(128) {
        return Some(Header::Skip("a78", 128));
    }
    if at(0, b"LYNX") && bigger(64) {
        return Some(Header::Skip("lynx", 64));
    }
    if at(0, &[0x37, 0x80, 0x40, 0x12]) && size.is_multiple_of(4) {
        return Some(Header::N64Swapped);
    }
    if at(0, &[0x40, 0x12, 0x37, 0x80]) && size.is_multiple_of(4) {
        return Some(Header::N64Little);
    }
    if SNES_EXTS.contains(&ext) && size > 512 && size % 1024 == 512 {
        return Some(Header::Skip("copier", 512));
    }
    None
}

struct Digests {
    crc: crc32fast::Hasher,
    md5: Md5,
    sha1: Sha1,
    size: u64,
}

impl Digests {
    fn new() -> Self {
        Self {
            crc: crc32fast::Hasher::new(),
            md5: Md5::new(),
            sha1: Sha1::new(),
            size: 0,
        }
    }
    fn update(&mut self, b: &[u8]) {
        self.crc.update(b);
        self.md5.update(b);
        self.sha1.update(b);
        self.size += b.len() as u64;
    }
    fn finish(self) -> Hashes {
        Hashes {
            size: self.size,
            crc32: format!("{:08x}", self.crc.finalize()),
            md5: Some(hex::encode(self.md5.finalize())),
            sha1: Some(hex::encode(self.sha1.finalize())),
            header: None,
            hl: None,
        }
    }
}

/// Fill `buf` from `r` (short only at the end of the file).
fn fill(r: &mut (impl Read + ?Sized), buf: &mut [u8]) -> io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(n)
}

/// Hash everything `r` gives (a file of `size` bytes named `name`), stopping with an
/// error of kind `Interrupted` if `cancel` is set.
pub fn hash_reader(r: &mut (impl Read + ?Sized), size: u64, name: &str, cancel: &AtomicBool) -> io::Result<Hashes> {
    let ext = crate::platforms::ext_of(name);
    let mut buf = vec![0u8; BLOCK.min(size.max(4096) as usize).next_multiple_of(4)];
    let mut all = Digests::new();
    let mut hl: Option<(Header, Digests)> = None;
    let mut first = true;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(stopped());
        }
        let n = fill(r, &mut buf)?;
        if n == 0 {
            break;
        }
        let block = &mut buf[..n];
        if first {
            first = false;
            if let Some(h) = detect(block, size, &ext) {
                hl = Some((h, Digests::new()));
            } else if size >= THREADED && n == BLOCK {
                // a big file with nothing to take out: its digests on threads of their own
                buf.truncate(n);
                return threaded(r, buf, true, true, cancel, &mut |_| Ok(()));
            }
            all.update(block);
            if let Some((Header::Skip(_, skip), d)) = hl.as_mut() {
                d.update(&block[(*skip).min(n)..]);
                continue;
            }
        } else {
            all.update(block);
        }
        match hl.as_mut() {
            Some((Header::Skip(..), d)) => d.update(block),
            Some((Header::N64Swapped, d)) => {
                for w in block.chunks_exact_mut(2) {
                    w.swap(0, 1);
                }
                d.update(block);
            }
            Some((Header::N64Little, d)) => {
                for w in block.chunks_exact_mut(4) {
                    w.reverse();
                }
                d.update(block);
            }
            None => {}
        }
        if n < buf.len() {
            break;
        }
    }
    let mut out = all.finish();
    if let Some((h, d)) = hl {
        out.header = Some(h.name().to_string());
        out.hl = Some(Box::new(d.finish()));
    }
    Ok(out)
}

fn stopped() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "stopped")
}

/// Read everything `r` gives, handing each block to `each` (a copy writes it), for its
/// CRC32 and, if `sha1`, its SHA1: what an import knows a file by and checks a copy by
/// (D133). A DAT's MD5 is left out, and the SHA1 is worked out on a thread of its own, so
/// the read goes as fast as the drive (and CRC32 alone faster still).
pub fn read_hashed(r: &mut (impl Read + ?Sized), sha1: bool, cancel: &AtomicBool, each: &mut dyn FnMut(&[u8]) -> io::Result<()>) -> io::Result<Hashes> {
    if cancel.load(Ordering::Relaxed) {
        return Err(stopped());
    }
    let mut first = vec![0u8; BLOCK];
    let n = fill(r, &mut first)?;
    first.truncate(n);
    threaded(r, first, false, sha1, cancel, each)
}

/// A file's CRC32 and SHA1 (`read_hashed`, the bytes going nowhere).
pub fn identity(r: &mut (impl Read + ?Sized), cancel: &AtomicBool) -> io::Result<Hashes> {
    read_hashed(r, true, cancel, &mut |_| Ok(()))
}

/// The rest of a read whose first block is `first`: CRC32 here, as each block is read,
/// and MD5 and SHA1 (those wanted) on threads of their own, handed the blocks as they come.
fn threaded(r: &mut (impl Read + ?Sized), first: Vec<u8>, md5: bool, sha1: bool, cancel: &AtomicBool, each: &mut dyn FnMut(&[u8]) -> io::Result<()>) -> io::Result<Hashes> {
    if first.len() < BLOCK {
        // all of it (a small file): no threads needed
        each(&first)?;
        let mut d = Digests::new();
        d.update(&first);
        let mut h = d.finish();
        h.md5 = h.md5.filter(|_| md5);
        h.sha1 = h.sha1.filter(|_| sha1);
        return Ok(h);
    }
    std::thread::scope(|s| {
        let mut to = vec![];
        let sha1 = sha1.then(|| {
            let (tx, rx) = mpsc::sync_channel::<Arc<Vec<u8>>>(4);
            to.push(tx);
            s.spawn(move || {
                let mut d = Sha1::new();
                for b in rx {
                    d.update(&b[..]);
                }
                hex::encode(d.finalize())
            })
        });
        let md5 = md5.then(|| {
            let (tx, rx) = mpsc::sync_channel::<Arc<Vec<u8>>>(4);
            to.push(tx);
            s.spawn(move || {
                let mut d = Md5::new();
                for b in rx {
                    d.update(&b[..]);
                }
                hex::encode(d.finalize())
            })
        });
        let mut crc = crc32fast::Hasher::new();
        let mut size = 0u64;
        let read = (|| -> io::Result<()> {
            // blocks handed on, kept to be read into again once the threads are done with them
            let mut sent: VecDeque<Arc<Vec<u8>>> = VecDeque::new();
            let mut block = first;
            loop {
                crc.update(&block);
                each(&block)?;
                size += block.len() as u64;
                let full = block.len() == BLOCK;
                let b = Arc::new(block);
                for tx in &to {
                    tx.send(b.clone()).map_err(|_| io::Error::other("a hashing thread stopped"))?;
                }
                if !full {
                    return Ok(());
                }
                sent.push_back(b);
                if cancel.load(Ordering::Relaxed) {
                    return Err(stopped());
                }
                let free = sent.front().is_some_and(|b| Arc::strong_count(b) == 1);
                let mut next = match free {
                    true => sent.pop_front().and_then(|b| Arc::try_unwrap(b).ok()).unwrap_or_default(),
                    false => {
                        if sent.len() > 8 {
                            sent.pop_front();
                        }
                        vec![]
                    }
                };
                next.resize(BLOCK, 0);
                let n = fill(r, &mut next)?;
                if n == 0 {
                    return Ok(());
                }
                next.truncate(n);
                block = next;
            }
        })();
        drop(to);
        let sha1 = sha1.map(|h| h.join().expect("the SHA1 thread"));
        let md5 = md5.map(|h| h.join().expect("the MD5 thread"));
        read?;
        Ok(Hashes { size, crc32: format!("{:08x}", crc.finalize()), md5, sha1, header: None, hl: None })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(bytes: &[u8], name: &str) -> Hashes {
        hash_reader(&mut &bytes[..], bytes.len() as u64, name, &AtomicBool::new(false)).unwrap()
    }

    #[test]
    fn known_hashes() {
        let x = h(b"The quick brown fox jumps over the lazy dog", "fox.txt");
        assert_eq!(x.crc32, "414fa339");
        assert_eq!(x.md5.as_deref(), Some("9e107d9d372bb6826bd81d3542a419d6"));
        assert_eq!(x.sha1.as_deref(), Some("2fd4e1c67a2d28fced849ee1bb76e7391b93eb12"));
        assert!(x.header.is_none() && x.hl.is_none());
        let empty = h(b"", "empty.gb");
        assert_eq!((empty.size, empty.crc32.as_str()), (0, "00000000"));
    }

    #[test]
    fn headers_get_a_second_set_of_hashes() {
        let rom: Vec<u8> = (0..40_000u32).map(|i| (i * 7 % 251) as u8).collect();
        let plain = h(&rom, "x.nes");
        let mut ines = b"NES\x1a\x02\x01\0\0\0\0\0\0\0\0\0\0".to_vec();
        ines.extend_from_slice(&rom);
        let with = h(&ines, "x.nes");
        assert_eq!(with.header.as_deref(), Some("ines"));
        assert_eq!(with.hl.as_deref().unwrap().sha1, plain.sha1);
        assert_ne!(with.sha1, plain.sha1);

        let mut a78 = vec![0u8; 128];
        a78[1..10].copy_from_slice(b"ATARI7800");
        a78.extend_from_slice(&rom);
        assert_eq!(h(&a78, "x.a78").hl.unwrap().crc32, plain.crc32);

        let snes: Vec<u8> = (0..2048u32).map(|i| i as u8).collect();
        let mut copier = vec![0u8; 512];
        copier.extend_from_slice(&snes);
        assert_eq!(h(&copier, "x.smc").hl.unwrap().md5, h(&snes, "x.sfc").md5);
        assert!(h(&copier, "x.bin").hl.is_none(), "the copier rule is only for SNES files");
        assert!(h(&snes, "x.sfc").hl.is_none());
    }

    #[test]
    fn n64_byte_orders_hash_as_big_endian() {
        // a z64 ROM larger than a block, so the swap crosses block boundaries
        let mut z64 = vec![0x80, 0x37, 0x12, 0x40];
        z64.extend((0..(BLOCK as u32 + 4096)).map(|i| (i % 253) as u8));
        let big = h(&z64, "x.z64");
        assert!(big.header.is_none());
        let v64: Vec<u8> = z64.chunks(2).flat_map(|w| [w[1], w[0]]).collect();
        let le: Vec<u8> = z64.chunks(4).flat_map(|w| [w[3], w[2], w[1], w[0]]).collect();
        let sv = h(&v64, "x.v64");
        assert_eq!(sv.header.as_deref(), Some("n64-v64"));
        assert_eq!(sv.hl.unwrap().sha1, big.sha1);
        let sl = h(&le, "x.n64");
        assert_eq!(sl.header.as_deref(), Some("n64-le"));
        assert_eq!(sl.hl.unwrap().sha1, big.sha1);
    }

    #[test]
    fn a_big_file_hashes_the_same_on_threads() {
        let big: Vec<u8> = (0..(THREADED as u32 + 12_345)).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
        let mut one = Digests::new();
        one.update(&big);
        let want = one.finish();
        assert_eq!(h(&big, "x.iso"), want);
        // only CRC32 and SHA1, every byte handed on
        let mut copy = vec![];
        let got = read_hashed(&mut &big[..], true, &AtomicBool::new(false), &mut |b| {
            copy.extend_from_slice(b);
            Ok(())
        })
        .unwrap();
        assert_eq!((got.size, &got.crc32, &got.sha1, got.md5), (want.size, &want.crc32, &want.sha1, None));
        assert!(copy == big);
        // small, empty, and exactly a block
        for n in [0, 10, BLOCK, BLOCK + 1] {
            let mut d = Digests::new();
            d.update(&big[..n]);
            let w = d.finish();
            let got = identity(&mut &big[..n], &AtomicBool::new(false)).unwrap();
            assert_eq!((got.size, got.crc32, got.sha1), (w.size, w.crc32, w.sha1), "{n} bytes");
        }
        let stop = AtomicBool::new(true);
        assert_eq!(identity(&mut &big[..], &stop).unwrap_err().kind(), io::ErrorKind::Interrupted);
        // CRC32 alone
        let crc = read_hashed(&mut &big[..], false, &AtomicBool::new(false), &mut |_| Ok(())).unwrap();
        assert_eq!((crc.size, &crc.crc32, crc.sha1, crc.md5), (want.size, &want.crc32, None, None));
        // a copy that fails part way says so
        let e = read_hashed(&mut &big[..], false, &AtomicBool::new(false), &mut |_| Err(io::Error::other("disk full"))).unwrap_err();
        assert_eq!(e.to_string(), "disk full");
    }

    #[test]
    fn stops_when_asked() {
        let data = vec![0u8; 3 * BLOCK];
        let stop = AtomicBool::new(true);
        let e = hash_reader(&mut &data[..], data.len() as u64, "x", &stop).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::Interrupted);
    }
}
