//! Import and export of WireGuard `.conf` files.
//!
//! Besides the standard fields, import understands the split-tunneling
//! extensions used by WireSock and TunnlTo (`AllowedApps`, `DisallowedApps`,
//! `DisallowedIPs`), so existing TunnlTo tunnels carry over.

use std::fmt::Write as _;
use std::net::IpAddr;

use ipnet::IpNet;
use serde::Serialize;

use crate::model::{AppMode, AppRule, AppRuleKind, DnsMode, Interface, Peer, Routing, Tunnel};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedTunnel {
    pub tunnel: Tunnel,
    /// Things the user should know about, e.g. ignored fields.
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("line {line}: {message}")]
pub struct ParseError {
    pub line: usize,
    pub message: String,
}

#[derive(Clone, Copy, PartialEq)]
enum Section {
    None,
    Interface,
    Peer,
    /// A second or later `[Peer]`, which is skipped.
    ExtraPeer,
}

#[derive(Default)]
struct Draft {
    private_key: Option<String>,
    addresses: Vec<IpNet>,
    dns: Vec<IpAddr>,
    mtu: Option<u16>,
    public_key: Option<String>,
    preshared_key: Option<String>,
    endpoint: Option<String>,
    allowed_ips: Vec<IpNet>,
    keepalive: Option<u16>,
    allowed_apps: Vec<AppRule>,
    disallowed_apps: Vec<AppRule>,
    disallowed_ips: Vec<IpNet>,
}

