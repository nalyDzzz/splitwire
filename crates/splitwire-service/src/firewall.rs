//! Covers the moments when no engine is running at all, for the kill switch.
//!
//! While sing-box runs, it enforces the kill switch itself (see
//! `EngineMode::Blocked`). But when it stops on its own, its virtual adapter
//! and routes vanish with it, and until a new engine is up, apps would
//! connect over the normal network. During those seconds the service blocks
//! everything with the Windows Filtering Platform, except sing-box (so it can
//! start), loopback and DHCP.
//!
//! The filters live in a dynamic WFP session: Windows removes them when the
//! session closes, including when the service exits or crashes, so a bug here
//! can't leave the machine offline.

use std::path::PathBuf;

/// Blocks all traffic except the engine's. Dropping the guard lifts the block.
pub trait Firewall: Send + Sync {
    fn block_all(&self) -> anyhow::Result<Box<dyn Send>>;
}

/// The real thing, backed by the Windows Filtering Platform.
pub struct Wfp {
    sing_box: PathBuf,
}

impl Wfp {
    pub fn new(sing_box: PathBuf) -> Self {
        Self { sing_box }
    }
}

impl Firewall for Wfp {
    #[cfg(windows)]
    fn block_all(&self) -> anyhow::Result<Box<dyn Send>> {
        Ok(Box::new(wfp::Block::engage(&self.sing_box)?))
    }

    #[cfg(not(windows))]
    fn block_all(&self) -> anyhow::Result<Box<dyn Send>> {
        let _ = &self.sing_box;
        anyhow::bail!("blocking traffic is only supported on Windows")
    }
}

#[cfg(windows)]
mod wfp {
    use std::path::Path;

    use anyhow::{anyhow, Context};
    use windows::core::{GUID, HRESULT, HSTRING, PCWSTR, PWSTR};
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::NetworkManagement::WindowsFilteringPlatform::*;

    /// `RPC_C_AUTHN_WINNT`: authenticate to the local filter engine as ourselves.
    const RPC_C_AUTHN_WINNT: u32 = 10;
    const IPPROTO_UDP: u8 = 17;
    const PERMIT_WEIGHT: u8 = 15;
    const BLOCK_WEIGHT: u8 = 0;

    /// An open dynamic WFP session holding the block filters.
    pub struct Block {
        engine: HANDLE,
    }

    // The handle is only used to close the session.
    unsafe impl Send for Block {}

    impl Block {
        pub fn engage(sing_box: &Path) -> anyhow::Result<Self> {
            let mut name = wide("Splitwire kill switch");
            let session = FWPM_SESSION0 {
                displayData: FWPM_DISPLAY_DATA0 { name: PWSTR(name.as_mut_ptr()), description: PWSTR::null() },
                flags: FWPM_SESSION_FLAG_DYNAMIC,
                ..Default::default()
            };
            let mut engine = HANDLE::default();
            check(
                unsafe { FwpmEngineOpen0(PCWSTR::null(), RPC_C_AUTHN_WINNT, None, Some(&session), &mut engine) },
                "opening the Windows Filtering Platform",
            )?;
            // From here, dropping `block` closes the session and removes
            // whatever was added.
            let block = Self { engine };
            check(unsafe { FwpmTransactionBegin0(engine, 0) }, "starting a filter transaction")?;
            match add_filters(engine, sing_box) {
                Ok(()) => check(unsafe { FwpmTransactionCommit0(engine) }, "committing the filters")?,
                Err(error) => {
                    unsafe { FwpmTransactionAbort0(engine) };
                    return Err(error);
                }
            }
            Ok(block)
        }
    }

    impl Drop for Block {
        fn drop(&mut self) {
            unsafe { FwpmEngineClose0(self.engine) };
        }
    }

