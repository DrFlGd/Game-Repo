//! The description and the small pictures, read from text and from files,
//! and every refusal. Every name here is made up.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

use serde_json::{json, Value};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

use super::*;

/// A folder of its own under the system's temporary folder, removed after.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "atlas-pack-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A JPEG's first bytes, enough for the check a reader makes.
const JPEG: &[u8] = &[0xff, 0xd8, 0xff, 0xe0, 0, 16, b'J', b'F', b'I', b'F', 0];

/// A version 6 description, as an app before small pictures wrote it.
fn header_v6() -> Value {
    json!({
        "format": "atlas-pack-file",
        "version": 6,
        "packId": "test-valley",
        "title": "Test Valley",
        "packVersion": "1.1.0",
        "detail": "standard",
        "exportedAt": "2026-11-11T09:30:00Z",
        "exportedBy": {"name": "Atlas", "version": "0.21.0", "platform": "windows"},
        "pack": {"id": "test-valley", "title": "Test Valley", "packVersion": "1.1.0", "schemaVersion": 2,
                 "attribution": [{"name": "Demo map site (made-up test data)"}]},
        "source": {"kind": "web-pages", "importer": {"id": "demo-maps", "title": "Demo map site"}},
        "contents": {"maps": 2, "layers": 2, "locations": 3, "categories": 1, "regions": 0, "checklists": 0, "guides": 4},
        "images": {"detail": "standard", "files": 5, "bytes": 500, "layers": []},
        "files": 9,
        "bytes": 900,
        "mine": {"lists": 1, "group": true}
    })
}

/// A version 7 description with every new field.
fn header_v7() -> Value {
    let mut header = header_v6();
    let object = header.as_object_mut().unwrap();
    object.insert("version".into(), json!(7));
    object.insert(
        "game".into(),
        json!({"title": "Isles", "system": "Saltbox 64"}),
    );
    object.insert("personalUse".into(), json!(true));
    object.insert(
        "maps".into(),
        json!([
            {"id": "test-valley:map:east", "name": "East Field", "width": 2048, "height": 2048, "locations": 2,
             "preview": "preview/east.jpg"},
            {"id": "test-valley:map:west", "name": "West Ridge", "width": 1024, "height": 512, "locations": 1,
             "preview": "preview/west.jpg"},
            {"id": "test-valley:map:cave", "name": "Cave", "width": 300, "height": 200, "locations": 0}
        ]),
    );
    object.insert(
        "guides".into(),
        json!([{"title": "Test Valley", "type": "wiki", "pages": 4}]),
    );
    object.insert("cover".into(), json!("preview/east.jpg"));
    object["source"]["name"] = json!("Example Wiki");
    header
}

