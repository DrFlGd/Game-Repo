//! How many pages a PDF has, read without a PDF library: the largest
//! `/Count` of a page tree node (`/Type /Pages`), looked for in the file as
//! it is and in the object streams it packs objects in (deflated with
//! `/FlateDecode`). The file is read in parts, never whole, and every part
//! of the search is bounded, so a damaged or hostile file only gives no
//! answer. An encrypted PDF whose page tree is in an object stream gives
//! none either: the caller then says how many pages it has.

use std::io::{Read, Seek, SeekFrom};

use flate2::read::ZlibDecoder;

/// Bytes read at once, and how much two reads overlap: a dictionary up to
/// [`OVERLAP`] long is whole in one of them.
const CHUNK: usize = 4 << 20;
const OVERLAP: usize = 256 << 10;
/// The most page tree nodes and object streams looked at.
const MAX_NODES: usize = 100_000;
const MAX_STREAMS: usize = 4096;
/// The largest object stream read, packed and unpacked.
const MAX_STREAM_BYTES: u64 = 32 << 20;
const MAX_UNPACKED_BYTES: u64 = 64 << 20;
/// How far around a name its dictionary is looked for.
const MAX_DICT: usize = OVERLAP;

/// Whether `start` (the first bytes of a file) is the start of a PDF.
pub(crate) fn looks_like_pdf(start: &[u8]) -> bool {
    find(&start[..start.len().min(1024)], b"%PDF-", 0).is_some()
}

/// The number of pages of the PDF `file` of `len` bytes, when it can be
/// read; none otherwise.
pub(crate) fn pdf_pages<R: Read + Seek>(file: &mut R, len: u64) -> Option<u32> {
    let mut most: u64 = 0;
    let mut streams: Vec<(u64, u64)> = Vec::new();
    let mut nodes = 0usize;
    let mut at: u64 = 0;
    let mut buffer = vec![0u8; CHUNK];
    while at < len {
        file.seek(SeekFrom::Start(at)).ok()?;
        let want = usize::try_from((len - at).min(CHUNK as u64)).ok()?;
        file.read_exact(&mut buffer[..want]).ok()?;
        let part = &buffer[..want];
        most = most.max(largest_count(part, &mut nodes));
        object_streams(part, at, &mut streams);
        if want < CHUNK {
            break;
        }
        at += (CHUNK - OVERLAP) as u64;
    }
    streams.sort_unstable();
    streams.dedup();
    for (start, length) in streams {
        if start.checked_add(length).is_none_or(|end| end > len) {
            continue;
        }
        file.seek(SeekFrom::Start(start)).ok()?;
        let mut packed = Vec::with_capacity(usize::try_from(length).ok()?);
        file.by_ref().take(length).read_to_end(&mut packed).ok()?;
        let mut unpacked = Vec::new();
        if ZlibDecoder::new(&packed[..])
            .take(MAX_UNPACKED_BYTES)
            .read_to_end(&mut unpacked)
            .is_err()
        {
            continue;
        }
        most = most.max(largest_count(&unpacked, &mut nodes));
    }
    u32::try_from(most).ok().filter(|&pages| pages > 0)
}

/// The largest `/Count` of a page tree node in `bytes`; `nodes` counts the
/// nodes looked at, up to [`MAX_NODES`].
fn largest_count(bytes: &[u8], nodes: &mut usize) -> u64 {
    let mut most = 0;
    let mut from = 0;
    while let Some(at) = find(bytes, b"/Pages", from) {
        from = at + 6;
        if *nodes >= MAX_NODES {
            break;
        }
        // `/Type /Pages`, not `/Pages 3 0 R` nor `/PagesMode`.
        if bytes.get(at + 6).is_some_and(|&c| is_regular(c)) || !after_type(bytes, at) {
            continue;
        }
        *nodes += 1;
        let Some((start, end)) = dictionary_around(bytes, at) else {
            continue;
        };
        if let Some(count) = direct_value(&bytes[start..end], b"/Count") {
            most = most.max(count);
        }
    }
    most
}

/// Whether the name at `at` follows `/Type` and space.
fn after_type(bytes: &[u8], at: usize) -> bool {
    let mut i = at;
    while i > 0 && is_space(bytes[i - 1]) {
        i -= 1;
    }
    i >= 5 && &bytes[i - 5..i] == b"/Type"
}