/// Parses a `.conf` file. `name` becomes the tunnel name (usually the file name).
pub fn parse(text: &str, name: &str) -> Result<ImportedTunnel, ParseError> {
    let mut section = Section::None;
    let mut draft = Draft::default();
    let mut warnings = Vec::new();
    let mut ignored = Vec::<String>::new();
    let mut peers = 0;

    for (index, raw) in text.lines().enumerate() {
        let line_no = index + 1;
        let fail = |message: String| ParseError { line: line_no, message };
        let line = raw.split('#').next().unwrap_or_default().trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }

        if line.starts_with('[') {
            section = match line.to_ascii_lowercase().as_str() {
                "[interface]" => Section::Interface,
                "[peer]" => {
                    peers += 1;
                    if peers == 1 { Section::Peer } else { Section::ExtraPeer }
                }
                other => return Err(fail(format!("Unknown section {other}"))),
            };
            continue;
        }

        let (raw_key, value) = line
            .split_once('=')
            .map(|(k, v)| (k.trim(), v.trim()))
            .ok_or_else(|| fail(format!("Expected \"Key = Value\", got \"{line}\"")))?;
        let key = raw_key.to_ascii_lowercase();

        if section == Section::ExtraPeer {
            continue;
        }

        // Split-tunneling extensions may appear in either section.
        match key.as_str() {
            "allowedapps" => {
                draft.allowed_apps.extend(list(value).map(app_rule_from_wiresock));
                continue;
            }
            "disallowedapps" => {
                draft.disallowed_apps.extend(list(value).map(app_rule_from_wiresock));
                continue;
            }
            "disallowedips" => {
                for item in list(value) {
                    draft.disallowed_ips.push(parse_net(item).map_err(fail)?);
                }
                continue;
            }
            _ => {}
        }

        match (section, key.as_str()) {
            (Section::None, _) => {
                return Err(fail("Settings must be inside [Interface] or [Peer]".into()))
            }
            (Section::Interface, "privatekey") => draft.private_key = Some(value.to_string()),
            (Section::Interface, "address") => {
                for item in list(value) {
                    draft.addresses.push(parse_net(item).map_err(fail)?);
                }
            }
            (Section::Interface, "dns") => {
                for item in list(value) {
                    match item.parse::<IpAddr>() {
                        Ok(ip) => draft.dns.push(ip),
                        Err(_) => warnings.push(format!(
                            "DNS search domain \"{item}\" isn't supported yet and was skipped"
                        )),
                    }
                }
            }
            (Section::Interface, "mtu") => {
                draft.mtu = Some(value.parse().map_err(|_| fail(format!("Invalid MTU \"{value}\"")))?)
            }
            (Section::Peer, "publickey") => draft.public_key = Some(value.to_string()),
            (Section::Peer, "presharedkey") => draft.preshared_key = Some(value.to_string()),
            (Section::Peer, "endpoint") => draft.endpoint = Some(value.to_string()),
            (Section::Peer, "allowedips") => {
                for item in list(value) {
                    draft.allowed_ips.push(parse_net(item).map_err(fail)?);
                }
            }
            (Section::Peer, "persistentkeepalive") => {
                draft.keepalive = if value.eq_ignore_ascii_case("off") {
                    None
                } else {
                    let secs: u16 = value
                        .parse()
                        .map_err(|_| fail(format!("Invalid PersistentKeepalive \"{value}\"")))?;
                    (secs > 0).then_some(secs)
                };
            }
            // Harmless for a client that doesn't listen or run hooks.
            (Section::Interface, "listenport" | "fwmark" | "saveconfig") => {}
            _ => ignored.push(raw_key.to_string()),
        }
    }

    if peers > 1 {
        warnings.push(format!(
            "This config has {peers} peers. Only the first one is used for now."
        ));
    }
    if !ignored.is_empty() {
        ignored.sort();
        ignored.dedup();
        warnings.push(format!("Ignored unsupported settings: {}", ignored.join(", ")));
    }

    let missing = |what: &str| ParseError { line: 0, message: format!("Missing {what}") };
    let private_key = draft.private_key.ok_or_else(|| missing("PrivateKey in [Interface]"))?;
    if peers == 0 {
        return Err(missing("[Peer] section"));
    }
    let public_key = draft.public_key.ok_or_else(|| missing("PublicKey in [Peer]"))?;
    let endpoint = draft.endpoint.ok_or_else(|| missing("Endpoint in [Peer]"))?;

    let app_mode = if draft.allowed_apps.is_empty() { AppMode::All } else { AppMode::Selected };
    for app in &draft.allowed_apps {
        if draft.disallowed_apps.iter().any(|d| d.same_target(app)) {
            warnings.push(format!(
                "\"{}\" is in both AllowedApps and DisallowedApps, so it will bypass the tunnel",
                app.value
            ));
        }
    }
    // Windows resolves DNS for every app in one place, so tunnel DNS can only
    // be all-or-nothing. Default to it when every app is tunneled anyway.
    let dns_mode = if !draft.dns.is_empty() && app_mode == AppMode::All {
        DnsMode::Tunnel
    } else {
        DnsMode::System
    };

    let tunnel = Tunnel {
        id: Tunnel::new_id(),
        name: name.trim().to_string(),
        interface: Interface {
            private_key,
            addresses: draft.addresses,
            dns: draft.dns,
            mtu: draft.mtu,
        },
        peer: Peer {
            public_key,
            preshared_key: draft.preshared_key,
            endpoint,
            allowed_ips: draft.allowed_ips,
            persistent_keepalive: draft.keepalive,
        },
        routing: Routing {
            app_mode,
            apps: draft.allowed_apps,
            bypass_apps: draft.disallowed_apps,
            bypass_ips: draft.disallowed_ips,
            dns_mode,
        },
    };

    Ok(ImportedTunnel { tunnel, warnings })
}

fn list(value: &str) -> impl Iterator<Item = &str> {
    value.split(',').map(str::trim).filter(|s| !s.is_empty())
}

/// Addresses without a prefix length are single hosts.
fn parse_net(value: &str) -> Result<IpNet, String> {
    value
        .parse::<IpNet>()
        .or_else(|_| value.parse::<IpAddr>().map(IpNet::from))
        .map_err(|_| format!("\"{value}\" isn't a valid IP address or CIDR range"))
}

