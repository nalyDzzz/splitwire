//! Generates the sing-box (1.14) config that runs a tunnel.
//!
//! How it works: a TUN adapter captures all traffic, a WireGuard endpoint
//! carries tunneled traffic, and route rules pick between the two by process
//! path and destination. Everything not matched goes out directly.
//!
//! The service generates this itself from a validated [`Tunnel`]; it never
//! accepts raw engine config from clients, because sing-box runs as SYSTEM.

use std::net::{IpAddr, Ipv6Addr, SocketAddr};

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

/// What the engine is started for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineMode {
    /// Route traffic through the tunnel.
    Tunnel,
    /// Kill switch while the tunnel is down: refuse the traffic that would
    /// have used the tunnel and let everything else go direct as usual. Has
    /// no WireGuard endpoint, so it starts without the server or DNS.
    Blocked,
}

#[derive(Debug, Clone)]
pub struct EngineOptions {
    pub mode: EngineMode,
    pub interface_name: String,
    /// Where sing-box serves its Clash-compatible API (for stats and connections).
    pub api_listen: SocketAddr,
    pub api_secret: String,
    /// sing-box log level: trace, debug, info, warn, error.
    pub log_level: String,
    /// Whether this machine has IPv6 internet outside the tunnel.
    pub host_ipv6: bool,
    /// The peer's address as resolved by the service. Used instead of the
    /// endpoint's hostname, so starting the engine never waits on DNS.
    pub peer_address: Option<IpAddr>,
}

