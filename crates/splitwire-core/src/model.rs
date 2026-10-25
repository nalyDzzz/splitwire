//! The tunnel model: what the UI edits, what gets saved, and what the service runs.
//!
//! Serialized as camelCase JSON; `apps/desktop/src/lib/types.ts` mirrors these types.

use std::net::IpAddr;

use ipnet::IpNet;
use serde::{Deserialize, Serialize};

use crate::keys;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tunnel {
    pub id: String,
    pub name: String,
    pub interface: Interface,
    pub peer: Peer,
    #[serde(default)]
    pub routing: Routing,
}

/// The local side of the WireGuard tunnel (`[Interface]` in a `.conf`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Interface {
    pub private_key: String,
    pub addresses: Vec<IpNet>,
    #[serde(default)]
    pub dns: Vec<IpAddr>,
    #[serde(default)]
    pub mtu: Option<u16>,
}

/// The remote side of the WireGuard tunnel (`[Peer]` in a `.conf`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Peer {
    pub public_key: String,
    #[serde(default)]
    pub preshared_key: Option<String>,
    /// `host:port`, `ip:port` or `[ipv6]:port`.
    pub endpoint: String,
    /// Destinations that go through the tunnel. Usually `0.0.0.0/0` and `::/0`.
    pub allowed_ips: Vec<IpNet>,
    #[serde(default)]
    pub persistent_keepalive: Option<u16>,
}

/// Which traffic goes through the tunnel.
///
/// A connection is tunneled when its app is in scope (every app, or one of
/// `apps`), its app is not in `bypass_apps`, its destination is inside the
/// peer's allowed IPs, and its destination is not in `bypass_ips`.
/// Bypass rules always win.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Routing {
    pub app_mode: AppMode,
    /// Apps to tunnel when `app_mode` is [`AppMode::Selected`].
    pub apps: Vec<AppRule>,
    /// Apps that never use the tunnel.
    pub bypass_apps: Vec<AppRule>,
    /// Destinations that never use the tunnel.
    pub bypass_ips: Vec<IpNet>,
    pub dns_mode: DnsMode,
}

