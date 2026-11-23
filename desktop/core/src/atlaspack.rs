//! Atlas pack files (`.atlaspack`, docs/PLAN.md, D215): what Game Repo reads of a pack
//! Atlas Importer wrote, by the *Atlas pack contract for Game Repo*.
//!
//! The reading is Atlas's own `atlas-pack` crate (the contract's block A3), a git
//! dependency pinned to Atlas's release tag v0.29.0 (format 10: packs of magazines, after
//! format 9's routes and a map's own icons kept for the person's own use); its
//! `docs/pack-file-format.md` is the reference for the format.
//! [`read_header`] reads the description from the front of the file only, and
//! [`read_preview`] one small picture the description lists, from the entries right behind
//! it. A file that's wrong is refused with the crate's [`Refusal`]: its sentence (not a pack
//! file, a newer format naming the app that wrote it, a damaged description, a file that
//! couldn't be opened), shown as it is, Game Repo doesn't guess; and its kind ([`kind_of`]),
//! kept beside the sentence. Nothing in a pack is ever unpacked to disk by the names inside
//! it, and no address in it is ever fetched. Only the small pictures under `preview/` are
//! ever read (the cover and the maps'), never a type's picture of a map's own icons (format
//! 9, `shareable: false`), which the contract says never leaves the person's own devices.
//!
//! What's Game Repo's own here: [`Description`], the crate's `Header` as a card reads it,
//! every field optional (a format 6 pack has no `game`, `maps`, `guides` or `cover`), so it
//! can also be kept in the index and read back from there; which of two files of a pack is
//! the newer ([`Description::newer`]); the name the contract gives a pack's file; and
//! whether a file's name says it's a pack.

use serde::Serialize;
use serde_json::Value;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::path::Path;

pub use atlas_pack::{Refusal, RefusalKind, EXTENSION, FORMAT, VERSION};

/// The release of Atlas's crate this build reads packs with (the tag `Cargo.toml` pins):
/// when it changes, every pack in the index is read again once (`maps::read_folder`), as a
/// newer crate reads more of a description (v0.25.0: `source.origin`; v0.26.0: `mine.routes`
/// and `contents.typePictures`; v0.29.0: `magazine` and `issues`) and newer formats (v0.25.0:
/// format 8, packs of guidebooks; v0.26.0: format 9, routes and a map's own icons; v0.29.0:
/// format 10, packs of magazines), which the index kept refused.
pub const READER: &str = "atlas-pack v0.29.0";

/// A pack's description, read from the front of its file by Atlas's crate: refused, with
/// the crate's [`Refusal`], when it isn't a pack file, needs a newer format than
/// [`VERSION`], is damaged (a field every pack has is missing, or its `packId` can't be
/// one), or can't be opened. The file is opened only for this one read.
pub fn read_header(path: &Path) -> Result<Description, Refusal> {
    let h = atlas_pack::read_header(path)?;
    Ok(Description::from_value(serde_json::to_value(&h).unwrap_or(Value::Null)))
}

/// One small picture of a pack (its cover or a map's) by the name its description gives,
/// read by Atlas's crate: a safe `preview/<name>.jpg` name the description lists, right
/// behind the description, a JPEG of at most 512 KB.
pub fn read_preview(path: &Path, name: &str) -> Result<Vec<u8>, Refusal> {
    atlas_pack::read_preview(path, name)
}

/// A refusal's kind as a word for the index and the page: `not-a-pack-file`,
/// `newer-format`, `damaged`, `cannot-open`, `no-such-picture` (`other` for a kind a newer
/// crate adds).
pub fn kind_of(r: &Refusal) -> &'static str {
    match r.kind() {
        RefusalKind::NotAPackFile => "not-a-pack-file",
        RefusalKind::NewerFormat => "newer-format",
        RefusalKind::Damaged => "damaged",
        RefusalKind::CannotOpen => "cannot-open",
        RefusalKind::NoSuchPicture => "no-such-picture",
        _ => "other",
    }
}

// ------------------------------------------------------------ the description

/// The app that wrote a pack (`exportedBy`).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Exporter {
    pub name: Option<String>,
    pub version: Option<String>,
    pub platform: Option<String>,
}

/// Where a pack was read from (`source`). The addresses it read (`source.pages`) are
/// counted, never kept here and never fetched.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Source {
    pub kind: Option<String>,
    /// The site's name where the pack records it, else its host name (block A3; a field of
    /// format 7's, in a file of any version)
    pub name: Option<String>,
    pub importer_id: Option<String>,
    pub importer_title: Option<String>,
    pub newest_page_at: Option<String>,
    pub oldest_page_at: Option<String>,
    pub pages: usize,
}

/// The game a pack is for (`game`, block A3): `title` is the title the packs were put under
/// in Atlas Importer.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Game {
    pub title: Option<String>,
    /// As the site named it: a hint for matching, never proof
    pub system: Option<String>,
}

/// A map in a pack (`maps[]`, block A3).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct MapInfo {
    pub id: Option<String>,
    pub name: Option<String>,
    pub width: Option<u64>,
    pub height: Option<u64>,
    pub locations: Option<u64>,
    /// Its small picture's name in the pack (`preview/<name>.jpg`); none for a map with no
    /// image, and none in a file below version 7
    pub preview: Option<String>,
}

/// A pack's pages, as a guide or a wiki (`guides[]`: one entry at most, as Atlas writes it).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct GuideInfo {
    pub title: Option<String>,
    /// guide (pages read in order) | wiki; any other is a type a newer Atlas knows ("other"
    /// as the crate reads it), shown as a guide
    pub kind: Option<String>,
    pub pages: Option<u64>,
}

/// The maker's own things a pack carries under `mine/` (`mine`: a map made in Atlas,
/// written with the person's own lists, locations, pages, notes and photos): how many of
/// each. Game Repo only says so; it never reads, strips or changes `mine/`, and a pack is
/// always copied, moved and sent whole.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Mine {
    pub lists: u64,
    pub locations: u64,
    pub types: u64,
    pub areas: u64,
    pub photos: u64,
    pub pages: u64,
    pub sections: u64,
    pub notes: u64,
    /// Their own routes (`mine/my-routes.json`, format 9)
    pub routes: u64,
    pub guide_links: bool,
    pub group: bool,
}

/// A pack's description (`atlaspack.json`), as a card reads it: every field optional, the
/// whole of it kept in `raw` (the crate's `Header` as JSON, which is what the index keeps).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Description {
    /// The lowest format needed to read it (absent: 1)
    pub version: u64,
    /// The pack's identity: the same every time it's written again
    pub pack_id: Option<String>,
    pub title: Option<String>,
    pub pack_version: Option<String>,
    /// overview | standard | full
    pub detail: Option<String>,
    pub exported_at: Option<String>,
    pub exported_by: Exporter,
    pub created: Option<String>,
    /// `pack.updated`: when its content last changed (what two files of a pack compare by)
    pub updated: Option<String>,
    pub source: Source,
    /// `contents`: maps, layers, locations, categories, regions, checklists, guides…, and
    /// from format 9 `typePictures` (the types drawn with the map's own icons, which the file
    /// carries for the person's own use)
    pub contents: BTreeMap<String, u64>,
    pub files: Option<u64>,
    pub bytes: Option<u64>,
    pub game: Game,
    /// For the person's own use: `personalUse`, or the file carries a map's own icons (format
    /// 9's `contents.typePictures`), pictures kept for their own use (`shareable: false`)
    /// that make the file one for their own devices only
    pub personal_use: bool,
    pub cover: Option<String>,
    pub maps: Vec<MapInfo>,
    pub guides: Vec<GuideInfo>,
    /// The person's own things it carries (`mine`), when it carries them
    pub mine: Option<Mine>,
    /// `pack.writtenBy.name`: the program that wrote the pack's own files (Atlas, Atlas
    /// Importer, Atlas Map Maker…); only "Atlas Map Maker" tells a sort ([`Description::sort`])
    pub pack_written_by: Option<String>,
    /// Its sort as the file says it (D231): `source.origin` (Atlas 0.25.0 and later write it),
    /// or a pack of guidebooks (format 8) or of magazines (format 10, D245), read by the
    /// crate's `Header::origin`; None when the
    /// file says neither, and the contract's rule from before tells it ([`Description::sort`])
    pub origin: Option<Sort>,
    /// The whole description as it was read
    #[serde(skip)]
    pub raw: Value,
}

