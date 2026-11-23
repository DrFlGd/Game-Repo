//! Guidebooks (version 8): writing a pack of them, reading it back, its
//! checks and refusals, counting pages, and `source.origin`. The PDF and
//! CBZ files are tiny ones made here; every name is made up.

use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

use flate2::{write::ZlibEncoder, Compression};
use serde_json::{json, Value};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipArchive, ZipWriter};

use super::*;

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "atlas-guidebooks-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A PDF of `pages` empty pages whose page tree is plain text, with an
/// outline whose `/Count` must not be taken for the pages.
fn pdf(pages: u32) -> Vec<u8> {
    let kids: Vec<String> = (0..pages).map(|i| format!("{} 0 R", 10 + i)).collect();
    let mut text = format!(
        "%PDF-1.4\n%\u{e2}\u{e3}\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R /Outlines 3 0 R >>\nendobj\n\
         2 0 obj\n<< /Type /Pages /Kids [{}] /Count {pages} /Resources << /Font << >> >> >>\nendobj\n\
         3 0 obj\n<< /Type /Outlines /Count 50 >>\nendobj\n",
        kids.join(" ")
    );
    for i in 0..pages {
        text.push_str(&format!(
            "{} 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] >>\nendobj\n",
            10 + i
        ));
    }
    text.push_str("trailer\n<< /Root 1 0 R >>\n%%EOF\n");
    text.into_bytes()
}

/// A PDF whose page tree is packed in a deflated object stream, as newer
/// PDFs have it, with a page tree node elsewhere whose count is a
/// reference.
fn pdf_with_object_stream(pages: u32) -> Vec<u8> {
    let objects = format!(
        "1 0 2 40 << /Type /Catalog /Pages 2 0 R >> << /Type /Pages /Kids [3 0 R] /Count {pages} >>"
    );
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(objects.as_bytes()).unwrap();
    let packed = encoder.finish().unwrap();
    let mut bytes = format!(
        "%PDF-1.5\n4 0 obj\n<< /Type /Pages /Count 9 0 R >>\nendobj\n\
         5 0 obj\n<< /Type /ObjStm /N 2 /First 8 /Length {} /Filter /FlateDecode >>\nstream\r\n",
        packed.len()
    )
    .into_bytes();
    bytes.extend_from_slice(&packed);
    bytes.extend_from_slice(b"\r\nendstream\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF\n");
    bytes
}

/// A CBZ of `pictures` page pictures, with a folder, a hidden file and a
/// note that are not pages.
fn cbz(pictures: u32) -> Vec<u8> {
    let mut zip = ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    zip.add_directory("pages/", options).unwrap();
    for i in 1..=pictures {
        zip.start_file(format!("pages/page{i}.png"), options)
            .unwrap();
        zip.write_all(b"\x89PNG\r\n\x1a\nnot really a picture")
            .unwrap();
    }
    zip.start_file("pages/.thumbs.png", options).unwrap();
    zip.write_all(b"x").unwrap();
    zip.start_file("__MACOSX/pages/._page1.png", options)
        .unwrap();
    zip.write_all(b"x").unwrap();
    zip.start_file("ComicInfo.xml", options).unwrap();
    zip.write_all(b"<ComicInfo/>").unwrap();
    zip.finish().unwrap().into_inner()
}

fn atlas() -> AppInfo {
    AppInfo {
        name: "Atlas".into(),
        version: "0.25.0".into(),
        platform: "linux".into(),
    }
}

fn pack(guidebooks: Vec<NewGuidebook>) -> GuidebookPack {
    GuidebookPack {
        pack_id: "guidebooks-hollow-marsh".into(),
        game_title: "Hollow Marsh".into(),
        written_by: atlas(),
        guidebooks,
    }
}

fn book(file: &Path, title: &str) -> NewGuidebook {
    NewGuidebook {
        file: file.to_owned(),
        title: title.into(),
        pages: None,
        text: None,
    }
}

/// An entry's bytes, and whether it was stored.
fn entry(file: &Path, name: &str) -> (Vec<u8>, bool) {
    let mut archive = ZipArchive::new(fs::File::open(file).unwrap()).unwrap();
    let mut entry = archive.by_name(name).unwrap();
    let stored = entry.compression() == CompressionMethod::Stored;
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes).unwrap();
    (bytes, stored)
}

fn names(file: &Path) -> Vec<String> {
    let archive = ZipArchive::new(fs::File::open(file).unwrap()).unwrap();
    archive
        .file_names()
        .map(|name| name.unwrap().into_owned())
        .collect()
}

