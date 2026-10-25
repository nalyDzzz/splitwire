//! Generates the sing-box (1.14) config that runs a tunnel.
//!
//! How it works: a TUN adapter captures all traffic, a WireGuard endpoint
//! carries tunneled traffic, and route rules pick between the two by process
//! path and destination. Everything not matched goes out directly.
//!
//! The service generates this itself from a validated [`Tunnel`]; it never
//! accepts raw engine config from clients, because sing-box runs as SYSTEM.

use std::net::SocketAddr;

use ipnet::IpNet;
use serde_json::{json, Value};

use crate::consts::{TUN_ADDRESS_V4, TUN_ADDRESS_V6};
use crate::model::{AppMode, AppRule, DnsMode, Tunnel};

/// Outbound tags. The service reads these back from the engine's connection list.
pub const TUNNEL_TAG: &str = "tunnel";
pub const DIRECT_TAG: &str = "direct";

const DNS_DIRECT_TAG: &str = "dns-direct";
const DNS_TUNNEL_TAG: &str = "dns-tunnel";
const DEFAULT_WG_MTU: u16 = 1420;

#[derive(Debug, Clone)]
pub struct EngineOptions {
    pub interface_name: String,
    /// Where sing-box serves its Clash-compatible API (for stats and connections).
    pub api_listen: SocketAddr,
    pub api_secret: String,
    /// sing-box log level: trace, debug, info, warn, error.
    pub log_level: String,
    /// Whether this machine has IPv6 internet outside the tunnel.
    pub host_ipv6: bool,
}

pub fn build_config(tunnel: &Tunnel, opts: &EngineOptions) -> Result<Value, String> {
    let endpoint = tunnel.endpoint()?;
    let routing = &tunnel.routing;

    let mut peer = json!({
        "address": endpoint.host,
        "port": endpoint.port,
        "public_key": tunnel.peer.public_key.trim(),
        "allowed_ips": nets(&tunnel.peer.allowed_ips),
    });
    if let Some(psk) = &tunnel.peer.preshared_key {
        peer["pre_shared_key"] = json!(psk.trim());
    }
    if let Some(keepalive) = tunnel.peer.persistent_keepalive.filter(|k| *k > 0) {
        peer["persistent_keepalive_interval"] = json!(keepalive);
    }

    let mut wireguard = json!({
        "type": "wireguard",
        "tag": TUNNEL_TAG,
        "mtu": tunnel.interface.mtu.unwrap_or(DEFAULT_WG_MTU),
        "address": nets(&tunnel.interface.addresses),
        "private_key": tunnel.interface.private_key.trim(),
        "peers": [peer],
    });
    // With auto_detect_interface on, sing-box binds the WireGuard socket to
    // the default adapter as dual-stack. On adapters without IPv6 that bind
    // fails, the endpoint stays down, and the handshake never happens. Pin
    // the socket to IPv4 unless the peer is an IPv6 address.
    if endpoint.host.parse::<std::net::Ipv6Addr>().is_err() {
        wireguard["inet4_bind_address"] = json!("0.0.0.0");
    }

    let mut dns_servers = vec![json!({ "type": "local", "tag": DNS_DIRECT_TAG })];
    let mut dns_final = DNS_DIRECT_TAG;
    if routing.dns_mode == DnsMode::Tunnel {
        if let Some(server) = tunnel.interface.dns.first() {
            dns_servers.push(json!({
                "type": "udp",
                "tag": DNS_TUNNEL_TAG,
                "server": server.to_string(),
                "detour": TUNNEL_TAG,
            }));
            dns_final = DNS_TUNNEL_TAG;
        }
    }

    let mut dns = json!({
        "servers": dns_servers,
        "final": dns_final,
        // Remember which domain each answered IP belongs to, so the
        // connection list can show names instead of bare IPs.
        "reverse_mapping": true,
    });
    // With no IPv6 anywhere, stop handing out IPv6 addresses so apps don't
    // try (and wait on) connections that can't work.
    if !opts.host_ipv6 && !tunnel.has_ipv6_address() {
        dns["strategy"] = json!("ipv4_only");
    }

    Ok(json!({
        "log": { "level": opts.log_level, "timestamp": false },
        "dns": dns,
        "endpoints": [wireguard],
        "inbounds": [{
            "type": "tun",
            "tag": "tun-in",
            "interface_name": opts.interface_name,
            // Always both families: with strict_route, a TUN without IPv6
            // makes sing-box block all IPv6, including ::1 loopback (which
            // breaks local dev servers). IPv-6-less hosts are handled by the
            // route rules and DNS strategy instead.
            "address": [TUN_ADDRESS_V4, TUN_ADDRESS_V6],
            "mtu": 9000,
            "auto_route": true,
            "strict_route": true,
            // The default "mixed" stack hands TCP to Windows through a local
            // listener, which only works if a firewall rule lets it in. When
            // the firewall blocks it, every TCP connection stalls and Windows'
            // dead-gateway detection silently routes around the tunnel. The
            // userspace stack has no such dependency.
            "stack": "gvisor",
        }],
        "outbounds": [{ "type": "direct", "tag": DIRECT_TAG }],
        "route": {
            "auto_detect_interface": true,
            "find_process": true,
            "default_domain_resolver": DNS_DIRECT_TAG,
            "rules": route_rules(tunnel, opts.host_ipv6),
            "final": DIRECT_TAG,
        },
        "experimental": {
            "clash_api": {
                "external_controller": opts.api_listen.to_string(),
                "secret": opts.api_secret,
            },
        },
    }))
}

