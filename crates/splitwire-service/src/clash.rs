//! Minimal client for sing-box's Clash-compatible API, used for live
//! connections and traffic counters. Blocking; call from `spawn_blocking`.

use std::net::SocketAddr;
use std::time::Duration;

use serde_json::Value;
use splitwire_core::ipc::{Connection, Route};
use splitwire_core::singbox::{DIRECT_TAG, TUNNEL_TAG};

pub struct ClashApi {
    agent: ureq::Agent,
    base: String,
    auth: String,
}

/// One poll of `/connections`.
pub struct Snapshot {
    /// Engine-wide byte counters, including closed connections.
    pub upload_total: u64,
    pub download_total: u64,
    pub connections: Vec<Connection>,
}

impl ClashApi {
    pub fn new(addr: SocketAddr, secret: &str) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(2)))
            .build()
            .into();
        Self { agent, base: format!("http://{addr}"), auth: format!("Bearer {secret}") }
    }

    /// Sends a request to `url` through `outbound` and returns the round trip in ms.
    pub fn delay(&self, outbound: &str, url: &str, timeout_ms: u32) -> anyhow::Result<u32> {
        let mut response = self
            .agent
            .get(format!("{}/proxies/{outbound}/delay", self.base))
            .query("url", url)
            .query("timeout", timeout_ms.to_string())
            .header("Authorization", &self.auth)
            .config()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_millis(u64::from(timeout_ms) + 2000)))
            .build()
            .call()?;
        let body: Value = response.body_mut().read_json()?;
        match body["delay"].as_u64() {
            Some(delay) => Ok(delay as u32),
            None => anyhow::bail!("{}", body["message"].as_str().unwrap_or("no response")),
        }
    }

    pub fn connections(&self) -> anyhow::Result<Snapshot> {
        let body: Value = self
            .agent
            .get(format!("{}/connections", self.base))
            .header("Authorization", &self.auth)
            .call()?
            .body_mut()
            .read_json()?;
        Ok(parse_snapshot(&body))
    }
}

fn parse_snapshot(body: &Value) -> Snapshot {
    let connections = body["connections"]
        .as_array()
        .map(|items| items.iter().filter_map(parse_connection).collect())
        .unwrap_or_default();
    Snapshot {
        upload_total: body["uploadTotal"].as_u64().unwrap_or(0),
        download_total: body["downloadTotal"].as_u64().unwrap_or(0),
        connections,
    }
}

fn parse_connection(item: &Value) -> Option<Connection> {
    let meta = &item["metadata"];
    let text = |v: &Value| v.as_str().filter(|s| !s.is_empty()).map(str::to_string);
    let process_path = text(&meta["processPath"]);
    let process_name = process_path
        .as_deref()
        .and_then(|p| p.rsplit(['\\', '/']).next())
        .map(str::to_string);
    let host = text(&meta["host"]).or_else(|| text(&meta["destinationIP"])).unwrap_or_default();
    // The Clash format sends ports as strings; accept numbers too.
    let port = match &meta["destinationPort"] {
        Value::String(s) => s.parse().unwrap_or(0),
        v => v.as_u64().unwrap_or(0) as u16,
    };
    let chains: Vec<&str> = item["chains"].as_array().map(|c| c.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    let route = if chains.contains(&TUNNEL_TAG) {
        Route::Tunnel
    } else if chains.contains(&DIRECT_TAG) {
        Route::Direct
    } else {
        Route::Other
    };

    Some(Connection {
        id: text(&item["id"])?,
        process_path,
        process_name,
        network: text(&meta["network"]).unwrap_or_else(|| "tcp".into()),
        host,
        port,
        route,
        upload: item["upload"].as_u64().unwrap_or(0),
        download: item["download"].as_u64().unwrap_or(0),
        started_at: text(&item["start"]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_clash_connections() {
        let body = serde_json::json!({
            "uploadTotal": 10, "downloadTotal": 20,
            "connections": [
                {
                    "id": "a",
                    "metadata": {
                        "network": "tcp", "destinationIP": "140.82.112.3", "destinationPort": "443",
                        "host": "github.com", "processPath": "C:\\Program Files\\nodejs\\node.exe"
                    },
                    "upload": 5, "download": 7, "start": "2026-10-25T10:00:00Z", "chains": ["tunnel"]
                },
                {
                    "id": "b",
                    "metadata": { "network": "udp", "destinationIP": "1.1.1.1", "destinationPort": 53, "host": "" },
                    "upload": 1, "download": 1, "chains": ["direct"]
                },
                { "metadata": {} }
            ]
        });
        let snapshot = parse_snapshot(&body);
        assert_eq!(snapshot.upload_total, 10);
        assert_eq!(snapshot.connections.len(), 2);
        let a = &snapshot.connections[0];
        assert_eq!(a.host, "github.com");
        assert_eq!(a.port, 443);
        assert_eq!(a.route, Route::Tunnel);
        assert_eq!(a.process_name.as_deref(), Some("node.exe"));
        let b = &snapshot.connections[1];
        assert_eq!(b.host, "1.1.1.1");
        assert_eq!(b.port, 53);
        assert_eq!(b.route, Route::Direct);
        assert_eq!(b.process_path, None);
    }
}
