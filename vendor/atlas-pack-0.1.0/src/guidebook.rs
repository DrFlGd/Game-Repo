//! Guidebooks in a pack file (version 8 of the format): published guides a
//! person has as one file, a PDF or a CBZ (a ZIP of page pictures), each
//! with the text of its pages when it is known.
//!
//! A guidebook is the entry `guidebooks/<name>.pdf` or
//! `guidebooks/<name>.cbz`, stored as it is (not deflated), so a reader can
//! read any part of it without unpacking it. The text of its pages, when
//! the file has it, is the entry `guidebooks/<name>.text.json`
//! ([`GuidebookText`]), stored too. The description lists each guidebook
//! ([`GuidebookFacts`]); the pack's own file `guidebooks.json`
//! ([`GuidebookList`]) lists them again for an app that reads the pack
//! unpacked into a folder.

use serde::{Deserialize, Serialize};

/// The folder of guidebooks in a pack: `guidebooks/<name>.pdf`,
/// `guidebooks/<name>.cbz` and `guidebooks/<name>.text.json`.
pub const GUIDEBOOK_DIR: &str = "guidebooks/";
/// The pack's own list of its guidebooks, beside `manifest.json`.
pub const GUIDEBOOK_LIST_FILE: &str = "guidebooks.json";
/// `format` of [`GuidebookList`].
pub const GUIDEBOOK_LIST_FORMAT: &str = "atlas-guidebooks";
/// `format` of a guidebook's text file ([`GuidebookText`]).
pub const GUIDEBOOK_TEXT_FORMAT: &str = "atlas-guidebook-text";
/// The version of the text file's format this crate writes and reads.
pub const GUIDEBOOK_TEXT_VERSION: u32 = 1;

/// The most guidebooks one file holds.
pub const MAX_GUIDEBOOKS: usize = 100;
/// The longest title of a guidebook, in characters.
pub const MAX_GUIDEBOOK_TITLE: usize = 200;
/// The longest name of a guidebook's entry, without `guidebooks/` and its
/// extension.
pub const MAX_GUIDEBOOK_NAME: usize = 80;
/// The largest guidebook file: 16 GB. A file over 4 GB is written with ZIP64
/// records.
pub const MAX_GUIDEBOOK_BYTES: u64 = 16 << 30;
/// The most pages one guidebook has.
pub const MAX_GUIDEBOOK_PAGES: u32 = 100_000;
/// The largest text file of one guidebook.
pub const MAX_GUIDEBOOK_TEXT_BYTES: u64 = 64 << 20;

/// What a guidebook file is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GuidebookType {
    /// A PDF file (`.pdf`).
    Pdf,
    /// A ZIP of page pictures (`.cbz`), read in the order of their names.
    Cbz,
}

impl GuidebookType {
    /// Its extension, without the dot.
    pub fn extension(self) -> &'static str {
        match self {
            GuidebookType::Pdf => "pdf",
            GuidebookType::Cbz => "cbz",
        }
    }

    /// The type of a file by its name's extension (any case); none for
    /// another file. A CBR file is not read: turn it into a CBZ first.
    pub fn of_file_name(name: &str) -> Option<Self> {
        let (_, extension) = name.rsplit_once('.')?;
        if extension.eq_ignore_ascii_case("pdf") {
            Some(GuidebookType::Pdf)
        } else if extension.eq_ignore_ascii_case("cbz") {
            Some(GuidebookType::Cbz)
        } else {
            None
        }
    }
}

/// One guidebook, as the description lists it (`guidebooks[]`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GuidebookFacts {
    /// Its entry name: `guidebooks/<name>.pdf` or `guidebooks/<name>.cbz`
    /// (see [`is_guidebook_name`]).
    pub file: String,
    /// Its title, as it is shown.
    pub title: String,
    /// How many pages it has: for a CBZ, its pictures.
    pub pages: u32,
    /// The size of its file in bytes.
    pub bytes: u64,
    /// The entry name of the text of its pages,
    /// `guidebooks/<name>.text.json` with the name of its file; absent
    /// when the file holds no text of it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

impl GuidebookFacts {
    /// Whether it is a PDF or a CBZ, by its entry name.
    pub fn file_type(&self) -> GuidebookType {
        GuidebookType::of_file_name(&self.file).unwrap_or(GuidebookType::Pdf)
    }
}

/// The pack's own list of its guidebooks (`guidebooks.json`), as the
/// description lists them, with the game they are for.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GuidebookList {
    /// Always `atlas-guidebooks`.
    pub format: String,
    /// Always 1.
    pub version: u32,
    pub game: crate::GameFacts,
    pub guidebooks: Vec<GuidebookFacts>,
}

/// The text of a guidebook's pages (`guidebooks/<name>.text.json`):
///
/// ```json
/// {"format": "atlas-guidebook-text", "version": 1,
///  "pages": [{"page": 1, "text": "Hollow Marsh\nThe official guide"}]}
/// ```
///
/// Pages are numbered from 1, in increasing order, each at most once; a
/// page without text is left out.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GuidebookText {
    pub format: String,
    pub version: u32,
    pub pages: Vec<PageText>,
}

/// The text of one page.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PageText {
    pub page: u32,
    pub text: String,
}

