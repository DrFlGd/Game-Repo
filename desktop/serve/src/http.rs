//! Just enough HTTP/1.1 for the server: a request read from a connection (kept open for
//! the next), and an answer written back, a file's streamed from disk a piece at a time
//! so a large ROM is never read into memory (D95).

use anyhow::{anyhow, bail, Result};
use std::io::{BufRead, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

/// A request's head is at most this long, and its body this long.
const MAX_HEAD: usize = 16 * 1024;
const MAX_BODY: usize = 32 * 1024 * 1024;
/// A file is sent this much at a time.
pub const PIECE: usize = 1 << 20;

#[derive(Debug, Default)]
pub struct Request {
    pub method: String,
    /// The path, decoded
    pub path: String,
    pub query: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub fn param(&self, name: &str) -> Option<&str> {
        self.query.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    pub fn has_param(&self, name: &str) -> bool {
        self.query.iter().any(|(k, _)| k == name)
    }

    /// Does the client want the connection closed after this answer?
    pub fn closes(&self) -> bool {
        self.header("connection").is_some_and(|c| c.eq_ignore_ascii_case("close"))
    }
}

/// `%20` and the like decoded (and `+` left as it is: paths, not forms).
pub fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push(h << 4 | l);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A path's parts encoded for a URL (`/` kept between them).
pub fn encode_path(p: &str) -> String {
    let mut out = String::with_capacity(p.len());
    for c in p.bytes() {
        if c.is_ascii_alphanumeric() || b"-._~/!$&'()*+,;=:@".contains(&c) {
            out.push(c as char);
        } else {
            out.push_str(&format!("%{c:02X}"));
        }
    }
    out
}

/// The next request on a connection; None when the client has closed it.
pub fn read_request<R: BufRead>(r: &mut R) -> Result<Option<Request>> {
    let mut head = Vec::new();
    loop {
        let mut line = Vec::new();
        let n = r.by_ref().take((MAX_HEAD + 1 - head.len().min(MAX_HEAD)) as u64).read_until(b'\n', &mut line)?;
        if n == 0 {
            if head.is_empty() {
                return Ok(None);
            }
            bail!("the request ended early");
        }
        if line == b"\r\n" || line == b"\n" {
            if head.is_empty() {
                // a blank line between requests
                continue;
            }
            break;
        }
        head.extend_from_slice(&line);
        if head.len() > MAX_HEAD {
            bail!("the request's head is too long");
        }
    }
    let text = String::from_utf8_lossy(&head);
    let mut lines = text.lines();
    let first = lines.next().ok_or_else(|| anyhow!("no request line"))?;
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("/").to_string();
    let headers: Vec<(String, String)> = lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect();
    let (path, query) = target.split_once('?').unwrap_or((target.as_str(), ""));
    let query = query
        .split('&')
        .filter(|q| !q.is_empty())
        .map(|q| {
            let (k, v) = q.split_once('=').unwrap_or((q, ""));
            (decode(&k.replace('+', " ")), decode(&v.replace('+', " ")))
        })
        .collect();
    let mut req = Request { method, path: decode(path), query, headers, body: vec![] };
    if req.header("transfer-encoding").is_some_and(|t| t.to_ascii_lowercase().contains("chunked")) {
        bail!("chunked bodies aren't taken");
    }
    let len: usize = req.header("content-length").map(|l| l.parse()).transpose()?.unwrap_or(0);
    if len > MAX_BODY {
        bail!("the request's body is too long");
    }
    req.body = vec![0; len];
    r.read_exact(&mut req.body)?;
    Ok(Some(req))
}

/// What's sent back.
pub enum Body {
    Bytes(Vec<u8>),
    /// A part of a file on disk, sent from `start` for `len` bytes
    File { path: PathBuf, start: u64, len: u64 },
}

pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Body,
}

impl Response {
    pub fn bytes(status: u16, content_type: &str, body: Vec<u8>) -> Self {
        Self { status, headers: vec![("Content-Type".into(), content_type.into())], body: Body::Bytes(body) }
    }

    pub fn json<T: serde::Serialize>(status: u16, v: &T) -> Self {
        Self::bytes(status, "application/json", serde_json::to_vec(v).unwrap_or_default())
    }

    /// An error, as JSON (`{"error": …}`).
    pub fn problem(status: u16, error: impl Into<String>) -> Self {
        Self::json(status, &gamerepo_wire::api::Problem { error: error.into() })
    }

    pub fn with(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    fn len(&self) -> u64 {
        match &self.body {
            Body::Bytes(b) => b.len() as u64,
            Body::File { len, .. } => *len,
        }
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        202 => "Accepted",
        204 => "No Content",
        206 => "Partial Content",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        410 => "Gone",
        416 => "Range Not Satisfiable",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "",
    }
}

/// Write an answer (only its head for a HEAD request); a file is read and sent a piece at
/// a time.
pub fn write_response<W: Write>(w: &mut W, resp: &Response, head_only: bool, close: bool) -> Result<()> {
    let mut head = format!("HTTP/1.1 {} {}\r\nContent-Length: {}\r\n", resp.status, reason(resp.status), resp.len());
    for (k, v) in &resp.headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    if close {
        head.push_str("Connection: close\r\n");
    }
    head.push_str("\r\n");
    w.write_all(head.as_bytes())?;
    if !head_only {
        match &resp.body {
            Body::Bytes(b) => w.write_all(b)?,
            Body::File { path, start, len } => {
                let mut f = std::fs::File::open(path)?;
                f.seek(SeekFrom::Start(*start))?;
                let mut left = *len;
                let mut buf = vec![0u8; PIECE];
                while left > 0 {
                    let n = f.read(&mut buf[..PIECE.min(left as usize)])?;
                    if n == 0 {
                        bail!("{} got shorter while it was sent", path.display());
                    }
                    w.write_all(&buf[..n])?;
                    left -= n as u64;
                }
            }
        }
    }
    w.flush()?;
    Ok(())
}

/// A `Range: bytes=…` header for a file of `total` bytes: (start, length). None: the
/// whole file; Err: a range it doesn't have (416). One range only (D95).
pub fn range(header: Option<&str>, total: u64) -> std::result::Result<Option<(u64, u64)>, ()> {
    let Some(r) = header.and_then(|h| h.trim().strip_prefix("bytes=")) else { return Ok(None) };
    if r.contains(',') {
        return Ok(None);
    }
    let (a, b) = r.split_once('-').ok_or(())?;
    let (start, end) = match (a.trim().parse::<u64>().ok(), b.trim().parse::<u64>().ok()) {
        (Some(s), Some(e)) => (s, e.min(total.saturating_sub(1))),
        (Some(s), None) => (s, total.saturating_sub(1)),
        (None, Some(n)) if n > 0 => (total.saturating_sub(n), total.saturating_sub(1)),
        _ => return Err(()),
    };
    if start >= total || end < start {
        return Err(());
    }
    Ok(Some((start, end - start + 1)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_is_read_and_the_next_one_after_it() {
        let raw = b"POST /v1/plans?x=1&q=a%20b HTTP/1.1\r\nHost: h\r\nContent-Length: 2\r\n\r\n{}GET /v1/files/library/A%20B.zip HTTP/1.1\r\nRange: bytes=10-\r\n\r\n";
        let mut r = std::io::BufReader::new(&raw[..]);
        let a = read_request(&mut r).unwrap().unwrap();
        assert_eq!((a.method.as_str(), a.path.as_str(), a.param("q"), a.body.as_slice()), ("POST", "/v1/plans", Some("a b"), &b"{}"[..]));
        let b = read_request(&mut r).unwrap().unwrap();
        assert_eq!((b.path.as_str(), b.header("range")), ("/v1/files/library/A B.zip", Some("bytes=10-")));
        assert!(read_request(&mut r).unwrap().is_none());
    }

    #[test]
    fn ranges() {
        assert_eq!(range(None, 100), Ok(None));
        assert_eq!(range(Some("bytes=40-"), 100), Ok(Some((40, 60))));
        assert_eq!(range(Some("bytes=0-9"), 100), Ok(Some((0, 10))));
        assert_eq!(range(Some("bytes=-10"), 100), Ok(Some((90, 10))));
        assert_eq!(range(Some("bytes=100-"), 100), Err(()));
        assert_eq!(encode_path("psx/Quest (USA).m3u/A#1.cue"), "psx/Quest%20(USA).m3u/A%231.cue");
        assert_eq!(decode(&encode_path("a b/ü%.zip")), "a b/ü%.zip");
    }
}