fn route_rules(tunnel: &Tunnel, host_ipv6: bool) -> Vec<Value> {
    let routing = &tunnel.routing;
    // Refusing straight away (TCP reset / ICMP unreachable) makes apps fall
    // back to IPv4 at once, instead of waiting on a connection that can't work.
    let reject_ipv6 = json!({ "ip_version": 6, "action": "reject" });
    let mut rules = vec![
        // Only client-first protocols, so server-first ones (MySQL, SSH...)
        // aren't delayed waiting for the sniffer. Used for display only.
        json!({ "port": [80, 443], "action": "sniff", "sniffer": ["http", "tls", "quic"] }),
    ];
    if !host_ipv6 && !tunnel.has_ipv6_address() {
        // Nothing can carry IPv6.
        rules.push(reject_ipv6.clone());
    }

    if !routing.bypass_apps.is_empty() {
        rules.push(json!({ "process_path_regex": regexes(&routing.bypass_apps), "outbound": DIRECT_TAG }));
    }
    if !routing.bypass_ips.is_empty() {
        rules.push(json!({ "ip_cidr": nets(&routing.bypass_ips), "outbound": DIRECT_TAG }));
    }
    rules.extend(tunnel_rules(tunnel));
    if !host_ipv6 && tunnel.has_ipv6_address() {
        // Only the tunnel can carry IPv6; anything left would go direct.
        rules.push(reject_ipv6);
    }
    rules
}

/// Rules that send in-scope traffic into the tunnel.
fn tunnel_rules(tunnel: &Tunnel) -> Vec<Value> {
    let routing = &tunnel.routing;
    // Which apps are in scope. `None` means every app.
    let scope = match routing.app_mode {
        AppMode::All => None,
        AppMode::Selected if routing.apps.is_empty() => return Vec::new(),
        AppMode::Selected => Some(regexes(&routing.apps)),
    };
    let scoped = |mut rule: Value| {
        if let Some(scope) = &scope {
            rule["process_path_regex"] = json!(scope);
        }
        rule
    };

    let mut rules = Vec::new();
    let (v4, v6): (Vec<IpNet>, Vec<IpNet>) =
        tunnel.peer.allowed_ips.iter().partition(|net| matches!(net, IpNet::V4(_)));
    let mut tunneled = v4;
    if tunnel.has_ipv6_address() {
        tunneled.extend(v6);
    } else if !v6.is_empty() {
        // The tunnel can't carry IPv6, but these apps are meant to be inside it.
        // Refuse IPv6 so they fall back to IPv4 instead of leaking outside.
        rules.push(scoped(json!({ "ip_cidr": nets(&v6), "action": "reject" })));
    }
    if !tunneled.is_empty() {
        rules.push(scoped(json!({ "ip_cidr": nets(&tunneled), "outbound": TUNNEL_TAG })));
    }
    rules
}

fn nets(nets: &[IpNet]) -> Vec<String> {
    nets.iter().map(ToString::to_string).collect()
}

