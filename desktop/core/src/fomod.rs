//! FOMOD installers (docs/PLAN.md, "Mods on installed games", D211): a mod with
//! `fomod/ModuleConfig.xml` has its install steps, groups and options read, its choices
//! asked once (by the page, one step at a time) and kept with the mod, and the files to put
//! in worked out from them: each a file or folder in the mod and where it goes in the
//! game's folder, in the order a later one wins. `fomod/info.xml` fills the mod's details.
//!
//! The format is the one Nexus Mods' installers (FOMM, Vortex, Mod Organizer 2) read:
//! steps (`installSteps`) shown when their `visible` conditions hold, each with groups of
//! options (`plugins`) to pick one, at most one, at least one, all, or any of; an option's
//! type (*Required*, *Recommended*, *Optional*, *Not usable*, *Could be usable*), fixed or
//! depending on the flags earlier options set and on files in the game's folder; files
//! always put in (`requiredInstallFiles`) and files put in when conditions hold
//! (`conditionalFileInstalls`). Game Repo never runs anything a FOMOD names (C# scripts,
//! `script.cs`, aren't read). A condition on another mod's plugin being active can't be
//! looked up (Game Repo doesn't read a game's load order), so it's asked as a question.

use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// An element of the XML: its name (lower case), attributes (names as written), children
/// and text.
#[derive(Clone, Debug, Default)]
struct El {
    name: String,
    attrs: Vec<(String, String)>,
    children: Vec<El>,
    text: String,
}

impl El {
    fn attr(&self, k: &str) -> Option<&str> {
        self.attrs.iter().find(|(n, _)| n.eq_ignore_ascii_case(k)).map(|(_, v)| v.as_str())
    }
    fn child(&self, n: &str) -> Option<&El> {
        self.children.iter().find(|c| c.name == n)
    }
    fn all<'a>(&'a self, n: &'a str) -> impl Iterator<Item = &'a El> + 'a {
        self.children.iter().filter(move |c| c.name == n)
    }
}

/// The text of a FOMOD file: UTF-16 (with its byte order mark, as most are written) or
/// UTF-8, its XML declaration left out (it may say UTF-16 of text that's now UTF-8).
pub fn text_of(bytes: &[u8]) -> String {
    let s = if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        let le = bytes[0] == 0xFF;
        let units: Vec<u16> = bytes[2..].chunks_exact(2).map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) }).collect();
        String::from_utf16_lossy(&units)
    } else if bytes.len() > 4 && bytes[0] == b'<' && bytes[1] == 0 {
        // UTF-16 LE without its mark
        let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes)).into_owned()
    };
    let s = s.trim_start_matches('\u{feff}').trim_start();
    match s.strip_prefix("<?xml").and_then(|r| r.find("?>").map(|i| &r[i + 2..])) {
        Some(rest) => rest.to_string(),
        None => s.to_string(),
    }
}

