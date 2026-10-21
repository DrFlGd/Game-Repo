//! Game Repo Go's updates on Android (docs/PLAN.md, "In-app updates", D139): the release
//! named in the update file (`latest-game-repo-go.json`, tools/update_files.py writes it) for
//! this device, and its APK checked against the public key Game Repo Go was built with and
//! the version it was signed for, as the updater on Windows and the Steam Deck checks an
//! installer (minisign: Ed25519 over the file's BLAKE2b, the key and the signature each
//! base64 once more, as the Tauri CLI writes them). The fetching and the installing are
//! Android's (desktop/client/src/android_updates.rs).

use base64::Engine;
use serde_json::Value;

/// A newer release, as its update file says.
#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub version: String,
    /// What's new: the release notes of this version and those before it
    pub notes: String,
    pub date: Option<String>,
    /// This device's APK, and its signature
    pub url: String,
    pub signature: String,
}

/// Is version `a` newer than `b`? (0.5.42 against 0.5.41)
pub fn newer(a: &str, b: &str) -> bool {
    let parts = |v: &str| -> Vec<u64> { v.split(['.', '-', '+']).take(3).map(|p| p.parse().unwrap_or(0)).collect() };
    parts(a) > parts(b)
}

/// The release an update file announces for `platform` ("android-aarch64"), if it's newer
/// than `version`.
pub fn release(file: &str, platform: &str, version: &str) -> Result<Option<Release>, String> {
    let v: Value = serde_json::from_str(file).map_err(|e| format!("The update file isn't what Game Repo publishes ({e})."))?;
    let latest = v["version"].as_str().ok_or("The update file has no version.")?;
    if !newer(latest, version) {
        return Ok(None);
    }
    let p = &v["platforms"][platform];
    let (Some(url), Some(signature)) = (p["url"].as_str(), p["signature"].as_str()) else {
        return Err(format!("Game Repo Go {latest} is out, but not for this device ({platform})."));
    };
    Ok(Some(Release {
        version: latest.to_string(),
        notes: v["notes"].as_str().unwrap_or_default().to_string(),
        date: v["pub_date"].as_str().map(String::from),
        url: url.to_string(),
        signature: signature.to_string(),
    }))
}

fn unwrap_box(b64: &str, what: &str) -> Result<String, String> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64.trim()).map_err(|_| format!("The {what} isn't what the Tauri CLI writes."))?;
    String::from_utf8(bytes).map_err(|_| format!("The {what} isn't what the Tauri CLI writes."))
}

/// Checks a downloaded update against `signature` (its .sig, as the update file carries it)
/// and the public key built in, and that it was signed as `version`.
pub fn verify(data: &[u8], signature: &str, pubkey: &str, version: &str) -> Result<(), String> {
    let key = minisign_verify::PublicKey::decode(&unwrap_box(pubkey, "public key")?).map_err(|e| format!("The public key built in can't be read ({e})."))?;
    let sig = minisign_verify::Signature::decode(&unwrap_box(signature, "signature")?).map_err(|e| format!("The update's signature can't be read ({e})."))?;
    key.verify(data, &sig, false).map_err(|_| "The update doesn't match its signature, so it wasn't installed.".to_string())?;
    let signed = sig.trusted_comment().split('\t').find_map(|f| f.strip_prefix("version:")).unwrap_or_default();
    if signed != version {
        return Err(format!("The update was signed as version {signed}, not {version}, so it wasn't installed."));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn b64(text: &str) -> String {
        base64::engine::general_purpose::STANDARD.encode(text)
    }

    /// A key, and a file signed with it as the Tauri CLI signs (the version in the trusted comment)
    fn signed(data: &[u8], version: &str) -> (String, String, minisign::KeyPair) {
        let kp = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let sig = sign(&kp, data, version);
        (b64(&kp.pk.to_box().unwrap().to_string()), sig, kp)
    }

    fn sign(kp: &minisign::KeyPair, data: &[u8], version: &str) -> String {
        let trusted = format!("timestamp:1760000000\tfile:Game Repo Go_{version}_arm64.apk\tversion:{version}");
        let sig = minisign::sign(Some(&kp.pk), &kp.sk, Cursor::new(data), Some(&trusted), Some("signature from tauri secret key")).unwrap();
        b64(&sig.to_string())
    }

    #[test]
    fn versions_compare_by_number() {
        assert!(newer("0.5.42", "0.5.41"));
        assert!(newer("0.5.100", "0.5.99"));
        assert!(newer("1.0.0", "0.9.9"));
        assert!(!newer("0.5.41", "0.5.41"));
        assert!(!newer("0.5.40", "0.5.41"));
    }

    #[test]
    fn the_update_file_names_this_devices_apk() {
        let file = r###"{"version": "0.5.42", "notes": "## 0.5.42", "pub_date": "2026-10-22T00:00:00Z",
            "platforms": {"android-aarch64": {"url": "https://example.org/Game.Repo.Go_0.5.42_arm64.apk", "signature": "c2ln"},
                          "windows-x86_64": {"url": "https://example.org/setup.exe", "signature": "c2ln"}}}"###;
        let r = release(file, "android-aarch64", "0.5.41").unwrap().unwrap();
        assert_eq!((r.version.as_str(), r.url.as_str(), r.signature.as_str(), r.notes.as_str()), ("0.5.42", "https://example.org/Game.Repo.Go_0.5.42_arm64.apk", "c2ln", "## 0.5.42"));
        assert_eq!(release(file, "android-aarch64", "0.5.42").unwrap(), None);
        assert!(release(file, "android-x86_64", "0.5.41").unwrap_err().contains("not for this device"));
        assert!(release("<html>", "android-aarch64", "0.5.41").is_err());
    }

    #[test]
    fn an_apk_is_checked_against_the_key_and_its_version() {
        let apk = b"an APK".repeat(1000);
        let (pubkey, sig, kp) = signed(&apk, "0.5.42");
        assert_eq!(verify(&apk, &sig, &pubkey, "0.5.42"), Ok(()));
        let mut changed = apk.clone();
        changed.push(b'!');
        assert!(verify(&changed, &sig, &pubkey, "0.5.42").unwrap_err().contains("doesn't match"));
        assert!(verify(&apk, &sig, &pubkey, "0.5.43").unwrap_err().contains("signed as version 0.5.42"));
        let (other, _, _) = signed(&apk, "0.5.42");
        assert!(verify(&apk, &sig, &other, "0.5.42").is_err());
        assert!(verify(&apk, &sign(&kp, &apk, "0.5.41"), &pubkey, "0.5.42").is_err());
        assert!(verify(&apk, "not a signature", &pubkey, "0.5.42").is_err());
    }
}
