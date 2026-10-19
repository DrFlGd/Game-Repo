//! Talking to Game Repo Server over HTTPS (D91): the server's certificate is taken at
//! pairing and pinned by its SHA-256 from then on, so a device talks only to the server it
//! paired with; every request but `info` and `pair` carries the device's token.

use anyhow::{anyhow, bail, Context, Result};
use gamerepo_wire::api::{self, Info, PairAsk, Paired};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A server this device is paired with.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Server {
    /// `192.168.1.20:47210`
    pub address: String,
    /// The certificate's SHA-256, pinned at pairing
    pub fingerprint: String,
    pub token: String,
    /// The server's id and the device's id on it
    pub server: String,
    pub device: String,
}

/// Accepts the certificate pinned (or, with none pinned, any, noting which), and checks the
/// server holds its key.
#[derive(Debug)]
struct Pinned {
    pin: Option<String>,
    seen: Mutex<Option<String>>,
    provider: Arc<rustls::crypto::CryptoProvider>,
}

pub const CHANGED: &str = "The server's certificate isn't the one this device paired with: pair it again (if the server wasn't set up again, something else on the network may be answering for it).";

impl ServerCertVerifier for Pinned {
    fn verify_server_cert(&self, end: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: &ServerName<'_>, _: &[u8], _: UnixTime) -> Result<ServerCertVerified, rustls::Error> {
        let fp = hex::encode(sha2::Sha256::digest(end.as_ref()));
        *self.seen.lock().unwrap() = Some(fp.clone());
        match &self.pin {
            Some(p) if *p != fp => Err(rustls::Error::General(CHANGED.into())),
            _ => Ok(ServerCertVerified::assertion()),
        }
    }

    fn verify_tls12_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

type Stream = rustls::StreamOwned<rustls::ClientConnection, TcpStream>;

/// A connection, and the certificate's fingerprint it saw.
fn connect(address: &str, pin: Option<&str>) -> Result<(BufReader<Stream>, String)> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = Arc::new(Pinned { pin: pin.map(String::from), seen: Mutex::new(None), provider: provider.clone() });
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(verifier.clone())
        .with_no_client_auth();
    let addr = address.to_socket_addrs().with_context(|| format!("{address} isn't an address"))?.next().ok_or_else(|| anyhow!("{address} isn't an address"))?;
    let tcp = TcpStream::connect_timeout(&addr, Duration::from_secs(5)).with_context(|| format!("The server at {address} isn't answering"))?;
    tcp.set_nodelay(true)?;
    tcp.set_read_timeout(Some(Duration::from_secs(60)))?;
    tcp.set_write_timeout(Some(Duration::from_secs(60)))?;
    let conn = rustls::ClientConnection::new(Arc::new(config), ServerName::try_from("gamerepo-server")?.to_owned())?;
    let mut s = rustls::StreamOwned::new(conn, tcp);
    // the handshake now, so a certificate that changed says so
    while s.conn.is_handshaking() {
        if let Err(e) = s.conn.complete_io(&mut s.sock) {
            if e.to_string().contains(CHANGED) {
                bail!("{CHANGED}");
            }
            return Err(e).with_context(|| format!("couldn't talk to the server at {address}"));
        }
    }
    let seen = verifier.seen.lock().unwrap().clone().unwrap_or_default();
    Ok((BufReader::with_capacity(256 * 1024, s), seen))
}

/// An answer's head.
pub struct Head {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub length: u64,
}