/// WireSock lists apps by executable name without `.exe`, or by full path.
fn app_rule_from_wiresock(value: &str) -> AppRule {
    let value = value.trim();
    if value.contains(['\\', '/']) {
        let is_file = value.to_ascii_lowercase().ends_with(".exe");
        AppRule {
            kind: if is_file { AppRuleKind::Path } else { AppRuleKind::Folder },
            value: value.to_string(),
            path_hint: is_file.then(|| value.to_string()),
        }
    } else {
        let name = if value.to_ascii_lowercase().ends_with(".exe") {
            value.to_string()
        } else {
            format!("{value}.exe")
        };
        AppRule::name(name)
    }
}

/// Writes a standard WireGuard `.conf`. With `include_app_rules`, also writes
/// WireSock-style `AllowedApps`/`DisallowedApps`/`DisallowedIPs` lines.
pub fn to_conf(tunnel: &Tunnel, include_app_rules: bool) -> String {
    let join = |items: Vec<String>| items.join(", ");
    let mut out = String::new();
    let i = &tunnel.interface;
    let p = &tunnel.peer;

    let _ = writeln!(out, "[Interface]");
    let _ = writeln!(out, "PrivateKey = {}", i.private_key);
    let _ = writeln!(out, "Address = {}", join(i.addresses.iter().map(ToString::to_string).collect()));
    if !i.dns.is_empty() {
        let _ = writeln!(out, "DNS = {}", join(i.dns.iter().map(ToString::to_string).collect()));
    }
    if let Some(mtu) = i.mtu {
        let _ = writeln!(out, "MTU = {mtu}");
    }

    let _ = writeln!(out, "\n[Peer]");
    let _ = writeln!(out, "PublicKey = {}", p.public_key);
    if let Some(psk) = &p.preshared_key {
        let _ = writeln!(out, "PresharedKey = {psk}");
    }
    let _ = writeln!(out, "Endpoint = {}", p.endpoint);
    let _ = writeln!(out, "AllowedIPs = {}", join(p.allowed_ips.iter().map(ToString::to_string).collect()));
    if let Some(keepalive) = p.persistent_keepalive {
        let _ = writeln!(out, "PersistentKeepalive = {keepalive}");
    }

    if include_app_rules {
        let wiresock_names = |rules: &[AppRule]| -> Vec<String> {
            rules
                .iter()
                .filter(|r| matches!(r.kind, AppRuleKind::Name | AppRuleKind::Path))
                .map(|r| match r.kind {
                    AppRuleKind::Name if r.value.to_ascii_lowercase().ends_with(".exe") => {
                        r.value[..r.value.len() - 4].to_string()
                    }
                    _ => r.value.clone(),
                })
                .collect()
        };
        let r = &tunnel.routing;
        if r.app_mode == AppMode::Selected && !r.apps.is_empty() {
            let _ = writeln!(out, "AllowedApps = {}", join(wiresock_names(&r.apps)));
        }
        if !r.bypass_apps.is_empty() {
            let _ = writeln!(out, "DisallowedApps = {}", join(wiresock_names(&r.bypass_apps)));
        }
        if !r.bypass_ips.is_empty() {
            let _ = writeln!(out, "DisallowedIPs = {}", join(r.bypass_ips.iter().map(ToString::to_string).collect()));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys;

    fn sample(private: &str, public: &str) -> String {
        format!(
            "[Interface]\n\
             PrivateKey = {private}\n\
             Address = 10.8.0.2/24\n\
             DNS = 1.1.1.1, corp.example\n\
             \n\
             [Peer]\n\
             PublicKey = {public}\n\
             PresharedKey = {public}\n\
             Endpoint = 203.0.113.10:51820\n\
             PersistentKeepalive = 0\n\
             AllowedApps = webstorm64, node, Beekeeper Studio, C:\\Tools\\go.exe, httpd\n\
             AllowedIPs = 0.0.0.0/0, ::/0\n\
             DisallowedApps = chrome, httpd\n\
             DisallowedIPs = 10.0.0.5\n\
             PostUp = echo hi # comment\n"
        )
    }

    #[test]
    fn imports_tunnlto_config() {
        let private = keys::generate_private_key();
        let public = keys::public_key(&keys::generate_private_key()).unwrap();
        let imported = parse(&sample(&private, &public), "wg").unwrap();
        let t = &imported.tunnel;

        assert_eq!(t.name, "wg");
        assert_eq!(t.interface.addresses[0].to_string(), "10.8.0.2/24");
        assert_eq!(t.interface.dns, vec!["1.1.1.1".parse::<IpAddr>().unwrap()]);
        assert_eq!(t.peer.endpoint, "203.0.113.10:51820");
        assert_eq!(t.peer.persistent_keepalive, None);
        assert_eq!(t.peer.allowed_ips.len(), 2);
        assert_eq!(t.routing.app_mode, AppMode::Selected);
        assert_eq!(t.routing.dns_mode, DnsMode::System);

        let apps: Vec<_> = t.routing.apps.iter().map(|a| (a.kind, a.value.as_str())).collect();
        assert_eq!(
            apps,
            [
                (AppRuleKind::Name, "webstorm64.exe"),
                (AppRuleKind::Name, "node.exe"),
                (AppRuleKind::Name, "Beekeeper Studio.exe"),
                (AppRuleKind::Path, r"C:\Tools\go.exe"),
                (AppRuleKind::Name, "httpd.exe"),
            ]
        );
        assert_eq!(t.routing.bypass_apps.len(), 2);
        assert_eq!(t.routing.bypass_ips[0].to_string(), "10.0.0.5/32");

        let warnings = imported.warnings.join("\n");
        assert!(warnings.contains("corp.example"), "{warnings}");
        assert!(warnings.contains("httpd.exe\" is in both"), "{warnings}");
        assert!(warnings.contains("PostUp"), "{warnings}");
        assert!(t.validate().is_ok(), "{:?}", t.validate());
    }

    #[test]
    fn full_tunnel_config_uses_tunnel_dns() {
        let private = keys::generate_private_key();
        let public = keys::public_key(&private).unwrap();
        let text = format!(
            "[Interface]\nPrivateKey={private}\nAddress=10.0.0.2\nDNS=10.0.0.1\n[Peer]\nPublicKey={public}\nEndpoint=vpn.example.com:51820\nAllowedIPs=0.0.0.0/0\n"
        );
        let t = parse(&text, "home").unwrap().tunnel;
        assert_eq!(t.routing.app_mode, AppMode::All);
        assert_eq!(t.routing.dns_mode, DnsMode::Tunnel);
        assert_eq!(t.interface.addresses[0].to_string(), "10.0.0.2/32");
    }

    #[test]
    fn reports_missing_fields_and_bad_lines() {
        assert!(parse("[Interface]\nAddress = 10.0.0.2/32\n", "x").unwrap_err().message.contains("PrivateKey"));
        let err = parse("[Interface]\nnonsense\n", "x").unwrap_err();
        assert_eq!(err.line, 2);
        assert!(parse("PrivateKey = abc\n", "x").is_err());
    }

    #[test]
    fn round_trips_through_to_conf() {
        let private = keys::generate_private_key();
        let public = keys::public_key(&private).unwrap();
        let original = parse(&sample(&private, &public), "wg").unwrap().tunnel;
        let again = parse(&to_conf(&original, true), "wg").unwrap().tunnel;
        assert_eq!(original.interface, again.interface);
        assert_eq!(original.peer, again.peer);
        assert_eq!(original.routing.apps.len(), again.routing.apps.len());
        assert_eq!(original.routing.bypass_ips, again.routing.bypass_ips);
    }
}