fn regexes(rules: &[AppRule]) -> Vec<String> {
    let mut out: Vec<String> = rules.iter().map(AppRule::to_path_regex).collect();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys;
    use crate::model::{AppRuleKind, Interface, Peer, Routing};

    fn tunnel(routing: Routing, addresses: &[&str]) -> Tunnel {
        let private = keys::generate_private_key();
        Tunnel {
            id: "t".into(),
            name: "wg".into(),
            interface: Interface {
                private_key: private,
                addresses: addresses.iter().map(|a| a.parse().unwrap()).collect(),
                dns: vec!["1.1.1.1".parse().unwrap()],
                mtu: None,
            },
            peer: Peer {
                public_key: keys::public_key(&keys::generate_private_key()).unwrap(),
                preshared_key: None,
                endpoint: "vpn.example.com:51820".into(),
                allowed_ips: vec!["0.0.0.0/0".parse().unwrap(), "::/0".parse().unwrap()],
                persistent_keepalive: Some(25),
            },
            routing,
        }
    }

    fn opts() -> EngineOptions {
        EngineOptions {
            interface_name: "Splitwire".into(),
            api_listen: "127.0.0.1:19090".parse().unwrap(),
            api_secret: "secret".into(),
            log_level: "info".into(),
            host_ipv6: true,
        }
    }

    #[test]
    fn ipv4_only_hosts_refuse_ipv6_instead_of_dropping_it() {
        let reject_v6 = json!({ "ip_version": 6, "action": "reject" });
        let routing = Routing { apps: vec![AppRule::name("node")], ..Routing::default() };
        let v4_tunnel = tunnel(routing.clone(), &["10.8.0.2/24"]);
        let ipv4_host = EngineOptions { host_ipv6: false, ..opts() };

        // The TUN always keeps IPv6 (strict_route would otherwise block ::1).
        let config = build_config(&v4_tunnel, &ipv4_host).unwrap();
        assert_eq!(config["inbounds"][0]["stack"], "gvisor");
        assert_eq!(config["inbounds"][0]["address"], json!([TUN_ADDRESS_V4, TUN_ADDRESS_V6]));
        assert_eq!(config["dns"]["strategy"], "ipv4_only");
        assert_eq!(config["route"]["rules"][1], reject_v6);

        // IPv6-capable host: nothing changes.
        let config = build_config(&v4_tunnel, &opts()).unwrap();
        assert!(config["dns"].get("strategy").is_none());
        assert!(!config["route"]["rules"].as_array().unwrap().contains(&reject_v6));

        // IPv6-capable tunnel on an IPv4-only host: tunneled IPv6 still goes
        // through; only what's left over is refused.
        let v6_tunnel = tunnel(routing, &["10.8.0.2/24", "fd00::2/128"]);
        let config = build_config(&v6_tunnel, &ipv4_host).unwrap();
        let rules = config["route"]["rules"].as_array().unwrap();
        assert!(config["dns"].get("strategy").is_none());
        assert_eq!(rules[rules.len() - 2]["outbound"], TUNNEL_TAG);
        assert_eq!(rules.last().unwrap(), &reject_v6);
    }

    #[test]
    fn selected_apps_without_ipv6_reject_v6_and_tunnel_v4() {
        let routing = Routing {
            app_mode: AppMode::Selected,
            apps: vec![AppRule::name("node.exe")],
            bypass_apps: vec![AppRule::name("chrome")],
            bypass_ips: vec!["10.0.0.0/8".parse().unwrap()],
            dns_mode: DnsMode::System,
        };
        let config = build_config(&tunnel(routing, &["10.8.0.2/24"]), &opts()).unwrap();
        let rules = config["route"]["rules"].as_array().unwrap();

        assert_eq!(rules[1]["outbound"], DIRECT_TAG);
        assert_eq!(rules[1]["process_path_regex"][0], r"(?i)(?:^|\\)chrome\.exe$");
        assert_eq!(rules[2]["ip_cidr"][0], "10.0.0.0/8");
        assert_eq!(rules[3]["action"], "reject");
        assert_eq!(rules[3]["ip_cidr"], json!(["::/0"]));
        assert_eq!(rules[4]["outbound"], TUNNEL_TAG);
        assert_eq!(rules[4]["ip_cidr"], json!(["0.0.0.0/0"]));
        assert_eq!(rules[4]["process_path_regex"][0], r"(?i)(?:^|\\)node\.exe$");
        assert_eq!(config["dns"]["final"], DNS_DIRECT_TAG);
        assert_eq!(config["endpoints"][0]["peers"][0]["address"], "vpn.example.com");
        assert_eq!(config["endpoints"][0]["peers"][0]["persistent_keepalive_interval"], 25);
        assert_eq!(config["endpoints"][0]["inet4_bind_address"], "0.0.0.0");
    }

    #[test]
    fn ipv6_peers_keep_dual_stack_bind() {
        let mut t = tunnel(Routing::default(), &["10.8.0.2/32"]);
        t.peer.endpoint = "[2001:db8::1]:51820".into();
        let config = build_config(&t, &opts()).unwrap();
        assert!(config["endpoints"][0].get("inet4_bind_address").is_none());
        assert_eq!(config["endpoints"][0]["peers"][0]["address"], "2001:db8::1");
    }

    #[test]
    fn all_apps_with_ipv6_tunnels_everything_and_uses_tunnel_dns() {
        let routing = Routing { app_mode: AppMode::All, dns_mode: DnsMode::Tunnel, ..Routing::default() };
        let config = build_config(&tunnel(routing, &["10.8.0.2/32", "fd00::2/128"]), &opts()).unwrap();
        let rules = config["route"]["rules"].as_array().unwrap();

        assert_eq!(rules.len(), 2);
        assert_eq!(rules[1]["ip_cidr"], json!(["0.0.0.0/0", "::/0"]));
        assert!(rules[1].get("process_path_regex").is_none());
        assert_eq!(config["dns"]["final"], DNS_TUNNEL_TAG);
        assert_eq!(config["dns"]["servers"][1]["detour"], TUNNEL_TAG);
    }

    #[test]
    fn selected_mode_with_no_apps_tunnels_nothing() {
        let config = build_config(&tunnel(Routing::default(), &["10.8.0.2/32"]), &opts()).unwrap();
        let rules = config["route"]["rules"].as_array().unwrap();
        assert!(rules.iter().all(|r| r.get("outbound") != Some(&json!(TUNNEL_TAG))));
    }

    #[test]
    fn folder_rules_become_prefix_regexes() {
        let routing = Routing {
            apps: vec![AppRule { kind: AppRuleKind::Folder, value: r"C:\Program Files\JetBrains".into(), path_hint: None }],
            ..Routing::default()
        };
        let config = build_config(&tunnel(routing, &["10.8.0.2/32"]), &opts()).unwrap();
        let last = config["route"]["rules"].as_array().unwrap().last().unwrap().clone();
        assert_eq!(last["process_path_regex"][0], r"(?i)^C:\\Program Files\\JetBrains\\");
    }

    /// Validates generated configs with a real sing-box binary when one is
    /// available (run `pnpm fetch:engine` first). Skipped otherwise.
    #[test]
    fn sing_box_accepts_generated_configs() {
        let exe = std::env::var("SPLITWIRE_SING_BOX").map(std::path::PathBuf::from).unwrap_or_else(|_| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/sing-box/sing-box.exe")
        });
        if !exe.exists() {
            eprintln!("skipping: {} not found", exe.display());
            return;
        }
        let routings = [
            Routing {
                app_mode: AppMode::Selected,
                apps: vec![
                    AppRule::name("node"),
                    AppRule { kind: AppRuleKind::Keyword, value: "docker".into(), path_hint: None },
                    AppRule { kind: AppRuleKind::Folder, value: r"C:\Program Files (x86)\Steam".into(), path_hint: None },
                ],
                bypass_apps: vec![AppRule::name("chrome")],
                bypass_ips: vec!["192.168.0.0/16".parse().unwrap()],
                dns_mode: DnsMode::System,
            },
            Routing { app_mode: AppMode::All, dns_mode: DnsMode::Tunnel, ..Routing::default() },
        ];
        let hosts = [opts(), EngineOptions { host_ipv6: false, ..opts() }];
        for (routing, host) in routings.iter().flat_map(|r| hosts.iter().map(move |h| (r.clone(), h))) {
            let config = build_config(&tunnel(routing, &["10.8.0.2/24"]), host).unwrap();
            let mut child = std::process::Command::new(&exe)
                .args(["check", "-c", "stdin"])
                .stdin(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            use std::io::Write;
            child.stdin.take().unwrap().write_all(config.to_string().as_bytes()).unwrap();
            let output = child.wait_with_output().unwrap();
            assert!(output.status.success(), "sing-box rejected config: {}", String::from_utf8_lossy(&output.stderr));
        }
    }
}