impl Head {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

fn send(r: &mut BufReader<Stream>, method: &str, path: &str, headers: &[(&str, String)], body: &[u8]) -> Result<Head> {
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: gamerepo-server\r\nContent-Length: {}\r\nConnection: close\r\n", body.len());
    for (k, v) in headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    let w = r.get_mut();
    w.write_all(head.as_bytes())?;
    w.write_all(body)?;
    w.flush()?;
    let mut line = String::new();
    r.read_line(&mut line)?;
    let status: u16 = line.split_whitespace().nth(1).and_then(|s| s.parse().ok()).ok_or_else(|| anyhow!("The server's answer can't be read"))?;
    let mut headers = vec![];
    loop {
        let mut l = String::new();
        if r.read_line(&mut l)? == 0 {
            bail!("The server's answer ended early");
        }
        let l = l.trim_end();
        if l.is_empty() {
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    let length = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-length")).and_then(|(_, v)| v.parse().ok()).unwrap_or(0);
    Ok(Head { status, headers, length })
}

fn read_body(r: &mut BufReader<Stream>, head: &Head) -> Result<Vec<u8>> {
    let mut b = vec![0; head.length as usize];
    r.read_exact(&mut b)?;
    Ok(b)
}

fn problem(status: u16, body: &[u8]) -> anyhow::Error {
    let v: serde_json::Value = serde_json::from_slice(body).unwrap_or_default();
    anyhow!("{}", v["error"].as_str().map(String::from).unwrap_or_else(|| format!("The server answered {status}.")))
}

/// What the server at `address` is (no pairing needed), and its certificate's fingerprint.
pub fn info(address: &str) -> Result<(Info, String)> {
    let (mut r, seen) = connect(address, None)?;
    let head = send(&mut r, "GET", "/v1/info", &[], &[])?;
    let body = read_body(&mut r, &head)?;
    if head.status != 200 {
        return Err(problem(head.status, &body));
    }
    let info: Info = serde_json::from_slice(&body)?;
    if info.api > api::API {
        bail!("The server is newer than this client (its API is {}, this client's {}): update the client.", info.api, api::API);
    }
    Ok((info, seen))
}

/// Pair with the server at `address` using the code Game Repo shows: its certificate is
/// pinned from now on.
pub fn pair(address: &str, code: &str, name: &str, kind: &str) -> Result<Server> {
    let (_, fingerprint) = info(address)?;
    let (mut r, seen) = connect(address, Some(&fingerprint))?;
    let ask = serde_json::to_vec(&PairAsk { code: code.into(), name: name.into(), kind: kind.into() })?;
    let head = send(&mut r, "POST", "/v1/pair", &[("Content-Type", "application/json".into())], &ask)?;
    let body = read_body(&mut r, &head)?;
    if head.status != 200 {
        return Err(problem(head.status, &body));
    }
    let p: Paired = serde_json::from_slice(&body)?;
    Ok(Server { address: address.into(), fingerprint: seen, token: p.token, server: p.server, device: p.device })
}

/// The answer to a request that wasn't a success.
#[derive(Debug)]
pub struct Refused {
    pub status: u16,
    pub message: String,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for Refused {}

impl Server {
    fn open(&self) -> Result<BufReader<Stream>> {
        Ok(connect(&self.address, Some(&self.fingerprint))?.0)
    }

    fn auth(&self) -> (&'static str, String) {
        ("Authorization", format!("Bearer {}", self.token))
    }

    /// A request, its answer as JSON. A refusal is a [`Refused`] error.
    pub fn call<T: for<'a> Deserialize<'a>>(&self, method: &str, path: &str, body: Option<&impl Serialize>) -> Result<T> {
        let mut r = self.open()?;
        let b = match body {
            Some(b) => serde_json::to_vec(b)?,
            None => vec![],
        };
        let head = send(&mut r, method, path, &[self.auth(), ("Content-Type", "application/json".into())], &b)?;
        let body = read_body(&mut r, &head)?;
        if head.status != 200 {
            let message = problem(head.status, &body).to_string();
            return Err(Refused { status: head.status, message }.into());
        }
        Ok(serde_json::from_slice(&body)?)
    }

    pub fn get<T: for<'a> Deserialize<'a>>(&self, path: &str) -> Result<T> {
        self.call("GET", path, None::<&()>)
    }

    /// A request whose answer's body is left to read (a download, for the tests).
    pub fn open_request(&self, method: &str, path: &str) -> Result<(Head, Box<dyn Read + Send>)> {
        let mut r = self.open()?;
        let head = send(&mut r, method, path, &[self.auth()], &[])?;
        Ok((head, Box::new(r)))
    }

    /// A file's SHA-1, waiting while the server reads or makes it.
    pub fn sha1_of(&self, url: &str, wait: Duration) -> Result<String> {
        let until = std::time::Instant::now() + wait;
        loop {
            let mut r = self.open()?;
            let head = send(&mut r, "GET", &format!("{url}?sha1"), &[self.auth()], &[])?;
            let body = read_body(&mut r, &head)?;
            match head.status {
                200 => {
                    let v: serde_json::Value = serde_json::from_slice(&body)?;
                    return v["sha1"].as_str().map(String::from).ok_or_else(|| anyhow!("no SHA-1"));
                }
                202 if std::time::Instant::now() < until => std::thread::sleep(Duration::from_millis(250)),
                202 => bail!("The server hasn't read that file's SHA-1 yet."),
                s => return Err(Refused { status: s, message: problem(s, &body).to_string() }.into()),
            }
        }
    }

    /// Fetch a file into `part`, carrying on from what's there when the file hasn't changed
    /// since (its ETag kept beside it); a file still being made is waited for. `stop_at`
    /// stops after that many bytes of the file (for the tests). Returns the bytes fetched now.
    pub fn fetch(&self, url: &str, part: &Path, stop_at: Option<u64>) -> Result<u64> {
        let tag_path = part.with_extension(format!("{}.etag", part.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default()));
        let have = std::fs::metadata(part).map(|m| m.len()).unwrap_or(0);
        let tag = std::fs::read_to_string(&tag_path).ok();
        loop {
            let mut r = self.open()?;
            let mut headers = vec![self.auth()];
            if let (true, Some(t)) = (have > 0, &tag) {
                headers.push(("Range", format!("bytes={have}-")));
                headers.push(("If-Range", t.clone()));
            }
            let head = send(&mut r, "GET", url, &headers, &[])?;
            match head.status {
                200 | 206 => {
                    let from = if head.status == 206 { have } else { 0 };
                    if let Some(d) = part.parent() {
                        std::fs::create_dir_all(d)?;
                    }
                    if let Some(t) = head.header("etag") {
                        std::fs::write(&tag_path, t)?;
                    }
                    let mut f = std::fs::OpenOptions::new().create(true).write(true).truncate(from == 0).append(from > 0).open(part)?;
                    let mut left = head.length;
                    let mut got = 0u64;
                    let mut buf = vec![0u8; 256 * 1024];
                    while left > 0 {
                        let want = (buf.len() as u64).min(left);
                        let want = match stop_at {
                            Some(s) if from + got + want > s => s.saturating_sub(from + got),
                            _ => want,
                        };
                        if want == 0 {
                            return Ok(got);
                        }
                        let n = r.read(&mut buf[..want as usize])?;
                        if n == 0 {
                            bail!("The server stopped sending {url}");
                        }
                        f.write_all(&buf[..n])?;
                        left -= n as u64;
                        got += n as u64;
                    }
                    f.sync_all()?;
                    return Ok(got);
                }
                202 | 429 => {
                    let _ = read_body(&mut r, &head);
                    std::thread::sleep(Duration::from_millis(head.header("retry-after").and_then(|s| s.parse::<u64>().ok()).map(|s| s.min(5) * 1000).unwrap_or(500)));
                }
                416 if have > 0 => {
                    // longer than the file is now: start again
                    let _ = std::fs::remove_file(part);
                    return self.fetch(url, part, stop_at);
                }
                s => {
                    let body = read_body(&mut r, &head).unwrap_or_default();
                    return Err(Refused { status: s, message: problem(s, &body).to_string() }.into());
                }
            }
        }
    }
}