#[test]
fn writes_a_pack_of_guidebooks_and_reads_it_back() {
    let dir = TempDir::new("write");
    let guide = dir.path("Official Guide.pdf");
    fs::write(&guide, pdf(3)).unwrap();
    let comic = dir.path("Marsh Maps.CBZ");
    fs::write(&comic, cbz(4)).unwrap();
    let target = dir.path("Hollow Marsh guidebooks.atlaspack");
    let mut first = book(&guide, "  Official Guide ");
    first.text = Some(vec![
        "Hollow Marsh\nThe official guide".into(),
        "   ".into(),
        "The heron sleeps at the mill.".into(),
    ]);
    let written =
        write_guidebook_pack(&pack(vec![first, book(&comic, "Marsh Maps")]), &target).unwrap();
    assert!(!dir.path("Hollow Marsh guidebooks.atlaspack.part").exists());

    let header = read_header(&target).unwrap();
    assert_eq!(header, written);
    assert_eq!(header.version, VERSION_WITH_GUIDEBOOKS);
    assert_eq!(header.source.origin, Some(Origin::Guidebooks));
    assert_eq!(header.origin(), Some(Origin::Guidebooks));
    assert_eq!(header.game.as_ref().unwrap().title, "Hollow Marsh");
    assert_eq!(header.title, "Hollow Marsh");
    assert_eq!(header.exported_by, atlas());
    assert_eq!(
        header.pack.written_by,
        Some(json!({"name": "Atlas", "version": "0.25.0"}))
    );
    assert_eq!(header.pack_version, "1.0.0");
    assert_eq!(
        header.guidebooks,
        vec![
            GuidebookFacts {
                file: "guidebooks/official-guide.pdf".into(),
                title: "Official Guide".into(),
                pages: 3,
                bytes: pdf(3).len() as u64,
                text: Some("guidebooks/official-guide.text.json".into()),
            },
            GuidebookFacts {
                file: "guidebooks/marsh-maps.cbz".into(),
                title: "Marsh Maps".into(),
                pages: 4,
                bytes: cbz(4).len() as u64,
                text: None,
            },
        ]
    );
    assert_eq!(header.guidebooks[1].file_type(), GuidebookType::Cbz);
    // The description first, then the pack's own files, then each
    // guidebook's text and file.
    assert_eq!(
        names(&target),
        [
            "atlaspack.json",
            "manifest.json",
            "guidebooks.json",
            "guidebooks/official-guide.text.json",
            "guidebooks/official-guide.pdf",
            "guidebooks/marsh-maps.cbz"
        ]
    );
    assert_eq!(header.files, 5);
    // Guidebooks and their text are stored, so they can be read in parts.
    let (bytes, stored) = entry(&target, "guidebooks/official-guide.pdf");
    assert!(stored);
    assert_eq!(bytes, pdf(3));
    let (bytes, stored) = entry(&target, "guidebooks/marsh-maps.cbz");
    assert!(stored);
    assert_eq!(bytes, cbz(4));
    let (text, stored) = entry(&target, "guidebooks/official-guide.text.json");
    assert!(stored);
    let text = parse_guidebook_text(&text, 3).unwrap();
    assert_eq!(text.format, GUIDEBOOK_TEXT_FORMAT);
    assert_eq!(
        text.pages,
        vec![
            PageText {
                page: 1,
                text: "Hollow Marsh\nThe official guide".into()
            },
            PageText {
                page: 3,
                text: "The heron sleeps at the mill.".into()
            },
        ]
    );
    // The pack reads as a pack folder too.
    let (manifest, _) = entry(&target, "manifest.json");
    let manifest: Value = serde_json::from_slice(&manifest).unwrap();
    assert_eq!(manifest["gameId"], "guidebooks-hollow-marsh");
    assert_eq!(manifest["title"], "Hollow Marsh");
    assert_eq!(manifest["schemaVersion"], 2);
    assert_eq!(manifest["writtenBy"]["name"], "Atlas");
    let (list, _) = entry(&target, "guidebooks.json");
    let list: GuidebookList = serde_json::from_slice(&list).unwrap();
    assert_eq!(list.format, GUIDEBOOK_LIST_FORMAT);
    assert_eq!(list.guidebooks, header.guidebooks);
    assert_eq!(list.game.title, "Hollow Marsh");
    // When it was made, in UTC.
    assert!(header.exported_at.ends_with('Z'), "{}", header.exported_at);
    assert_eq!(header.exported_at.len(), 20);
}