/// Writes a pack file by hand: the description first, then the entries,
/// JSON deflated and the rest stored, as the app writes them.
fn write_file(path: &Path, header: &[u8], entries: &[(&str, &[u8])]) {
    let mut zip = ZipWriter::new(fs::File::create(path).unwrap());
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    zip.start_file(HEADER_FILE, deflated).unwrap();
    zip.write_all(header).unwrap();
    for (name, bytes) in entries {
        let options = if name.ends_with(".json") {
            deflated
        } else {
            stored
        };
        zip.start_file(*name, options).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
}

fn text(value: &Value) -> Vec<u8> {
    serde_json::to_vec_pretty(value).unwrap()
}

#[test]
fn reads_the_new_fields_and_a_description_without_them() {
    let header = parse_header(&header_v7().to_string()).unwrap();
    assert_eq!(header.version, VERSION_WITH_PREVIEWS);
    assert_eq!(
        header.game,
        Some(GameFacts {
            title: "Isles".into(),
            system: Some("Saltbox 64".into())
        })
    );
    assert!(header.personal_use);
    assert_eq!(header.source.name.as_deref(), Some("Example Wiki"));
    assert_eq!(header.maps.len(), 3);
    assert_eq!(header.maps[1].preview.as_deref(), Some("preview/west.jpg"));
    assert_eq!((header.maps[1].width, header.maps[1].locations), (1024, 1));
    assert_eq!(header.maps[2].preview, None);
    assert_eq!(
        header.guides,
        [GuideFacts {
            title: "Test Valley".into(),
            guide_type: GuideType::Wiki,
            pages: 4
        }]
    );
    assert_eq!(header.cover.as_deref(), Some("preview/east.jpg"));
    assert_eq!(header.previews(), ["preview/east.jpg", "preview/west.jpg"]);
    assert!(header.lists_preview("preview/west.jpg"));
    assert!(!header.lists_preview("preview/cave.jpg"));

    // Version 6: none of them, and nothing made up.
    let old = parse_header(&serde_json::to_string(&header_v6()).unwrap()).unwrap();
    assert_eq!(old.version, VERSION_WITH_TITLE);
    assert_eq!(old.game, None);
    assert!(!old.personal_use);
    assert_eq!(old.source.name, None);
    assert!(old.maps.is_empty() && old.guides.is_empty() && old.cover.is_none());
    assert!(old.previews().is_empty());
    assert_eq!(old.mine.map(|mine| mine.group), Some(true));
    // Written again, it has none of their names either.
    let written = serde_json::to_value(&old).unwrap();
    for key in ["game", "personalUse", "maps", "guides", "cover"] {
        assert!(written.get(key).is_none(), "{key}");
    }
    assert!(written["source"].get("name").is_none());
    // A type of pages this crate does not know reads as another type.
    let mut newer = header_v7();
    newer["guides"][0]["type"] = json!("almanac");
    let header = parse_header(&newer.to_string()).unwrap();
    assert_eq!(header.guides[0].guide_type, GuideType::Other);
}

#[test]
fn refuses_a_newer_version_and_what_is_no_pack_file() {
    let mut newer = header_v7();
    newer["version"] = json!(VERSION + 1);
    newer["exportedBy"] =
        json!({"name": "Atlas\u{202e}", "version": "9.1.0\nx", "platform": "linux"});
    let error = parse_header(&newer.to_string()).unwrap_err();
    // The kind lets an app give its own advice after what is wrong; shown
    // as it is, the sentence names no app to renew.
    assert_eq!(error.kind(), RefusalKind::NewerFormat);
    assert_eq!(
        error.what(),
        "This pack file was made by Atlas 9.1.0 x in a newer format (version 9)."
    );
    assert_eq!(
        error.to_string(),
        "This pack file was made by Atlas 9.1.0 x in a newer format (version 9). A newer version of this app is needed to open it."
    );
    assert_eq!(String::from(error.clone()), error.to_string());
    for (key, value) in [
        ("format", json!("atlas-progress")),
        ("version", json!(0)),
        ("version", json!("7")),
    ] {
        let mut other = header_v7();
        other[key] = value;
        assert_eq!(
            parse_header(&other.to_string()).unwrap_err().to_string(),
            NOT_A_PACK_FILE,
            "{key}"
        );
    }
    assert_eq!(
        parse_header("not json").unwrap_err().to_string(),
        NOT_A_PACK_FILE
    );
    let mut damaged = header_v7();
    damaged["maps"] = json!("East Field");
    assert!(parse_header(&damaged.to_string())
        .unwrap_err()
        .to_string()
        .contains("damaged"));
    assert_eq!(
        parse_header("not json").unwrap_err().kind(),
        RefusalKind::NotAPackFile
    );
}

#[test]
fn refuses_a_description_without_what_every_pack_file_has() {
    // The fields a pack file has had since version 1 are always there: a
    // description without one of them is damaged, in every version.
    for field in [
        "packId",
        "title",
        "packVersion",
        "detail",
        "exportedAt",
        "exportedBy",
        "pack",
        "source",
        "contents",
        "images",
        "files",
        "bytes",
    ] {
        for mut header in [header_v6(), header_v7()] {
            header.as_object_mut().unwrap().remove(field);
            let error = parse_header(&header.to_string()).unwrap_err();
            assert_eq!(error.kind(), RefusalKind::Damaged, "{field}");
            assert!(error.to_string().contains("damaged"), "{field}: {error}");
        }
    }
    // The fields added since are optional.
    for field in ["game", "personalUse", "maps", "guides", "cover", "mine"] {
        let mut header = header_v7();
        header.as_object_mut().unwrap().remove(field);
        assert!(parse_header(&header.to_string()).is_ok(), "{field}");
    }
}

#[test]
fn refuses_a_pack_id_that_cannot_be_one() {
    for good in ["test-valley", "0", "a_b-c9", &"a".repeat(64)] {
        assert!(is_pack_id(good), "{good}");
    }
    for bad in [
        "",
        "Test-Valley",
        "-valley",
        "_valley",
        "test valley",
        "test/valley",
        "..",
        "t\u{e9}st",
        &"a".repeat(65),
    ] {
        assert!(!is_pack_id(bad), "{bad:?}");
        let mut header = header_v7();
        header["packId"] = json!(bad);
        let error = parse_header(&header.to_string()).unwrap_err();
        assert_eq!(error.kind(), RefusalKind::Damaged, "{bad:?}");
        assert_eq!(
            error.to_string(),
            "This pack file names a game id that cannot be used.",
            "{bad:?}"
        );
    }
}

#[test]
fn tells_safe_names_of_small_pictures_and_leaves_out_others() {
    for good in [
        "preview/east.jpg",
        "preview/0.jpg",
        "preview/west-ridge_2.jpg",
    ] {
        assert!(is_preview_name(good), "{good}");
    }
    let long = format!("preview/{}.jpg", "a".repeat(MAX_PREVIEW_NAME + 1));
    for bad in [
        "preview/.jpg",
        "preview/-east.jpg",
        "preview/East.jpg",
        "preview/east.JPG",
        "preview/east.png",
        "preview/a/b.jpg",
        "preview/../east.jpg",
        "preview\\east.jpg",
        "/preview/east.jpg",
        "pictures/east.jpg",
        "preview/east.jpg/",
        "preview/e st.jpg",
        long.as_str(),
    ] {
        assert!(!is_preview_name(bad), "{bad}");
    }
    let mut header = header_v7();
    header["maps"][0]["preview"] = json!("preview/../manifest.jpg");
    header["maps"][1]["preview"] = json!("C:/preview/west.jpg");
    header["cover"] = json!("pictures/gate.jpg");
    let header = parse_header(&header.to_string()).unwrap();
    assert!(header.maps.iter().all(|map| map.preview.is_none()));
    assert_eq!(header.cover, None);
}

#[test]
fn reads_the_description_and_a_small_picture_from_a_file() {
    let dir = TempDir::new("read");
    let file = dir.0.join("valley.atlaspack");
    let west = [JPEG, b"west"].concat();
    write_file(
        &file,
        &text(&header_v7()),
        &[
            ("preview/east.jpg", JPEG),
            ("preview/west.jpg", &west),
            ("manifest.json", b"{}"),
            ("tiles/a/0/0/0.png", b"tile"),
        ],
    );
    let header = read_header(&file).unwrap();
    assert_eq!(header.cover.as_deref(), Some("preview/east.jpg"));
    assert_eq!(read_preview(&file, "preview/east.jpg").unwrap(), JPEG);
    assert_eq!(read_preview(&file, "preview/west.jpg").unwrap(), west);
    // From a stream read in order, too.
    let opened = fs::File::open(&file).unwrap();
    assert_eq!(read_preview_from(opened, "preview/west.jpg").unwrap(), west);
    let opened = fs::File::open(&file).unwrap();
    assert_eq!(read_header_from(opened).unwrap(), header);

    // Only the start of the file is read: a copy cut short after the small
    // pictures, without its list of contents, reads the same.
    let bytes = fs::read(&file).unwrap();
    let end = bytes
        .windows(b"manifest.json".len())
        .position(|window| window == b"manifest.json")
        .unwrap();
    let cut = dir.0.join("cut.atlaspack");
    fs::write(&cut, &bytes[..end]).unwrap();
    assert_eq!(read_header(&cut).unwrap(), header);
    assert_eq!(read_preview(&cut, "preview/west.jpg").unwrap(), west);

    // A version 6 file has none.
    let old = dir.0.join("old.atlaspack");
    write_file(&old, &text(&header_v6()), &[("manifest.json", b"{}")]);
    assert_eq!(read_header(&old).unwrap().version, 6);
    assert!(read_preview(&old, "preview/east.jpg").is_err());
}

#[test]
fn refuses_hostile_files() {
    let dir = TempDir::new("hostile");
    let header = text(&header_v7());
    let no_such = |file: &Path, name: &str| {
        let error = read_preview(file, name).unwrap_err();
        assert_eq!(error.kind(), RefusalKind::NoSuchPicture, "{name}");
        let error = error.to_string();
        assert!(error.contains("no small picture"), "{name}: {error}");
    };

    // A description larger than the cap, and a first entry that is another file.
    let large = dir.0.join("large.atlaspack");
    let mut padded = header.clone();
    padded.resize(MAX_HEADER_BYTES as usize + 1, b' ');
    write_file(&large, &padded, &[]);
    assert_eq!(
        read_header(&large).unwrap_err().to_string(),
        NOT_A_PACK_FILE
    );
    let second = dir.0.join("second.atlaspack");
    {
        let mut zip = ZipWriter::new(fs::File::create(&second).unwrap());
        zip.start_file("manifest.json", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"{}").unwrap();
        zip.start_file(HEADER_FILE, SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&header).unwrap();
        zip.finish().unwrap();
    }
    assert_eq!(
        read_header(&second).unwrap_err().to_string(),
        NOT_A_PACK_FILE
    );
    let text_file = dir.0.join("text.atlaspack");
    fs::write(&text_file, "hello").unwrap();
    assert_eq!(
        read_header(&text_file).unwrap_err().to_string(),
        NOT_A_PACK_FILE
    );
    assert!(read_header(&dir.0.join("missing.atlaspack"))
        .unwrap_err()
        .to_string()
        .contains("Could not open"));

    // A small picture the description does not list, an unsafe name, and
    // one listed that the file does not hold.
    let file = dir.0.join("valley.atlaspack");
    write_file(
        &file,
        &header,
        &[
            ("preview/east.jpg", JPEG),
            ("preview/cave.jpg", JPEG),
            ("manifest.json", b"{}"),
            ("preview/west.jpg", JPEG),
        ],
    );
    no_such(&file, "preview/cave.jpg");
    no_such(&file, "preview/../east.jpg");
    no_such(&file, "preview/East.jpg");
    no_such(&file, "manifest.json");
    // Listed, but after the first entry that is no small picture: not read.
    no_such(&file, "preview/west.jpg");
    assert_eq!(read_preview(&file, "preview/east.jpg").unwrap(), JPEG);

    // An entry with an unsafe name among the small pictures, one too large,
    // and one that is no JPEG.
    let unsafe_name = dir.0.join("unsafe.atlaspack");
    write_file(
        &unsafe_name,
        &header,
        &[("preview/Up.jpg", JPEG), ("preview/west.jpg", JPEG)],
    );
    assert!(read_preview(&unsafe_name, "preview/west.jpg")
        .unwrap_err()
        .to_string()
        .contains("cannot be read"));
    let too_large = dir.0.join("too-large.atlaspack");
    let mut big = JPEG.to_vec();
    big.resize(MAX_PREVIEW_BYTES as usize + 1, 0);
    write_file(&too_large, &header, &[("preview/east.jpg", &big)]);
    assert!(read_preview(&too_large, "preview/east.jpg")
        .unwrap_err()
        .to_string()
        .contains("cannot be read"));
    let not_jpeg = dir.0.join("not-jpeg.atlaspack");
    write_file(&not_jpeg, &header, &[("preview/east.jpg", b"\x89PNG\r\n")]);
    assert!(read_preview(&not_jpeg, "preview/east.jpg")
        .unwrap_err()
        .to_string()
        .contains("not a JPEG"));
    // A newer file is refused before any small picture is read.
    let mut newer = header_v7();
    newer["version"] = json!(VERSION + 1);
    let newer_file = dir.0.join("newer.atlaspack");
    write_file(&newer_file, &text(&newer), &[("preview/east.jpg", JPEG)]);
    assert!(read_preview(&newer_file, "preview/east.jpg")
        .unwrap_err()
        .to_string()
        .contains("newer format"));
}
