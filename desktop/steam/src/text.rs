//! Steam's text VDF (`config.vdf`, `loginusers.vdf`): quoted keys, each with a quoted
//! value or a block in braces. Read whole and written back as Steam writes it (tabs, a key
//! and its value two tabs apart), so a file read and written unchanged is the same file.

use anyhow::{bail, Result};

#[derive(Clone, Debug, PartialEq)]
pub enum Text {
    Str(String),
    Map(Vec<(String, Text)>),
}

impl Text {
    /// A value in a block, by its key (any case).
    pub fn get(&self, key: &str) -> Option<&Text> {
        match self {
            Text::Map(m) => m.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v),
            Text::Str(_) => None,
        }
    }

    pub fn str(&self, key: &str) -> Option<String> {
        match self.get(key)? {
            Text::Str(s) => Some(s.clone()),
            Text::Map(_) => None,
        }
    }
}

#[derive(Debug, PartialEq)]
enum Token {
    Word(String),
    Open,
    Close,
}

fn tokens(text: &str) -> Vec<Token> {
    let mut out = vec![];
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                let mut s = String::new();
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => match chars.next() {
                            Some('n') => s.push('\n'),
                            Some('t') => s.push('\t'),
                            Some(n) => s.push(n),
                            None => {}
                        },
                        '"' => break,
                        _ => s.push(c),
                    }
                }
                out.push(Token::Word(s));
            }
            '{' => out.push(Token::Open),
            '}' => out.push(Token::Close),
            // a comment, to the end of its line
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            // a condition (`[$WIN32]`), which Steam's own files don't use: passed over
            '[' => {
                for c in chars.by_ref() {
                    if c == ']' {
                        break;
                    }
                }
            }
            c if c.is_whitespace() => {}
            c => {
                // a word without quotes
                let mut s = String::from(c);
                while let Some(&n) = chars.peek() {
                    if n.is_whitespace() || n == '"' || n == '{' || n == '}' {
                        break;
                    }
                    s.push(n);
                    chars.next();
                }
                out.push(Token::Word(s));
            }
        }
    }
    out
}

fn block(t: &[Token], at: &mut usize, depth: usize) -> Result<Vec<(String, Text)>> {
    if depth > 64 {
        bail!("the file is nested too deep");
    }
    let mut m = vec![];
    while *at < t.len() {
        match &t[*at] {
            Token::Close if depth > 0 => {
                *at += 1;
                return Ok(m);
            }
            Token::Close | Token::Open => bail!("the file has a brace where a name should be"),
            Token::Word(k) => {
                *at += 1;
                match t.get(*at) {
                    Some(Token::Word(v)) => {
                        *at += 1;
                        m.push((k.clone(), Text::Str(v.clone())));
                    }
                    Some(Token::Open) => {
                        *at += 1;
                        m.push((k.clone(), Text::Map(block(t, at, depth + 1)?)));
                    }
                    _ => bail!("{k} has no value"),
                }
            }
        }
    }
    if depth > 0 {
        bail!("the file ends inside a block");
    }
    Ok(m)
}

/// Read a text VDF file: its top level.
pub fn parse_text(text: &str) -> Result<Vec<(String, Text)>> {
    let t = tokens(text);
    block(&t, &mut 0, 0)
}

fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn put(out: &mut String, m: &[(String, Text)], depth: usize) {
    let tabs = "\t".repeat(depth);
    for (k, v) in m {
        match v {
            Text::Str(s) => {
                out.push_str(&format!("{tabs}{}\t\t{}\n", quote(k), quote(s)));
            }
            Text::Map(inner) => {
                out.push_str(&format!("{tabs}{}\n{tabs}{{\n", quote(k)));
                put(out, inner, depth + 1);
                out.push_str(&format!("{tabs}}}\n"));
            }
        }
    }
}

/// Write a text VDF file, as Steam does.
pub fn write_text(m: &[(String, Text)]) -> String {
    let mut out = String::new();
    put(&mut out, m, 0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_text_file_reads_and_writes_back_the_same() {
        let text = "\"InstallConfigStore\"\n{\n\t\"Software\"\n\t{\n\t\t\"Valve\"\n\t\t{\n\t\t\t\"Steam\"\n\t\t\t{\n\t\t\t\t\"SentryFile\"\t\t\"C:\\\\Program Files (x86)\\\\Steam\\\\ssfn\"\n\t\t\t\t\"Quote\"\t\t\"a \\\"b\\\" c\"\n\t\t\t\t\"Empty\"\n\t\t\t\t{\n\t\t\t\t}\n\t\t\t}\n\t\t}\n\t}\n}\n";
        let top = parse_text(text).unwrap();
        let steam = top[0].1.get("software").unwrap().get("valve").unwrap().get("Steam").unwrap();
        assert_eq!(steam.str("SentryFile").as_deref(), Some(r"C:\Program Files (x86)\Steam\ssfn"));
        assert_eq!(steam.str("Quote").as_deref(), Some("a \"b\" c"));
        assert_eq!(write_text(&top), text);
        // comments and words without quotes are read too
        let loose = parse_text("// made by hand\nusers { 7656 { Name Ann } }").unwrap();
        assert_eq!(loose[0].1.get("7656").unwrap().str("name").as_deref(), Some("Ann"));
        assert!(parse_text("\"a\" { \"b\" \"c\"").is_err());
    }
}
