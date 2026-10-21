//! Folders with games (library roots): kept in library.json as
//! `roots: [{id, path, platform?}]` (D15), a path inside the library relative to it,
//! one outside absolute. On a computer where a root's path isn't there it shows as
//! not found, and pointing it elsewhere keeps its id (and so everything matched under it).

use crate::library::{now, valid_id, Library, APP_DIR};
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Root {
    pub id: String,
    /// As stored: relative to the library ("." for the library itself) or absolute
    pub path: String,
    pub platform: Option<String>,
    /// Where it is on this computer
    pub dir: PathBuf,
}

impl Root {
    pub fn found(&self) -> bool {
        self.dir.is_dir()
    }
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id, "path": self.path, "platform": self.platform,
            "dir": self.dir.display().to_string(), "found": self.found(),
            "platform_name": self.platform.as_deref().map(crate::platforms::name_of),
        })
    }
}

fn resolve(lib: &Library, path: &str) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else if path == "." || path.is_empty() {
        lib.root().to_path_buf()
    } else {
        lib.root().join(p)
    }
}

/// The library's folders with games.
pub fn list(lib: &Library) -> Vec<Root> {
    lib.meta()["roots"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| {
            let id = r["id"].as_str()?.to_string();
            let path = r["path"].as_str()?.to_string();
            Some(Root {
                dir: resolve(lib, &path),
                platform: r["platform"].as_str().filter(|p| !p.is_empty()).map(String::from),
                id,
                path,
            })
        })
        .collect()
}

fn save(lib: &Library, roots: &[Root]) -> Result<()> {
    let v: Vec<Value> = roots
        .iter()
        .map(|r| {
            let mut o = json!({ "id": r.id, "path": r.path });
            if let Some(p) = &r.platform {
                o["platform"] = json!(p);
            }
            o
        })
        .collect();
    lib.update_meta(json!({ "roots": v }))?;
    Ok(())
}

/// How `dir` is stored: relative when it's inside the library.
fn stored(lib: &Library, dir: &Path) -> String {
    match lib.relative(dir) {
        Some(rel) if rel.is_empty() => ".".into(),
        Some(rel) => rel,
        None => dir.display().to_string(),
    }
}

