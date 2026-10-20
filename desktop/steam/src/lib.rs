//! Steam's own files on a computer (D28, D109): the non-Steam games a user has
//! (`userdata/<id>/config/shortcuts.vdf`, binary VDF), the Proton chosen for each
//! (`config/config.vdf`, text VDF), the users signed in, and whether Steam is running.
//! Steam rewrites these files itself, so they're only written while it's closed, backed up
//! first; only the shortcuts with a program's own tag are that program's, and every other
//! is kept as it was. Game Repo's Steam export and Game Repo Go share this.

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

mod text;
pub use text::{parse_text, write_text, Text};

// ------------------------------------------------------------ binary VDF

/// A value in a binary VDF file. Strings are kept as bytes, so shortcuts that aren't a
/// program's own are written back exactly as they were.
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
/// it, in quotes) and its name, with the top bit set. The grid's artwork and the Proton
/// chosen for it are named by it.
pub fn shortcut_id(exe: &str, name: &str) -> u32 {
    let mut h = crc32fast::Hasher::new();
    h.update(exe.as_bytes());
    h.update(name.as_bytes());
    h.finalize() | 0x8000_0000
}

/// A path in quotes, as Steam writes a shortcut's program and folder.
pub fn quoted(p: &str) -> String {
    format!("\"{}\"", p.trim_matches('"'))
}

/// A non-Steam shortcut.
#[derive(Clone, Debug, PartialEq)]
pub struct Shortcut {
    pub name: String,
    /// The program, in quotes as Steam writes it
    pub exe: String,
    /// Its folder, in quotes
    pub start_dir: String,
    pub args: String,
    pub icon: String,
    pub appid: u32,
}

impl Shortcut {
    /// A shortcut to `program`, started in `start_dir`, its id worked out as Steam does.
    pub fn new(name: &str, program: &Path, start_dir: &Path, args: &str, icon: &str) -> Shortcut {
        let exe = quoted(&program.display().to_string());
        Shortcut {
            appid: shortcut_id(&exe, name),
            name: name.to_string(),
            exe,
            start_dir: quoted(&start_dir.display().to_string()),
            args: args.to_string(),
            icon: icon.to_string(),
        }
    }

    /// As shortcuts.vdf has it, with the program's tag.
    pub fn to_vdf(&self, tag: &str) -> Vdf {
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
            (b"tags".to_vec(), Vdf::Map(vec![(b"0".to_vec(), Vdf::Str(tag.as_bytes().to_vec()))])),
        ])
    }
}

/// Whether a shortcut in the file has the tag (is that program's).
pub fn is_tagged(v: &Vdf, tag: &str) -> bool {
    match v.get("tags") {
        Some(Vdf::Map(tags)) => tags.iter().any(|(_, t)| matches!(t, Vdf::Str(s) if s == tag.as_bytes())),
        _ => false,
    }
}

/// The shortcuts in a shortcuts.vdf (its "shortcuts" map's values).
pub fn shortcuts_of(top: &[(Vec<u8>, Vdf)]) -> Vec<Vdf> {
    top.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(b"shortcuts"))
        .and_then(|(_, v)| match v {
            Vdf::Map(m) => Some(m.iter().map(|(_, v)| v.clone()).collect()),
            _ => None,
        })
        .unwrap_or_default()
}

/// The file with `list` as its shortcuts, numbered from 0, anything else in it kept.
pub fn with_shortcuts(top: Vec<(Vec<u8>, Vdf)>, list: Vec<Vdf>) -> Vec<(Vec<u8>, Vdf)> {
    let mut new_top: Vec<(Vec<u8>, Vdf)> = top.into_iter().filter(|(k, _)| !k.eq_ignore_ascii_case(b"shortcuts")).collect();
    new_top.insert(0, (b"shortcuts".to_vec(), Vdf::Map(list.into_iter().enumerate().map(|(i, v)| (i.to_string().into_bytes(), v)).collect())));
    new_top
}

// ------------------------------------------------------------ Steam on this computer

fn home_dir() -> PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(windows)]
fn command(program: &str) -> std::process::Command {
    use std::os::windows::process::CommandExt;
    let mut c = std::process::Command::new(program);
    c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    c
}

