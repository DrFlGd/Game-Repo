//! Steam (docs/PLAN.md 3.4, "Phase 4 design", D28): PC games with a program to start
//! become non-Steam shortcuts, with their artwork in Steam's grid folder. Steam keeps its
//! shortcuts in `userdata/<id>/config/shortcuts.vdf`, a binary VDF file it rewrites
//! itself, so it's written only while Steam is closed, backed up first, and only the
//! shortcuts tagged "Game Repo" are the app's: every other is kept as it is.

use crate::library::{now, Library};
use crate::pcgames;
use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

/// The tag that marks the app's shortcuts.
pub const TAG: &str = "Game Repo";
/// Backups of shortcuts.vdf kept.
const BACKUPS: usize = 5;

// ------------------------------------------------------------ binary VDF

/// A value in a binary VDF file. Strings are kept as bytes, so shortcuts the app doesn't
/// own are written back exactly as they were.
#[derive(Clone, Debug, PartialEq)]
pub enum Vdf {
    Map(Vec<(Vec<u8>, Vdf)>),
    Str(Vec<u8>),
    Int(u32),
    /// Any other type (float, 64-bit…): its type byte and bytes
    Raw(u8, Vec<u8>),
}

impl Vdf {
    pub fn get(&self, key: &str) -> Option<&Vdf> {
        match self {
            Vdf::Map(m) => m.iter().find(|(k, _)| k.eq_ignore_ascii_case(key.as_bytes())).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn str(&self, key: &str) -> Option<String> {
        match self.get(key)? {
            Vdf::Str(s) => Some(String::from_utf8_lossy(s).into_owned()),
            _ => None,
        }
    }

    pub fn int(&self, key: &str) -> Option<u32> {
        match self.get(key)? {
            Vdf::Int(n) => Some(*n),
            _ => None,
        }
    }
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn byte(&mut self) -> Result<u8> {
        let v = *self.b.get(self.at).ok_or_else(|| anyhow!("shortcuts.vdf ends early"))?;
        self.at += 1;
        Ok(v)
    }

    fn cstr(&mut self) -> Result<Vec<u8>> {
        let rest = &self.b[self.at..];
        let end = rest.iter().position(|&c| c == 0).ok_or_else(|| anyhow!("shortcuts.vdf ends inside a name"))?;
        self.at += end + 1;
        Ok(rest[..end].to_vec())
    }

    fn take(&mut self, n: usize) -> Result<Vec<u8>> {
        let v = self.b.get(self.at..self.at + n).ok_or_else(|| anyhow!("shortcuts.vdf ends early"))?.to_vec();
        self.at += n;
        Ok(v)
    }

    fn map(&mut self, depth: usize) -> Result<Vec<(Vec<u8>, Vdf)>> {
        if depth > 32 {
            bail!("shortcuts.vdf is nested too deep");
        }
        let mut m = vec![];
        loop {
            let t = match self.b.get(self.at) {
                None if depth == 0 => return Ok(m),
                None => bail!("shortcuts.vdf ends early"),
                Some(_) => self.byte()?,
            };
            if t == 0x08 {
                return Ok(m);
            }
            let k = self.cstr()?;
            let v = match t {
                0x00 => Vdf::Map(self.map(depth + 1)?),
                0x01 => Vdf::Str(self.cstr()?),
                0x02 => Vdf::Int(u32::from_le_bytes(self.take(4)?.try_into().unwrap())),
                0x03 | 0x04 | 0x06 => Vdf::Raw(t, self.take(4)?),
                0x07 | 0x0a => Vdf::Raw(t, self.take(8)?),
                other => bail!("shortcuts.vdf has a value of a type the app doesn't know ({other})"),
            };
            m.push((k, v));
        }
    }
}

/// Read a binary VDF file: its top level.
pub fn parse(b: &[u8]) -> Result<Vec<(Vec<u8>, Vdf)>> {
    Reader { b, at: 0 }.map(0)
}

fn put(out: &mut Vec<u8>, m: &[(Vec<u8>, Vdf)]) {
    for (k, v) in m {
        let (t, body): (u8, Vec<u8>) = match v {
            Vdf::Map(inner) => {
                let mut b = vec![];
                put(&mut b, inner);
                b.push(0x08);
                (0x00, b)
            }
            Vdf::Str(s) => {
                let mut b = s.clone();
                b.push(0);
                (0x01, b)
            }
            Vdf::Int(n) => (0x02, n.to_le_bytes().to_vec()),
            Vdf::Raw(t, b) => (*t, b.clone()),
        };
        out.push(t);
        out.extend_from_slice(k);
        out.push(0);
        out.extend(body);
    }
}

/// Write a binary VDF file's top level.
pub fn write(m: &[(Vec<u8>, Vdf)]) -> Vec<u8> {
    let mut out = vec![];
    put(&mut out, m);
    out.push(0x08);
    out
}

// ------------------------------------------------------------ a shortcut

/// Steam's id for a non-Steam shortcut: the CRC32 of its program (as the shortcut writes
/// it, in quotes) and its name, with the top bit set. The grid's artwork is named by it.
pub fn shortcut_id(exe: &str, name: &str) -> u32 {
    let mut h = crc32fast::Hasher::new();
    h.update(exe.as_bytes());
    h.update(name.as_bytes());
    h.finalize() | 0x8000_0000
}

fn quoted(p: &str) -> String {
    format!("\"{}\"", p.trim_matches('"'))
}

/// One of the app's shortcuts.
#[derive(Clone, Debug, PartialEq)]
pub struct Shortcut {
    pub game: String,
    pub name: String,
    /// The program, in quotes as Steam writes it
    pub exe: String,
    pub start_dir: String,
    pub args: String,
    pub icon: String,
    pub appid: u32,
}

impl Shortcut {
    fn to_vdf(&self) -> Vdf {
        let s = |k: &str, v: &str| (k.as_bytes().to_vec(), Vdf::Str(v.as_bytes().to_vec()));
        let i = |k: &str, v: u32| (k.as_bytes().to_vec(), Vdf::Int(v));
        Vdf::Map(vec![
            i("appid", self.appid),
            s("AppName", &self.name),
            s("Exe", &self.exe),
            s("StartDir", &self.start_dir),
            s("icon", &self.icon),
            s("ShortcutPath", ""),
            s("LaunchOptions", &self.args),
            i("IsHidden", 0),
            i("AllowDesktopConfig", 1),
            i("AllowOverlay", 1),
            i("OpenVR", 0),
            i("Devkit", 0),
            s("DevkitGameID", ""),
            i("DevkitOverrideAppID", 0),
            i("LastPlayTime", 0),
            s("FlatpakAppID", ""),
            (b"tags".to_vec(), Vdf::Map(vec![(b"0".to_vec(), Vdf::Str(TAG.as_bytes().to_vec()))])),
        ])
    }
}

/// Whether a shortcut in the file is one of the app's (tagged "Game Repo").
fn is_ours(v: &Vdf) -> bool {
    match v.get("tags") {
        Some(Vdf::Map(tags)) => tags.iter().any(|(_, t)| matches!(t, Vdf::Str(s) if s == TAG.as_bytes())),
        _ => false,
    }
}

/// The shortcuts in a shortcuts.vdf: (key, value) of its "shortcuts" map.
fn shortcuts_of(top: &[(Vec<u8>, Vdf)]) -> Vec<Vdf> {
    top.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(b"shortcuts"))
        .and_then(|(_, v)| match v {
            Vdf::Map(m) => Some(m.iter().map(|(_, v)| v.clone()).collect()),
            _ => None,
        })
        .unwrap_or_default()
}

/// The app's shortcut for each PC game with a program, its artwork beside it.
pub fn shortcuts(lib: &Library) -> (Vec<(Shortcut, BTreeMap<String, PathBuf>)>, Vec<String>) {
    let mut out = vec![];
    let mut without = vec![];
    for e in pcgames::list(lib) {
        let Some(l) = &e.game.launch else {
            without.push(e.game.title.clone());
            continue;
        };
        let prog = pcgames::program_path(&e.dir, l);
        let start = if l.dir.is_empty() { prog.parent().map(Path::to_path_buf).unwrap_or_default() } else { pcgames::program_path(&e.dir, &pcgames::Launch { program: l.dir.clone(), ..Default::default() }) };
        let exe = quoted(&prog.display().to_string());
        let name = e.game.title.clone();
        let media: BTreeMap<String, PathBuf> = pcgames::media(lib, &e.game.id)
            .into_iter()
            .filter_map(|(k, v)| Some((k, lib.resolve(v.as_str()?).ok()?)))
            .collect();
        let s = Shortcut {
            game: e.game.id.clone(),
            appid: shortcut_id(&exe, &name),
            icon: media.get("icon").map(|p| p.display().to_string()).unwrap_or_else(|| prog.display().to_string()),
            name,
            exe,
            start_dir: quoted(&start.display().to_string()),
            args: l.args.clone(),
        };
        out.push((s, media));
    }
    (out, without)
}

// ------------------------------------------------------------ Steam on this computer

/// Steam's folder: the one set in Settings, else where Steam says (Windows' registry),
/// else the usual places.
pub fn find(set: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = set.filter(|p| p.join("userdata").is_dir() || p.join("steam.exe").is_file()) {
        return Some(p.to_path_buf());
    }
    let mut tries: Vec<PathBuf> = vec![];
    #[cfg(windows)]
    {
        let out = crate::tools::command(Path::new("reg")).args(["query", r"HKCU\Software\Valve\Steam", "/v", "SteamPath"]).output();
        if let Ok(o) = out {
            let text = String::from_utf8_lossy(&o.stdout);
            if let Some(l) = text.lines().find(|l| l.contains("SteamPath")) {
                if let Some(at) = l.find("REG_SZ") {
                    tries.push(PathBuf::from(l[at + 6..].trim()));
                }
            }
        }
        tries.push(PathBuf::from(r"C:\Program Files (x86)\Steam"));
        tries.push(PathBuf::from(r"C:\Program Files\Steam"));
    }
    #[cfg(not(windows))]
    {
        let home = crate::config::home_dir();
        tries.push(home.join(".steam/steam"));
        tries.push(home.join(".local/share/Steam"));
    }
    tries.into_iter().find(|p| p.join("userdata").is_dir())
}

/// Whether Steam is running (its shortcuts can't be written while it is).
pub fn running() -> bool {
    #[cfg(windows)]
    {
        crate::tools::command(Path::new("tasklist"))
            .args(["/FI", "IMAGENAME eq steam.exe", "/NH"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_lowercase().contains("steam.exe"))
            .unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("pgrep").args(["-x", "steam"]).output().map(|o| o.status.success()).unwrap_or(false)
    }
}

/// Steam's 32-bit account id from a SteamID64 (`userdata/<id>` is named by it).
pub fn account_id(steam64: &str) -> Option<u64> {
    steam64.parse::<u64>().ok()?.checked_sub(76_561_197_960_265_728)
}

/// The users Steam has signed in on this computer: `userdata/<id>/`, named from
/// `config/loginusers.vdf` (a text VDF), the most recent first.
pub fn users(steam: &Path) -> Vec<Value> {
    let text = std::fs::read_to_string(steam.join("config/loginusers.vdf")).unwrap_or_default();
    let named = text_vdf(&text);
    let mut v: Vec<Value> = std::fs::read_dir(steam.join("userdata"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let id = e.file_name().to_string_lossy().into_owned();
            if !id.chars().all(|c| c.is_ascii_digit()) || id == "0" {
                return None;
            }
            let u = named.iter().find(|(k, _)| account_id(k).map(|a| a.to_string()) == Some(id.clone()));
            let field = |f: &str| u.and_then(|(_, m)| m.iter().find(|(k, _)| k.eq_ignore_ascii_case(f)).map(|(_, v)| v.clone())).unwrap_or_default();
            Some(json!({
                "id": id, "name": field("PersonaName"), "account": field("AccountName"), "recent": field("MostRecent") == "1",
                "signed_in": field("Timestamp").parse::<u64>().unwrap_or(0),
                "shortcuts": e.path().join("config/shortcuts.vdf").is_file(),
            }))
        })
        .collect();
    // Steam no longer always writes MostRecent: the latest sign-in is the recent one then
    if !v.iter().any(|u| u["recent"] == true) {
        if let Some(u) = v.iter_mut().filter(|u| u["signed_in"].as_u64() > Some(0)).max_by_key(|u| u["signed_in"].as_u64()) {
            u["recent"] = json!(true);
        }
    }
    v.sort_by_key(|u| (u["recent"] != true, std::cmp::Reverse(u["signed_in"].as_u64().unwrap_or(0)), u["name"].as_str().unwrap_or("").to_lowercase()));
    v
}

/// The users block of a text VDF (`"users" { "<id>" { "Key" "Value" … } }`): id → fields.
fn text_vdf(text: &str) -> Vec<(String, Vec<(String, String)>)> {
    let mut tokens = vec![];
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                let mut s = String::new();
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => {
                            if let Some(n) = chars.next() {
                                s.push(n);
                            }
                        }
                        '"' => break,
                        _ => s.push(c),
                    }
                }
                tokens.push(s);
            }
            '{' => tokens.push("{".into()),
            '}' => tokens.push("}".into()),
            _ => {}
        }
    }
    let mut out = vec![];
    let mut i = 0;
    let mut depth = 0;
    while i < tokens.len() {
        match tokens[i].as_str() {
            "{" => depth += 1,
            "}" => depth -= 1,
            k if depth == 1 && tokens.get(i + 1).map(String::as_str) == Some("{") => {
                let mut fields = vec![];
                let mut j = i + 2;
                while j + 1 < tokens.len() && tokens[j] != "}" {
                    if tokens[j + 1] == "{" {
                        // a block inside: skip it
                        let mut d = 0;
                        while j < tokens.len() {
                            if tokens[j] == "{" {
                                d += 1;
                            } else if tokens[j] == "}" {
                                d -= 1;
                                if d == 0 {
                                    break;
                                }
                            }
                            j += 1;
                        }
                        j += 1;
                        continue;
                    }
                    fields.push((tokens[j].clone(), tokens[j + 1].clone()));
                    j += 2;
                }
                out.push((k.to_string(), fields));
                i = j + 1;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    out
}

fn config_dir(steam: &Path, user: &str) -> Result<PathBuf> {
    if user.is_empty() || !user.chars().all(|c| c.is_ascii_digit()) {
        bail!("Which Steam user?");
    }
    let d = steam.join("userdata").join(user).join("config");
    if !d.parent().is_some_and(Path::is_dir) {
        bail!("Steam has no user {user} on this computer.");
    }
    Ok(d)
}

/// What the app last wrote for a user (kept on this computer): the shortcuts' ids and the
/// grid files.
fn record_path(data_dir: &Path, user: &str) -> PathBuf {
    data_dir.join("steam").join(format!("{user}.json"))
}

/// What writing the shortcuts would do: those added, changed and removed, and the others kept.
pub fn preview(lib: &Library, steam: &Path, user: &str) -> Result<Value> {
    let dir = config_dir(steam, user)?;
    let current = match std::fs::read(dir.join("shortcuts.vdf")) {
        Ok(b) => shortcuts_of(&parse(&b).context("shortcuts.vdf can't be read")?),
        Err(_) => vec![],
    };
    let (mine, without) = shortcuts(lib);
    let ours: Vec<&Vdf> = current.iter().filter(|v| is_ours(v)).collect();
    let others = current.len() - ours.len();
    let ids_now: HashSet<u32> = ours.iter().filter_map(|v| v.int("appid")).collect();
    let mut add = vec![];
    let mut same = vec![];
    for (s, media) in &mine {
        let row = json!({ "game": s.game, "name": s.name, "exe": s.exe.trim_matches('"'), "appid": s.appid, "art": media.keys().collect::<Vec<_>>() });
        if ids_now.contains(&s.appid) {
            same.push(row);
        } else {
            add.push(row);
        }
    }
    let keep: HashSet<u32> = mine.iter().map(|(s, _)| s.appid).collect();
    let remove: Vec<Value> = ours.iter().filter(|v| v.int("appid").is_some_and(|a| !keep.contains(&a))).map(|v| json!({ "name": v.str("AppName"), "appid": v.int("appid") })).collect();
    Ok(json!({
        "steam": steam.display().to_string(), "user": user, "add": add, "update": same, "remove": remove, "others": others,
        "without_program": without, "running": running(),
    }))
}

/// Write the app's shortcuts for a user (D28): only while Steam is closed (`steam_running`
/// says), shortcuts.vdf backed up first, every shortcut that isn't the app's kept, the
/// artwork copied to the grid folder.
pub fn write_shortcuts(lib: &Library, data_dir: &Path, steam: &Path, user: &str, steam_running: bool) -> Result<Value> {
    if steam_running {
        bail!("Steam is running: close it (Steam › Exit), then add the shortcuts.");
    }
    let dir = config_dir(steam, user)?;
    std::fs::create_dir_all(&dir)?;
    let file = dir.join("shortcuts.vdf");
    let old = std::fs::read(&file).ok();
    let top = match &old {
        Some(b) => parse(b).context("shortcuts.vdf can't be read, so it's left as it is")?,
        None => vec![],
    };
    // the backup (or a note that there was no file)
    let stamp = now().replace([':', '-'], "").replace('T', "-").replace('Z', "");
    let backup = dir.join(format!("shortcuts.vdf.gamerepo-{stamp}"));
    std::fs::write(&backup, old.as_deref().unwrap_or(b""))?;
    prune_backups(&dir);
    let (mine, _) = shortcuts(lib);
    let mut list: Vec<Vdf> = shortcuts_of(&top).into_iter().filter(|v| !is_ours(v)).collect();
    let others = list.len();
    let mut seen = HashSet::new();
    for (s, _) in &mine {
        if seen.insert(s.appid) {
            list.push(s.to_vdf());
        }
    }
    let mut new_top: Vec<(Vec<u8>, Vdf)> = top.into_iter().filter(|(k, _)| !k.eq_ignore_ascii_case(b"shortcuts")).collect();
    new_top.insert(0, (b"shortcuts".to_vec(), Vdf::Map(list.into_iter().enumerate().map(|(i, v)| (i.to_string().into_bytes(), v)).collect())));
    crate::config::write_atomic(&file, &write(&new_top))?;
    // the artwork: what the app wrote before goes, then this time's
    let grid = dir.join("grid");
    let rec_path = record_path(data_dir, user);
    let rec = crate::config::read_json_object(&rec_path);
    for f in rec["grid"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        let _ = std::fs::remove_file(grid.join(f));
    }
    let mut written = vec![];
    for (s, media) in &mine {
        for (kind, suffix) in [("cover", "p"), ("banner", ""), ("hero", "_hero"), ("logo", "_logo"), ("icon", "_icon")] {
            let Some(src) = media.get(kind) else { continue };
            let ext = src.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_else(|| "png".into());
            let name = format!("{}{suffix}.{ext}", s.appid);
            std::fs::create_dir_all(&grid)?;
            if std::fs::copy(src, grid.join(&name)).is_ok() {
                written.push(name);
            }
        }
    }
    crate::config::write_json(
        &rec_path,
        &json!({ "steam": steam.display().to_string(), "user": user, "written": now(), "backup": backup.display().to_string(),
                 "appids": mine.iter().map(|(s, _)| s.appid).collect::<Vec<_>>(), "grid": written }),
    )?;
    Ok(json!({ "shortcuts": mine.len(), "others": others, "art": written.len(), "backup": backup.display().to_string() }))
}

fn prune_backups(dir: &Path) {
    let mut b: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("shortcuts.vdf.gamerepo-")))
        .collect();
    b.sort();
    while b.len() > BACKUPS {
        let _ = std::fs::remove_file(b.remove(0));
    }
}