impl GuidebookText {
    /// The text of `pages` (the first is page 1), leaving out pages with no
    /// text but space.
    pub fn of_pages(pages: &[String]) -> Self {
        Self {
            format: GUIDEBOOK_TEXT_FORMAT.to_owned(),
            version: GUIDEBOOK_TEXT_VERSION,
            pages: pages
                .iter()
                .enumerate()
                .filter(|(_, text)| !text.trim().is_empty())
                .map(|(index, text)| PageText {
                    page: u32::try_from(index + 1).unwrap_or(u32::MAX),
                    text: text.clone(),
                })
                .collect(),
        }
    }
}

/// Reads a guidebook's text file: at most [`MAX_GUIDEBOOK_TEXT_BYTES`], in
/// its format and version, its pages numbered from 1 in increasing order up
/// to `pages`. A plain sentence when it cannot be used.
pub fn parse_guidebook_text(bytes: &[u8], pages: u32) -> Result<GuidebookText, String> {
    let unusable = || "The text of this guidebook cannot be read.".to_owned();
    if bytes.len() as u64 > MAX_GUIDEBOOK_TEXT_BYTES {
        return Err("The text of this guidebook is too large.".to_owned());
    }
    let text: GuidebookText = serde_json::from_slice(bytes).map_err(|_| unusable())?;
    if text.format != GUIDEBOOK_TEXT_FORMAT {
        return Err(unusable());
    }
    if text.version > GUIDEBOOK_TEXT_VERSION {
        return Err(
            "The text of this guidebook was written in a newer format, so it cannot be read."
                .to_owned(),
        );
    }
    let mut last = 0;
    for page in &text.pages {
        if page.page <= last || page.page > pages {
            return Err(unusable());
        }
        last = page.page;
    }
    Ok(text)
}

/// Whether `name` is an entry name a guidebook may have:
/// `guidebooks/<name>.pdf` or `guidebooks/<name>.cbz`, the name 1 to
/// [`MAX_GUIDEBOOK_NAME`] characters of lower case letters, digits, `-` and
/// `_`, starting with a letter or a digit. Lower case only, so no two names
/// differ only by case.
pub fn is_guidebook_name(name: &str) -> bool {
    stem_of(name).is_some()
}

/// The name of a guidebook's entry without its folder and extension.
fn stem_of(name: &str) -> Option<&str> {
    let rest = name.strip_prefix(GUIDEBOOK_DIR)?;
    let stem = rest
        .strip_suffix(".pdf")
        .or_else(|| rest.strip_suffix(".cbz"))?;
    is_stem(stem).then_some(stem)
}

pub(crate) fn is_stem(stem: &str) -> bool {
    (1..=MAX_GUIDEBOOK_NAME).contains(&stem.len())
        && stem.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && stem
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// The entry name of the text of the guidebook `file` names:
/// `guidebooks/<name>.text.json`; none for a name that is not a
/// guidebook's.
pub fn text_name_of(file: &str) -> Option<String> {
    stem_of(file).map(|stem| format!("{GUIDEBOOK_DIR}{stem}.text.json"))
}

/// An entry name for a guidebook titled `title`, of `file_type`, that none
/// of `taken` has: its title in lower case letters and digits joined by
/// `-`, with `-2`, `-3`… after it when that is taken.
pub fn guidebook_name_of(title: &str, file_type: GuidebookType, taken: &[&str]) -> String {
    let mut stem = String::new();
    for word in title
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
    {
        if stem.len() + word.len() + 1 > MAX_GUIDEBOOK_NAME - 4 {
            break;
        }
        if !stem.is_empty() {
            stem.push('-');
        }
        stem.push_str(&word.to_ascii_lowercase());
    }
    if stem.is_empty() {
        stem.push_str("guidebook");
    }
    // Two guidebooks never share a name, whatever their types.
    let used = |stem: &str| {
        taken
            .iter()
            .any(|name| stem_of(name).is_some_and(|other| other == stem))
    };
    let mut candidate = stem.clone();
    let mut next = 2;
    while used(&candidate) {
        candidate = format!("{stem}-{next}");
        next += 1;
    }
    format!("{GUIDEBOOK_DIR}{candidate}.{}", file_type.extension())
}

/// Checks the guidebooks a description lists: at most [`MAX_GUIDEBOOKS`],
/// each with a safe entry name of its own, a title, a number of pages and
/// a size within the limits, and a text entry, when it has one, of the same
/// name. A plain sentence when one cannot be used.
pub(crate) fn check_guidebooks(guidebooks: &[GuidebookFacts]) -> Result<(), String> {
    if guidebooks.len() > MAX_GUIDEBOOKS {
        return Err("This pack file lists too many guidebooks.".to_owned());
    }
    let unusable = || "This pack file lists a guidebook that cannot be used.".to_owned();
    let mut stems: Vec<&str> = Vec::with_capacity(guidebooks.len());
    for guidebook in guidebooks {
        let stem = stem_of(&guidebook.file).ok_or_else(unusable)?;
        if stems.contains(&stem) {
            return Err(unusable());
        }
        stems.push(stem);
        let title = guidebook.title.trim();
        if title.is_empty() || title.chars().count() > MAX_GUIDEBOOK_TITLE {
            return Err(unusable());
        }
        if !(1..=MAX_GUIDEBOOK_PAGES).contains(&guidebook.pages)
            || !(1..=MAX_GUIDEBOOK_BYTES).contains(&guidebook.bytes)
        {
            return Err(unusable());
        }
        if let Some(text) = &guidebook.text {
            if text_name_of(&guidebook.file).as_deref() != Some(text.as_str()) {
                return Err(unusable());
            }
        }
    }
    Ok(())
}
