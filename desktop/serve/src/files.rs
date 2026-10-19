//! A plan's files, fetched (D95): a file in a folder with games
//! (`/v1/files/roots/<folder's id>/<path>`), a game's media or documents in the library
//! folder (`/v1/files/library/<path>`), or a made one (`/v1/files/made/<id>`). Nothing
//! else can be fetched: never `_library/`'s settings, journal or DATs, nothing hidden (the
//! set-aside folders), nothing outside. One range per request, so a download picks up where
//! it stopped; a file's ETag is its size and modified time, so resuming a file that changed
//! (`If-Range`) starts it again.

use crate::http::{self, Body, Request, Response};
use crate::made;
use gamerepo_core::library::{rel_inside, Library};
use gamerepo_core::roots::Root;
use std::path::{Component, Path, PathBuf};

/// Hidden parts (the set-aside and staging folders) are never served.
fn hidden(rel: &Path) -> bool {
    rel.components().any(|c| match c {
        Component::Normal(n) => n.to_string_lossy().starts_with('.'),
        _ => true,
    })
}

/// Can this path in the library folder be fetched? Its games' media and documents in
/// `_library/games/…/media|docs/` and `_library/docs/`, and anything outside `_library/`
/// that isn't hidden.
fn library_allows(rel: &Path) -> bool {
    if hidden(rel) {
        return false;
    }
    let mut parts = rel.components().map(|c| c.as_os_str().to_string_lossy().to_lowercase());
    match parts.next().as_deref() {
        // a game's pictures and documents, not its details or what sources answered
        Some("_library") => match parts.next().as_deref() {
            Some("games") => parts.any(|p| p == "media" || p == "docs"),
            Some("docs") => parts.next().is_some(),
            _ => false,
        },
        Some(_) => true,
        None => false,
    }
}

fn slashes(p: &Path) -> String {
    p.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/")
}

/// Where a library file a plan copies is fetched from; None: it can't be.
pub fn url_of(lib: &Library, roots: &[Root], path: &Path) -> Option<String> {
    // the deepest folder that holds it (a folder with games can be in the library folder)
    let mut best: Option<(usize, String)> = None;
    for r in roots {
        if let Ok(rel) = path.strip_prefix(&r.dir) {
            if !hidden(rel) && rel.components().next().is_some() {
                let depth = r.dir.components().count();
                if best.as_ref().is_none_or(|(d, _)| depth > *d) {
                    best = Some((depth, format!("/v1/files/roots/{}/{}", http::encode_path(&r.id), http::encode_path(&slashes(rel)))));
                }
            }
        }
    }
    if let Some((_, u)) = best {
        return Some(u);
    }
    let rel = path.strip_prefix(lib.root()).ok()?;
    library_allows(rel).then(|| format!("/v1/files/library/{}", http::encode_path(&slashes(rel))))
}

/// The library file a `/v1/files/library/…` or `/v1/files/roots/…` path names, if it can be fetched.
pub fn resolve(lib: &Library, roots: &[Root], rest: &str) -> Option<PathBuf> {
    if let Some(rel) = rest.strip_prefix("library/") {
        let rel = rel_inside(rel).ok()?;
        return library_allows(&rel).then(|| lib.root().join(rel));
    }
    let (id, rel) = rest.strip_prefix("roots/")?.split_once('/')?;
    let rel = rel_inside(rel).ok()?;
    if hidden(&rel) {
        return None;
    }
    roots.iter().find(|r| r.id == id).map(|r| r.dir.join(rel))
}

/// A file's ETag: its size and modified time.
pub fn etag(path: &Path) -> Option<String> {
    made::stamp(path).map(|(s, t)| format!("\"{s}-{t}\""))
}

/// A file sent whole or from a range, as asked.
pub fn send(req: &Request, path: &Path, content_type: &str) -> Response {
    let Ok(meta) = std::fs::metadata(path) else { return Response::problem(404, "That file isn't in the library now: ask for the plan again.") };
    if !meta.is_file() {
        return Response::problem(404, "That isn't a file.");
    }
    let total = meta.len();
    let tag = etag(path).unwrap_or_default();
    if req.header("if-none-match").is_some_and(|t| t == tag) {
        return Response::bytes(304, content_type, vec![]).with("ETag", tag);
    }
    // a resume of a file that has changed since starts again (If-Range)
    let wanted = match req.header("if-range") {
        Some(t) if t != tag => None,
        _ => req.header("range"),
    };
    match http::range(wanted, total) {
        Ok(None) => Response { status: 200, headers: vec![("Content-Type".into(), content_type.into())], body: Body::File { path: path.to_path_buf(), start: 0, len: total } }
            .with("ETag", tag)
            .with("Accept-Ranges", "bytes"),
        Ok(Some((start, len))) => Response { status: 206, headers: vec![("Content-Type".into(), content_type.into())], body: Body::File { path: path.to_path_buf(), start, len } }
            .with("ETag", tag)
            .with("Accept-Ranges", "bytes")
            .with("Content-Range", format!("bytes {start}-{}/{total}", start + len - 1)),
        Err(()) => Response::bytes(416, "text/plain", vec![]).with("Content-Range", format!("bytes */{total}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_games_media_and_documents_are_served_from_the_library_folder() {
        for (rel, ok) in [
            ("_library/games/gb/Alpha/media/cover.png", true),
            ("_library/docs/General/Guide.pdf", true),
            ("BIOS/psx/scph1001.bin", true),
            ("_library/library.json", false),
            ("_library/journal/x.json", false),
            ("_library/dats/gb/a.dat", false),
            ("_library/games", false),
            ("_library/games/gb/Alpha/game.json", false),
            ("_library/games/gb/Alpha/docs/Alpha Manual.pdf", true),
            ("Roms/.gamerepo-set-aside/x/a.zip", false),
        ] {
            assert_eq!(library_allows(Path::new(rel)), ok, "{rel}");
        }
    }
}
