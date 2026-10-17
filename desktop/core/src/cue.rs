//! Cue sheets and GDI files (docs/PLAN.md, "2b", Disc sets and the DAT): the tracks
//! they list, the files they name, and the same sheet with those files renamed.

use std::collections::HashMap;

/// A CD frame (sector) as a raw track holds it.
pub const SECTOR: u64 = 2352;

/// One track of a cue sheet.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub number: u32,
    /// The cue's mode: MODE1/2352, MODE2/2352, AUDIO, MODE1/2048…
    pub mode: String,
    /// INDEX 00 and INDEX 01, in frames from the start of its file
    pub index0: Option<u64>,
    pub index1: u64,
    /// PREGAP and POSTGAP, in frames (not in the file)
    pub pregap: u64,
    pub postgap: u64,
}

/// One FILE of a cue sheet and its tracks.
#[derive(Clone, Debug, PartialEq)]
pub struct File {
    pub name: String,
    /// BINARY, MOTOROLA, WAVE…
    pub kind: String,
    pub tracks: Vec<Track>,
}

impl Track {
    /// Bytes per frame in its file.
    pub fn sector_size(&self) -> u64 {
        match self.mode.to_ascii_uppercase().as_str() {
            "MODE1/2048" | "MODE2/2048" => 2048,
            "MODE2/2336" => 2336,
            "MODE2/2324" => 2324,
            _ => SECTOR,
        }
    }

    pub fn is_audio(&self) -> bool {
        self.mode.eq_ignore_ascii_case("AUDIO")
    }

    /// The type CHD's track metadata gives it.
    pub fn chd_type(&self) -> &'static str {
        match self.mode.to_ascii_uppercase().as_str() {
            "AUDIO" => "AUDIO",
            "MODE1/2048" => "MODE1",
            "MODE1/2352" => "MODE1_RAW",
            "MODE2/2336" => "MODE2",
            "MODE2/2048" => "MODE2_FORM1",
            "MODE2/2324" => "MODE2_FORM2",
            "CDI/2352" | "MODE2/2352" => "MODE2_RAW",
            _ => "MODE1_RAW",
        }
    }

    /// Where its bytes start in its file: its INDEX 00 when the pregap is in the file.
    pub fn start(&self) -> u64 {
        self.index0.unwrap_or(self.index1)
    }
}

