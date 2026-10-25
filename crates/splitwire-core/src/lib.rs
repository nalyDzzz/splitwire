//! Shared building blocks for Splitwire.
//!
//! - [`model`]: the tunnel model the UI edits and the service runs.
//! - [`wgconf`]: import/export of WireGuard `.conf` files (including WireSock/TunnlTo extensions).
//! - [`singbox`]: turns a [`model::Tunnel`] into a sing-box engine config.
//! - [`ipc`]: the request/response protocol spoken over the service's named pipe.
//! - [`keys`]: WireGuard key generation.

pub mod consts;
pub mod ipc;
pub mod keys;
pub mod model;
pub mod singbox;
pub mod wgconf;

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}