/// Steam's folder: the one given, else where Steam says (Windows' registry), else the
/// usual places (on Linux its own, the `~/.steam/steam` link, and Flathub's).
pub fn find(set: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = set.filter(|p| p.join("userdata").is_dir() || p.join("steam.exe").is_file()) {
        return Some(p.to_path_buf());
    }
    let mut tries: Vec<PathBuf> = vec![];
    #[cfg(windows)]
    {
        if let Ok(o) = command("reg").args(["query", r"HKCU\Software\Valve\Steam", "/v", "SteamPath"]).output() {
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
        let home = home_dir();
        tries.push(home.join(".steam/steam"));
        tries.push(home.join(".local/share/Steam"));
        tries.push(home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"));
    }
    tries.into_iter().find(|p| p.join("userdata").is_dir())
}

/// Whether Steam is running (its files can't be written while it is).
pub fn running() -> bool {
    #[cfg(windows)]
    {
        command("tasklist")
            .args(["/FI", "IMAGENAME eq steam.exe", "/NH"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_lowercase().contains("steam.exe"))
            .unwrap_or(false)
    }
    #[cfg(target_os = "linux")]
    {
        // a process named steam that hasn't ended (one ended but not yet reaped doesn't count)
        std::fs::read_dir("/proc").into_iter().flatten().flatten().any(|e| {
            let Ok(stat) = std::fs::read_to_string(e.path().join("stat")) else { return false };
            let (Some(open), Some(close)) = (stat.find('('), stat.rfind(')')) else { return false };
            &stat[open + 1..close] == "steam" && !stat[close + 1..].trim_start().starts_with(['Z', 'X'])
        })
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        std::process::Command::new("pgrep").args(["-x", "steam"]).output().map(|o| o.status.success()).unwrap_or(false)
    }
}

/// Steam's 32-bit account id from a SteamID64 (`userdata/<id>` is named by it).
pub fn account_id(steam64: &str) -> Option<u64> {
    steam64.parse::<u64>().ok()?.checked_sub(76_561_197_960_265_728)
}

/// The users Steam has signed in on this computer: `userdata/<id>/`, named from
/// `config/loginusers.vdf`, the most recent first.
pub fn users(steam: &Path) -> Vec<Value> {
    let text = std::fs::read_to_string(steam.join("config/loginusers.vdf")).unwrap_or_default();
    let top = parse_text(&text).unwrap_or_default();
    let named: Vec<(String, Text)> = match top.iter().find(|(k, _)| k.eq_ignore_ascii_case("users")) {
        Some((_, Text::Map(m))) => m.clone(),
        _ => vec![],
    };
    let mut v: Vec<Value> = std::fs::read_dir(steam.join("userdata"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let id = e.file_name().to_string_lossy().into_owned();
            if !id.chars().all(|c| c.is_ascii_digit()) || id == "0" {
                return None;
            }
            let u = named.iter().find(|(k, _)| account_id(k).map(|a| a.to_string()) == Some(id.clone())).map(|(_, v)| v);
            let field = |f: &str| u.and_then(|m| m.str(f)).unwrap_or_default();
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

/// A user's config folder (`userdata/<id>/config`).
pub fn config_dir(steam: &Path, user: &str) -> Result<PathBuf> {
    if user.is_empty() || !user.chars().all(|c| c.is_ascii_digit()) {
        bail!("Which Steam user?");
    }
    let d = steam.join("userdata").join(user).join("config");
    if !d.parent().is_some_and(Path::is_dir) {
        bail!("Steam has no user {user} on this computer.");
    }
    Ok(d)
}

/// Written whole or not at all: to a file beside it, then put in its place.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension(format!("{}.tmp", path.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default()));
    std::fs::write(&tmp, bytes).with_context(|| format!("{} can't be written", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("{} can't be written", path.display()))?;
    Ok(())
}

/// A copy of `file` as it is now (empty when there's none), `<name>.<prefix>-<stamp>` beside
/// it, keeping that prefix's newest `keep`.
pub fn backup(file: &Path, prefix: &str, stamp: &str, keep: usize) -> Result<PathBuf> {
    let dir = file.parent().ok_or_else(|| anyhow!("{} has no folder", file.display()))?;
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let start = format!("{name}.{prefix}-");
    let b = dir.join(format!("{start}{stamp}"));
    std::fs::create_dir_all(dir)?;
    std::fs::write(&b, std::fs::read(file).unwrap_or_default())?;
    let mut all: Vec<PathBuf> =
        std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(&start))).collect();
    all.sort();
    while all.len() > keep {
        let _ = std::fs::remove_file(all.remove(0));
    }
    Ok(b)
}

/// Put `tag`'s shortcuts in a user's shortcuts.vdf: those it had go, `list` comes, every
/// other shortcut stays as it was. Backed up first (`prefix`, `stamp`). Gives the number of
/// others kept and the backup.
pub fn put_shortcuts(steam: &Path, user: &str, tag: &str, list: &[Shortcut], prefix: &str, stamp: &str) -> Result<(usize, PathBuf)> {
    let dir = config_dir(steam, user)?;
    std::fs::create_dir_all(&dir)?;
    let file = dir.join("shortcuts.vdf");
    let top = match std::fs::read(&file) {
        Ok(b) => parse(&b).context("shortcuts.vdf can't be read, so it's left as it is")?,
        Err(_) => vec![],
    };
    let b = backup(&file, prefix, stamp, 5)?;
    let mut all: Vec<Vdf> = shortcuts_of(&top).into_iter().filter(|v| !is_tagged(v, tag)).collect();
    let others = all.len();
    let mut seen = std::collections::HashSet::new();
    for s in list {
        if seen.insert(s.appid) {
            all.push(s.to_vdf(tag));
        }
    }
    write_atomic(&file, &write(&with_shortcuts(top, all)))?;
    Ok((others, b))
}

/// The ids of `tag`'s shortcuts in a user's shortcuts.vdf.
pub fn tagged_ids(steam: &Path, user: &str, tag: &str) -> Vec<u32> {
    let Ok(dir) = config_dir(steam, user) else { return vec![] };
    let Ok(b) = std::fs::read(dir.join("shortcuts.vdf")) else { return vec![] };
    shortcuts_of(&parse(&b).unwrap_or_default()).iter().filter(|v| is_tagged(v, tag)).filter_map(|v| v.int("appid")).collect()
}

// ------------------------------------------------------------ Proton for a shortcut

const MAPPING: [&str; 5] = ["InstallConfigStore", "Software", "Valve", "Steam", "CompatToolMapping"];

/// The compatibility tool (a Proton) Steam uses for a game, from `config/config.vdf`.
pub fn compat_tool(steam: &Path, appid: u32) -> Option<String> {
    let top = parse_text(&std::fs::read_to_string(steam.join("config/config.vdf")).ok()?).ok()?;
    let mut at: &[(String, Text)] = &top;
    for k in MAPPING {
        match at.iter().find(|(n, _)| n.eq_ignore_ascii_case(k)) {
            Some((_, Text::Map(m))) => at = m,
            _ => return None,
        }
    }
    match at.iter().find(|(n, _)| *n == appid.to_string()) {
        Some((_, v)) => v.str("name").filter(|s| !s.is_empty()),
        None => None,
    }
}

/// Choose the compatibility tool for games in `config/config.vdf`: Some(tool) sets it
/// (`proton_experimental`, `proton_9`…), None takes the choice out. Everything else in the
/// file is kept. Backed up first, as shortcuts.vdf is.
pub fn put_compat_tools(steam: &Path, tools: &[(u32, Option<String>)], prefix: &str, stamp: &str) -> Result<()> {
    let file = steam.join("config/config.vdf");
    let text = std::fs::read_to_string(&file).unwrap_or_default();
    let mut top = if text.trim().is_empty() { vec![] } else { parse_text(&text).context("Steam's config.vdf can't be read, so it's left as it is")? };
    let mut at: &mut Vec<(String, Text)> = &mut top;
    for k in MAPPING {
        let i = match at.iter().position(|(n, _)| n.eq_ignore_ascii_case(k)) {
            Some(i) => i,
            None => {
                at.push((k.to_string(), Text::Map(vec![])));
                at.len() - 1
            }
        };
        if !matches!(at[i].1, Text::Map(_)) {
            at[i].1 = Text::Map(vec![]);
        }
        at = match &mut at[i].1 {
            Text::Map(m) => m,
            Text::Str(_) => unreachable!(),
        };
    }
    for (appid, tool) in tools {
        let id = appid.to_string();
        at.retain(|(n, _)| *n != id);
        if let Some(t) = tool {
            let s = |k: &str, v: &str| (k.to_string(), Text::Str(v.to_string()));
            at.push((id, Text::Map(vec![s("name", t), s("config", ""), s("priority", "250")])));
        }
    }
    backup(&file, prefix, stamp, 5)?;
    write_atomic(&file, write_text(&top).as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("gamerepo-steam-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

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
        assert!(!is_tagged(&other, "Game Repo"));
        // Steam's rule for a shortcut's id
        assert_eq!(shortcut_id("\"C:\\Games\\game.exe\"", "Game") & 0x8000_0000, 0x8000_0000);
        assert_eq!(account_id("76561197960287930"), Some(22202));
    }

    #[test]
    fn a_programs_shortcuts_go_beside_the_others_and_its_proton_with_them() {
        let dir = temp_dir("put");
        let steam = dir.join("Steam");
        let cfg = steam.join("userdata/22202/config");
        std::fs::create_dir_all(&cfg).unwrap();
        std::fs::create_dir_all(steam.join("config")).unwrap();
        std::fs::write(
            steam.join("config/loginusers.vdf"),
            "\"users\"\n{\n\t\"76561197960287930\"\n\t{\n\t\t\"AccountName\"\t\t\"jerred\"\n\t\t\"PersonaName\"\t\t\"Jerred\"\n\t\t\"MostRecent\"\t\t\"1\"\n\t}\n}\n",
        )
        .unwrap();
        let u = users(&steam);
        assert_eq!((u[0]["id"].as_str(), u[0]["name"].as_str(), u[0]["recent"].as_bool()), (Some("22202"), Some("Jerred"), Some(true)));
        let theirs = Vdf::Map(vec![(b"appid".to_vec(), Vdf::Int(0x9000_0001)), (b"AppName".to_vec(), Vdf::Str(b"Theirs".to_vec()))]);
        std::fs::write(cfg.join("shortcuts.vdf"), write(&[(b"shortcuts".to_vec(), Vdf::Map(vec![(b"0".to_vec(), theirs.clone())]))])).unwrap();
        let s = Shortcut::new("Zeta Quest", Path::new("/home/deck/Games/Zeta Quest (1999)/zeta.exe"), Path::new("/home/deck/Games/Zeta Quest (1999)"), "", "");
        assert_eq!(s.appid, shortcut_id("\"/home/deck/Games/Zeta Quest (1999)/zeta.exe\"", "Zeta Quest"));
        let (others, b) = put_shortcuts(&steam, "22202", "Game Repo Go", &[s.clone()], "gamerepo-go", "20261020-120000").unwrap();
        assert_eq!(others, 1);
        assert!(b.is_file());
        // twice: still one of ours
        put_shortcuts(&steam, "22202", "Game Repo Go", &[s.clone()], "gamerepo-go", "20261020-120001").unwrap();
        let list = shortcuts_of(&parse(&std::fs::read(cfg.join("shortcuts.vdf")).unwrap()).unwrap());
        assert_eq!(list.len(), 2);
        assert_eq!(list[0], theirs, "the user's own shortcut is kept as it was");
        assert!(is_tagged(&list[1], "Game Repo Go") && !is_tagged(&list[1], "Game Repo"));
        assert_eq!(tagged_ids(&steam, "22202", "Game Repo Go"), [s.appid]);
        // a tag's shortcuts are only its own: Game Repo's export leaves Game Repo Go's
        put_shortcuts(&steam, "22202", "Game Repo", &[], "gamerepo", "20261020-120002").unwrap();
        assert_eq!(tagged_ids(&steam, "22202", "Game Repo Go"), [s.appid]);

        // Proton, in a config.vdf with plenty else in it
        let config = "\"InstallConfigStore\"\n{\n\t\"Software\"\n\t{\n\t\t\"Valve\"\n\t\t{\n\t\t\t\"Steam\"\n\t\t\t{\n\t\t\t\t\"BaseInstallFolder_1\"\t\t\"/run/media/deck/SD\"\n\t\t\t\t\"CompatToolMapping\"\n\t\t\t\t{\n\t\t\t\t\t\"1234\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"name\"\t\t\"proton_9\"\n\t\t\t\t\t\t\"config\"\t\t\"\"\n\t\t\t\t\t\t\"priority\"\t\t\"250\"\n\t\t\t\t\t}\n\t\t\t\t}\n\t\t\t}\n\t\t}\n\t}\n\t\"Music\"\n\t{\n\t\t\"CrawlSteamInstallFolders\"\t\t\"1\"\n\t}\n}\n";
        std::fs::write(steam.join("config/config.vdf"), config).unwrap();
        put_compat_tools(&steam, &[(s.appid, Some("proton_experimental".into()))], "gamerepo-go", "20261020-120003").unwrap();
        assert_eq!(compat_tool(&steam, s.appid).as_deref(), Some("proton_experimental"));
        assert_eq!(compat_tool(&steam, 1234).as_deref(), Some("proton_9"), "another game's choice is kept");
        let text = std::fs::read_to_string(steam.join("config/config.vdf")).unwrap();
        assert!(text.contains("\"BaseInstallFolder_1\"\t\t\"/run/media/deck/SD\"") && text.contains("\"CrawlSteamInstallFolders\"\t\t\"1\""));
        put_compat_tools(&steam, &[(s.appid, None)], "gamerepo-go", "20261020-120004").unwrap();
        assert_eq!(compat_tool(&steam, s.appid), None);
        assert_eq!(std::fs::read_to_string(steam.join("config/config.vdf")).unwrap(), config, "taken out again, the file is as Steam wrote it");
        // a Steam that has no config.vdf yet
        std::fs::remove_file(steam.join("config/config.vdf")).unwrap();
        put_compat_tools(&steam, &[(s.appid, Some("proton_experimental".into()))], "gamerepo-go", "20261020-120005").unwrap();
        assert_eq!(compat_tool(&steam, s.appid).as_deref(), Some("proton_experimental"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