impl Default for Routing {
    fn default() -> Self {
        Self {
            app_mode: AppMode::Selected,
            apps: Vec::new(),
            bypass_apps: Vec::new(),
            bypass_ips: Vec::new(),
            dns_mode: DnsMode::System,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AppMode {
    /// Every app uses the tunnel, except bypassed ones.
    All,
    /// Only the listed apps use the tunnel.
    Selected,
}

/// Where DNS lookups go. Windows resolves names for every app in one system
/// service, so this can't be decided per app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DnsMode {
    /// Use the DNS servers of the normal network connection.
    System,
    /// Send every lookup to the tunnel's DNS servers, through the tunnel.
    Tunnel,
}

/// Matches processes by their executable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppRule {
    pub kind: AppRuleKind,
    pub value: String,
    /// Full path of an executable this rule was created from. Only used to
    /// show an icon; never used for matching.
    #[serde(default)]
    pub path_hint: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AppRuleKind {
    /// Exact executable name, e.g. `node.exe` (the `.exe` is optional).
    Name,
    /// Exact executable path, e.g. `C:\Tools\node.exe`.
    Path,
    /// Every executable inside a folder, including subfolders.
    Folder,
    /// Executable name contains this text, e.g. `docker`.
    Keyword,
}

impl AppRule {
    pub fn name(value: impl Into<String>) -> Self {
        Self { kind: AppRuleKind::Name, value: value.into(), path_hint: None }
    }

    /// Regular expression (Go RE2 syntax) matched case-insensitively against
    /// the full process path, as sing-box's `process_path_regex` expects.
    pub fn to_path_regex(&self) -> String {
        let value = normalize_rule_value(self.kind, &self.value);
        match self.kind {
            AppRuleKind::Name => {
                let stem = strip_exe(&value);
                format!(r"(?i)(?:^|\\){}\.exe$", escape_regex(stem))
            }
            AppRuleKind::Path => format!(r"(?i)^{}$", escape_regex(&value)),
            AppRuleKind::Folder => format!(r"(?i)^{}\\", escape_regex(&value)),
            AppRuleKind::Keyword => format!(r"(?i)(?:^|\\)[^\\]*{}[^\\]*$", escape_regex(&value)),
        }
    }

    /// Whether two rules match the same thing (case-insensitive).
    pub fn same_target(&self, other: &AppRule) -> bool {
        self.kind == other.kind
            && normalize_rule_value(self.kind, &self.value)
                .eq_ignore_ascii_case(&normalize_rule_value(other.kind, &other.value))
    }

    fn validate(&self, field: &str, errors: &mut Vec<ValidationError>) {
        let value = self.value.trim();
        if value.is_empty() {
            errors.push(ValidationError::new(field, "App rule is empty"));
            return;
        }
        if value.chars().any(char::is_control) {
            errors.push(ValidationError::new(field, "App rule contains control characters"));
        }
        if value.len() > 1024 {
            errors.push(ValidationError::new(field, "App rule is too long"));
        }
        let has_separator = value.contains(['\\', '/']);
        match self.kind {
            AppRuleKind::Name | AppRuleKind::Keyword if has_separator => errors.push(
                ValidationError::new(field, "Use a path or folder rule to match by location"),
            ),
            AppRuleKind::Path | AppRuleKind::Folder if !looks_absolute(value) => errors.push(
                ValidationError::new(field, "Path must be absolute, like C:\\Program Files\\App"),
            ),
            _ => {}
        }
    }
}

fn normalize_rule_value(kind: AppRuleKind, value: &str) -> String {
    let value = value.trim();
    match kind {
        AppRuleKind::Path | AppRuleKind::Folder => {
            value.replace('/', "\\").trim_end_matches('\\').to_string()
        }
        AppRuleKind::Name | AppRuleKind::Keyword => value.to_string(),
    }
}

fn strip_exe(name: &str) -> &str {
    let len = name.len();
    if len > 4 && name.is_char_boundary(len - 4) && name[len - 4..].eq_ignore_ascii_case(".exe") {
        &name[..len - 4]
    } else {
        name
    }
}

fn looks_absolute(path: &str) -> bool {
    let bytes = path.as_bytes();
    let drive = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/');
    drive || path.starts_with(r"\\") || path.starts_with("//")
}

/// Escapes RE2 metacharacters (same set as Go's `regexp.QuoteMeta`).
fn escape_regex(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for ch in text.chars() {
        if r"\.+*?()|[]{}^$".contains(ch) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("{field}: {message}")]
pub struct ValidationError {
    /// camelCase path of the offending field, e.g. `peer.endpoint` or `routing.apps.2`.
    pub field: String,
    pub message: String,
}

impl ValidationError {
    pub fn new(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self { field: field.into(), message: message.into() }
    }
}

/// A `host:port` endpoint split into its parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
}

pub fn parse_endpoint(input: &str) -> Result<Endpoint, String> {
    let input = input.trim();
    let (host, port) = if let Some(rest) = input.strip_prefix('[') {
        let (host, rest) = rest.split_once(']').ok_or("Missing closing ] in IPv6 endpoint")?;
        let port = rest.strip_prefix(':').ok_or("Endpoint needs a port, like [::1]:51820")?;
        (host, port)
    } else {
        let (host, port) = input
            .rsplit_once(':')
            .ok_or("Endpoint needs a port, like vpn.example.com:51820")?;
        if host.contains(':') {
            return Err("Wrap IPv6 endpoints in brackets, like [2001:db8::1]:51820".into());
        }
        (host, port)
    };
    if host.is_empty() {
        return Err("Endpoint is missing a host".into());
    }
    let port: u16 = port.parse().map_err(|_| format!("Invalid port \"{port}\""))?;
    if port == 0 {
        return Err("Port can't be 0".into());
    }
    let is_ip = host.parse::<IpAddr>().is_ok();
    let is_hostname = host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        });
    if !is_ip && !is_hostname {
        return Err(format!("\"{host}\" isn't a valid hostname or IP address"));
    }
    Ok(Endpoint { host: host.to_string(), port })
}

impl Tunnel {
    pub fn new_id() -> String {
        uuid::Uuid::new_v4().to_string()
    }

    pub fn endpoint(&self) -> Result<Endpoint, String> {
        parse_endpoint(&self.peer.endpoint)
    }

    pub fn has_ipv6_address(&self) -> bool {
        self.interface.addresses.iter().any(|a| matches!(a, IpNet::V6(_)))
    }

