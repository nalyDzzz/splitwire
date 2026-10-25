//! Protocol between the desktop app and the service.
//!
//! Transport: the named pipe in [`crate::consts::PIPE_NAME`]. Each connection
//! carries one request and one response, each a single line of JSON.

use serde::{Deserialize, Serialize};

use crate::model::Tunnel;

/// Largest message either side will read.
pub const MAX_MESSAGE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Request {
    Hello,
    GetStatus,
    /// Starts the tunnel, replacing any running one.
    Connect {
        tunnel: Box<Tunnel>,
        #[serde(default)]
        options: ConnectOptions,
    },
    Disconnect,
    GetConnections,
    /// Log entries with `seq` greater than `after`.
    GetLogs { after: u64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Response {
    Hello {
        protocol: u32,
        service_version: String,
        engine_version: Option<String>,
    },
    Status { status: EngineStatus },
    Connections { connections: Vec<Connection> },
    Logs { entries: Vec<LogEntry> },
    Ok,
    Error { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ConnectOptions {
    /// Periodically fetch a test page through the tunnel (see [`Health`]).
    pub health_check: bool,
}

impl Default for ConnectOptions {
    fn default() -> Self {
        Self { health_check: true }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Disconnecting,
    /// The last connect attempt failed or the engine stopped unexpectedly.
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    pub state: ConnectionState,
    pub tunnel_id: Option<String>,
    pub tunnel_name: Option<String>,
    /// Unix milliseconds when the tunnel came up.
    pub connected_at: Option<i64>,
    pub error: Option<String>,
    pub traffic: Traffic,
    /// Whether traffic actually gets through the tunnel. Only meaningful while connected.
    pub health: Health,
}

impl EngineStatus {
    pub fn disconnected() -> Self {
        Self {
            state: ConnectionState::Disconnected,
            tunnel_id: None,
            tunnel_name: None,
            connected_at: None,
            error: None,
            traffic: Traffic::default(),
            health: Health::Unknown,
        }
    }
}

/// Result of periodically sending a test request through the tunnel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Health {
    /// Not connected, or not checked yet.
    Unknown,
    /// Waiting for the first result.
    Checking,
    /// A request made it through and back.
    Ok { latency_ms: u32 },
    /// Requests through the tunnel are failing (no handshake, server down,
    /// or the server doesn't forward traffic).
    Failing { message: String },
    /// Not probed: the check is turned off, or the tunnel only carries some
    /// networks so a public test page proves nothing.
    Unsupported,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Traffic {
    pub tunnel: RouteTraffic,
    pub direct: RouteTraffic,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteTraffic {
    /// Bytes sent since the tunnel came up.
    pub upload: u64,
    /// Bytes received since the tunnel came up.
    pub download: u64,
    /// Bytes per second over the last sample.
    pub upload_rate: u64,
    pub download_rate: u64,
    /// Open connections right now.
    pub connections: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Route {
    Tunnel,
    Direct,
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub id: String,
    pub process_path: Option<String>,
    pub process_name: Option<String>,
    /// `tcp` or `udp`.
    pub network: String,
    /// Domain when known, otherwise IP.
    pub host: String,
    pub port: u16,
    pub route: Route,
    pub upload: u64,
    pub download: u64,
    /// RFC 3339 start time as reported by the engine.
    pub started_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LogSource {
    Service,
    Engine,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    pub seq: u64,
    /// Unix milliseconds.
    pub time: i64,
    pub level: LogLevel,
    pub source: LogSource,
    pub message: String,
}

/// Serializes a message as one line of JSON.
pub fn encode<T: Serialize>(message: &T) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(message).expect("IPC messages always serialize");
    bytes.push(b'\n');
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_format_is_tagged_camel_case() {
        let json = String::from_utf8(encode(&Request::GetLogs { after: 3 })).unwrap();
        assert_eq!(json, "{\"type\":\"getLogs\",\"after\":3}\n");

        let hello = Response::Hello { protocol: 1, service_version: "0.1.0".into(), engine_version: None };
        let value = serde_json::to_value(&hello).unwrap();
        assert_eq!(value["type"], "hello");
        assert_eq!(value["serviceVersion"], "0.1.0");

        let parsed: Request = serde_json::from_str("{\"type\":\"disconnect\"}").unwrap();
        assert!(matches!(parsed, Request::Disconnect));
    }

    #[test]
    fn connect_options_default_to_health_check_on() {
        assert!(ConnectOptions::default().health_check);
        let parsed: ConnectOptions = serde_json::from_str("{}").unwrap();
        assert!(parsed.health_check);
        let parsed: ConnectOptions = serde_json::from_str("{\"healthCheck\":false}").unwrap();
        assert!(!parsed.health_check);
    }
}