/// Words of a line, a quoted name kept whole.
fn words(line: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut quoted = false;
    for c in line.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// mm:ss:ff as frames (75 a second).
fn msf(s: &str) -> Option<u64> {
    let p: Vec<u64> = s.split(':').map(|x| x.trim().parse().ok()).collect::<Option<_>>()?;
    (p.len() == 3).then(|| (p[0] * 60 + p[1]) * 75 + p[2])
}

/// The files and tracks of a cue sheet (lines it doesn't know are left out).
pub fn parse(text: &str) -> Vec<File> {
    let mut files: Vec<File> = vec![];
    for line in text.lines() {
        let w = words(line);
        let Some(cmd) = w.first().map(|c| c.to_ascii_uppercase()) else { continue };
        match cmd.as_str() {
            "FILE" if w.len() >= 2 => files.push(File {
                name: w[1].clone(),
                kind: w.get(2).cloned().unwrap_or_else(|| "BINARY".into()).to_ascii_uppercase(),
                tracks: vec![],
            }),
            "TRACK" if w.len() >= 3 => {
                if let Some(f) = files.last_mut() {
                    f.tracks.push(Track {
                        number: w[1].parse().unwrap_or(0),
                        mode: w[2].clone(),
                        index0: None,
                        index1: 0,
                        pregap: 0,
                        postgap: 0,
                    });
                }
            }
            "INDEX" | "PREGAP" | "POSTGAP" => {
                let Some(t) = files.last_mut().and_then(|f| f.tracks.last_mut()) else { continue };
                match (cmd.as_str(), w.len()) {
                    ("INDEX", 3..) => match (w[1].parse::<u32>(), msf(&w[2])) {
                        (Ok(0), Some(at)) => t.index0 = Some(at),
                        (Ok(1), Some(at)) => t.index1 = at,
                        _ => {}
                    },
                    ("PREGAP", 2..) => t.pregap = msf(&w[1]).unwrap_or(0),
                    ("POSTGAP", 2..) => t.postgap = msf(&w[1]).unwrap_or(0),
                    _ => {}
                }
            }
            _ => {}
        }
    }
    files
}

/// The file names a cue sheet or GDI names, in order.
pub fn file_names(text: &str, gdi: bool) -> Vec<String> {
    if gdi {
        return gdi_tracks(text).into_iter().map(|t| t.file).collect();
    }
    parse(text).into_iter().map(|f| f.name).collect()
}

/// A track of a GDI file: `number lba type sector_size "file" offset`.
#[derive(Clone, Debug, PartialEq)]
pub struct GdiTrack {
    pub number: u32,
    pub lba: u64,
    /// 0 audio, 4 data
    pub kind: u32,
    pub sector: u64,
    pub file: String,
}

pub fn gdi_tracks(text: &str) -> Vec<GdiTrack> {
    text.lines()
        .skip(1)
        .filter_map(|l| {
            let w = words(l);
            (w.len() >= 5).then(|| GdiTrack {
                number: w[0].parse().unwrap_or(0),
                lba: w[1].parse().unwrap_or(0),
                kind: w[2].parse().unwrap_or(4),
                sector: w[3].parse().unwrap_or(SECTOR),
                file: w[4].clone(),
            })
        })
        .collect()
}

/// The sheet with the files it names renamed (`renames`: old name -> new), all else as it was.
pub fn rewrite(text: &str, renames: &HashMap<String, String>, gdi: bool) -> String {
    let mut out = String::with_capacity(text.len() + 64);
    for (i, line) in text.split_inclusive('\n').enumerate() {
        let body = line.trim_end_matches(['\r', '\n']);
        let end = &line[body.len()..];
        let w = words(body);
        let old = if gdi {
            (i > 0 && w.len() >= 5).then(|| w[4].clone())
        } else {
            (w.first().is_some_and(|c| c.eq_ignore_ascii_case("FILE")) && w.len() >= 2).then(|| w[1].clone())
        };
        match old.and_then(|o| renames.get(&o).map(|n| (o, n))) {
            Some((o, n)) => {
                let quoted = format!("\"{o}\"");
                let new_quoted = format!("\"{n}\"");
                let line = if body.contains(&quoted) {
                    body.replacen(&quoted, &new_quoted, 1)
                } else if n.contains(' ') {
                    body.replacen(&o, &new_quoted, 1)
                } else {
                    body.replacen(&o, n, 1)
                };
                out.push_str(&line);
            }
            None => out.push_str(body),
        }
        out.push_str(end);
    }
    out
}

/// Whether a file name is a cue sheet or a GDI (the files that list a disc's tracks).
pub fn is_sheet(name: &str) -> bool {
    matches!(crate::platforms::ext_of(name).as_str(), "cue" | "gdi")
}

#[cfg(test)]
mod tests {
    use super::*;

    const CUE: &str = "FILE \"Game (Track 1).bin\" BINARY\r\n  TRACK 01 MODE2/2352\r\n    INDEX 01 00:00:00\r\nFILE \"Game (Track 2).bin\" BINARY\r\n  TRACK 02 AUDIO\r\n    INDEX 00 00:00:00\r\n    INDEX 01 00:02:00\r\n";

    #[test]
    fn reads_a_cue_and_renames_its_files() {
        let f = parse(CUE);
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].tracks[0].chd_type(), "MODE2_RAW");
        assert_eq!(f[1].tracks[0].index0, Some(0));
        assert_eq!(f[1].tracks[0].index1, 150);
        assert!(f[1].tracks[0].is_audio());
        assert_eq!(file_names(CUE, false), ["Game (Track 1).bin", "Game (Track 2).bin"]);
        let map = HashMap::from([("Game (Track 2).bin".to_string(), "Other (Track 2).bin".to_string())]);
        let out = rewrite(CUE, &map, false);
        assert!(out.contains("FILE \"Other (Track 2).bin\" BINARY\r\n"));
        assert!(out.contains("FILE \"Game (Track 1).bin\" BINARY\r\n"));
        assert_eq!(out.len(), CUE.len() + 1);
        let unquoted = rewrite("FILE a.bin BINARY\n", &HashMap::from([("a.bin".into(), "b c.bin".into())]), false);
        assert_eq!(unquoted, "FILE \"b c.bin\" BINARY\n");
    }

    #[test]
    fn reads_a_gdi_and_renames_its_files() {
        let gdi = "3\n1 0 4 2352 track01.bin 0\n2 600 0 2352 \"track 02.raw\" 0\n3 45000 4 2352 track03.bin 0\n";
        let t = gdi_tracks(gdi);
        assert_eq!(t.len(), 3);
        assert_eq!((t[1].kind, t[1].file.as_str(), t[2].lba), (0, "track 02.raw", 45000));
        let out = rewrite(gdi, &HashMap::from([("track 02.raw".into(), "Game (Track 2).raw".into())]), true);
        assert!(out.contains("2 600 0 2352 \"Game (Track 2).raw\" 0\n"));
        assert!(out.starts_with("3\n1 0 4 2352 track01.bin 0\n"));
    }
}
