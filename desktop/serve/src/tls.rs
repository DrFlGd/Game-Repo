//! The server's certificate (D91): made the first time (ECDSA P-256), kept in its data
//! folder, and served with rustls. A device pins it at pairing by its SHA-256, so later
//! connections can't be listened to or answered by another machine.

use anyhow::{anyhow, Context, Result};
use ring::rand::SecureRandom;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use sha2::Digest;
use std::path::Path;
use std::sync::Arc;

pub struct Identity {
    pub config: Arc<rustls::ServerConfig>,
    /// The certificate's SHA-256, as a device pins it
    pub fingerprint: String,
}

/// The certificate in `dir` (`cert.pem` and `key.pem`), made the first time.
pub fn identity(dir: &Path) -> Result<Identity> {
    let (cert_path, key_path) = (dir.join("cert.pem"), dir.join("key.pem"));
    if !cert_path.is_file() || !key_path.is_file() {
        std::fs::create_dir_all(dir)?;
        let made = rcgen::generate_simple_self_signed(vec!["gamerepo-server".to_string(), "localhost".to_string()])?;
        write_new(&key_path, made.signing_key.serialize_pem().as_bytes())?;
        write_new(&cert_path, made.cert.pem().as_bytes())?;
    }
    let cert = CertificateDer::from_pem_file(&cert_path).map_err(|e| anyhow!("{} can't be read: {e}", cert_path.display()))?;
    let key = PrivateKeyDer::from_pem_file(&key_path).map_err(|e| anyhow!("{} can't be read: {e}", key_path.display()))?;
    let fingerprint = hex::encode(sha2::Sha256::digest(cert.as_ref()));
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .context("the server's certificate can't be used")?;
    Ok(Identity { config: Arc::new(config), fingerprint })
}

/// A file written whole through a temporary name (the key readable only by its owner where
/// that can be said).
pub fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).with_context(|| format!("couldn't write {}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// `n` random bytes, as hex.
pub fn random_hex(n: usize) -> String {
    let mut b = vec![0u8; n];
    ring::rand::SystemRandom::new().fill(&mut b).expect("the system's random numbers");
    hex::encode(b)
}

/// A random number below `n`.
pub fn random_below(n: u32) -> u32 {
    let mut b = [0u8; 8];
    ring::rand::SystemRandom::new().fill(&mut b).expect("the system's random numbers");
    (u64::from_le_bytes(b) % n as u64) as u32
}

/// SHA-256, as hex.
pub fn sha256_hex(s: &[u8]) -> String {
    hex::encode(sha2::Sha256::digest(s))
}

/// Two secrets compared in the same time whatever they hold.
pub fn same_secret(a: &str, b: &str) -> bool {
    let (x, y) = (sha2::Sha256::digest(a.as_bytes()), sha2::Sha256::digest(b.as_bytes()));
    x.iter().zip(y.iter()).fold(0u8, |d, (p, q)| d | (p ^ q)) == 0
}
