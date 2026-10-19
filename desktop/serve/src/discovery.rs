//! The server announced on the home network (D91): DNS-SD over multicast DNS, service
//! `_gamerepo._tcp`, with the library's name, the server's id, its version and the API's
//! in the TXT record. A device can also be given the address by hand.

use anyhow::Result;
use gamerepo_wire::api::{Info, SERVICE};
use std::collections::HashMap;

pub struct Announced {
    daemon: mdns_sd::ServiceDaemon,
    fullname: String,
}

/// The instance's name: the library's, with the server's id so two of them differ.
fn instance(info: &Info) -> String {
    let name: String = info.library.chars().filter(|c| !c.is_control() && *c != '.').take(40).collect();
    format!("{} ({})", if name.trim().is_empty() { "Game Repo" } else { name.trim() }, &info.server[..info.server.len().min(6)])
}

/// Announce the server on `port`, on every network it's on, until it's dropped.
pub fn announce(info: &Info, port: u16) -> Result<Announced> {
    let daemon = mdns_sd::ServiceDaemon::new()?;
    let txt: HashMap<String, String> = info.txt().into_iter().collect();
    let host = format!("gamerepo-{}.local.", &info.server[..info.server.len().min(12)]);
    let svc = mdns_sd::ServiceInfo::new(SERVICE, &instance(info), &host, "", port, txt)?.enable_addr_auto();
    let fullname = svc.get_fullname().to_string();
    daemon.register(svc)?;
    Ok(Announced { daemon, fullname })
}

impl Drop for Announced {
    fn drop(&mut self) {
        if let Ok(rx) = self.daemon.unregister(&self.fullname) {
            let _ = rx.recv_timeout(std::time::Duration::from_secs(1));
        }
        let _ = self.daemon.shutdown();
    }
}