/// `pack.writtenBy.name` of a pack made by Atlas Map Maker, exactly (the contract).
pub const MAP_MAKER: &str = "Atlas Map Maker";

/// The sorts of pack the contract tells apart ([`Description::sort`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Sort {
    /// Read from a site by Atlas Importer
    Site,
    /// A map made in an Atlas app (Atlas, or Atlas Importer)
    MadeInAtlas,
    /// A map made by Atlas Map Maker
    MapMaker,
    /// A pack that came with an app
    App,
    /// A pack of guidebooks (format 8: published guides as PDF or CBZ files, D231)
    Guidebooks,
    /// A pack of the issues of a magazine (format 10, D245): of no game, one per magazine
    Magazines,
}

impl Sort {
    /// Its one word, as the card, Game Repo Server and Go say it: the words of Atlas's
    /// `source.origin` (site, made-in-atlas, map-maker, guidebooks, magazines), and `app`.
    pub fn word(self) -> &'static str {
        match self {
            Sort::Site => "site",
            Sort::MadeInAtlas => "made-in-atlas",
            Sort::MapMaker => "map-maker",
            Sort::App => "app",
            Sort::Guidebooks => "guidebooks",
            Sort::Magazines => "magazines",
        }
    }

    /// The sort a word names ([`Sort::word`]); None for any other word.
    pub fn from_word(w: &str) -> Option<Sort> {
        [Sort::Site, Sort::MadeInAtlas, Sort::MapMaker, Sort::App, Sort::Guidebooks, Sort::Magazines].into_iter().find(|s| s.word() == w.trim())
    }

    /// The sort the crate's `Origin` names; None for a word it doesn't know (`Other`).
    fn of_origin(o: atlas_pack::Origin) -> Option<Sort> {
        match o {
            atlas_pack::Origin::Site => Some(Sort::Site),
            atlas_pack::Origin::MadeInAtlas => Some(Sort::MadeInAtlas),
            atlas_pack::Origin::MapMaker => Some(Sort::MapMaker),
            atlas_pack::Origin::Guidebooks => Some(Sort::Guidebooks),
            atlas_pack::Origin::Magazines => Some(Sort::Magazines),
            _ => None,
        }
    }
}

/// The sort a description says itself (D231), by the crate's `Header::origin`: when its
/// `source.origin` is a word the crate knows, or it carries guidebooks (format 8) or a
/// magazine's issues (format 10). None when
/// it says neither (then [`Description::sort`] keeps the contract's rule from before: the
/// crate's own guess for such a file would take a pack that came with an app and was exported
/// by Atlas as made in Atlas), or when the JSON isn't a whole `Header` (a test's, or a
/// description from before kept in the index).
fn origin_of(raw: &Value) -> Option<Sort> {
    let h: atlas_pack::Header = serde_json::from_value(raw.clone()).ok()?;
    let says = h.source.origin.is_some_and(|o| o != atlas_pack::Origin::Other) || !h.guidebooks.is_empty() || !h.issues.is_empty();
    says.then(|| h.origin()).flatten().and_then(Sort::of_origin)
}

fn text(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(String::from)
}

fn number(v: &Value, k: &str) -> Option<u64> {
    num_of(v.get(k)?)
}

fn num_of(x: &Value) -> Option<u64> {
    x.as_u64().or_else(|| x.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64)).or_else(|| x.as_str()?.trim().parse().ok())
}

impl Description {
    /// A description already checked by [`read_header`] (the crate's `Header` as JSON, or as
    /// the index kept it): what isn't there, or isn't what it should be, is left out.
    pub fn from_value(raw: Value) -> Description {
        let v = &raw;
        let by = v.get("exportedBy").cloned().unwrap_or(Value::Null);
        let pack = v.get("pack").cloned().unwrap_or(Value::Null);
        let src = v.get("source").cloned().unwrap_or(Value::Null);
        let importer = src.get("importer").cloned().unwrap_or(Value::Null);
        let game = v.get("game").cloned().unwrap_or(Value::Null);
        let list = |k: &str| v.get(k).and_then(Value::as_array).cloned().unwrap_or_default();
        Description {
            version: number(v, "version").unwrap_or(1),
            pack_id: text(v, "packId").or_else(|| text(&pack, "id")),
            title: text(v, "title").or_else(|| text(&pack, "title")),
            pack_version: text(v, "packVersion").or_else(|| text(&pack, "packVersion")),
            detail: text(v, "detail").or_else(|| v.get("images").and_then(|i| text(i, "detail"))),
            exported_at: text(v, "exportedAt"),
            exported_by: Exporter { name: text(&by, "name"), version: text(&by, "version"), platform: text(&by, "platform") },
            created: text(&pack, "created"),
            updated: text(&pack, "updated"),
            source: Source {
                kind: text(&src, "kind"),
                name: text(&src, "name"),
                importer_id: text(&importer, "id"),
                importer_title: text(&importer, "title"),
                newest_page_at: text(&src, "newestPageAt"),
                oldest_page_at: text(&src, "oldestPageAt"),
                pages: src.get("pages").and_then(Value::as_array).map(Vec::len).unwrap_or(0),
            },
            contents: v.get("contents").and_then(Value::as_object).map(|m| m.iter().filter_map(|(k, x)| Some((k.clone(), num_of(x)?))).collect()).unwrap_or_default(),
            files: number(v, "files"),
            bytes: number(v, "bytes"),
            game: Game { title: text(&game, "title"), system: text(&game, "system") },
            personal_use: v.get("personalUse").and_then(Value::as_bool).unwrap_or(false) || v.get("contents").and_then(|c| number(c, "typePictures")).is_some_and(|n| n > 0),
            cover: text(v, "cover"),
            maps: list("maps")
                .iter()
                .filter(|m| m.is_object())
                .map(|m| MapInfo { id: text(m, "id"), name: text(m, "name"), width: number(m, "width"), height: number(m, "height"), locations: number(m, "locations"), preview: text(m, "preview") })
                .collect(),
            guides: list("guides")
                .iter()
                .filter(|g| g.is_object())
                .map(|g| GuideInfo { title: text(g, "title"), kind: text(g, "type"), pages: number(g, "pages") })
                .collect(),
            mine: v.get("mine").filter(|m| m.is_object()).map(|m| Mine {
                lists: number(m, "lists").unwrap_or(0),
                locations: number(m, "locations").unwrap_or(0),
                types: number(m, "types").unwrap_or(0),
                areas: number(m, "areas").unwrap_or(0),
                photos: number(m, "photos").unwrap_or(0),
                pages: number(m, "pages").unwrap_or(0),
                sections: number(m, "sections").unwrap_or(0),
                notes: number(m, "notes").unwrap_or(0),
                routes: number(m, "routes").unwrap_or(0),
                guide_links: m.get("guideLinks").and_then(Value::as_bool).unwrap_or(false),
                group: m.get("group").and_then(Value::as_bool).unwrap_or(false),
            }),
            pack_written_by: pack.get("writtenBy").and_then(|w| w.as_str().map(String::from).or_else(|| text(w, "name"))).map(|n| n.trim().to_string()).filter(|n| !n.is_empty()),
            origin: origin_of(&raw),
            raw,
        }
    }

