//! Reading the start of a pack file in order: the description, then the
//! small pictures right after it. Nothing else of the file is read, so a
//! file that is cut short after them, or one being read from a stream that
//! cannot go back, works as well as a whole one.

use std::{
    fs::File,
    io::{BufReader, Read},
    path::Path,
};

use zip::{read::read_zipfile_from_stream, CompressionMethod};

use crate::{
    is_preview_name, parse_header, Header, Refusal, RefusalKind, HEADER_FILE, MAX_HEADER_BYTES,
    MAX_PREVIEWS, MAX_PREVIEW_BYTES, PREVIEW_DIR,
};

fn no_such_preview() -> Refusal {
    Refusal::new(
        RefusalKind::NoSuchPicture,
        "This pack file has no small picture of that name.",
    )
}

/// Reads a pack file's description, its first entry, without reading the
/// rest of the file. Refuses a file that is not a pack file, and one made
/// in a newer version of the format, with a message that names the app
/// that made it.
pub fn read_header(file: &Path) -> Result<Header, Refusal> {
    read_header_from(open(file)?)
}

/// [`read_header`] from the start of a file being read in order, such as
/// one that a system's file picker opened and that cannot seek: only the
/// first entry is read.
pub fn read_header_from(file: impl Read) -> Result<Header, Refusal> {
    first_entry(&mut BufReader::new(file))
}

/// Reads one small picture of a pack file by its entry name
/// (`preview/<name>.jpg`), which its description must list. Only the
/// description and the small pictures before it are read. Refuses a name
/// that is not a safe one, a picture larger than [`MAX_PREVIEW_BYTES`],
/// and one that is not a JPEG.
pub fn read_preview(file: &Path, name: &str) -> Result<Vec<u8>, Refusal> {
    if !is_preview_name(name) {
        return Err(no_such_preview());
    }
    read_preview_from(open(file)?, name)
}

/// [`read_preview`] from the start of a file being read in order.
pub fn read_preview_from(file: impl Read, name: &str) -> Result<Vec<u8>, Refusal> {
    if !is_preview_name(name) {
        return Err(no_such_preview());
    }
    let mut reader = BufReader::new(file);
    let header = first_entry(&mut reader)?;
    if !header.lists_preview(name) {
        return Err(no_such_preview());
    }
    let damaged =
        || Refusal::damaged("This pack file is damaged: its small pictures cannot be read.");
    for _ in 0..MAX_PREVIEWS {
        let Some(mut entry) = read_zipfile_from_stream(&mut reader).map_err(|_| damaged())? else {
            return Err(no_such_preview());
        };
        let entry_name = entry.name().map_err(|_| damaged())?.into_owned();
        // The small pictures come right after the description; past them
        // there is none.
        if !entry_name.starts_with(PREVIEW_DIR) {
            return Err(no_such_preview());
        }
        let size = entry.size();
        if !is_preview_name(&entry_name)
            || entry.is_dir()
            || entry.is_symlink()
            || size > MAX_PREVIEW_BYTES
            || !matches!(
                entry.compression(),
                CompressionMethod::Stored | CompressionMethod::Deflated
            )
        {
            return Err(Refusal::damaged(
                "This pack file holds a small picture that cannot be read.",
            ));
        }
        let mut bytes = Vec::with_capacity(usize::try_from(size).unwrap_or(0));
        (&mut entry)
            .take(MAX_PREVIEW_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| damaged())?;
        if bytes.len() as u64 != size {
            return Err(damaged());
        }
        if entry_name == name {
            if !is_jpeg(&bytes) {
                return Err(Refusal::damaged(
                    "The small picture in this pack file is not a JPEG picture.",
                ));
            }
            return Ok(bytes);
        }
    }
    Err(no_such_preview())
}

fn open(file: &Path) -> Result<File, Refusal> {
    File::open(file).map_err(|error| {
        Refusal::new(
            RefusalKind::CannotOpen,
            format!("Could not open the file: {error}"),
        )
    })
}

/// The description, read from the first entry of a stream, at most
/// [`MAX_HEADER_BYTES`]; the stream is left at the next entry.
fn first_entry<R: Read>(reader: &mut R) -> Result<Header, Refusal> {
    let mut entry = match read_zipfile_from_stream(reader) {
        Ok(Some(entry)) => entry,
        _ => return Err(Refusal::not_a_pack_file()),
    };
    if entry.name().ok().as_deref() != Some(HEADER_FILE) || entry.size() > MAX_HEADER_BYTES {
        return Err(Refusal::not_a_pack_file());
    }
    let mut text = String::new();
    (&mut entry)
        .take(MAX_HEADER_BYTES)
        .read_to_string(&mut text)
        .map_err(|_| Refusal::not_a_pack_file())?;
    // Dropping the entry passes over what is left of it, so the stream is
    // at the next entry.
    drop(entry);
    parse_header(&text)
}

/// Whether bytes start as a JPEG picture does.
fn is_jpeg(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xff, 0xd8, 0xff])
}