/// When the app last wrote a user's shortcuts.
pub fn last_written(data_dir: &Path, user: &str) -> Value {
    let rec = crate::config::read_json_object(&record_path(data_dir, user));
    if rec.get("written").is_none() {
        return Value::Null;
    }
    json!({ "written": rec["written"], "shortcuts": rec["appids"].as_array().map_or(0, Vec::len), "backup": rec["backup"] })
}

/// Take the app's shortcuts (and the artwork it added) out of Steam, keeping every
/// other shortcut as it is now; shortcuts.vdf is backed up first, as for a write.
pub fn remove_shortcuts(data_dir: &Path, steam: &Path, user: &str, steam_running: bool) -> Result<Value> {
    if steam_running {
        bail!("Steam is running: close it (Steam › Exit) first.");
    }
    let dir = config_dir(steam, user)?;
    let file = dir.join("shortcuts.vdf");
    let mut removed = 0;
    if let Ok(old) = std::fs::read(&file) {
        let top = parse(&old).context("shortcuts.vdf can't be read, so it's left as it is")?;
        let stamp = now().replace([':', '-'], "").replace('T', "-").replace('Z', "");
        std::fs::write(dir.join(format!("shortcuts.vdf.gamerepo-{stamp}")), &old)?;
        prune_backups(&dir);
        let all = shortcuts_of(&top);
        let kept: Vec<Vdf> = all.iter().filter(|v| !is_ours(v)).cloned().collect();
        removed = all.len() - kept.len();
        let mut new_top: Vec<(Vec<u8>, Vdf)> = top.into_iter().filter(|(k, _)| !k.eq_ignore_ascii_case(b"shortcuts")).collect();
        new_top.insert(0, (b"shortcuts".to_vec(), Vdf::Map(kept.into_iter().enumerate().map(|(i, v)| (i.to_string().into_bytes(), v)).collect())));
        crate::config::write_atomic(&file, &write(&new_top))?;
    }
    let rec_path = record_path(data_dir, user);
    let rec = crate::config::read_json_object(&rec_path);
    let grid = dir.join("grid");
    let mut art = 0;
    for f in rec["grid"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        if std::fs::remove_file(grid.join(f)).is_ok() {
            art += 1;
        }
    }
    let _ = std::fs::remove_file(&rec_path);
    Ok(json!({ "removed": removed, "art_removed": art }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::tests::temp_dir;

    #[test]
    fn a_vdf_file_reads_and_writes_back_the_same() {
        let other = Vdf::Map(vec![
            (b"appid".to_vec(), Vdf::Int(0x9234_5678)),
            (b"AppName".to_vec(), Vdf::Str("Someone else's é".as_bytes().to_vec())),
            (b"Exe".to_vec(), Vdf::Str(b"\"C:\\x.exe\"".to_vec())),
            (b"LastPlayTime".to_vec(), Vdf::Int(1_700_000_000)),
            (b"weird".to_vec(), Vdf::Raw(0x07, vec![1, 2, 3, 4, 5, 6, 7, 8])),
            (b"tags".to_vec(), Vdf::Map(vec![(b"0".to_vec(), Vdf::Str(b"Favorites".to_vec()))])),
        ]);
        let top = vec![(b"shortcuts".to_vec(), Vdf::Map(vec![(b"0".to_vec(), other.clone())]))];
        let bytes = write(&top);
        assert_eq!(bytes[0], 0x00);
        assert_eq!(&bytes[1..11], b"shortcuts\0");
        assert_eq!(&bytes[bytes.len() - 2..], &[0x08, 0x08]);
        assert_eq!(parse(&bytes).unwrap(), top);
        assert_eq!(other.str("appname").as_deref(), Some("Someone else's é"));
        assert!(!is_ours(&other));
        // Steam's rule for a shortcut's id
        assert_eq!(shortcut_id("\"C:\\Games\\game.exe\"", "Game") & 0x8000_0000, 0x8000_0000);
        assert_eq!(account_id("76561197960287930"), Some(22202));
    }

    #[test]
    fn shortcuts_are_written_beside_the_others_backed_up_and_put_back() {
        let dir = temp_dir("steam-write");
        let lib = Library::open(dir.join("lib")).unwrap();
        // a PC game with a program and a cover
        let game_dir = pcgames::folder(&lib).join("Zeta Quest (1999)");
        std::fs::create_dir_all(game_dir.join("extracted")).unwrap();
        std::fs::write(game_dir.join("extracted/zeta.exe"), b"exe").unwrap();
        let g = pcgames::PcGame {
            id: "pc-1".into(), title: "Zeta Quest".into(), year: Some(1999),
            launch: Some(pcgames::Launch { program: "extracted/zeta.exe".into(), ..Default::default() }), ..Default::default()
        };
        crate::config::write_json(&game_dir.join("game.json"), &serde_json::to_value(&g).unwrap()).unwrap();
        pcgames::save_media(&lib, "pc-1", "cover", "jpg", b"jpeg").unwrap();
        // Steam with one user and a shortcut of their own
        let steam = dir.join("Steam");
        let cfg = steam.join("userdata/22202/config");
        std::fs::create_dir_all(&cfg).unwrap();
        std::fs::create_dir_all(steam.join("config")).unwrap();
        std::fs::write(steam.join("config/loginusers.vdf"), "\"users\"\n{\n\t\"76561197960287930\"\n\t{\n\t\t\"AccountName\"\t\t\"jerred\"\n\t\t\"PersonaName\"\t\t\"Jerred\"\n\t\t\"MostRecent\"\t\t\"1\"\n\t}\n}\n").unwrap();
        let theirs = Vdf::Map(vec![(b"appid".to_vec(), Vdf::Int(0x9000_0001)), (b"AppName".to_vec(), Vdf::Str(b"Theirs".to_vec()))]);
        let before = write(&[(b"shortcuts".to_vec(), Vdf::Map(vec![(b"0".to_vec(), theirs.clone())]))]);
        std::fs::write(cfg.join("shortcuts.vdf"), &before).unwrap();
        let u = users(&steam);
        assert_eq!((u[0]["id"].as_str(), u[0]["name"].as_str(), u[0]["recent"].as_bool()), (Some("22202"), Some("Jerred"), Some(true)));
        let p = preview(&lib, &steam, "22202").unwrap();
        assert_eq!((p["add"].as_array().unwrap().len(), p["others"].as_u64()), (1, Some(1)));
        assert!(write_shortcuts(&lib, &dir.join("data"), &steam, "22202", true).is_err(), "not while Steam runs");
        let r = write_shortcuts(&lib, &dir.join("data"), &steam, "22202", false).unwrap();
        assert_eq!((r["shortcuts"].as_u64(), r["others"].as_u64(), r["art"].as_u64()), (Some(1), Some(1), Some(1)));
        let list = shortcuts_of(&parse(&std::fs::read(cfg.join("shortcuts.vdf")).unwrap()).unwrap());
        assert_eq!(list.len(), 2);
        assert_eq!(list[0], theirs, "the user's own shortcut is kept as it was");
        let mine = &list[1];
        assert!(is_ours(mine));
        let exe = mine.str("Exe").unwrap();
        assert!(exe.starts_with('"') && exe.ends_with("zeta.exe\""));
        let appid = mine.int("appid").unwrap();
        assert_eq!(appid, shortcut_id(&exe, "Zeta Quest"));
        assert!(cfg.join(format!("grid/{appid}p.jpg")).is_file());
        // again: the same shortcut, not a second one
        write_shortcuts(&lib, &dir.join("data"), &steam, "22202", false).unwrap();
        let p = preview(&lib, &steam, "22202").unwrap();
        assert_eq!((p["add"].as_array().unwrap().len(), p["update"].as_array().unwrap().len()), (0, 1));
        // taken out: the user's own shortcut stays, the artwork goes
        let r = remove_shortcuts(&dir.join("data"), &steam, "22202", false).unwrap();
        assert_eq!(r["removed"].as_u64(), Some(1));
        let back = shortcuts_of(&parse(&std::fs::read(cfg.join("shortcuts.vdf")).unwrap()).unwrap());
        assert_eq!(back, [theirs]);
        assert!(!cfg.join(format!("grid/{appid}p.jpg")).exists());
        let backups = std::fs::read_dir(&cfg).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with("shortcuts.vdf.gamerepo-")).count();
        assert!(backups >= 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
