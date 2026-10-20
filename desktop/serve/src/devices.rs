//! Paired devices and pairing (D91, S3): *Pair a device* shows a six-digit code for ten
//! minutes; a device sending it gets a token, of which the server keeps only the SHA-256.
//! Five wrong codes end the pairing. The devices are kept in `devices.json` in the server's
//! data folder, each with when it was paired and last seen, its address and what it holds.

use crate::tls;
use anyhow::{bail, Result};
use gamerepo_wire::api::{Holding, PairAsk, Paired};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// How long a pairing code works.
pub const CODE_LIFE: Duration = Duration::from_secs(10 * 60);
/// Wrong codes before a pairing ends.
pub const TRIES: u32 = 5;
/// A device's last-seen time is written at most this often.
const SEEN_EVERY: u64 = 60;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub kind: String,
    /// Seconds since 1970
    pub paired: u64,
    pub seen: u64,
    pub address: String,
    /// The token's SHA-256 (the token itself is only on the device)
    pub token: String,
    pub holding: Holding,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct File {
    devices: Vec<Device>,
}

/// A pairing in progress.
struct Pairing {
    code: String,
    until: Instant,
    wrong: u32,
}

pub struct Devices {
    path: PathBuf,
    list: Vec<Device>,
    pairing: Option<Pairing>,
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Why a pairing was refused.
#[derive(Debug, PartialEq)]
pub enum Refused {
    /// No pairing is going on (or it ran out, or ended after too many wrong codes)
    NotPairing,
    Wrong,
    /// That was the last try
    Ended,
}

impl Refused {
    pub fn message(&self) -> &'static str {
        match self {
            Refused::NotPairing => "No device is being paired now: choose Pair a device in Game Repo's Settings and enter the code it shows.",
            Refused::Wrong => "That isn't the code Game Repo shows.",
            Refused::Ended => "That was the fifth wrong code, so the pairing has ended: choose Pair a device again for a new code.",
        }
    }
}

impl Devices {
    pub fn load(dir: &Path) -> Self {
        let path = dir.join("devices.json");
        let list = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice::<File>(&b).ok()).map(|f| f.devices).unwrap_or_default();
        Self { path, list, pairing: None }
    }

    fn save(&self) -> Result<()> {
        let text = serde_json::to_vec_pretty(&File { devices: self.list.clone() })?;
        tls::write_new(&self.path, &text)
    }

    pub fn list(&self) -> &[Device] {
        &self.list
    }

    /// Start pairing (again): a new code, good for ten minutes.
    pub fn start_pairing(&mut self) -> String {
        let code = format!("{:06}", tls::random_below(1_000_000));
        self.pairing = Some(Pairing { code: code.clone(), until: Instant::now() + CODE_LIFE, wrong: 0 });
        code
    }

    pub fn end_pairing(&mut self) {
        self.pairing = None;
    }

    /// The code being shown and how long it has left, if a pairing is going on.
    pub fn pairing(&mut self) -> Option<(String, Duration)> {
        if self.pairing.as_ref().is_some_and(|p| Instant::now() >= p.until) {
            self.pairing = None;
        }
        self.pairing.as_ref().map(|p| (p.code.clone(), p.until.saturating_duration_since(Instant::now())))
    }

    /// A device asks to pair with the code shown: it gets its token, and the pairing ends.
    pub fn pair(&mut self, ask: &PairAsk, address: &str, server: &str) -> Result<std::result::Result<Paired, Refused>> {
        if self.pairing().is_none() {
            return Ok(Err(Refused::NotPairing));
        }
        let p = self.pairing.as_mut().unwrap();
        if !tls::same_secret(ask.code.trim(), &p.code) {
            p.wrong += 1;
            if p.wrong >= TRIES {
                self.pairing = None;
                return Ok(Err(Refused::Ended));
            }
            return Ok(Err(Refused::Wrong));
        }
        self.pairing = None;
        let token = tls::random_hex(32);
        let name = ask.name.trim().chars().take(80).collect::<String>();
        let d = Device {
            id: tls::random_hex(8),
            name: if name.is_empty() { "A device".into() } else { name },
            kind: ask.kind.trim().chars().take(20).collect(),
            paired: now(),
            seen: now(),
            address: address.to_string(),
            token: tls::sha256_hex(token.as_bytes()),
            holding: Holding::default(),
        };
        let id = d.id.clone();
        self.list.push(d);
        self.save()?;
        Ok(Ok(Paired { token, device: id, server: server.to_string() }))
    }

    /// The device a token is for, marked as seen from `address`.
    pub fn by_token(&mut self, token: &str, address: &str) -> Option<Device> {
        let hash = tls::sha256_hex(token.as_bytes());
        let d = self.list.iter_mut().find(|d| tls::same_secret(&d.token, &hash))?;
        let changed = d.address != address || now().saturating_sub(d.seen) >= SEEN_EVERY;
        d.seen = now();
        d.address = address.to_string();
        let found = d.clone();
        if changed {
            let _ = self.save();
        }
        Some(found)
    }

    pub fn set_holding(&mut self, id: &str, h: Holding) -> Result<()> {
        if let Some(d) = self.list.iter_mut().find(|d| d.id == id) {
            d.holding = h;
            self.save()?;
        }
        Ok(())
    }

    /// Remove a device: its token no longer works (C2, S3).
    pub fn remove(&mut self, id: &str) -> Result<()> {
        let before = self.list.len();
        self.list.retain(|d| d.id != id);
        if self.list.len() == before {
            bail!("There's no device {id}.");
        }
        self.save()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_pairs_once_and_five_wrong_ones_end_the_pairing() {
        let dir = std::env::temp_dir().join(format!("gamerepo-serve-devices-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut d = Devices::load(&dir);
        let ask = |code: &str| PairAsk { code: code.into(), name: "Phone".into(), kind: "android".into() };
        assert_eq!(d.pair(&ask("123456"), "192.168.1.5", "srv").unwrap(), Err(Refused::NotPairing));
        let code = d.start_pairing();
        let wrong = if code == "000000" { "000001" } else { "000000" };
        let paired = d.pair(&ask(&code), "192.168.1.5", "srv").unwrap().unwrap();
        assert_eq!(paired.token.len(), 64);
        assert_eq!(d.pair(&ask(&code), "192.168.1.5", "srv").unwrap(), Err(Refused::NotPairing), "a code pairs one device");
        // the token is kept only as its hash, and finds the device
        let saved = std::fs::read_to_string(dir.join("devices.json")).unwrap();
        assert!(!saved.contains(&paired.token));
        assert_eq!(d.by_token(&paired.token, "192.168.1.6").unwrap().id, paired.device);
        assert!(d.by_token("nope", "x").is_none());
        // five wrong codes end it
        let code = d.start_pairing();
        let wrong = if code == wrong { "999999" } else { wrong };
        for _ in 0..4 {
            assert_eq!(d.pair(&ask(wrong), "a", "srv").unwrap(), Err(Refused::Wrong));
        }
        assert_eq!(d.pair(&ask(wrong), "a", "srv").unwrap(), Err(Refused::Ended));
        assert_eq!(d.pair(&ask(&code), "a", "srv").unwrap(), Err(Refused::NotPairing), "the right code is too late");
        // removed: its token no longer works, also after loading again
        d.remove(&paired.device).unwrap();
        assert!(Devices::load(&dir).by_token(&paired.token, "x").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