    pub fn validate(&self) -> Result<(), Vec<ValidationError>> {
        let mut errors = Vec::new();
        let err = |errors: &mut Vec<ValidationError>, field: &str, message: &str| {
            errors.push(ValidationError::new(field, message))
        };

        if self.name.trim().is_empty() {
            err(&mut errors, "name", "Give the tunnel a name");
        } else if self.name.chars().count() > 64 {
            err(&mut errors, "name", "Name is too long (64 characters max)");
        }

        if !keys::is_valid_key(&self.interface.private_key) {
            err(&mut errors, "interface.privateKey", "Private key must be a base64 WireGuard key");
        }
        if self.interface.addresses.is_empty() {
            err(&mut errors, "interface.addresses", "Add at least one address, like 10.8.0.2/32");
        }
        if let Some(mtu) = self.interface.mtu {
            if !(1280..=9000).contains(&mtu) {
                err(&mut errors, "interface.mtu", "MTU must be between 1280 and 9000");
            }
        }

        if !keys::is_valid_key(&self.peer.public_key) {
            err(&mut errors, "peer.publicKey", "Public key must be a base64 WireGuard key");
        }
        if let Some(psk) = &self.peer.preshared_key {
            if !keys::is_valid_key(psk) {
                err(&mut errors, "peer.presharedKey", "Preshared key must be a base64 WireGuard key");
            }
        }
        if let Err(message) = self.endpoint() {
            err(&mut errors, "peer.endpoint", &message);
        }
        if self.peer.allowed_ips.is_empty() {
            err(&mut errors, "peer.allowedIps", "Add at least one allowed IP, like 0.0.0.0/0");
        }

        for (i, rule) in self.routing.apps.iter().enumerate() {
            rule.validate(&format!("routing.apps.{i}"), &mut errors);
        }
        for (i, rule) in self.routing.bypass_apps.iter().enumerate() {
            rule.validate(&format!("routing.bypassApps.{i}"), &mut errors);
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_rule_regex_matches_exe_name_only() {
        let rule = AppRule::name("node");
        assert_eq!(rule.to_path_regex(), r"(?i)(?:^|\\)node\.exe$");
        assert_eq!(AppRule::name("Node.EXE").to_path_regex(), r"(?i)(?:^|\\)Node\.exe$");
        assert_eq!(
            AppRule::name("Beekeeper Studio").to_path_regex(),
            r"(?i)(?:^|\\)Beekeeper Studio\.exe$"
        );
    }

    #[test]
    fn path_and_folder_rules_escape_and_normalize() {
        let path = AppRule { kind: AppRuleKind::Path, value: "C:/Tools (x86)/a+b.exe".into(), path_hint: None };
        assert_eq!(path.to_path_regex(), r"(?i)^C:\\Tools \(x86\)\\a\+b\.exe$");

        let folder = AppRule { kind: AppRuleKind::Folder, value: r"C:\Program Files\JetBrains\".into(), path_hint: None };
        assert_eq!(folder.to_path_regex(), r"(?i)^C:\\Program Files\\JetBrains\\");

        let keyword = AppRule { kind: AppRuleKind::Keyword, value: "docker".into(), path_hint: None };
        assert_eq!(keyword.to_path_regex(), r"(?i)(?:^|\\)[^\\]*docker[^\\]*$");
    }

    #[test]
    fn same_target_ignores_case_and_trailing_slash() {
        let a = AppRule { kind: AppRuleKind::Folder, value: r"C:\Apps\".into(), path_hint: None };
        let b = AppRule { kind: AppRuleKind::Folder, value: "c:/apps".into(), path_hint: None };
        assert!(a.same_target(&b));
        assert!(!AppRule::name("node").same_target(&AppRule::name("python")));
    }

    #[test]
    fn parses_endpoints() {
        assert_eq!(parse_endpoint("203.0.113.10:51820").unwrap().port, 51820);
        assert_eq!(parse_endpoint("vpn.example.com:443").unwrap().host, "vpn.example.com");
        assert_eq!(parse_endpoint("[2001:db8::1]:51820").unwrap().host, "2001:db8::1");
        assert!(parse_endpoint("2001:db8::1:51820").is_err());
        assert!(parse_endpoint("vpn.example.com").is_err());
        assert!(parse_endpoint("bad host:1").is_err());
        assert!(parse_endpoint("host:0").is_err());
    }

    #[test]
    fn validation_reports_fields() {
        let tunnel = Tunnel {
            id: "x".into(),
            name: " ".into(),
            interface: Interface { private_key: "nope".into(), addresses: vec![], dns: vec![], mtu: Some(100) },
            peer: Peer {
                public_key: keys::public_key(&keys::generate_private_key()).unwrap(),
                preshared_key: None,
                endpoint: "nope".into(),
                allowed_ips: vec![],
                persistent_keepalive: None,
            },
            routing: Routing {
                apps: vec![AppRule::name(r"C:\x\node.exe")],
                ..Routing::default()
            },
        };
        let fields: Vec<String> = tunnel.validate().unwrap_err().into_iter().map(|e| e.field).collect();
        assert_eq!(
            fields,
            [
                "name",
                "interface.privateKey",
                "interface.addresses",
                "interface.mtu",
                "peer.endpoint",
                "peer.allowedIps",
                "routing.apps.0"
            ]
        );
    }
}
