//! Saved tunnels and settings, kept in the app's local data folder.
//!
//! Tunnels contain private keys, so on Windows the file is encrypted with
//! DPAPI for the current user (the same approach as WireGuard for Windows).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use splitwire_core::model::Tunnel;

const TUNNELS_FILE: &str = "tunnels.dat";
const SETTINGS_FILE: &str = "settings.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub close_to_tray: bool,
    pub launch_at_login: bool,
    pub connect_on_launch: bool,
    pub last_tunnel_id: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self { close_to_tray: true, launch_at_login: false, connect_on_launch: false, last_tunnel_id: None }
    }
}

#[derive(Serialize, Deserialize)]
struct TunnelsFile {
    version: u32,
    tunnels: Vec<Tunnel>,
}

pub struct Store {
    dir: PathBuf,
    pub tunnels: Vec<Tunnel>,
    pub settings: Settings,
}

impl Store {
    pub fn load(dir: PathBuf) -> anyhow::Result<Self> {
        fs::create_dir_all(&dir).with_context(|| format!("can't create {}", dir.display()))?;

        let tunnels = match fs::read(dir.join(TUNNELS_FILE)) {
            Ok(bytes) => {
                let plain = protect::unprotect(&bytes).context("can't decrypt saved tunnels")?;
                serde_json::from_slice::<TunnelsFile>(&plain).context("saved tunnels are corrupt")?.tunnels
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e).context("can't read saved tunnels"),
        };

        let settings = match fs::read(dir.join(SETTINGS_FILE)) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(_) => Settings::default(),
        };

        Ok(Self { dir, tunnels, settings })
    }

    pub fn tunnel(&self, id: &str) -> Option<&Tunnel> {
        self.tunnels.iter().find(|t| t.id == id)
    }

    pub fn upsert(&mut self, tunnel: Tunnel) -> anyhow::Result<()> {
        match self.tunnels.iter_mut().find(|t| t.id == tunnel.id) {
            Some(existing) => *existing = tunnel,
            None => self.tunnels.push(tunnel),
        }
        self.save_tunnels()
    }

    pub fn remove(&mut self, id: &str) -> anyhow::Result<()> {
        self.tunnels.retain(|t| t.id != id);
        if self.settings.last_tunnel_id.as_deref() == Some(id) {
            self.settings.last_tunnel_id = None;
            self.save_settings()?;
        }
        self.save_tunnels()
    }

    pub fn save_tunnels(&self) -> anyhow::Result<()> {
        let json = serde_json::to_vec(&TunnelsFile { version: 1, tunnels: self.tunnels.clone() })?;
        write_atomic(&self.dir.join(TUNNELS_FILE), &protect::protect(&json)?)
    }

    pub fn save_settings(&self) -> anyhow::Result<()> {
        write_atomic(&self.dir.join(SETTINGS_FILE), &serde_json::to_vec_pretty(&self.settings)?)
    }
}

/// Writes to a temp file and renames it over the target, so a crash can't
/// leave a half-written file.
fn write_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes).with_context(|| format!("can't write {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("can't replace {}", path.display()))
}

#[cfg(windows)]
mod protect {
    use anyhow::bail;
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    pub fn protect(data: &[u8]) -> anyhow::Result<Vec<u8>> {
        run(data, true)
    }

    pub fn unprotect(data: &[u8]) -> anyhow::Result<Vec<u8>> {
        run(data, false)
    }

    fn run(data: &[u8], encrypt: bool) -> anyhow::Result<Vec<u8>> {
        let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
        let mut output = CRYPT_INTEGER_BLOB::default();
        let result = unsafe {
            if encrypt {
                CryptProtectData(&input, None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut output)
            } else {
                CryptUnprotectData(&input, None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut output)
            }
        };
        if let Err(error) = result {
            bail!("DPAPI failed: {error}");
        }
        let bytes = unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
        unsafe {
            LocalFree(Some(HLOCAL(output.pbData as *mut _)));
        }
        Ok(bytes)
    }
}

#[cfg(not(windows))]
mod protect {
    pub fn protect(data: &[u8]) -> anyhow::Result<Vec<u8>> {
        Ok(data.to_vec())
    }

    pub fn unprotect(data: &[u8]) -> anyhow::Result<Vec<u8>> {
        Ok(data.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_encrypted_tunnels() {
        let dir = std::env::temp_dir().join(format!("splitwire-store-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let text = format!(
            "[Interface]\nPrivateKey={}\nAddress=10.0.0.2/32\n[Peer]\nPublicKey={}\nEndpoint=1.2.3.4:51820\nAllowedIPs=0.0.0.0/0\n",
            splitwire_core::keys::generate_private_key(),
            splitwire_core::keys::public_key(&splitwire_core::keys::generate_private_key()).unwrap()
        );
        let tunnel = splitwire_core::wgconf::parse(&text, "t").unwrap().tunnel;

        let mut store = Store::load(dir.clone()).unwrap();
        store.upsert(tunnel.clone()).unwrap();
        let raw = fs::read(dir.join(TUNNELS_FILE)).unwrap();
        if cfg!(windows) {
            assert!(!String::from_utf8_lossy(&raw).contains(&tunnel.interface.private_key));
        }

        let reloaded = Store::load(dir.clone()).unwrap();
        assert_eq!(reloaded.tunnels, vec![tunnel]);
        let _ = fs::remove_dir_all(&dir);
    }
}