fn parse_xml(text: &str) -> Result<El> {
    use quick_xml::events::Event;
    let mut r = quick_xml::Reader::from_str(text);
    let mut stack: Vec<El> = vec![El { name: "#root".into(), ..Default::default() }];
    let attrs = |e: &quick_xml::events::BytesStart| -> Vec<(String, String)> {
        e.attributes()
            .flatten()
            .map(|a| {
                let k = a.key.local_name().as_ref().to_string();
                let v = a.normalized_value(quick_xml::XmlVersion::Implicit1_0).map(|v| v.into_owned()).unwrap_or_else(|_| a.value.to_string());
                (k, v)
            })
            .collect()
    };
    loop {
        match r.read_event().map_err(|e| anyhow!("its ModuleConfig.xml can't be read at byte {}: {e}", r.buffer_position()))? {
            Event::Start(e) => stack.push(El { name: e.local_name().as_ref().to_lowercase(), attrs: attrs(&e), ..Default::default() }),
            Event::Empty(e) => {
                let el = El { name: e.local_name().as_ref().to_lowercase(), attrs: attrs(&e), ..Default::default() };
                stack.last_mut().unwrap().children.push(el);
            }
            Event::Text(t) => {
                let s = t.xml10_content().into_owned();
                stack.last_mut().unwrap().text.push_str(&s);
            }
            Event::CData(t) => stack.last_mut().unwrap().text.push_str(&t.into_inner()),
            Event::GeneralRef(g) => {
                let name = g.xml10_content().into_owned();
                let ch = match name.as_str() {
                    "amp" => "&".to_string(),
                    "lt" => "<".into(),
                    "gt" => ">".into(),
                    "quot" => "\"".into(),
                    "apos" => "'".into(),
                    n => n.strip_prefix("#x").and_then(|h| u32::from_str_radix(h, 16).ok()).or_else(|| n.strip_prefix('#').and_then(|d| d.parse().ok())).and_then(char::from_u32).map(String::from).unwrap_or_default(),
                };
                stack.last_mut().unwrap().text.push_str(&ch);
            }
            Event::End(_) => {
                if stack.len() > 1 {
                    let el = stack.pop().unwrap();
                    stack.last_mut().unwrap().children.push(el);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    while stack.len() > 1 {
        let el = stack.pop().unwrap();
        stack.last_mut().unwrap().children.push(el);
    }
    Ok(stack.pop().unwrap())
}

/// A condition: on flags set by options picked, on a file in the game's folder, or on
/// the game's or the mod manager's version (taken as met).
#[derive(Clone, Debug, PartialEq)]
pub enum Dep {
    All(bool, Vec<Dep>),
    File(String, String),
    Flag(String, String),
    Met,
}

fn dep_of(e: &El) -> Dep {
    match e.name.as_str() {
        "filedependency" => Dep::File(norm(e.attr("file").unwrap_or("")), e.attr("state").unwrap_or("Active").to_string()),
        "flagdependency" => Dep::Flag(e.attr("flag").unwrap_or("").to_string(), e.attr("value").unwrap_or("").to_string()),
        "gamedependency" | "fommdependency" | "fomodversiondependency" => Dep::Met,
        // `dependencies`, `moduleDependencies`, `visible`: their conditions, And unless Or
        _ => Dep::All(!e.attr("operator").is_some_and(|o| o.eq_ignore_ascii_case("or")), e.children.iter().map(dep_of).collect()),
    }
}

/// A file or folder of the mod to put in: where it comes from (in the mod, `/`) and where
/// it goes (in the game's folder, `/`; empty: the top), in order of `priority`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Put {
    pub from: String,
    pub to: String,
    pub folder: bool,
    #[serde(skip_serializing_if = "is_zero")]
    pub priority: i64,
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

/// A path as a FOMOD writes it (`\`, any case, `.\` in front) with `/`.
fn norm(p: &str) -> String {
    p.replace('\\', "/").split('/').filter(|x| !x.is_empty() && *x != "." && *x != "..").collect::<Vec<_>>().join("/")
}

fn puts_of(e: Option<&El>) -> Vec<Put> {
    let Some(e) = e else { return vec![] };
    e.children
        .iter()
        .filter(|c| c.name == "file" || c.name == "folder")
        .map(|c| {
            let folder = c.name == "folder";
            let from = norm(c.attr("source").unwrap_or(""));
            let to = match c.attr("destination") {
                None => from.clone(),
                // a file's destination is its path in the game's folder; one that's a folder
                // (empty, or ending in a slash) takes the file's own name
                Some(d) if !folder && (d.trim().is_empty() || d.ends_with(['/', '\\'])) => {
                    let name = from.rsplit('/').next().unwrap_or("").to_string();
                    [norm(d), name].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join("/")
                }
                Some(d) => norm(d),
            };
            Put { from, to, folder, priority: c.attr("priority").and_then(|p| p.trim().parse().ok()).unwrap_or(0) }
        })
        .collect()
}

/// An option's type, fixed or by conditions.
#[derive(Clone, Debug, PartialEq)]
enum Kind {
    Fixed(String),
    By { default: String, patterns: Vec<(Dep, String)> },
}

#[derive(Clone, Debug)]
struct Plugin {
    name: String,
    description: String,
    image: String,
    puts: Vec<Put>,
    flags: Vec<(String, String)>,
    kind: Kind,
}

#[derive(Clone, Debug)]
struct Group {
    name: String,
    /// SelectExactlyOne | SelectAtMostOne | SelectAtLeastOne | SelectAll | SelectAny
    kind: String,
    plugins: Vec<Plugin>,
}

#[derive(Clone, Debug)]
struct Step {
    name: String,
    visible: Option<Dep>,
    groups: Vec<Group>,
}

/// A FOMOD as read.
#[derive(Clone, Debug)]
pub struct Fomod {
    pub name: String,
    pub image: String,
    needs: Option<Dep>,
    required: Vec<Put>,
    steps: Vec<Step>,
    conditional: Vec<(Dep, Vec<Put>)>,
}

/// `order`: Explicit keeps them as written; Ascending (the default) or Descending sorts by name.
fn ordered<T>(mut v: Vec<T>, order: Option<&str>, name: impl Fn(&T) -> String) -> Vec<T> {
    match order.map(str::to_lowercase).as_deref() {
        Some("explicit") => v,
        Some("descending") => {
            v.sort_by_key(|x| std::cmp::Reverse(name(x).to_lowercase()));
            v
        }
        _ => {
            v.sort_by_key(|x| name(x).to_lowercase());
            v
        }
    }
}

fn type_name(n: &str) -> String {
    match n.to_lowercase().as_str() {
        "required" => "Required",
        "recommended" => "Recommended",
        "notusable" => "NotUsable",
        "couldbeusable" => "CouldBeUsable",
        _ => "Optional",
    }
    .into()
}

/// `ModuleConfig.xml` read.
pub fn parse(bytes: &[u8]) -> Result<Fomod> {
    let root = parse_xml(&text_of(bytes))?;
    let c = root.child("config").ok_or_else(|| anyhow!("its ModuleConfig.xml has no <config>."))?;
    let steps = c
        .child("installsteps")
        .map(|s| {
            let list: Vec<Step> = s
                .all("installstep")
                .map(|st| Step {
                    name: st.attr("name").unwrap_or("").to_string(),
                    visible: st.child("visible").map(dep_of),
                    groups: st
                        .child("optionalfilegroups")
                        .map(|gs| {
                            let groups: Vec<Group> = gs
                                .all("group")
                                .map(|g| {
                                    let plugins = g
                                        .child("plugins")
                                        .map(|ps| {
                                            let list: Vec<Plugin> = ps
                                                .all("plugin")
                                                .map(|p| {
                                                    let td = p.child("typedescriptor");
                                                    let kind = match td.and_then(|t| t.child("dependencytype")) {
                                                        Some(dt) => Kind::By {
                                                            default: type_name(dt.child("defaulttype").and_then(|d| d.attr("name")).unwrap_or("Optional")),
                                                            patterns: dt
                                                                .child("patterns")
                                                                .map(|ps| ps.all("pattern").map(|pt| (pt.child("dependencies").map(dep_of).unwrap_or(Dep::Met), type_name(pt.child("type").and_then(|t| t.attr("name")).unwrap_or("Optional")))).collect())
                                                                .unwrap_or_default(),
                                                        },
                                                        None => Kind::Fixed(type_name(td.and_then(|t| t.child("type")).and_then(|t| t.attr("name")).unwrap_or("Optional"))),
                                                    };
                                                    Plugin {
                                                        name: p.attr("name").unwrap_or("").to_string(),
                                                        description: p.child("description").map(|d| d.text.trim().to_string()).unwrap_or_default(),
                                                        image: p.child("image").and_then(|i| i.attr("path")).map(norm).unwrap_or_default(),
                                                        puts: puts_of(p.child("files")),
                                                        flags: p.child("conditionflags").map(|f| f.all("flag").map(|x| (x.attr("name").unwrap_or("").to_string(), x.text.trim().to_string())).collect()).unwrap_or_default(),
                                                        kind,
                                                    }
                                                })
                                                .collect();
                                            ordered(list, ps.attr("order"), |p| p.name.clone())
                                        })
                                        .unwrap_or_default();
                                    Group { name: g.attr("name").unwrap_or("").to_string(), kind: g.attr("type").unwrap_or("SelectAny").to_string(), plugins }
                                })
                                .collect();
                            ordered(groups, gs.attr("order"), |g| g.name.clone())
                        })
                        .unwrap_or_default(),
                })
                .collect();
            ordered(list, s.attr("order"), |s| s.name.clone())
        })
        .unwrap_or_default();
    let conditional = c
        .child("conditionalfileinstalls")
        .and_then(|ci| ci.child("patterns"))
        .map(|ps| ps.all("pattern").map(|pt| (pt.child("dependencies").map(dep_of).unwrap_or(Dep::Met), puts_of(pt.child("files")))).collect())
        .unwrap_or_default();
    Ok(Fomod {
        name: c.child("modulename").map(|n| n.text.trim().to_string()).unwrap_or_default(),
        image: c.child("moduleimage").and_then(|i| i.attr("path")).map(norm).unwrap_or_default(),
        needs: c.child("moduledependencies").map(dep_of),
        required: puts_of(c.child("requiredinstallfiles")),
        steps,
        conditional,
    })
}

/// `info.xml`: the mod's name, author, version, web address and description.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Info {
    pub name: String,
    pub author: String,
    pub version: String,
    pub website: String,
    pub description: String,
}

pub fn info(bytes: &[u8]) -> Info {
    let Ok(root) = parse_xml(&text_of(bytes)) else { return Info::default() };
    let Some(f) = root.child("fomod") else { return Info::default() };
    let t = |n: &str| f.child(n).map(|x| x.text.trim().to_string()).unwrap_or_default();
    Info { name: t("name"), author: t("author"), version: t("version"), website: t("website"), description: t("description") }
}

/// What the user chose: the options picked in each group (by `<step>/<group>`, the indexes
/// as listed), and the answer to each question on a file (`Active`, `Inactive`, `Missing`).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Choices {
    pub picks: BTreeMap<String, Vec<String>>,
    pub files: BTreeMap<String, String>,
}

/// An option as the page shows it.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Option_ {
    pub name: String,
    pub description: String,
    pub image: String,
    /// Required | Recommended | Optional | NotUsable | CouldBeUsable
    #[serde(rename = "type")]
    pub kind: String,
    pub picked: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct GroupShown {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub options: Vec<Option_>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct StepShown {
    pub name: String,
    pub visible: bool,
    pub groups: Vec<GroupShown>,
}

/// A question the page asks: a file another mod puts in, and whether it's there and active.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Question {
    pub file: String,
    pub answer: String,
}

/// The FOMOD with the choices so far: each step and whether it's shown, each option's
/// type and whether it's picked (the defaults where nothing's chosen), the questions, and
/// the files that would go in.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Resolved {
    pub name: String,
    pub image: String,
    pub steps: Vec<StepShown>,
    pub questions: Vec<Question>,
    pub flags: BTreeMap<String, String>,
    pub puts: Vec<Put>,
    /// Why it can't go in (its own conditions not met)
    pub cannot: String,
}

fn files_in(d: &Dep, out: &mut Vec<String>) {
    match d {
        Dep::File(f, _) => {
            if !out.iter().any(|x| x.eq_ignore_ascii_case(f)) {
                out.push(f.clone());
            }
        }
        Dep::All(_, v) => v.iter().for_each(|x| files_in(x, out)),
        _ => {}
    }
}

fn holds(d: &Dep, flags: &BTreeMap<String, String>, files: &dyn Fn(&str) -> String) -> bool {
    match d {
        Dep::Met => true,
        Dep::Flag(f, v) => flags.get(f).map(String::as_str).unwrap_or("") == v.as_str(),
        Dep::File(f, s) => files(f).eq_ignore_ascii_case(s),
        Dep::All(true, v) => v.iter().all(|x| holds(x, flags, files)),
        Dep::All(false, v) => v.is_empty() || v.iter().any(|x| holds(x, flags, files)),
    }
}

impl Fomod {
    /// Whether it asks anything: a step with an option to choose.
    pub fn asks(&self) -> bool {
        self.steps.iter().any(|s| s.groups.iter().any(|g| !g.plugins.is_empty()))
    }

    /// The choices worked through, in order. `here(file)`: whether a file is in the game's
    /// folder now (the answer a question has before it's answered: `Active`, or `Missing`).
    pub fn resolve(&self, c: &Choices, here: &dyn Fn(&str) -> bool) -> Resolved {
        let mut asked = vec![];
        for d in self.steps.iter().filter_map(|s| s.visible.as_ref()).chain(self.needs.iter()).chain(self.conditional.iter().map(|(d, _)| d)) {
            files_in(d, &mut asked);
        }
        for s in &self.steps {
            for g in &s.groups {
                for p in &g.plugins {
                    if let Kind::By { patterns, .. } = &p.kind {
                        for (d, _) in patterns {
                            files_in(d, &mut asked);
                        }
                    }
                }
            }
        }
        let questions: Vec<Question> = asked
            .iter()
            .map(|f| Question { file: f.clone(), answer: c.files.iter().find(|(k, _)| k.eq_ignore_ascii_case(f)).map(|(_, v)| v.clone()).unwrap_or_else(|| if here(f) { "Active".into() } else { "Missing".into() }) })
            .collect();
        let files = |f: &str| questions.iter().find(|q| q.file.eq_ignore_ascii_case(f)).map(|q| q.answer.clone()).unwrap_or_else(|| "Missing".into());
        let mut out = Resolved { name: self.name.clone(), image: self.image.clone(), questions: questions.clone(), ..Default::default() };
        if let Some(n) = &self.needs {
            if !holds(n, &BTreeMap::new(), &files) {
                out.cannot = "Its installer says the game doesn't have what it needs (a file or another mod).".into();
            }
        }
        let mut flags: BTreeMap<String, String> = BTreeMap::new();
        let mut puts: Vec<(Put, usize)> = self.required.iter().cloned().map(|p| (p, 0)).collect();
        let mut seq = 1;
        for (si, s) in self.steps.iter().enumerate() {
            let visible = s.visible.as_ref().is_none_or(|v| holds(v, &flags, &files));
            let mut shown = StepShown { name: s.name.clone(), visible, groups: vec![] };
            for (gi, g) in s.groups.iter().enumerate() {
                let id = format!("{si}/{gi}");
                let kinds: Vec<String> = g
                    .plugins
                    .iter()
                    .map(|p| match &p.kind {
                        Kind::Fixed(k) => k.clone(),
                        Kind::By { default, patterns } => patterns.iter().find(|(d, _)| holds(d, &flags, &files)).map(|(_, k)| k.clone()).unwrap_or_else(|| default.clone()),
                    })
                    .collect();
                let usable = |i: usize| kinds[i] != "NotUsable";
                let all = g.kind.eq_ignore_ascii_case("SelectAll");
                let one = g.kind.eq_ignore_ascii_case("SelectExactlyOne") || g.kind.eq_ignore_ascii_case("SelectAtMostOne");
                let mut picked: Vec<bool> = match c.picks.get(&id) {
                    Some(names) => g.plugins.iter().enumerate().map(|(i, p)| usable(i) && (names.iter().any(|n| n == &p.name) || kinds[i] == "Required")).collect(),
                    None => (0..g.plugins.len()).map(|i| usable(i) && (all || kinds[i] == "Required" || kinds[i] == "Recommended")).collect(),
                };
                if all {
                    picked = (0..g.plugins.len()).map(usable).collect();
                }
                if one && picked.iter().filter(|x| **x).count() > 1 {
                    let first = picked.iter().position(|x| *x).unwrap();
                    picked = (0..picked.len()).map(|i| i == first).collect();
                }
                let needs_one = g.kind.eq_ignore_ascii_case("SelectExactlyOne") || g.kind.eq_ignore_ascii_case("SelectAtLeastOne");
                if needs_one && !picked.iter().any(|x| *x) {
                    if let Some(i) = (0..g.plugins.len()).find(|&i| usable(i)) {
                        picked[i] = true;
                    }
                }
                let mut gs = GroupShown { id, name: g.name.clone(), kind: g.kind.clone(), options: vec![] };
                for (i, p) in g.plugins.iter().enumerate() {
                    if visible && picked[i] {
                        for (k, v) in &p.flags {
                            flags.insert(k.clone(), v.clone());
                        }
                        for x in &p.puts {
                            puts.push((x.clone(), seq));
                        }
                        seq += 1;
                    }
                    gs.options.push(Option_ { name: p.name.clone(), description: p.description.clone(), image: p.image.clone(), kind: kinds[i].clone(), picked: picked[i] });
                }
                shown.groups.push(gs);
            }
            out.steps.push(shown);
        }
        for (d, v) in &self.conditional {
            if holds(d, &flags, &files) {
                for x in v {
                    puts.push((x.clone(), seq));
                }
                seq += 1;
            }
        }
        // a later one wins at the same priority; a higher priority wins over both
        puts.sort_by_key(|(p, i)| (p.priority, *i));
        out.puts = puts.into_iter().map(|(p, _)| p).collect();
        out.flags = flags;
        out
    }
}

/// Where a mod's file (by its path in the mod, `/`) goes, by the FOMOD's files: every place,
/// with its rank (later wins over earlier at the same place).
pub fn places(puts: &[Put], rel: &str) -> Vec<(String, usize)> {
    let lower = rel.to_lowercase();
    let mut out = vec![];
    for (i, p) in puts.iter().enumerate() {
        let from = p.from.to_lowercase();
        let rest = if p.folder {
            if from.is_empty() {
                Some(rel.to_string())
            } else {
                lower.strip_prefix(&from).and_then(|r| r.strip_prefix('/')).map(|_| rel[from.len() + 1..].to_string())
            }
        } else if lower == from {
            Some(String::new())
        } else {
            None
        };
        if let Some(rest) = rest {
            let at = [p.to.clone(), rest].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join("/");
            if !at.is_empty() {
                out.push((at, i));
            }
        }
    }
    out
}

/// The folder a mod's FOMOD is in (its `fomod/` folder's parent), among the mod's files'
/// paths: `""` at the top, else that folder; None: it has none.
pub fn root_of<'a>(paths: impl Iterator<Item = &'a str>) -> Option<String> {
    let mut best: Option<String> = None;
    for p in paths {
        let n = p.replace('\\', "/");
        let l = n.to_lowercase();
        let Some(at) = l.strip_suffix("fomod/moduleconfig.xml") else { continue };
        if !(at.is_empty() || at.ends_with('/')) {
            continue;
        }
        let root = n[..at.len()].trim_end_matches('/').to_string();
        if best.as_ref().is_none_or(|b| root.len() < b.len()) {
            best = Some(root);
        }
    }
    best
}