#[test]
fn takes_the_pages_given_and_counts_those_of_a_packed_page_tree() {
    let dir = TempDir::new("pages");
    let packed = dir.path("packed.pdf");
    fs::write(&packed, pdf_with_object_stream(212)).unwrap();
    assert_eq!(guidebook_pages(&packed), Ok(212));
    assert!(guidebook_pages(&dir.path("missing.pdf")).is_err());
    let comic = dir.path("comic.cbz");
    fs::write(&comic, cbz(2)).unwrap();
    assert_eq!(guidebook_pages(&comic), Ok(2));

    // A PDF with no page tree to read: the pages must be given.
    let blank = dir.path("blank.pdf");
    fs::write(
        &blank,
        b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog >>\nendobj\n%%EOF\n",
    )
    .unwrap();
    let target = dir.path("out.atlaspack");
    let refused = write_guidebook_pack(&pack(vec![book(&blank, "Blank")]), &target).unwrap_err();
    assert_eq!(
        refused.message(),
        "The number of pages of blank.pdf could not be read from the file."
    );
    let mut given = book(&blank, "Blank");
    given.pages = Some(12);
    let header = write_guidebook_pack(&pack(vec![given]), &target).unwrap();
    assert_eq!(header.guidebooks[0].pages, 12);
    assert_eq!(header.guidebooks[0].file, "guidebooks/blank.pdf");
}

#[test]
fn adds_a_guidebook_to_a_pack_and_takes_one_out() {
    let dir = TempDir::new("rewrite");
    let guide = dir.path("guide.pdf");
    fs::write(&guide, pdf(2)).unwrap();
    let target = dir.path("pack.atlaspack");
    let mut first = book(&guide, "Official Guide");
    first.text = Some(vec!["One".into(), "Two".into()]);
    write_guidebook_pack(&pack(vec![first]), &target).unwrap();
    let before = read_header(&target).unwrap();

    // A second guidebook with the same title gets a name of its own.
    let second = dir.path("second.pdf");
    fs::write(&second, pdf(5)).unwrap();
    fs::remove_file(&guide).unwrap();
    let added = rewrite_guidebook_pack(
        &target,
        &|_| true,
        &pack(vec![book(&second, "Official Guide")]),
        &target,
    )
    .unwrap();
    assert_eq!(added.pack_version, "1.0.1");
    assert_eq!(added.pack.created, before.pack.created);
    let files: Vec<&str> = added.guidebooks.iter().map(|g| g.file.as_str()).collect();
    assert_eq!(
        files,
        [
            "guidebooks/official-guide.pdf",
            "guidebooks/official-guide-2.pdf"
        ]
    );
    // The kept guidebook and its text are copied as they were.
    assert_eq!(entry(&target, "guidebooks/official-guide.pdf").0, pdf(2));
    let (text, stored) = entry(&target, "guidebooks/official-guide.text.json");
    assert!(stored);
    assert_eq!(parse_guidebook_text(&text, 2).unwrap().pages.len(), 2);
    assert_eq!(read_header(&target).unwrap(), added);

    // Taking the first one out.
    let taken = rewrite_guidebook_pack(
        &target,
        &|facts| facts.file != "guidebooks/official-guide.pdf",
        &pack(Vec::new()),
        &target,
    )
    .unwrap();
    assert_eq!(taken.guidebooks.len(), 1);
    assert_eq!(taken.guidebooks[0].file, "guidebooks/official-guide-2.pdf");
    assert_eq!(taken.pack_version, "1.0.2");
    assert!(!names(&target)
        .iter()
        .any(|name| name.contains("official-guide.")));

    // Nothing left is refused: the caller deletes the pack instead.
    let none = rewrite_guidebook_pack(&target, &|_| false, &pack(Vec::new()), &target);
    assert_eq!(
        none.unwrap_err().message(),
        "A pack of guidebooks holds at least one guidebook."
    );
    // Another game's pack is not rewritten under this one's id.
    let mut other = pack(Vec::new());
    other.pack_id = "guidebooks-other".into();
    assert_eq!(
        rewrite_guidebook_pack(&target, &|_| true, &other, &target)
            .unwrap_err()
            .message(),
        "This pack file is not the pack of guidebooks of this game."
    );
    assert_eq!(read_header(&target).unwrap(), taken);
}