fn norm(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// Check `dir` can be a folder with games next to `others`.
fn check_dir(lib: &Library, dir: &Path, others: &[Root]) -> Result<PathBuf> {
    if !dir.is_dir() {
        bail!("{} isn't a folder on this computer.", dir.display());
    }
    check_place(lib, dir, others)
}

/// Check a folder (which may not exist yet) could be one with games next to `others`.
fn check_place(lib: &Library, dir: &Path, others: &[Root]) -> Result<PathBuf> {
    if !dir.is_absolute() {
        bail!("Choose a folder by its full path.");
    }
    // a folder not made yet is compared by the nearest one above it that exists
    let mut base = dir.to_path_buf();
    let mut rest = vec![];
    while !base.exists() {
        match (base.file_name().map(|n| n.to_os_string()), base.parent()) {
            (Some(n), Some(p)) => {
                rest.push(n);
                base = p.to_path_buf();
            }
            _ => break,
        }
    }
    let d = rest.into_iter().rev().fold(norm(&base), |p, n| p.join(n));
    if d.starts_with(norm(&lib.root().join(APP_DIR))) {
        bail!("The library's own _library folder can't hold games.");
    }
    for r in others.iter().filter(|r| r.found()) {
        let o = norm(&r.dir);
        if d == o {
            bail!("{} is already a folder with games.", dir.display());
        }
        if d.starts_with(&o) {
            bail!("{} is inside {}, which is already scanned.", dir.display(), r.dir.display());
        }
        if o.starts_with(&d) {
            bail!("{} holds {}, which is already a folder with games: remove that one first.", dir.display(), r.dir.display());
        }
    }
    Ok(d)
}

fn check_platform(platform: Option<&str>) -> Result<Option<String>> {
    match platform.map(str::trim).filter(|p| !p.is_empty()) {
        None => Ok(None),
        Some(p) => {
            valid_id(p)?;
            Ok(Some(p.to_string()))
        }
    }
}

/// Whether `p` is in the library folder (or is it).
pub fn in_library(lib: &Library, p: &Path) -> bool {
    let (k, top) = (crate::organise::key(p), crate::organise::key(lib.root()));
    let top = top.trim_end_matches('/');
    k == top || k.starts_with(&format!("{top}/"))
}

/// Whether `dir` could be added as a folder with games (it isn't inside one, nor holds one).
pub fn check_new(lib: &Library, dir: &Path) -> Result<()> {
    check_place(lib, dir, &list(lib)).map(|_| ())
}

/// Add a folder with games (optionally marked with its platform).
pub fn add(lib: &Library, dir: &Path, platform: Option<&str>) -> Result<Root> {
    lib.writable()?;
    let mut roots = list(lib);
    check_dir(lib, dir, &roots)?;
    let platform = check_platform(platform)?;
    let mut n = roots.len() + 1;
    let id = loop {
        use sha2::{Digest, Sha256};
        let seed = format!("{}{}{n}", dir.display(), now());
        let id = format!("root-{}", &hex::encode(Sha256::digest(seed.as_bytes()))[..10]);
        if !roots.iter().any(|r| r.id == id) {
            break id;
        }
        n += 1;
    };
    let root = Root {
        id,
        path: stored(lib, dir),
        platform,
        dir: dir.to_path_buf(),
    };
    roots.push(root.clone());
    save(lib, &roots)?;
    Ok(root)
}

/// Point a folder with games elsewhere (on this computer) and/or set its platform; its id stays.
pub fn update(lib: &Library, id: &str, dir: Option<&Path>, platform: Option<Option<&str>>) -> Result<Root> {
    lib.writable()?;
    let mut roots = list(lib);
    let Some(i) = roots.iter().position(|r| r.id == id) else {
        bail!("There's no folder with games {id}.");
    };
    if let Some(dir) = dir {
        let others: Vec<Root> = roots.iter().filter(|r| r.id != id).cloned().collect();
        check_dir(lib, dir, &others)?;
        roots[i].path = stored(lib, dir);
        roots[i].dir = dir.to_path_buf();
    }
    if let Some(p) = platform {
        roots[i].platform = check_platform(p)?;
    }
    save(lib, &roots)?;
    Ok(roots[i].clone())
}

/// Stop scanning a folder (nothing on disk is touched).
pub fn remove(lib: &Library, id: &str) -> Result<()> {
    lib.writable()?;
    let mut roots = list(lib);
    let before = roots.len();
    roots.retain(|r| r.id != id);
    if roots.len() == before {
        bail!("There's no folder with games {id}.");
    }
    save(lib, &roots)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::tests::temp_dir;

    #[test]
    fn roots_are_kept_in_the_library() {
        let dir = temp_dir("roots");
        let lib = Library::open(dir.join("Lib")).unwrap();
        std::fs::create_dir_all(dir.join("Lib/SNES")).unwrap();
        std::fs::create_dir_all(dir.join("Elsewhere/GB")).unwrap();
        let a = add(&lib, &dir.join("Lib/SNES"), None).unwrap();
        assert_eq!(a.path, "SNES");
        let b = add(&lib, &dir.join("Elsewhere"), Some("gb")).unwrap();
        assert!(Path::new(&b.path).is_absolute());
        assert!(add(&lib, &dir.join("Elsewhere/GB"), None).is_err(), "inside another root");
        assert!(add(&lib, &dir.join("Lib"), None).is_err(), "holds another root");
        assert!(add(&lib, &dir.join("Lib/_library"), None).is_err());
        assert!(add(&lib, &dir.join("Nowhere"), None).is_err());
        let all = list(&Library::open(dir.join("Lib")).unwrap());
        assert_eq!(all.len(), 2);
        assert_eq!(all[1].platform.as_deref(), Some("gb"));
        // pointed elsewhere, it keeps its id
        std::fs::create_dir_all(dir.join("Moved")).unwrap();
        let moved = update(&lib, &b.id, Some(&dir.join("Moved")), Some(None)).unwrap();
        assert_eq!(moved.id, b.id);
        assert!(moved.platform.is_none());
        std::fs::remove_dir_all(dir.join("Moved")).unwrap();
        assert!(!list(&lib)[1].found());
        remove(&lib, &a.id).unwrap();
        assert_eq!(list(&lib).len(), 1);
        assert!(dir.join("Lib/SNES").is_dir(), "nothing on disk is touched");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