/// Choices kept with a mod checked against its FOMOD as it is: any group's picks naming an
/// option it doesn't have are refused.
pub fn check(f: &Fomod, c: &Choices) -> Result<()> {
    for (id, names) in &c.picks {
        let (s, g) = id.split_once('/').and_then(|(a, b)| Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?))).ok_or_else(|| anyhow!("{id} isn't one of its groups."))?;
        let g = f.steps.get(s).and_then(|st| st.groups.get(g)).ok_or_else(|| anyhow!("{id} isn't one of its groups."))?;
        if let Some(n) = names.iter().find(|n| !g.plugins.iter().any(|p| &&p.name == n)) {
            bail!("{n} isn't one of {}'s options.", g.name);
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const CONFIG: &str = r#"<?xml version="1.0" encoding="UTF-16"?>
<config xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <moduleName>Better Things</moduleName>
  <moduleImage path="fomod\images\main.png" />
  <requiredInstallFiles>
    <folder source="Core" destination="" />
  </requiredInstallFiles>
  <installSteps order="Explicit">
    <installStep name="Textures">
      <optionalFileGroups order="Explicit">
        <group name="Resolution" type="SelectExactlyOne">
          <plugins order="Explicit">
            <plugin name="2K">
              <description>Sharper &amp; bigger.</description>
              <image path="fomod\images\2k.png" />
              <files><folder source="Textures\2K" destination="textures" priority="1" /></files>
              <conditionFlags><flag name="res">2k</flag></conditionFlags>
              <typeDescriptor><type name="Optional" /></typeDescriptor>
            </plugin>
            <plugin name="1K">
              <description>Lighter.</description>
              <files><folder source="Textures\1K" destination="textures" /></files>
              <conditionFlags><flag name="res">1k</flag></conditionFlags>
              <typeDescriptor><type name="Recommended" /></typeDescriptor>
            </plugin>
          </plugins>
        </group>
      </optionalFileGroups>
    </installStep>
    <installStep name="Patches">
      <visible><flagDependency flag="res" value="2k" /></visible>
      <optionalFileGroups>
        <group name="Extras" type="SelectAny">
          <plugins>
            <plugin name="Unofficial Patch fix">
              <description>For the unofficial patch.</description>
              <files><file source="Patches\fix.esp" /></files>
              <typeDescriptor><dependencyType><defaultType name="NotUsable" />
                <patterns><pattern><dependencies operator="And"><fileDependency file="Unofficial Patch.esp" state="Active" /></dependencies><type name="Recommended" /></pattern></patterns>
              </dependencyType></typeDescriptor>
            </plugin>
            <plugin name="A readme">
              <description>Notes.</description>
              <files><file source="readme.txt" destination="Docs\" /></files>
              <typeDescriptor><type name="Optional" /></typeDescriptor>
            </plugin>
          </plugins>
        </group>
      </optionalFileGroups>
    </installStep>
  </installSteps>
  <conditionalFileInstalls><patterns>
    <pattern><dependencies><flagDependency flag="res" value="2k" /></dependencies><files><file source="Extra\2k.ini" destination="Better.ini" /></files></pattern>
  </patterns></conditionalFileInstalls>
</config>"#;

    pub(crate) fn utf16(s: &str) -> Vec<u8> {
        let mut b = vec![0xFF, 0xFE];
        for u in s.encode_utf16() {
            b.extend(u.to_le_bytes());
        }
        b
    }

    #[test]
    fn a_fomod_is_read_its_choices_worked_through_and_its_files_placed() {
        let f = parse(&utf16(CONFIG)).unwrap();
        assert_eq!((f.name.as_str(), f.image.as_str(), f.asks()), ("Better Things", "fomod/images/main.png", true));
        // the defaults: 1K (recommended), so the second step isn't shown
        let none = |_: &str| false;
        let r = f.resolve(&Choices::default(), &none);
        assert_eq!(r.steps[0].groups[0].options.iter().map(|o| (o.name.as_str(), o.picked)).collect::<Vec<_>>(), vec![("2K", false), ("1K", true)]);
        assert!(!r.steps[1].visible);
        assert_eq!(r.puts.iter().map(|p| (p.from.as_str(), p.to.as_str())).collect::<Vec<_>>(), vec![("Core", ""), ("Textures/1K", "textures")]);
        assert_eq!(r.steps[0].groups[0].options[0].description, "Sharper & bigger.");
        assert_eq!(r.questions, vec![Question { file: "Unofficial Patch.esp".into(), answer: "Missing".into() }]);
        // 2K picked: the step shown, its fix not usable until the patch is said to be active
        let mut c = Choices::default();
        c.picks.insert("0/0".into(), vec!["2K".into()]);
        c.picks.insert("1/0".into(), vec!["Unofficial Patch fix".into(), "A readme".into()]);
        let r = f.resolve(&c, &none);
        assert!(r.steps[1].visible);
        assert_eq!(r.steps[1].groups[0].options.iter().map(|o| (o.kind.as_str(), o.picked)).collect::<Vec<_>>(), vec![("Optional", true), ("NotUsable", false)], "by name, as no order is given");
        c.files.insert("Unofficial Patch.esp".into(), "Active".into());
        let r = f.resolve(&c, &none);
        assert_eq!(r.steps[1].groups[0].options[1].kind, "Recommended");
        let puts: Vec<(&str, &str)> = r.puts.iter().map(|p| (p.from.as_str(), p.to.as_str())).collect();
        assert_eq!(puts, vec![("Core", ""), ("readme.txt", "Docs/readme.txt"), ("Patches/fix.esp", "Patches/fix.esp"), ("Extra/2k.ini", "Better.ini"), ("Textures/2K", "textures")], "priority 1 last");
        // where each of the mod's files goes
        assert_eq!(places(&r.puts, "Core/Data/a.bsa"), vec![("Data/a.bsa".to_string(), 0)]);
        assert_eq!(places(&r.puts, "textures/2k/sky.dds"), vec![("textures/sky.dds".to_string(), 4)]);
        assert!(places(&r.puts, "Textures/1K/sky.dds").is_empty());
        assert!(check(&f, &c).is_ok());
        c.picks.insert("0/0".into(), vec!["4K".into()]);
        assert!(check(&f, &c).is_err());
        assert_eq!(root_of(["Mod/fomod/ModuleConfig.xml", "Mod/Core/x", "Mod/sub/fomod/moduleconfig.xml"].into_iter()), Some("Mod".into()));
        assert_eq!(root_of(["fomod/ModuleConfig.xml"].into_iter()), Some(String::new()));
        assert_eq!(root_of(["notfomod/ModuleConfig.xml"].into_iter()), None);
        let i = info(br#"<?xml version="1.0"?><fomod><Name>Better Things</Name><Author>Someone</Author><Version MachineVersion="1.2">1.2</Version><Website>https://www.nexusmods.com/x/mods/1</Website></fomod>"#);
        assert_eq!((i.name.as_str(), i.author.as_str(), i.version.as_str(), i.website.as_str()), ("Better Things", "Someone", "1.2", "https://www.nexusmods.com/x/mods/1"));
    }
}