    /// The title to match a game by and name its folder by: `game.title`, else `title`.
    pub fn game_title(&self) -> Option<&str> {
        self.game.title.as_deref().or(self.title.as_deref())
    }

    /// The site's name: `source.name`, else `source.importer.title`.
    pub fn source_name(&self) -> Option<&str> {
        self.source.name.as_deref().or(self.source.importer_title.as_deref())
    }

    /// The sort of pack it is, by the contract's rule (its answer of 2026-11-19, safe to build
    /// on for every pack Atlas 0.23.0 writes): `source.kind` `web-pages` is a pack read from a
    /// site; `pack.writtenBy.name` exactly "Atlas Map Maker" one made by Map Maker;
    /// `source.kind` `unknown` with another writer a map made in an Atlas app (in Atlas, or
    /// in Atlas Importer, whose packs say "Atlas Importer" there); `source.kind` `app` one that
    /// came with an app. None: it doesn't say. Never by `pack.writtenBy.name` alone, nor by
    /// `mine` (a pack exported by hand can carry your own things whatever its sort).
    ///
    /// D231: the sort the file says itself comes first ([`Description::origin`]: Atlas
    /// 0.25.0's `source.origin`, or a pack of guidebooks), read by the crate's
    /// `Header::origin`; this rule stays for the files that say neither.
    pub fn sort(&self) -> Option<Sort> {
        if let Some(o) = self.origin {
            return Some(o);
        }
        let kind = self.source.kind.as_deref();
        if kind == Some("web-pages") {
            return Some(Sort::Site);
        }
        if self.pack_written_by.as_deref() == Some(MAP_MAKER) {
            return Some(Sort::MapMaker);
        }
        match kind {
            Some("unknown") => Some(Sort::MadeInAtlas),
            Some("app") => Some(Sort::App),
            _ => None,
        }
    }

    /// The app a map was made in, for "Made in Atlas" ([`Description::sort`]): "Atlas" for
    /// one made in an Atlas app (Atlas Importer's too), "Atlas Map Maker" for one made there.
    /// None for any other sort.
    pub fn made_in(&self) -> Option<&'static str> {
        match self.sort()? {
            Sort::MadeInAtlas => Some("Atlas"),
            Sort::MapMaker => Some(MAP_MAKER),
            Sort::Site | Sort::App | Sort::Guidebooks | Sort::Magazines => None,
        }
    }

    /// The site it was read from, where a card names one: its name ([`source_name`]) for a
    /// pack read from a site, or one that doesn't say its sort; never for another sort.
    ///
    /// [`source_name`]: Description::source_name
    pub fn site(&self) -> Option<&str> {
        match self.sort() {
            Some(Sort::Site) | None => self.source_name(),
            Some(_) => None,
        }
    }

    /// Where it's from, in the words a card says it: the site's name, "Made in Atlas",
    /// "Made in Atlas Map Maker", "Came with an app", "Guidebooks" or "Magazines". None when it
    /// doesn't say.
    pub fn from_words(&self) -> Option<String> {
        match self.sort() {
            Some(Sort::App) => Some("Came with an app".into()),
            Some(Sort::Guidebooks) => Some("Guidebooks".into()),
            Some(Sort::Magazines) => Some("Magazines".into()),
            _ => self.made_in().map(|m| format!("Made in {m}")).or_else(|| self.site().map(String::from)),
        }
    }

    /// What a card says of the person's own things a pack carries (`mine`), as plain words:
    /// "With your own locations and notes"; "With your own things" when it doesn't say how
    /// many of what. None when it carries none.
    pub fn own_things(&self) -> Option<String> {
        let m = self.mine.as_ref()?;
        let kinds: Vec<&str> = [
            (m.lists, "lists"),
            (m.locations, "locations"),
            (m.types, "types"),
            (m.areas, "areas"),
            (m.pages + m.sections, "guide pages"),
            (m.notes, "notes"),
            (m.routes, "routes"),
            (m.photos, "photos"),
        ]
        .into_iter()
        .filter(|(n, _)| *n > 0)
        .map(|(_, k)| k)
        .collect();
        Some(match kinds.split_last() {
            None => "With your own things".to_string(),
            Some((last, [])) => format!("With your own {last}"),
            Some((last, rest)) => format!("With your own {} and {last}", rest.join(", ")),
        })
    }

    /// The name the contract gives a pack: `<game title> - <source name>.atlaspack`, or
    /// `<game title>.atlaspack` for a pack from no site.
    pub fn file_name(&self) -> Option<String> {
        let t = self.game_title()?;
        Some(match self.source_name() {
            Some(s) => format!("{t} - {s}.{EXTENSION}"),
            None => format!("{t}.{EXTENSION}"),
        })
    }

    /// The small pictures it names (its cover and its maps'), each only once, and only safe
    /// `preview/<name>.jpg` names: the only names [`read_preview`] reads.
    pub fn previews(&self) -> Vec<String> {
        let mut out: Vec<String> = vec![];
        for n in self.cover.iter().chain(self.maps.iter().filter_map(|m| m.preview.as_ref())) {
            if atlas_pack::is_preview_name(n) && !out.contains(n) {
                out.push(n.clone());
            }
        }
        out
    }

    /// Which of two descriptions of a pack is the newer: the later `pack.updated`, then
    /// the later `exportedAt` (Equal: neither is).
    pub fn newer(&self, other: &Description) -> Ordering {
        gamerepo_wire::packs::newer(self.updated.as_deref(), self.exported_at.as_deref(), other.updated.as_deref(), other.exported_at.as_deref())
    }

    /// Who wrote it, for a sentence: "Atlas Importer 0.22.0".
    pub fn written_by(&self) -> Option<String> {
        let n = self.exported_by.name.as_deref()?;
        Some(match &self.exported_by.version {
            Some(v) => format!("{n} {v}"),
            None => n.to_string(),
        })
    }
}

/// Whether a file's name says it's a pack: `<name>.atlaspack`, not one still being
/// written (`.part`, Atlas Importer's `.atlas-importer.part` among them).
pub fn is_pack_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(&format!(".{EXTENSION}")) && !lower.starts_with('.')
}

/// What Atlas Importer's unfinished file of a pack ends in, beside the pack it writes (the
/// contract): never shown, read, moved or deleted by Game Repo, as another computer may be
/// writing it. The Importer clears its own leftovers.
pub const IMPORTER_PART: &str = ".atlas-importer.part";

/// Whether a file is Atlas Importer's unfinished pack ([`IMPORTER_PART`]).
pub fn is_importer_part(name: &str) -> bool {
    name.to_lowercase().ends_with(IMPORTER_PART)
}

/// What a card shows of a description, as JSON for the page.
pub fn card(d: &Description) -> Value {
    let mut v = serde_json::to_value(d).unwrap_or_default();
    if let Some(o) = v.as_object_mut() {
        o.insert("game_title".into(), d.game_title().map(Value::from).unwrap_or(Value::Null));
        o.insert("source_name".into(), d.source_name().map(Value::from).unwrap_or(Value::Null));
        o.insert("written_by".into(), d.written_by().map(Value::from).unwrap_or(Value::Null));
        o.insert("made_in".into(), d.made_in().map(Value::from).unwrap_or(Value::Null));
        o.insert("sort".into(), d.sort().map(|s| Value::from(s.word())).unwrap_or(Value::Null));
        o.insert("site".into(), d.site().map(Value::from).unwrap_or(Value::Null));
        o.insert("from".into(), d.from_words().map(Value::from).unwrap_or(Value::Null));
        o.insert("own_things".into(), d.own_things().map(Value::from).unwrap_or(Value::Null));
    }
    v
}