#[test]
fn refuses_what_cannot_be_a_guidebook_and_writes_nothing() {
    let dir = TempDir::new("refuse");
    let target = dir.path("pack.atlaspack");
    let guide = dir.path("guide.pdf");
    fs::write(&guide, pdf(2)).unwrap();
    let text_file = dir.path("notes.txt");
    fs::write(&text_file, "notes").unwrap();
    let not_pdf = dir.path("fake.pdf");
    fs::write(&not_pdf, b"PK\x03\x04 a zip").unwrap();
    let not_cbz = dir.path("fake.cbz");
    fs::write(&not_cbz, pdf(1)).unwrap();
    let empty_cbz = dir.path("empty.cbz");
    fs::write(&empty_cbz, cbz(0)).unwrap();
    let cbr = dir.path("comic.cbr");
    fs::write(&cbr, b"Rar!").unwrap();

    let refusal = |guidebooks: Vec<NewGuidebook>| {
        write_guidebook_pack(&pack(guidebooks), &target)
            .unwrap_err()
            .to_string()
    };
    assert_eq!(
        refusal(vec![book(&text_file, "Notes")]),
        "notes.txt is not a PDF or CBZ file. Only PDF and CBZ files can be guidebooks."
    );
    assert_eq!(
        refusal(vec![book(&cbr, "Comic")]),
        "comic.cbr is not a PDF or CBZ file. Only PDF and CBZ files can be guidebooks."
    );
    assert_eq!(
        refusal(vec![book(&not_pdf, "Fake")]),
        "fake.pdf is not the PDF file its name says it is."
    );
    assert_eq!(
        refusal(vec![book(&not_cbz, "Fake")]),
        "fake.cbz is not the CBZ file its name says it is."
    );
    assert_eq!(
        refusal(vec![book(&empty_cbz, "Empty")]),
        "empty.cbz holds no pictures of pages."
    );
    assert_eq!(
        refusal(vec![book(&guide, "   ")]),
        "The title of a guidebook must be 1 to 200 characters."
    );
    assert_eq!(
        refusal(vec![book(&guide, &"a".repeat(201))]),
        "The title of a guidebook must be 1 to 200 characters."
    );
    let mut long_text = book(&guide, "Guide");
    long_text.text = Some(vec!["one".into(), "two".into(), "three".into()]);
    assert_eq!(
        refusal(vec![long_text]),
        "The text given for guide.pdf has more pages than the guidebook."
    );
    let mut no_pages = book(&guide, "Guide");
    no_pages.pages = Some(0);
    assert_eq!(
        refusal(vec![no_pages]),
        "The number of pages of guide.pdf must be 1 to 100000."
    );
    assert_eq!(
        refusal(Vec::new()),
        "A pack of guidebooks holds at least one guidebook."
    );
    let mut bad_id = pack(vec![book(&guide, "Guide")]);
    bad_id.pack_id = "Guidebooks of Hollow Marsh".into();
    assert!(write_guidebook_pack(&bad_id, &target).is_err());
    let mut no_title = pack(vec![book(&guide, "Guide")]);
    no_title.game_title = " ".into();
    assert_eq!(
        write_guidebook_pack(&no_title, &target)
            .unwrap_err()
            .message(),
        "The title of the game must be 1 to 200 characters."
    );
    let too_many = (0..=MAX_GUIDEBOOKS)
        .map(|i| {
            let mut one = book(&guide, &format!("Guide {i}"));
            one.pages = Some(2);
            one
        })
        .collect();
    assert_eq!(
        refusal(too_many),
        "A pack of guidebooks holds at most 100 guidebooks."
    );
    assert!(!target.exists());
    assert!(!dir.path("pack.atlaspack.part").exists());
}

/// A version 8 description of a pack of guidebooks.
fn header_v8() -> Value {
    json!({
        "format": "atlas-pack-file",
        "version": 8,
        "packId": "guidebooks-hollow-marsh",
        "title": "Hollow Marsh",
        "game": {"title": "Hollow Marsh"},
        "packVersion": "1.0.0",
        "detail": "full",
        "exportedAt": "2026-11-19T09:30:00Z",
        "exportedBy": {"name": "Other App", "version": "2.0.0", "platform": "windows"},
        "pack": {"id": "guidebooks-hollow-marsh", "title": "Hollow Marsh", "packVersion": "1.0.0",
                 "schemaVersion": 2, "attribution": []},
        "source": {"kind": "unknown", "origin": "guidebooks"},
        "contents": {},
        "images": {"detail": "full", "files": 0, "bytes": 0, "layers": []},
        "files": 4,
        "bytes": 148_000_100,
        "guidebooks": [
            {"file": "guidebooks/official-guide.pdf", "title": "Official Guide", "pages": 212,
             "bytes": 148_000_000, "text": "guidebooks/official-guide.text.json"}
        ]
    })
}

