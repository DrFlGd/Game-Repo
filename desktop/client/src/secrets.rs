//! Where Game Repo Go keeps its pairing token (D97): Windows Credential Manager, as a
//! generic credential "Game Repo Go/<name>" for the signed-in user. On Linux a file in the
//! client's data folder only its user can read (D108): Game Mode can't unlock a keyring.

use gamerepo_device::app::Secrets;
use std::path::Path;

pub fn store(data: &Path) -> Box<dyn Secrets> {
    #[cfg(windows)]
    {
        let _ = data;
        Box::new(windows::Credentials)
    }
    #[cfg(not(windows))]
    {
        // one an earlier version wrote readable by others too
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(data.join("secrets.json"), std::fs::Permissions::from_mode(0o600));
        }
        Box::new(gamerepo_device::app::FileSecrets(data.join("secrets.json")))
    }
}

#[cfg(windows)]
mod windows {
    use anyhow::{bail, Result};
    use gamerepo_device::app::Secrets;
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::{ERROR_NOT_FOUND, FILETIME};
    use windows_sys::Win32::Security::Credentials::{CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC};

    pub struct Credentials;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain([0]).collect()
    }

    fn target(name: &str) -> Vec<u16> {
        wide(&format!("Game Repo Go/{name}"))
    }

    impl Secrets for Credentials {
        fn get(&self, name: &str) -> Option<String> {
            let t = target(name);
            let mut p: *mut CREDENTIALW = null_mut();
            // SAFETY: t is a NUL-ended UTF-16 string; p is set by CredReadW and freed below
            unsafe {
                if CredReadW(t.as_ptr(), CRED_TYPE_GENERIC, 0, &mut p) == 0 || p.is_null() {
                    return None;
                }
                let c = &*p;
                let blob = if c.CredentialBlob.is_null() { vec![] } else { std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize).to_vec() };
                CredFree(p as *const _);
                String::from_utf8(blob).ok()
            }
        }

        fn set(&self, name: &str, value: &str) -> Result<()> {
            let mut t = target(name);
            let mut user = wide("Game Repo Go");
            let mut blob = value.as_bytes().to_vec();
            let c = CREDENTIALW {
                Flags: 0,
                Type: CRED_TYPE_GENERIC,
                TargetName: t.as_mut_ptr(),
                Comment: null_mut(),
                LastWritten: FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 },
                CredentialBlobSize: blob.len() as u32,
                CredentialBlob: blob.as_mut_ptr(),
                Persist: CRED_PERSIST_LOCAL_MACHINE,
                AttributeCount: 0,
                Attributes: null_mut(),
                TargetAlias: null_mut(),
                UserName: user.as_mut_ptr(),
            };
            // SAFETY: every pointer in c lives until CredWriteW returns
            if unsafe { CredWriteW(&c, 0) } == 0 {
                bail!("Windows Credential Manager didn't keep the pairing: {}", std::io::Error::last_os_error());
            }
            Ok(())
        }

        fn delete(&self, name: &str) -> Result<()> {
            let t = target(name);
            // SAFETY: t is a NUL-ended UTF-16 string
            if unsafe { CredDeleteW(t.as_ptr(), CRED_TYPE_GENERIC, 0) } == 0 {
                let e = std::io::Error::last_os_error();
                if e.raw_os_error() != Some(ERROR_NOT_FOUND as i32) {
                    bail!("Windows Credential Manager didn't forget the pairing: {e}");
                }
            }
            Ok(())
        }
    }
}