    fn add_filters(engine: HANDLE, sing_box: &Path) -> anyhow::Result<()> {
        let mut name = wide("Splitwire kill switch");
        let sublayer_key = random_guid();
        let sublayer = FWPM_SUBLAYER0 {
            subLayerKey: sublayer_key,
            displayData: FWPM_DISPLAY_DATA0 { name: PWSTR(name.as_mut_ptr()), description: PWSTR::null() },
            // Evaluated first, so the block can't be outvoted by a permit
            // somewhere else. (A block in any sublayer beats a plain permit.)
            weight: u16::MAX,
            ..Default::default()
        };
        check(unsafe { FwpmSubLayerAdd0(engine, &sublayer, None) }, "adding the filter sublayer")?;

        let app_id = AppId::of(sing_box)?;
        let loopback = condition(
            FWPM_CONDITION_FLAGS,
            FWP_MATCH_FLAGS_ALL_SET,
            FWP_CONDITION_VALUE0 { r#type: FWP_UINT32, Anonymous: FWP_CONDITION_VALUE0_0 { uint32: FWP_CONDITION_FLAG_IS_LOOPBACK } },
        );
        let engine_app = condition(
            FWPM_CONDITION_ALE_APP_ID,
            FWP_MATCH_EQUAL,
            FWP_CONDITION_VALUE0 { r#type: FWP_BYTE_BLOB_TYPE, Anonymous: FWP_CONDITION_VALUE0_0 { byteBlob: app_id.0 } },
        );
        let udp = condition(
            FWPM_CONDITION_IP_PROTOCOL,
            FWP_MATCH_EQUAL,
            FWP_CONDITION_VALUE0 { r#type: FWP_UINT8, Anonymous: FWP_CONDITION_VALUE0_0 { uint8: IPPROTO_UDP } },
        );
        let port = |field: GUID, port: u16| {
            condition(
                field,
                FWP_MATCH_EQUAL,
                FWP_CONDITION_VALUE0 { r#type: FWP_UINT16, Anonymous: FWP_CONDITION_VALUE0_0 { uint16: port } },
            )
        };

        // (layer, conditions that let DHCP through on that layer)
        let layers = [
            (FWPM_LAYER_ALE_AUTH_CONNECT_V4, [udp, port(FWPM_CONDITION_IP_REMOTE_PORT, 67)]),
            (FWPM_LAYER_ALE_AUTH_CONNECT_V6, [udp, port(FWPM_CONDITION_IP_REMOTE_PORT, 547)]),
            (FWPM_LAYER_ALE_AUTH_RECV_ACCEPT_V4, [udp, port(FWPM_CONDITION_IP_LOCAL_PORT, 68)]),
            (FWPM_LAYER_ALE_AUTH_RECV_ACCEPT_V6, [udp, port(FWPM_CONDITION_IP_LOCAL_PORT, 546)]),
        ];
        for (layer, dhcp) in layers {
            add_filter(engine, layer, sublayer_key, &[loopback], FWP_ACTION_PERMIT, PERMIT_WEIGHT, "Allow loopback")?;
            add_filter(engine, layer, sublayer_key, &[engine_app], FWP_ACTION_PERMIT, PERMIT_WEIGHT, "Allow the engine")?;
            add_filter(engine, layer, sublayer_key, &dhcp, FWP_ACTION_PERMIT, PERMIT_WEIGHT, "Allow DHCP")?;
            add_filter(engine, layer, sublayer_key, &[], FWP_ACTION_BLOCK, BLOCK_WEIGHT, "Block everything else")?;
        }
        Ok(())
    }

    fn add_filter(
        engine: HANDLE,
        layer: GUID,
        sublayer: GUID,
        conditions: &[FWPM_FILTER_CONDITION0],
        action: FWP_ACTION_TYPE,
        weight: u8,
        name: &str,
    ) -> anyhow::Result<()> {
        let mut name_w = wide(&format!("Splitwire kill switch: {name}"));
        let mut conditions = conditions.to_vec();
        let filter = FWPM_FILTER0 {
            displayData: FWPM_DISPLAY_DATA0 { name: PWSTR(name_w.as_mut_ptr()), description: PWSTR::null() },
            layerKey: layer,
            subLayerKey: sublayer,
            weight: FWP_VALUE0 { r#type: FWP_UINT8, Anonymous: FWP_VALUE0_0 { uint8: weight } },
            numFilterConditions: conditions.len() as u32,
            filterCondition: if conditions.is_empty() { std::ptr::null_mut() } else { conditions.as_mut_ptr() },
            action: FWPM_ACTION0 { r#type: action, ..Default::default() },
            ..Default::default()
        };
        check(unsafe { FwpmFilterAdd0(engine, &filter, None, None) }, &format!("adding filter \"{name}\""))
    }

    fn condition(field: GUID, match_type: FWP_MATCH_TYPE, value: FWP_CONDITION_VALUE0) -> FWPM_FILTER_CONDITION0 {
        FWPM_FILTER_CONDITION0 { fieldKey: field, matchType: match_type, conditionValue: value }
    }

    /// The WFP application id of an executable, freed on drop.
    struct AppId(*mut FWP_BYTE_BLOB);

    impl AppId {
        fn of(path: &Path) -> anyhow::Result<Self> {
            // Debug builds find sing-box through `..`; the id has to match
            // the path the process actually runs from.
            let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
            let path = path.as_path();
            let mut blob = std::ptr::null_mut();
            check(
                unsafe { FwpmGetAppIdFromFileName0(&HSTRING::from(path), &mut blob) },
                &format!("getting the application id of {}", path.display()),
            )?;
            Ok(Self(blob))
        }
    }

    impl Drop for AppId {
        fn drop(&mut self) {
            unsafe { FwpmFreeMemory0(&mut self.0 as *mut *mut FWP_BYTE_BLOB as *mut *mut core::ffi::c_void) };
        }
    }

    fn check(code: u32, what: &str) -> anyhow::Result<()> {
        if code == 0 {
            return Ok(());
        }
        // FWP_E_* codes are HRESULTs already; the rest are Win32 errors.
        let hresult = if code & 0x8000_0000 != 0 { HRESULT(code as i32) } else { HRESULT::from_win32(code) };
        Err(anyhow!("{}", windows::core::Error::from(hresult).message())).context(format!("{what} failed (0x{code:08x})"))
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn random_guid() -> GUID {
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).expect("the OS random number generator failed");
        GUID::from_u128(u128::from_le_bytes(bytes))
    }
}

/// Stands in for WFP in tests: records whether a block is in place.
#[cfg(test)]
pub mod fake {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[derive(Clone, Default)]
    pub struct FakeFirewall {
        /// Blocks in place right now.
        pub active: Arc<AtomicUsize>,
        /// Blocks ever put in place.
        pub engaged: Arc<AtomicUsize>,
    }

    impl FakeFirewall {
        pub fn active(&self) -> usize {
            self.active.load(Ordering::SeqCst)
        }

        pub fn engaged(&self) -> usize {
            self.engaged.load(Ordering::SeqCst)
        }
    }

    struct Guard(Arc<AtomicUsize>);

    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    impl super::Firewall for FakeFirewall {
        fn block_all(&self) -> anyhow::Result<Box<dyn Send>> {
            self.active.fetch_add(1, Ordering::SeqCst);
            self.engaged.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(Guard(self.active.clone())))
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use std::io::Write;
    use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
    use std::process::{Command, Stdio};
    use std::time::Duration;

    use super::*;
    use crate::e2e_tests::{try_socks_connect, wait_for_port};

    /// Puts real filters in place, so it needs an elevated terminal:
    /// `cargo test -p splitwire-service -- --ignored wfp`. Disconnect
    /// Splitwire first: an installed copy's engine isn't the one let through.
    ///
    /// While it runs (about 3 seconds), new connections from every app on
    /// this machine are blocked; connections already open aren't affected.
    #[test]
    #[ignore = "needs an elevated terminal and briefly blocks new connections"]
    fn wfp_blocks_everything_but_the_engine_and_lifts_on_drop() {
        assert!(crate::win::is_elevated(), "run this from an elevated terminal");
        let sing_box = crate::paths::sing_box_exe().expect("run `pnpm fetch:engine` first");
        let outside = SocketAddr::from((Ipv4Addr::new(1, 1, 1, 1), 443));
        let reachable = || TcpStream::connect_timeout(&outside, Duration::from_secs(3)).is_ok();
        assert!(reachable(), "needs internet access to 1.1.1.1:443");

        // sing-box with only a local SOCKS port, to ask it to connect out.
        let socks_port = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap().local_addr().unwrap().port();
        let mut engine = Command::new(&sing_box)
            .args(["run", "-c", "stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start sing-box");
        let config = serde_json::json!({
            "log": { "level": "warn" },
            "inbounds": [{ "type": "mixed", "listen": "127.0.0.1", "listen_port": socks_port }],
            "outbounds": [{ "type": "direct", "tag": "direct" }],
            "route": { "auto_detect_interface": true, "final": "direct" },
        });
        engine.stdin.take().unwrap().write_all(config.to_string().as_bytes()).unwrap();
        wait_for_port(socks_port);

        let local = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let block = Wfp::new(sing_box).block_all().expect("engaging the block");
        let others_blocked = !reachable();
        let loopback_open = TcpStream::connect(local.local_addr().unwrap()).is_ok();
        let engine_out = try_socks_connect(socks_port, outside);
        drop(block);
        let lifted = reachable();
        let _ = engine.kill();
        let _ = engine.wait();

        assert!(others_blocked, "a program other than sing-box got out while blocked");
        assert!(loopback_open, "loopback was blocked");
        assert!(engine_out.is_ok(), "sing-box couldn't get out while blocked: {:?}", engine_out.err());
        assert!(lifted, "the block didn't lift");
    }
}