fn parsed(value: &Value) -> Result<Header, Refusal> {
    parse_header(&value.to_string())
}

#[test]
fn checks_the_guidebooks_a_description_lists() {
    let header = parsed(&header_v8()).unwrap();
    assert_eq!(header.guidebooks.len(), 1);
    assert_eq!(header.guidebooks[0].pages, 212);
    assert_eq!(header.origin(), Some(Origin::Guidebooks));

    let damaged = |change: &dyn Fn(&mut Value)| {
        let mut value = header_v8();
        change(&mut value);
        let refusal = parsed(&value).unwrap_err();
        assert_eq!(refusal.kind(), RefusalKind::Damaged, "{refusal}");
        refusal.to_string()
    };
    let unusable = "This pack file lists a guidebook that cannot be used.";
    assert_eq!(
        damaged(&|v| v["guidebooks"][0]["file"] = json!("guidebooks/../escape.pdf")),
        unusable
    );
    assert_eq!(
        damaged(&|v| v["guidebooks"][0]["file"] = json!("maps/official-guide.pdf")),
        unusable
    );
    assert_eq!(
        damaged(&|v| v["guidebooks"][0]["file"] = json!("guidebooks/Official-Guide.pdf")),
        unusable
    );
    assert_eq!(
        damaged(&|v| v["guidebooks"][0]["file"] = json!("guidebooks/official-guide.cbr")),
        unusable
    );
    assert_eq!(
        damaged(&|v| v["guidebooks"][0]["text"] = json!("guidebooks/other.text.json")),
        unusable
    );
    assert_eq!(
        damaged(&|v| v["guidebooks"][0]["pages"] = json!(0)),
        unusable
    );
    assert_eq!(
        damaged(&|v| v["guidebooks"][0]["bytes"] = json!(MAX_GUIDEBOOK_BYTES + 1)),
        unusable
    );
    assert_eq!(
        damaged(&|v| v["guidebooks"][0]["title"] = json!("x".repeat(201))),
        unusable
    );
    assert_eq!(
        damaged(&|v| {
            let one = v["guidebooks"][0].clone();
            v["guidebooks"] = json!([one.clone(), one]);
        }),
        unusable
    );
    assert_eq!(
        damaged(&|v| {
            let one = v["guidebooks"][0].clone();
            v["guidebooks"] = Value::Array(
                (0..=MAX_GUIDEBOOKS)
                    .map(|i| {
                        let mut one = one.clone();
                        one["file"] = json!(format!("guidebooks/g{i}.pdf"));
                        one["text"] = Value::Null;
                        one
                    })
                    .collect(),
            );
        }),
        "This pack file lists too many guidebooks."
    );
    // A guidebook without its pages or size is damaged.
    assert!(damaged(&|v| {
        v["guidebooks"][0].as_object_mut().unwrap().remove("pages");
    })
    .starts_with("This pack file's description is damaged"));

    // In a description of an older version they are left out.
    let mut older = header_v8();
    older["version"] = json!(7);
    older["guidebooks"][0]["file"] = json!("not even a name");
    assert!(parsed(&older).unwrap().guidebooks.is_empty());
    // A newer version is refused, naming the app that made it.
    let mut newer = header_v8();
    newer["version"] = json!(VERSION + 1);
    let refusal = parsed(&newer).unwrap_err();
    assert_eq!(refusal.kind(), RefusalKind::NewerFormat);
    assert_eq!(
        refusal.what(),
        format!(
            "This pack file was made by Other App 2.0.0 in a newer format (version {}).",
            VERSION + 1
        )
    );
}