pub fn build_config(tunnel: &Tunnel, opts: &EngineOptions) -> Result<Value, String> {
    let blocked = opts.mode == EngineMode::Blocked;
    let routing = &tunnel.routing;
    let endpoints = if blocked { Vec::new() } else { vec![wireguard_endpoint(tunnel, opts.peer_address)?] };

    let mut dns_servers = vec![json!({ "type": "local", "tag": DNS_DIRECT_TAG })];
    let mut dns_rules = Vec::new();
    let mut dns_final = DNS_DIRECT_TAG;
    if routing.dns_mode == DnsMode::Tunnel {
        if let Some(server) = tunnel.interface.dns.first() {
            if blocked {
                // Lookups are meant to go through the tunnel. Refuse them
                // rather than leak every name to the normal network.
                dns_rules.push(json!({ "action": "reject" }));
            } else {
                dns_servers.push(json!({
                    "type": "udp",
                    "tag": DNS_TUNNEL_TAG,
                    "server": server.to_string(),
                    "detour": TUNNEL_TAG,
                }));
                dns_final = DNS_TUNNEL_TAG;
            }
        }
    }

    let mut dns = json!({
        "servers": dns_servers,
        "final": dns_final,
        // Remember which domain each answered IP belongs to, so the
        // connection list can show names instead of bare IPs.
        "reverse_mapping": true,
    });
    if !dns_rules.is_empty() {
        dns["rules"] = json!(dns_rules);
    }
    // With no IPv6 anywhere, stop handing out IPv6 addresses so apps don't
    // try (and wait on) connections that can't work.
    if !opts.host_ipv6 && !tunnel.has_ipv6_address() {
        dns["strategy"] = json!("ipv4_only");
    }

    // Where in-scope traffic goes.
    let to_tunnel = if blocked { json!({ "action": "reject" }) } else { json!({ "outbound": TUNNEL_TAG }) };

    Ok(json!({
        "log": { "level": opts.log_level, "timestamp": false },
        "dns": dns,
        "endpoints": endpoints,
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
            "rules": route_rules(tunnel, opts.host_ipv6, &to_tunnel),
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

fn wireguard_endpoint(tunnel: &Tunnel, peer_address: Option<IpAddr>) -> Result<Value, String> {
    let endpoint = tunnel.endpoint()?;
    let host = peer_address.map_or(endpoint.host, |ip| ip.to_string());

    let mut peer = json!({
        "address": host,
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
    if host.parse::<Ipv6Addr>().is_err() {
        wireguard["inet4_bind_address"] = json!("0.0.0.0");
    }
    Ok(wireguard)
}

/// `to_tunnel` is merged into the rules that pick tunneled traffic: an
/// outbound normally, a reject action in [`EngineMode::Blocked`].
fn route_rules(tunnel: &Tunnel, host_ipv6: bool, to_tunnel: &Value) -> Vec<Value> {
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
    rules.extend(tunnel_rules(tunnel, to_tunnel));
    if !host_ipv6 && tunnel.has_ipv6_address() {
        // Only the tunnel can carry IPv6; anything left would go direct.
        rules.push(reject_ipv6);
    }
    rules
}

/// Rules that pick in-scope traffic and send it where `to_tunnel` says.
fn tunnel_rules(tunnel: &Tunnel, to_tunnel: &Value) -> Vec<Value> {
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
        let mut rule = scoped(json!({ "ip_cidr": nets(&tunneled) }));
        for (key, value) in to_tunnel.as_object().into_iter().flatten() {
            rule[key] = value.clone();
        }
        rules.push(rule);
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
            mode: EngineMode::Tunnel,
            interface_name: "Splitwire".into(),
            api_listen: "127.0.0.1:19090".parse().unwrap(),
            api_secret: "secret".into(),
            log_level: "info".into(),
            host_ipv6: true,
            peer_address: None,
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
            kill_switch: false,
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

    #[test]
    fn resolved_peer_address_replaces_the_hostname() {
        let t = tunnel(Routing::default(), &["10.8.0.2/32"]);
        let v4 = EngineOptions { peer_address: Some("203.0.113.10".parse().unwrap()), ..opts() };
        let config = build_config(&t, &v4).unwrap();
        assert_eq!(config["endpoints"][0]["peers"][0]["address"], "203.0.113.10");
        assert_eq!(config["endpoints"][0]["peers"][0]["port"], 51820);
        assert_eq!(config["endpoints"][0]["inet4_bind_address"], "0.0.0.0");

        // A hostname that only resolves to IPv6 keeps the dual-stack bind.
        let v6 = EngineOptions { peer_address: Some("2001:db8::1".parse().unwrap()), ..opts() };
        let config = build_config(&t, &v6).unwrap();
        assert_eq!(config["endpoints"][0]["peers"][0]["address"], "2001:db8::1");
        assert!(config["endpoints"][0].get("inet4_bind_address").is_none());
    }

    #[test]
    fn blocked_mode_refuses_what_the_tunnel_would_carry() {
        let routing = Routing {
            app_mode: AppMode::Selected,
            apps: vec![AppRule::name("qbittorrent")],
            bypass_apps: vec![AppRule::name("chrome")],
            bypass_ips: vec!["10.0.0.0/8".parse().unwrap()],
            dns_mode: DnsMode::System,
            kill_switch: true,
        };
        let t = tunnel(routing, &["10.8.0.2/24"]);
        let normal = build_config(&t, &opts()).unwrap();
        let blocked = build_config(&t, &EngineOptions { mode: EngineMode::Blocked, ..opts() }).unwrap();

        // No WireGuard endpoint, so nothing can fail to resolve or bind.
        assert_eq!(blocked["endpoints"], json!([]));
        assert!(!blocked.to_string().contains(&format!("\"{TUNNEL_TAG}\"")));

        // Same rules, same order: only the tunnel rule turns into a reject.
        let normal_rules = normal["route"]["rules"].as_array().unwrap();
        let blocked_rules = blocked["route"]["rules"].as_array().unwrap();
        assert_eq!(normal_rules.len(), blocked_rules.len());
        let last = blocked_rules.last().unwrap();
        assert_eq!(last["action"], "reject");
        assert!(last.get("outbound").is_none());
        assert_eq!(last["ip_cidr"], json!(["0.0.0.0/0"]));
        assert_eq!(last["process_path_regex"][0], r"(?i)(?:^|\\)qbittorrent\.exe$");
        for (a, b) in normal_rules.iter().zip(blocked_rules).take(normal_rules.len() - 1) {
            assert_eq!(a, b);
        }
        assert_eq!(blocked["route"]["final"], DIRECT_TAG);
        assert_eq!(blocked["inbounds"], normal["inbounds"]);
        // Normal DNS still works for everything else.
        assert!(blocked["dns"].get("rules").is_none());
    }

    #[test]
    fn blocked_mode_refuses_lookups_meant_for_the_tunnel() {
        let routing = Routing { app_mode: AppMode::All, dns_mode: DnsMode::Tunnel, kill_switch: true, ..Routing::default() };
        let blocked = build_config(&tunnel(routing, &["10.8.0.2/32"]), &EngineOptions { mode: EngineMode::Blocked, ..opts() }).unwrap();
        assert_eq!(blocked["dns"]["rules"], json!([{ "action": "reject" }]));
        assert_eq!(blocked["dns"]["final"], DNS_DIRECT_TAG);
        assert_eq!(blocked["dns"]["servers"].as_array().unwrap().len(), 1);
        let rules = blocked["route"]["rules"].as_array().unwrap();
        assert_eq!(rules.last().unwrap()["action"], "reject");
        assert!(rules.last().unwrap().get("process_path_regex").is_none());
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
                kill_switch: true,
            },
            Routing { app_mode: AppMode::All, dns_mode: DnsMode::Tunnel, ..Routing::default() },
        ];
        let hosts = [
            opts(),
            EngineOptions { host_ipv6: false, ..opts() },
            EngineOptions { peer_address: Some("203.0.113.10".parse().unwrap()), ..opts() },
            EngineOptions { mode: EngineMode::Blocked, ..opts() },
            EngineOptions { mode: EngineMode::Blocked, host_ipv6: false, ..opts() },
        ];
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
