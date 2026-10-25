//! Names shared by the service, the desktop app and the installer.
//! Renaming the project mostly means changing this file.

pub const APP_NAME: &str = "Splitwire";

pub const SERVICE_NAME: &str = "SplitwireService";
pub const SERVICE_DISPLAY_NAME: &str = "Splitwire Tunnel Service";
pub const SERVICE_DESCRIPTION: &str =
    "Runs Splitwire tunnels and routes the apps you choose through them.";

/// Named pipe the service listens on. Interactive users may connect.
pub const PIPE_NAME: &str = r"\\.\pipe\splitwire-service";

/// Bumped whenever the IPC protocol changes incompatibly.
pub const PROTOCOL_VERSION: u32 = 1;

/// Name of the virtual adapter that captures traffic while a tunnel is up.
pub const TUN_INTERFACE_NAME: &str = "Splitwire";

/// Addresses for the virtual adapter. 198.18.0.0/15 is reserved for benchmarking
/// (RFC 2544), so it won't collide with home, Docker or WSL networks. The IPv6
/// address is a unique local address.
pub const TUN_ADDRESS_V4: &str = "198.18.0.1/30";
pub const TUN_ADDRESS_V6: &str = "fdfe:dcba:9876::1/126";
