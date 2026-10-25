use std::path::PathBuf;

use anyhow::{bail, Context};

/// Finds sing-box.exe: `SPLITWIRE_SING_BOX`, then next to this executable,
/// then (debug builds only) the repo's `vendor/sing-box/`.
pub fn sing_box_exe() -> anyhow::Result<PathBuf> {
    if let Some(path) = std::env::var_os("SPLITWIRE_SING_BOX") {
        let path = PathBuf::from(path);
        if !path.is_file() {
            bail!("SPLITWIRE_SING_BOX points to {}, which doesn't exist", path.display());
        }
        return Ok(path);
    }

    let exe = std::env::current_exe().context("can't locate the service executable")?;
    let mut candidates = vec![exe.with_file_name("sing-box.exe")];
    if cfg!(debug_assertions) {
        candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vendor/sing-box/sing-box.exe"));
    }
    // Not canonicalized: that yields `\\?\C:\...` paths, which sing-box passes
    // to the Windows Firewall API (which rejects them).
    if let Some(found) = candidates.iter().find(|c| c.is_file()) {
        return Ok(found.clone());
    }
    bail!(
        "sing-box.exe not found (looked in {}). Run `pnpm fetch:engine` or set SPLITWIRE_SING_BOX.",
        candidates.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
    )
}

/// `%ProgramData%\Splitwire\logs`, used when running as a service.
pub fn service_log_dir() -> PathBuf {
    let base = std::env::var_os("ProgramData").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
    base.join(splitwire_core::consts::APP_NAME).join("logs")
}