#[test]
fn reads_the_origin_and_works_it_out_for_a_file_without_it() {
    let origin = |change: &dyn Fn(&mut Value)| {
        let mut value = header_v8();
        value["version"] = json!(7);
        value.as_object_mut().unwrap().remove("guidebooks");
        value["source"] = json!({"kind": "unknown"});
        change(&mut value);
        let header = parsed(&value).unwrap();
        (header.source.origin, header.origin())
    };
    for (word, value) in [
        ("site", Origin::Site),
        ("made-in-atlas", Origin::MadeInAtlas),
        ("map-maker", Origin::MapMaker),
        ("guidebooks", Origin::Guidebooks),
    ] {
        assert_eq!(value.as_str(), word);
        assert_eq!(
            origin(&|v| v["source"]["origin"] = json!(word)),
            (Some(value), Some(value))
        );
    }
    // A word from a newer file, or no word, falls back to the old rules.
    assert_eq!(
        origin(&|v| v["source"]["origin"] = json!("telescope")),
        (Some(Origin::Other), None)
    );
    assert_eq!(origin(&|v| v["source"]["origin"] = json!(12)), (None, None));
    assert_eq!(
        origin(&|v| {
            v["source"] = json!({"kind": "web-pages", "origin": "telescope"});
        }),
        (Some(Origin::Other), Some(Origin::Site))
    );
    assert_eq!(
        origin(&|v| v["pack"]["writtenBy"] = json!({"name": "Atlas Map Maker", "version": "1.0.0"})),
        (None, Some(Origin::MapMaker))
    );
    assert_eq!(
        origin(
            &|v| v["pack"]["writtenBy"] = json!({"name": "Atlas Map Maker 2", "version": "1.0.0"})
        ),
        (None, None)
    );
    assert_eq!(
        origin(&|v| v["exportedBy"]["name"] = json!("Atlas")),
        (None, Some(Origin::MadeInAtlas))
    );
    assert_eq!(
        origin(&|v| v["exportedBy"]["name"] = json!("Atlas Importer")),
        (None, Some(Origin::MadeInAtlas))
    );
    assert_eq!(
        origin(&|v| v["exportedBy"]["name"] = json!("Atlases of Fen")),
        (None, None)
    );
    // Written only when known.
    let header = parsed(&header_v8()).unwrap();
    let mut without = header.clone();
    without.source.origin = None;
    assert!(!serde_json::to_string(&without).unwrap().contains("origin"));
    assert!(serde_json::to_string(&header)
        .unwrap()
        .contains(r#""origin":"guidebooks""#));
}

#[test]
fn names_guidebooks_and_reads_their_text_within_bounds() {
    assert_eq!(
        guidebook_name_of("Official Guide: Part 2!", GuidebookType::Pdf, &[]),
        "guidebooks/official-guide-part-2.pdf"
    );
    assert_eq!(
        guidebook_name_of("…", GuidebookType::Cbz, &[]),
        "guidebooks/guidebook.cbz"
    );
    assert_eq!(
        guidebook_name_of(
            "Maps",
            GuidebookType::Cbz,
            &["guidebooks/maps.pdf", "guidebooks/maps-2.cbz"]
        ),
        "guidebooks/maps-3.cbz"
    );
    let long = guidebook_name_of(&"word ".repeat(40), GuidebookType::Pdf, &[]);
    assert!(is_guidebook_name(&long), "{long}");
    assert_eq!(
        text_name_of("guidebooks/maps.cbz").as_deref(),
        Some("guidebooks/maps.text.json")
    );
    assert_eq!(text_name_of("maps.cbz"), None);
    assert_eq!(
        GuidebookType::of_file_name("Guide.PDF"),
        Some(GuidebookType::Pdf)
    );
    assert_eq!(GuidebookType::of_file_name("comic.cbr"), None);

    let text = |value: Value, pages: u32| parse_guidebook_text(value.to_string().as_bytes(), pages);
    let ok = json!({"format": GUIDEBOOK_TEXT_FORMAT, "version": 1,
                    "pages": [{"page": 1, "text": "a"}, {"page": 4, "text": "b"}]});
    assert_eq!(text(ok.clone(), 4).unwrap().pages.len(), 2);
    assert!(text(ok.clone(), 3).is_err());
    let mut out_of_order = ok.clone();
    out_of_order["pages"] = json!([{"page": 4, "text": "b"}, {"page": 1, "text": "a"}]);
    assert!(text(out_of_order, 4).is_err());
    let mut page_zero = ok.clone();
    page_zero["pages"] = json!([{"page": 0, "text": "a"}]);
    assert!(text(page_zero, 4).is_err());
    let mut other = ok.clone();
    other["format"] = json!("notes");
    assert!(text(other, 4).is_err());
    let mut newer = ok;
    newer["version"] = json!(2);
    assert_eq!(
        text(newer, 4).unwrap_err(),
        "The text of this guidebook was written in a newer format, so it cannot be read."
    );
    let huge = vec![b' '; MAX_GUIDEBOOK_TEXT_BYTES as usize + 1];
    assert_eq!(
        parse_guidebook_text(&huge, 4).unwrap_err(),
        "The text of this guidebook is too large."
    );
}