#[cfg(test)]
pub(crate) mod tests {
    //! Game Repo's side of reading packs: what a card gets from a description, the crate's
    //! sentences passed on as they are, and the newer of two files. Packs are made here as
    //! `docs/pack-file-format.md` in Atlas's repository says Atlas writes them (the
    //! description deflated first, the small pictures stored right behind it, then the
    //! rest); made-up games and sites only.
    use super::*;
    use serde_json::json;
    use std::fs::File;
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    use zip::CompressionMethod;

    /// A small JPEG's first bytes and some more (enough to be told a JPEG).
    pub(crate) fn jpeg(seed: u8) -> Vec<u8> {
        let mut b = vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 16, b'J', b'F', b'I', b'F', 0];
        b.extend((0..200u32).map(|i| (i as u8).wrapping_mul(seed)));
        b.extend([0xFF, 0xD9]);
        b
    }

    /// A description of a made-up game and site with only what a pack from before block A3
    /// has (and a field from later, which a reader ignores).
    pub(crate) fn format6(id: &str, title: &str, updated: &str) -> Value {
        json!({
            "format": "atlas-pack-file", "version": 1, "packId": id, "title": title, "packVersion": "1.0.0", "detail": "full",
            "exportedAt": "2026-11-18T21:04:11Z",
            "exportedBy": { "name": "Atlas Importer", "version": "0.22.0", "platform": "windows" },
            "pack": { "id": id, "title": title, "packVersion": "1.0.0", "schemaVersion": 2, "attribution": [], "created": "2026-11-02T10:00:00Z", "updated": updated },
            "source": { "kind": "web-pages", "importer": { "id": "example-maps", "title": "Example Maps" },
                "pages": [{ "url": "https://maps.example/a-page", "mapId": "map:one", "importedAt": updated }],
                "newestPageAt": updated, "oldestPageAt": "2026-11-02T10:00:00Z" },
            "contents": { "maps": 2, "layers": 2, "locations": 1840, "categories": 46, "regions": 12, "checklists": 3, "guides": 210 },
            "images": { "detail": "full", "files": 21844, "bytes": 913000000, "layers": [] },
            "files": 21861, "bytes": 931000000,
            "aFieldFromLater": { "kept": true }
        })
    }

    /// Block A3's fields over [`format6`], as built: the game, the site's name, the maps with
    /// their small pictures (a third map with no image has none), one entry of pages (a
    /// wiki), and the cover, the first map's picture; version 7, as it carries pictures.
    pub(crate) fn format7(id: &str, game: &str, updated: &str) -> Value {
        let mut d = format6(id, game, updated);
        let o = d.as_object_mut().unwrap();
        o.insert("version".into(), json!(7));
        o.insert("game".into(), json!({ "title": game, "system": "Saltbox 64" }));
        o.insert("source".into(), json!({ "kind": "web-pages", "name": "Example Maps", "importer": { "id": "example-maps", "title": "Example Maps" }, "newestPageAt": updated }));
        o.insert("personalUse".into(), json!(true));
        o.insert("cover".into(), json!("preview/map-one.jpg"));
        o.insert("maps".into(), json!([
            { "id": "map:one", "name": "The Marsh", "width": 16384, "height": 16384, "locations": 1210, "preview": "preview/map-one.jpg" },
            { "id": "map:two", "name": "The Caves", "width": 8192, "height": 8192, "locations": 630, "preview": "preview/map-two.jpg" },
            { "id": "map:three", "name": "The Attic", "width": 1024, "height": 1024, "locations": 0 }
        ]));
        o.insert("guides".into(), json!([{ "title": "The Marsh Wiki", "type": "wiki", "pages": 210 }]));
        d
    }

    /// A map made in Atlas (block A6): from no site (`source.kind` `unknown`, no name and no
    /// importer), written by Atlas (`pack.writtenBy`), with the maker's own things counted in
    /// `mine` (they're under `mine/` in the file: [`write_pack_with`]); version 5, as it
    /// carries notes and no small pictures.
    pub(crate) fn made_in_atlas(id: &str, game: &str, updated: &str) -> Value {
        let mut d = format6(id, game, updated);
        let o = d.as_object_mut().unwrap();
        o.insert("version".into(), json!(5));
        o.insert("game".into(), json!({ "title": game }));
        o.insert("exportedBy".into(), json!({ "name": "Atlas", "version": "0.23.0", "platform": "windows" }));
        o["pack"]["writtenBy"] = json!({ "name": "Atlas", "version": "0.23.0" });
        o.insert("source".into(), json!({ "kind": "unknown" }));
        o.insert("mine".into(), json!({ "lists": 0, "locations": 12, "types": 2, "areas": 0, "photos": 0, "guideLinks": false, "pages": 0, "sections": 0, "notes": 3, "group": false }));
        d
    }

    /// A map made by Atlas Map Maker (block A6): from no site, its manifest written by Atlas
    /// Map Maker, the file by Atlas Importer's keeper, and none of the maker's own things.
    pub(crate) fn made_by_map_maker(id: &str, game: &str, updated: &str) -> Value {
        let mut d = made_in_atlas(id, game, updated);
        let o = d.as_object_mut().unwrap();
        o.remove("mine");
        o.insert("version".into(), json!(1));
        o.insert("exportedBy".into(), json!({ "name": "Atlas Importer", "version": "0.23.0", "platform": "windows" }));
        o["pack"]["writtenBy"] = json!({ "name": "Atlas Map Maker", "version": "0.2.0" });
        d
    }

    /// The maker's own things as a map made in Atlas carries them, under `mine/`.
    pub(crate) fn mine_entries() -> Vec<(&'static str, Vec<u8>)> {
        vec![
            ("mine/my-locations.json", br#"{"locations":[{"id":"x1","name":"A chest by the well"}]}"#.to_vec()),
            ("mine/my-notes.json", br#"{"notes":[{"location":"x1","text":"Come back with the key"}]}"#.to_vec()),
            ("mine/my-pictures/well.jpg", jpeg(7)),
        ]
    }

    /// A pack as Atlas writes one: the description deflated first, the small pictures stored
    /// right behind it, then a JSON file and an image.
    pub(crate) fn write_pack(path: &Path, description: &Value, previews: &[(&str, Vec<u8>)]) {
        write_pack_with(path, description, previews, &[]);
    }

    /// [`write_pack`] with more entries at its end (the maker's own things under `mine/`, as
    /// Atlas writes them last).
    pub(crate) fn write_pack_with(path: &Path, description: &Value, previews: &[(&str, Vec<u8>)], more: &[(&str, Vec<u8>)]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut z = zip::ZipWriter::new(File::create(path).unwrap());
        let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        z.start_file("atlaspack.json", deflated).unwrap();
        z.write_all(serde_json::to_string_pretty(description).unwrap().as_bytes()).unwrap();
        for (n, b) in previews {
            z.start_file(*n, stored).unwrap();
            z.write_all(b).unwrap();
        }
        z.start_file("manifest.json", deflated).unwrap();
        z.write_all(b"{}").unwrap();
        z.start_file("images/tile-0-0.png", stored).unwrap();
        z.write_all(&[0u8; 300]).unwrap();
        for (n, b) in more {
            z.start_file(*n, deflated).unwrap();
            z.write_all(b).unwrap();
        }
        z.finish().unwrap();
    }

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("gamerepo-test-atlaspack-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_pack_from_before_block_a3_falls_back_to_its_title_site_and_counts() {
        let d = dir("six");
        let p = d.join("Hollow Bog - Example Maps.atlaspack");
        write_pack(&p, &format6("hollow-bog-example-maps", "Hollow Bog", "2026-11-17T08:30:00Z"), &[]);
        let x = read_header(&p).unwrap();
        assert_eq!((x.version, x.pack_id.as_deref(), x.title.as_deref()), (1, Some("hollow-bog-example-maps"), Some("Hollow Bog")));
        assert_eq!((x.game_title(), x.source_name(), x.detail.as_deref()), (Some("Hollow Bog"), Some("Example Maps"), Some("full")));
        assert_eq!((x.contents.get("maps"), x.contents.get("guides"), x.source.pages), (Some(&2), Some(&210), 1));
        assert_eq!((x.updated.as_deref(), x.written_by().as_deref()), (Some("2026-11-17T08:30:00Z"), Some("Atlas Importer 0.22.0")));
        assert!(x.maps.is_empty() && x.cover.is_none() && !x.personal_use && x.previews().is_empty());
        assert_eq!(x.file_name().as_deref(), Some("Hollow Bog - Example Maps.atlaspack"));
        assert!(read_preview(&p, "preview/map-one.jpg").is_err(), "no picture it doesn't list");
        // what the index keeps reads back the same
        assert_eq!(Description::from_value(x.raw.clone()), x);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn block_a3s_names_and_pictures_and_a_lower_version_without_pictures() {
        let d = dir("seven");
        let p = d.join("pack.atlaspack");
        let (one, two) = (jpeg(3), jpeg(5));
        write_pack(&p, &format7("marsh-maps", "Hollow Bog", "2026-11-17T08:30:00Z"), &[("preview/map-one.jpg", one.clone()), ("preview/map-two.jpg", two.clone())]);
        let x = read_header(&p).unwrap();
        assert_eq!((x.version, x.game.system.as_deref(), x.source_name(), x.personal_use), (7, Some("Saltbox 64"), Some("Example Maps"), true));
        assert_eq!(x.maps.iter().map(|m| (m.name.clone().unwrap(), m.locations.unwrap())).collect::<Vec<_>>(), [("The Marsh".to_string(), 1210), ("The Caves".to_string(), 630), ("The Attic".to_string(), 0)]);
        assert_eq!(x.guides.iter().map(|g| (g.kind.clone().unwrap(), g.pages.unwrap())).collect::<Vec<_>>(), [("wiki".to_string(), 210)]);
        assert_eq!(x.previews(), ["preview/map-one.jpg", "preview/map-two.jpg"]);
        assert_eq!(read_preview(&p, "preview/map-one.jpg").unwrap(), one);
        assert_eq!(read_preview(&p, "preview/map-two.jpg").unwrap(), two);
        // only the names the description lists, and only safe ones
        assert!(read_preview(&p, "manifest.json").is_err());
        assert!(read_preview(&p, "preview/../manifest.json").is_err());
        // a picture that isn't a JPEG, or is over 512 KB, isn't read: the crate's sentence
        let q = d.join("png.atlaspack");
        let mut big = jpeg(7);
        big.resize(513 << 10, 0);
        write_pack(&q, &format7("marsh-maps", "Hollow Bog", "2026-11-17T08:30:00Z"), &[("preview/map-one.jpg", b"\x89PNG not a jpeg".to_vec()), ("preview/map-two.jpg", big)]);
        let e = read_preview(&q, "preview/map-one.jpg").unwrap_err();
        assert_eq!((kind_of(&e), e.to_string().as_str()), ("damaged", "The small picture in this pack file is not a JPEG picture."));
        assert_eq!(kind_of(&read_preview(&p, "preview/map-three.jpg").unwrap_err()), "no-such-picture");
        assert!(read_preview(&q, "preview/map-two.jpg").is_err());
        // a pack whose maps have no picture keeps a lower version, with the other new fields
        let mut six = format7("marsh-maps", "Hollow Bog", "2026-11-17T08:30:00Z");
        six["version"] = json!(6);
        six.as_object_mut().unwrap().remove("cover");
        for m in six["maps"].as_array_mut().unwrap() {
            m.as_object_mut().unwrap().remove("preview");
        }
        write_pack(&q, &six, &[]);
        let y = read_header(&q).unwrap();
        assert_eq!((y.version, y.game_title(), y.source_name(), y.maps.len(), y.guides.len()), (6, Some("Hollow Bog"), Some("Example Maps"), 3, 1));
        assert!(y.previews().is_empty() && y.cover.is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_guide_of_a_type_not_known_and_a_name_that_isnt_safe_are_taken_as_the_crate_takes_them() {
        let d = dir("unknown");
        let p = d.join("pack.atlaspack");
        let mut v = format7("marsh-maps", "Hollow Bog", "2026-11-17T08:30:00Z");
        v["guides"] = json!([{ "title": "The Marsh Atlas", "type": "atlas-of-the-future", "pages": 12 }]);
        v["maps"][1]["preview"] = json!("preview/../../evil.jpg");
        write_pack(&p, &v, &[("preview/map-one.jpg", jpeg(3))]);
        let x = read_header(&p).unwrap();
        assert_eq!((x.guides[0].kind.as_deref(), x.guides[0].pages), (Some("other"), Some(12)), "a type it doesn't know is accepted");
        assert_eq!((x.maps[1].preview.as_deref(), x.previews()), (None, vec!["preview/map-one.jpg".to_string()]), "an unsafe name is left out");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The crate's refusal as Game Repo keeps it: (kind, sentence).
    fn refused(r: Result<Description, Refusal>) -> (&'static str, String) {
        let e = r.unwrap_err();
        (kind_of(&e), e.to_string())
    }

    #[test]
    fn a_file_that_isnt_a_pack_or_is_too_new_gets_the_crates_sentence_and_kind() {
        let d = dir("wrong");
        let not = ("not-a-pack-file", "This file is not an Atlas pack file.".to_string());
        let text = d.join("notes.atlaspack");
        std::fs::write(&text, b"just some text").unwrap();
        assert_eq!(refused(read_header(&text)), not);
        std::fs::write(&text, b"").unwrap();
        assert_eq!(refused(read_header(&text)), not);
        // another format
        let other = d.join("other.atlaspack");
        let mut wrong = format6("x", "X", "2026-01-01T00:00:00Z");
        wrong["format"] = json!("something-else");
        write_pack(&other, &wrong, &[]);
        assert_eq!(refused(read_header(&other)), not);
        // version 11 is newer than the crate reads (v0.29.0 reads 10): the sentence names what wrote it, and its
        // advice names no app (the app reading it is the one to update), shown as it is
        let mut eleven = format7("x", "X", "2026-01-01T00:00:00Z");
        eleven["version"] = json!(11);
        write_pack(&other, &eleven, &[]);
        assert_eq!(
            refused(read_header(&other)),
            ("newer-format", "This pack file was made by Atlas Importer 0.22.0 in a newer format (version 11). A newer version of this app is needed to open it.".to_string())
        );
        // a description that's missing a field every pack has (the contract's answer 2): damaged
        for field in ["format", "version", "packId", "title", "packVersion", "detail", "exportedAt", "exportedBy", "pack", "source", "contents", "images", "files", "bytes"] {
            let mut damaged = format6("x", "X", "2026-01-01T00:00:00Z");
            damaged.as_object_mut().unwrap().remove(field);
            write_pack(&other, &damaged, &[]);
            let (kind, said) = refused(read_header(&other));
            // without format or version it isn't a pack file at all
            let want = if matches!(field, "format" | "version") { "not-a-pack-file" } else { "damaged" };
            assert_eq!(kind, want, "{field}: {said}");
        }
        for (part, field) in [("pack", "id"), ("pack", "title"), ("pack", "packVersion"), ("pack", "schemaVersion"), ("source", "kind")] {
            let mut damaged = format6("x", "X", "2026-01-01T00:00:00Z");
            damaged[part].as_object_mut().unwrap().remove(field);
            write_pack(&other, &damaged, &[]);
            let (kind, said) = refused(read_header(&other));
            assert!(kind == "damaged" && said.starts_with("This pack file's description is damaged:"), "{part}.{field}: {said}");
        }
        // what may be absent is: the game, the site's name, personalUse, maps, guides, cover, mine
        let mut lean = format6("x", "X", "2026-01-01T00:00:00Z");
        lean["source"] = json!({ "kind": "web-pages" });
        write_pack(&other, &lean, &[]);
        let x = read_header(&other).unwrap();
        assert_eq!((x.game_title(), x.source_name(), x.file_name().as_deref()), (Some("X"), None, Some("X.atlaspack")));
        // a packId that can't be one (the contract's answer 3: 1 to 64 lower case letters,
        // digits, - and _, from a letter or digit) is damaged, an empty one too
        for id in ["", "Hollow-Bog", "-hollow", "hollow bog", "hollow.bog", &"a".repeat(65)] {
            let mut bad = format6(id, "X", "2026-01-01T00:00:00Z");
            bad["pack"]["id"] = json!("x");
            write_pack(&other, &bad, &[]);
            assert_eq!(refused(read_header(&other)), ("damaged", "This pack file names a game id that cannot be used.".to_string()), "{id:?}");
        }
        for id in ["x", "0-hollow_bog", &"a".repeat(64)] {
            write_pack(&other, &format6(id, "X", "2026-01-01T00:00:00Z"), &[]);
            assert_eq!(read_header(&other).unwrap().pack_id.as_deref(), Some(id));
        }
        // a file that isn't there can't be opened
        assert_eq!(refused(read_header(&d.join("gone.atlaspack"))).0, "cannot-open");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_pack_from_no_site_is_made_in_atlas_and_says_what_of_yours_it_carries() {
        let dir = dir("made");
        // a map made in Atlas: no site, named by its game alone, with the maker's own things
        let path = dir.join("Delta.atlaspack");
        write_pack_with(&path, &made_in_atlas("delta-my-map", "Delta", "2026-11-18T08:30:00Z"), &[], &mine_entries());
        let d = read_header(&path).unwrap();
        assert_eq!((d.source.kind.as_deref(), d.source_name(), d.made_in()), (Some("unknown"), None, Some("Atlas")));
        assert_eq!(d.file_name().as_deref(), Some("Delta.atlaspack"), "no \"Delta - .atlaspack\"");
        assert_eq!(d.mine.as_ref().map(|m| (m.locations, m.types, m.notes)), Some((12, 2, 3)));
        assert_eq!(d.own_things().as_deref(), Some("With your own locations, types and notes"));
        let c = card(&d);
        assert_eq!((c["source_name"].clone(), c["made_in"].clone(), c["own_things"].clone()), (Value::Null, json!("Atlas"), json!("With your own locations, types and notes")));
        assert_eq!((c["sort"].clone(), c["site"].clone(), c["from"].clone()), (json!("made-in-atlas"), Value::Null, json!("Made in Atlas")));
        assert_eq!(c["mine"]["notes"], 3);
        // one by Atlas Map Maker: made there, nothing of yours
        let mm = Description::from_value(made_by_map_maker("beta-map-maker", "Beta", "2026-11-18T08:30:00Z"));
        assert_eq!((mm.made_in(), mm.own_things(), mm.file_name().as_deref()), (Some("Atlas Map Maker"), None, Some("Beta.atlaspack")));
        // a pack read from a site is never "made in" anything, even one that names no site
        let site = Description::from_value(format7("delta-example-maps", "Delta", "2026-11-18T08:30:00Z"));
        assert_eq!((site.made_in(), site.own_things(), site.file_name().as_deref()), (None, None, Some("Delta - Example Maps.atlaspack")));
        let mut v = format6("delta-example-maps", "Delta", "2026-11-18T08:30:00Z");
        v["source"] = json!({ "kind": "web-pages" });
        assert_eq!(Description::from_value(v).made_in(), None);
        // the maker's own things however many: one kind, several, or not said
        let mut v = made_in_atlas("x", "X", "2026-11-18T08:30:00Z");
        v["mine"] = json!({ "photos": 4 });
        assert_eq!(Description::from_value(v.clone()).own_things().as_deref(), Some("With your own photos"));
        v["mine"] = json!({});
        assert_eq!(Description::from_value(v).own_things().as_deref(), Some("With your own things"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A pack of format 9 (Atlas 0.26.0): the pack's routes (`routes.json`), the person's own
    /// (`mine/my-routes.json`, counted as `routes` in `mine`), and the map's own icons (types
    /// that name a picture, the pictures in `pictures.json` with `shareable: false`, counted as
    /// `typePictures` in `contents`).
    pub(crate) fn format9(id: &str, game: &str, updated: &str) -> (Value, Vec<(&'static str, Vec<u8>)>) {
        let mut d = format7(id, game, updated);
        d["version"] = json!(9);
        d["contents"]["typePictures"] = json!(2);
        d["mine"] = json!({ "lists": 1, "locations": 4, "routes": 3 });
        let more = vec![
            ("routes.json", br#"{"routes":[{"id":"r1","name":"The long way round","points":[[1,2],[3,4]]}]}"#.to_vec()),
            ("categories.json", br#"{"categories":[{"id":"chest","icon":"chest","picture":"pictures/type-chest.png"},{"id":"well","icon":"water","picture":"pictures/type-well.png"}]}"#.to_vec()),
            ("pictures.json", br#"{"pictures":[{"name":"pictures/type-chest.png","shareable":false},{"name":"pictures/type-well.png","shareable":false,"cut":[0,0,32,32]}]}"#.to_vec()),
            ("pictures/type-chest.png", vec![0x89, b'P', b'N', b'G', 1, 2, 3]),
            ("pictures/type-well.png", vec![0x89, b'P', b'N', b'G', 4, 5, 6]),
            ("mine/my-routes.json", br#"{"routes":[{"id":"m1"},{"id":"m2"},{"id":"m3"}]}"#.to_vec()),
        ];
        (d, more)
    }

    #[test]
    fn a_pack_of_format_9_with_routes_and_the_maps_icons_is_read_and_its_icons_never_shown() {
        let dir = dir("format9");
        let path = dir.join("Delta - Example Maps.atlaspack");
        let (v, more) = format9("delta-example-maps", "Delta", "2026-11-20T08:30:00Z");
        write_pack_with(&path, &v, &[("preview/map-one.jpg", jpeg(3)), ("preview/map-two.jpg", jpeg(5))], &more);
        // read as any pack, not refused as a newer format
        let d = read_header(&path).unwrap();
        assert_eq!((d.version, d.pack_id.as_deref(), d.sort()), (9, Some("delta-example-maps"), Some(Sort::Site)));
        // its counts: the maps' icons in contents, the person's own routes in mine
        assert_eq!((d.contents.get("typePictures"), d.contents.get("maps"), d.contents.get("locations")), (Some(&2), Some(&2), Some(&1840)));
        assert_eq!(d.mine.as_ref().map(|m| (m.lists, m.locations, m.routes)), Some((1, 4, 3)));
        assert_eq!(d.own_things().as_deref(), Some("With your own lists, locations and routes"));
        let c = card(&d);
        assert_eq!((c["contents"]["typePictures"].clone(), c["mine"]["routes"].clone(), c["personal_use"].clone()), (json!(2), json!(3), json!(true)));
        // the map's icons make the file one for the person's own devices, even without personalUse
        let mut v2 = v.clone();
        v2.as_object_mut().unwrap().remove("personalUse");
        assert!(Description::from_value(v2.clone()).personal_use);
        v2["contents"]["typePictures"] = json!(0);
        assert!(!Description::from_value(v2).personal_use, "no icons, no personalUse: not for own use");
        // only the cover and the maps' small pictures are ever read: never a type's picture
        assert_eq!(d.previews(), ["preview/map-one.jpg", "preview/map-two.jpg"]);
        assert_eq!(read_preview(&path, "preview/map-two.jpg").unwrap(), jpeg(5));
        for name in ["pictures/type-chest.png", "pictures/type-well.png", "routes.json", "mine/my-routes.json"] {
            assert!(read_preview(&path, name).is_err(), "{name} is never read as a picture");
        }
        // the crate's own version: 10 (v0.29.0, magazines), which reads format 9 as before
        assert_eq!(VERSION, 10);
        // a description without the new fields has none
        let old = Description::from_value(format7("x", "X", "2026-01-01T00:00:00Z"));
        assert_eq!((old.contents.get("typePictures"), old.mine.map(|m| m.routes)), (None, None));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_sort_of_a_pack_is_told_by_the_contracts_rule() {
        let sort = |d: &Description| (d.sort().map(Sort::word), d.made_in(), d.site().map(String::from), d.from_words());
        let words = |s: &str| Some(s.to_string());
        // read from a site: the site's name, whoever wrote it
        let site = Description::from_value(format7("delta-example-maps", "Delta", "2026-11-18T08:30:00Z"));
        assert_eq!(sort(&site), (Some("site"), None, words("Example Maps"), words("Example Maps")));
        let mut v = format6("delta-example-maps", "Delta", "2026-11-18T08:30:00Z");
        v["pack"]["writtenBy"] = json!({ "name": "Atlas" });
        v["source"] = json!({ "kind": "web-pages" });
        assert_eq!(sort(&Description::from_value(v)), (Some("site"), None, None, None), "a site's pack that names no site says nothing");
        // a map made in Atlas
        let atlas = Description::from_value(made_in_atlas("delta-my-map", "Delta", "2026-11-18T08:30:00Z"));
        assert_eq!(sort(&atlas), (Some("made-in-atlas"), Some("Atlas"), None, words("Made in Atlas")));
        // a map made in Atlas Importer: written by "Atlas Importer", from no site, made in Atlas
        let mut v = made_in_atlas("delta-importer-map", "Delta", "2026-11-18T08:30:00Z");
        v["exportedBy"] = json!({ "name": "Atlas Importer", "version": "0.23.0", "platform": "linux" });
        v["pack"]["writtenBy"] = json!({ "name": "Atlas Importer", "version": "0.23.0" });
        v.as_object_mut().unwrap().remove("mine");
        let importer = Description::from_value(v.clone());
        assert_eq!(sort(&importer), (Some("made-in-atlas"), Some("Atlas"), None, words("Made in Atlas")), "never Made in Atlas Importer");
        assert_eq!(card(&importer)["from"], "Made in Atlas");
        // one with a site's name all the same is still by its kind
        v["source"] = json!({ "kind": "unknown", "name": "Example Maps" });
        assert_eq!(sort(&Description::from_value(v)), (Some("made-in-atlas"), Some("Atlas"), None, words("Made in Atlas")));
        // by Atlas Map Maker: its writer's name exactly, whatever the kind
        let mm = Description::from_value(made_by_map_maker("beta-map-maker", "Beta", "2026-11-18T08:30:00Z"));
        assert_eq!(sort(&mm), (Some("map-maker"), Some("Atlas Map Maker"), None, words("Made in Atlas Map Maker")));
        let mut v = made_by_map_maker("beta-map-maker", "Beta", "2026-11-18T08:30:00Z");
        v["pack"]["writtenBy"] = json!({ "name": "atlas map maker" });
        assert_eq!(sort(&Description::from_value(v)).0, Some("made-in-atlas"), "only the exact name");
        // came with an app: plain words, its own things or not (never by `mine`)
        let mut v = made_in_atlas("gamma-guide", "Gamma", "2026-11-18T08:30:00Z");
        v["source"] = json!({ "kind": "app", "name": "Gamma Companion" });
        let app = Description::from_value(v);
        assert_eq!(sort(&app), (Some("app"), None, None, words("Came with an app")));
        assert_eq!(app.own_things().as_deref(), Some("With your own locations, types and notes"), "said on a line of its own");
        assert_eq!((card(&app)["sort"].clone(), card(&app)["from"].clone()), (json!("app"), json!("Came with an app")));
        // any other kind, or none: the site's name if it gives one, else nothing
        let mut v = format6("old", "Old", "2026-11-18T08:30:00Z");
        v["source"] = json!({ "kind": "something-new", "importer": { "id": "example-maps", "title": "Example Maps" } });
        assert_eq!(sort(&Description::from_value(v.clone())), (None, None, words("Example Maps"), words("Example Maps")));
        v.as_object_mut().unwrap().remove("source");
        v["pack"]["writtenBy"] = json!({ "name": "Atlas" });
        assert_eq!(sort(&Description::from_value(v)), (None, None, None, None), "not Made in Atlas by its writer alone");
        // the one word of each sort, read back (for Atlas's planned field)
        for s in [Sort::Site, Sort::MadeInAtlas, Sort::MapMaker, Sort::App, Sort::Guidebooks, Sort::Magazines] {
            assert_eq!(Sort::from_word(s.word()), Some(s));
        }
        assert_eq!(Sort::from_word("atlas"), None);
    }

    /// D231: a pack that says its sort (`source.origin`, Atlas 0.25.0) is taken at its word,
    /// read through the crate's `Header::origin`, over the rule from before; a word the crate
    /// doesn't know, or one that isn't text, leaves the rule to tell it.
    #[test]
    fn a_pack_that_says_its_sort_is_taken_at_its_word() {
        let dir = std::env::temp_dir().join(format!("gamerepo-atlaspack-origin-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let sort = |d: &Description| (d.sort().map(Sort::word), d.made_in(), d.site().map(String::from), d.from_words());
        let words = |s: &str| Some(s.to_string());
        let with = |mut v: Value, origin: Value| {
            v["source"]["origin"] = origin;
            v
        };
        let file = dir.join("p.atlaspack");
        let read = |v: &Value| {
            write_pack(&file, v, &[]);
            read_header(&file).unwrap()
        };
        // each word, read from the file by the crate, as Atlas 0.25.0 writes it
        let site = read(&with(format7("delta-example-maps", "Delta", "2026-11-18T08:30:00Z"), json!("site")));
        assert_eq!((site.origin, sort(&site)), (Some(Sort::Site), (Some("site"), None, words("Example Maps"), words("Example Maps"))));
        let atlas = read(&with(made_in_atlas("delta-my-map", "Delta", "2026-11-18T08:30:00Z"), json!("made-in-atlas")));
        assert_eq!(sort(&atlas), (Some("made-in-atlas"), Some("Atlas"), None, words("Made in Atlas")));
        let mm = read(&with(made_by_map_maker("beta-map-maker", "Beta", "2026-11-18T08:30:00Z"), json!("map-maker")));
        assert_eq!(sort(&mm), (Some("map-maker"), Some("Atlas Map Maker"), None, words("Made in Atlas Map Maker")));
        // the word wins over the rule: a pack read from web pages that says it was made in
        // Atlas, and one by "Atlas Map Maker" that says it's a site's
        let said = read(&with(format7("delta-example-maps", "Delta", "2026-11-18T08:30:00Z"), json!("made-in-atlas")));
        assert_eq!(sort(&said), (Some("made-in-atlas"), Some("Atlas"), None, words("Made in Atlas")));
        let mut v = with(made_by_map_maker("beta-map-maker", "Beta", "2026-11-18T08:30:00Z"), json!("site"));
        v["source"]["name"] = json!("Example Maps");
        assert_eq!(sort(&read(&v)), (Some("site"), None, words("Example Maps"), words("Example Maps")));
        // the word kept in the index (the description as JSON) reads the same
        assert_eq!(Description::from_value(said.raw.clone()).sort(), Some(Sort::MadeInAtlas));
        // a word the crate doesn't know, or not text: the rule from before, never refused
        for odd in [json!("a-word-from-later"), json!(7), json!({ "kind": "site" })] {
            let mut v = made_in_atlas("gamma-guide", "Gamma", "2026-11-18T08:30:00Z");
            v["source"] = json!({ "kind": "app", "origin": odd });
            let d = read(&v);
            assert_eq!((d.origin, sort(&d)), (None, (Some("app"), None, None, words("Came with an app"))), "{odd}");
        }
        // no word: the rule, as before (a pack that came with an app stays one, though the
        // crate's own guess would take it as made in Atlas, by its exporter)
        let mut v = made_in_atlas("gamma-guide", "Gamma", "2026-11-18T08:30:00Z");
        v["source"] = json!({ "kind": "app" });
        assert_eq!(sort(&read(&v)).0, Some("app"));
        // the card says it
        let c = card(&said);
        assert_eq!((c["sort"].clone(), c["from"].clone(), c["made_in"].clone()), (json!("made-in-atlas"), json!("Made in Atlas"), json!("Atlas")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A small PDF of `pages` pages, as a guidebook is (made up).
    pub(crate) fn pdf(pages: u32) -> Vec<u8> {
        let kids: Vec<String> = (0..pages).map(|i| format!("{} 0 R", 10 + i)).collect();
        let mut text = format!("%PDF-1.4\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages /Kids [{}] /Count {pages} >>\nendobj\n", kids.join(" "));
        for i in 0..pages {
            text.push_str(&format!("{} 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] >>\nendobj\n", 10 + i));
        }
        text.push_str("trailer\n<< /Root 1 0 R >>\n%%EOF\n");
        text.into_bytes()
    }

    /// A pack of guidebooks of a made-up game (format 8), written by Atlas's own crate as
    /// another app writes one, with one guidebook of three pages.
    pub(crate) fn guidebook_pack(path: &Path, id: &str, game: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let book = path.with_extension("source.pdf");
        std::fs::write(&book, pdf(3)).unwrap();
        let pack = atlas_pack::GuidebookPack {
            pack_id: id.into(),
            game_title: game.into(),
            written_by: atlas_pack::AppInfo { name: "Example Shelf".into(), version: "1.0.0".into(), platform: "linux".into() },
            guidebooks: vec![atlas_pack::NewGuidebook { file: book.clone(), title: format!("{game} Field Guide"), pages: None, text: None }],
        };
        atlas_pack::write_guidebook_pack(&pack, path).unwrap();
        std::fs::remove_file(&book).unwrap();
    }

    /// Format 8 (v0.25.0): a pack of guidebooks is read, not refused as newer or damaged,
    /// and its sort is Guidebooks, which a card says.
    #[test]
    fn a_pack_of_guidebooks_is_read_and_its_sort_is_guidebooks() {
        let dir = std::env::temp_dir().join(format!("gamerepo-atlaspack-guidebooks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("Hollow Marsh guidebooks.atlaspack");
        guidebook_pack(&file, "guidebooks-hollow-marsh", "Hollow Marsh");
        assert!(VERSION >= 8, "the crate reads format 8 (v0.29.0 reads up to 10)");
        let d = read_header(&file).unwrap();
        assert_eq!((d.version, d.pack_id.as_deref(), d.game_title()), (8, Some("guidebooks-hollow-marsh"), Some("Hollow Marsh")));
        assert_eq!((d.sort(), d.made_in(), d.site(), d.from_words().as_deref()), (Some(Sort::Guidebooks), None, None, Some("Guidebooks")));
        let c = card(&d);
        assert_eq!((c["sort"].clone(), c["from"].clone(), c["site"].clone()), (json!("guidebooks"), json!("Guidebooks"), Value::Null));
        // a description that lists guidebooks without saying its origin is one too
        let mut raw = d.raw.clone();
        raw["source"].as_object_mut().unwrap().remove("origin");
        assert_eq!(Description::from_value(raw).sort(), Some(Sort::Guidebooks));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A pack of a made-up magazine's issues (format 10), written by Atlas's own crate as
    /// another app writes one, with one issue of four pages.
    pub(crate) fn magazine_pack(path: &Path, id: &str, title: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let issue = path.with_extension("source.pdf");
        std::fs::write(&issue, pdf(4)).unwrap();
        let pack = atlas_pack::MagazinePack {
            pack_id: id.into(),
            magazine: atlas_pack::MagazineFacts { title: title.into(), publisher: Some("Reed Press".into()) },
            written_by: atlas_pack::AppInfo { name: "Example Shelf".into(), version: "1.0.0".into(), platform: "linux".into() },
            issues: vec![atlas_pack::NewIssue {
                file: issue.clone(),
                title: "Issue 41".into(),
                date: Some("2026-04".into()),
                number: Some(41),
                pages: None,
                text: None,
                cover: None,
                games: vec![atlas_pack::IssueGame { title: "Hollow Marsh".into(), system: None, game_id: None, pages: vec![[2, 3]] }],
            }],
        };
        atlas_pack::write_magazine_pack(&pack, path).unwrap();
        std::fs::remove_file(&issue).unwrap();
    }

    /// Format 10 (v0.29.0, D245): a pack of magazines is read, not refused as newer or
    /// damaged, and its sort is Magazines, which a card says; it's of no game.
    #[test]
    fn a_pack_of_magazines_is_read_and_its_sort_is_magazines() {
        let dir = std::env::temp_dir().join(format!("gamerepo-atlaspack-magazines-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("Marsh Monthly.atlaspack");
        magazine_pack(&file, "magazines-marsh-monthly", "Marsh Monthly");
        assert!(VERSION >= 10, "the crate reads format 10");
        let d = read_header(&file).unwrap();
        assert_eq!((d.version, d.pack_id.as_deref(), d.game.title.as_deref(), d.game_title()), (10, Some("magazines-marsh-monthly"), None, Some("Marsh Monthly")), "no game: its title is the magazine's");
        assert_eq!((d.sort(), d.made_in(), d.site(), d.from_words().as_deref()), (Some(Sort::Magazines), None, None, Some("Magazines")));
        assert_eq!(d.raw["issues"][0]["games"][0]["pages"], json!([[2, 3]]));
        let c = card(&d);
        assert_eq!((c["sort"].clone(), c["from"].clone(), c["site"].clone()), (json!("magazines"), json!("Magazines"), Value::Null));
        // a description that lists issues without saying its origin is one too
        let mut raw = d.raw.clone();
        raw["source"].as_object_mut().unwrap().remove("origin");
        assert_eq!(Description::from_value(raw).sort(), Some(Sort::Magazines));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_newer_of_two_files_of_a_pack_is_told_by_its_dates() {
        let a = Description::from_value(format6("p", "P", "2026-11-17T08:30:00Z"));
        let mut b = Description::from_value(format6("p", "P", "2026-11-17T09:30:00+02:00"));
        assert_eq!(a.newer(&b), Ordering::Greater, "08:30 UTC is after 07:30 UTC");
        b.updated = a.updated.clone();
        b.exported_at = Some("2026-11-19T00:00:00.5Z".into());
        assert_eq!(b.newer(&a), Ordering::Greater, "exportedAt decides a tie");
        b.exported_at = a.exported_at.clone();
        assert_eq!(a.newer(&b), Ordering::Equal);
        assert!(is_pack_name("A - B.atlaspack") && is_pack_name("x.ATLASPACK") && !is_pack_name("A - B.atlaspack.part") && !is_pack_name("x.part"));
        assert!(!is_pack_name("A - B.atlaspack.atlas-importer.part") && is_importer_part("A - B.atlaspack.atlas-importer.part") && is_importer_part("x.ATLAS-IMPORTER.PART"));
        assert!(!is_importer_part("A - B.atlaspack.part") && !is_importer_part("A - B.atlaspack"));
    }
}