/// Records where the data of each object stream (`/Type /ObjStm`) deflated
/// with `/FlateDecode` and with a direct `/Length` starts in the file, and
/// its length; `base` is where `bytes` start in the file.
fn object_streams(bytes: &[u8], base: u64, streams: &mut Vec<(u64, u64)>) {
    let mut from = 0;
    while let Some(at) = find(bytes, b"/ObjStm", from) {
        from = at + 7;
        if streams.len() >= MAX_STREAMS {
            return;
        }
        let Some((start, end)) = dictionary_around(bytes, at) else {
            continue;
        };
        let dict = &bytes[start..end];
        if find(dict, b"/FlateDecode", 0).is_none() || find(dict, b"/DecodeParms", 0).is_some() {
            continue;
        }
        let Some(length) = direct_value(dict, b"/Length") else {
            continue;
        };
        if length == 0 || length > MAX_STREAM_BYTES {
            continue;
        }
        // `>>`, space, `stream`, then an end of line.
        let mut i = end + 2;
        while i < bytes.len() && is_space(bytes[i]) {
            i += 1;
        }
        if !bytes[i..].starts_with(b"stream") {
            continue;
        }
        i += 6;
        if bytes[i..].starts_with(b"\r\n") {
            i += 2;
        } else if bytes.get(i).is_some_and(|&c| c == b'\n' || c == b'\r') {
            i += 1;
        } else {
            continue;
        }
        streams.push((base + i as u64, length));
    }
}

/// The dictionary `<< … >>` that holds `at`: where its `<<` and its `>>`
/// start, within [`MAX_DICT`] bytes on each side.
fn dictionary_around(bytes: &[u8], at: usize) -> Option<(usize, usize)> {
    let lowest = at.saturating_sub(MAX_DICT);
    let mut depth = 0usize;
    let mut i = at;
    let start = loop {
        if i < lowest + 2 {
            return None;
        }
        match &bytes[i - 2..i] {
            b">>" => {
                depth += 1;
                i -= 2;
            }
            b"<<" => {
                if depth == 0 {
                    break i - 2;
                }
                depth -= 1;
                i -= 2;
            }
            _ => i -= 1,
        }
    };
    let highest = (at + MAX_DICT).min(bytes.len());
    let mut i = at;
    while i + 2 <= highest {
        match &bytes[i..i + 2] {
            b"<<" => {
                depth += 1;
                i += 2;
            }
            b">>" => {
                if depth == 0 {
                    return Some((start, i));
                }
                depth -= 1;
                i += 2;
            }
            _ => i += 1,
        }
    }
    None
}

/// The whole number after `key` at the top level of a dictionary's bytes
/// (from its `<<`), when it is direct: not a reference such as `12 0 R`.
fn direct_value(dict: &[u8], key: &[u8]) -> Option<u64> {
    let mut depth = 0usize;
    let mut i = 2;
    while i < dict.len() {
        if dict[i..].starts_with(b"<<") {
            depth += 1;
            i += 2;
        } else if dict[i..].starts_with(b">>") {
            depth = depth.saturating_sub(1);
            i += 2;
        } else if depth == 0
            && dict[i..].starts_with(key)
            && !dict.get(i + key.len()).is_some_and(|&c| is_regular(c))
        {
            let mut j = i + key.len();
            while j < dict.len() && is_space(dict[j]) {
                j += 1;
            }
            let digits = dict[j..].iter().take_while(|c| c.is_ascii_digit()).count();
            if digits == 0 || digits > 12 {
                return None;
            }
            let value: u64 = std::str::from_utf8(&dict[j..j + digits])
                .ok()?
                .parse()
                .ok()?;
            // A reference: `<number> <generation> R`.
            let mut k = j + digits;
            while k < dict.len() && is_space(dict[k]) {
                k += 1;
            }
            let generation = dict[k..].iter().take_while(|c| c.is_ascii_digit()).count();
            if generation > 0 {
                let mut r = k + generation;
                while r < dict.len() && is_space(dict[r]) {
                    r += 1;
                }
                if dict.get(r) == Some(&b'R') {
                    return None;
                }
            }
            return Some(value);
        } else {
            i += 1;
        }
    }
    None
}

fn find(bytes: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if from >= bytes.len() {
        return None;
    }
    bytes[from..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|at| at + from)
}

/// PDF white space.
fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' | b'\0')
}

/// A character that continues a name: neither space nor a delimiter.
fn is_regular(c: u8) -> bool {
    !is_space(c) && !b"()<>[]{}/%".contains(&c)
}
