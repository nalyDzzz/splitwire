//! End-to-end check of the generated engine config, without admin rights.
//!
//! Runs two real sing-box processes: a WireGuard "server" peer, and a client
//! using Splitwire's generated config with the TUN inbound swapped for a
//! local SOCKS inbound. A TCP echo through the client must travel over a
//! real WireGuard handshake, and the Clash API must report it as tunneled.
//!
//! Skipped when sing-box isn't available (run `pnpm fetch:engine`).

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use splitwire_core::ipc::Route;
use splitwire_core::keys;
use splitwire_core::model::{AppMode, DnsMode, Interface, Peer, Routing, Tunnel};
use splitwire_core::singbox::{build_config, EngineOptions, TUNNEL_TAG};

use crate::clash::ClashApi;

struct Engine(Child);

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn sing_box() -> Option<PathBuf> {
    crate::paths::sing_box_exe().ok()
}

/// The address of the default-route adapter. No packets are sent.
fn lan_ipv4() -> Option<Ipv4Addr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
    match socket.local_addr().ok()? {
        SocketAddr::V4(addr) if !addr.ip().is_loopback() && !addr.ip().is_unspecified() => Some(*addr.ip()),
        _ => None,
    }
}

fn free_port() -> u16 {
    TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap().local_addr().unwrap().port()
}

fn free_udp_port() -> u16 {
    UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).unwrap().local_addr().unwrap().port()
}

fn start(exe: &PathBuf, config: &Value) -> Engine {
    let mut child = Command::new(exe)
        .args(["run", "-c", "stdin", "--disable-color"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start sing-box");
    child.stdin.take().unwrap().write_all(config.to_string().as_bytes()).unwrap();
    Engine(child)
}

fn wait_for_port(port: u16) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if TcpStream::connect((Ipv4Addr::LOCALHOST, port)).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("port {port} never opened");
}

/// SOCKS5 CONNECT to `target` through the proxy on `proxy_port`.
fn socks_connect(proxy_port: u16, target: SocketAddr) -> TcpStream {
    let SocketAddr::V4(target) = target else { panic!("IPv4 only") };
    let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, proxy_port)).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    stream.write_all(&[5, 1, 0]).unwrap();
    let mut reply = [0u8; 2];
    stream.read_exact(&mut reply).unwrap();
    assert_eq!(reply, [5, 0], "SOCKS auth negotiation failed");
    let mut request = vec![5, 1, 0, 1];
    request.extend_from_slice(&target.ip().octets());
    request.extend_from_slice(&target.port().to_be_bytes());
    stream.write_all(&request).unwrap();
    let mut reply = [0u8; 10];
    stream.read_exact(&mut reply).unwrap();
    assert_eq!(reply[1], 0, "SOCKS connect refused (code {})", reply[1]);
    stream
}

#[test]
fn tcp_round_trips_through_a_real_wireguard_tunnel() {
    let Some(exe) = sing_box() else {
        eprintln!("skipping: sing-box not found");
        return;
    };
    let Some(lan_ip) = lan_ipv4() else {
        eprintln!("skipping: no IPv4 network");
        return;
    };

    // Echo server the far side of the tunnel will dial. Not on loopback: the
    // tunnel's userspace network stack keeps 127.0.0.0/8 to itself.
    let echo = TcpListener::bind((lan_ip, 0)).unwrap();
    let echo_addr = echo.local_addr().unwrap();
    std::thread::spawn(move || {
        for mut stream in echo.incoming().flatten() {
            std::thread::spawn(move || {
                let mut buf = [0u8; 1024];
                while let Ok(n) = stream.read(&mut buf) {
                    if n == 0 || stream.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
            });
        }
    });

    let server_key = keys::generate_private_key();
    let client_key = keys::generate_private_key();
    let wg_port = free_udp_port();

    let server = start(
        &exe,
        &json!({
            "log": { "level": "warn" },
            "endpoints": [{
                "type": "wireguard", "tag": "wg-server", "address": ["10.99.0.1/24"],
                "private_key": server_key, "listen_port": wg_port,
                "peers": [{ "public_key": keys::public_key(&client_key).unwrap(), "allowed_ips": ["10.99.0.2/32"] }],
            }],
            "outbounds": [{ "type": "direct", "tag": "direct" }],
            "route": { "final": "direct" },
        }),
    );

    let tunnel = Tunnel {
        id: "e2e".into(),
        name: "e2e".into(),
        interface: Interface {
            private_key: client_key,
            addresses: vec!["10.99.0.2/24".parse().unwrap()],
            dns: vec![],
            mtu: None,
        },
        peer: Peer {
            public_key: keys::public_key(&server_key).unwrap(),
            preshared_key: None,
            endpoint: format!("{lan_ip}:{wg_port}"),
            allowed_ips: vec!["0.0.0.0/0".parse().unwrap()],
            persistent_keepalive: Some(25),
        },
        routing: Routing { app_mode: AppMode::All, dns_mode: DnsMode::System, ..Routing::default() },
    };
    let api_port = free_port();
    let socks_port = free_port();
    let mut config = build_config(
        &tunnel,
        &EngineOptions {
            interface_name: "unused".into(),
            api_listen: SocketAddr::from((Ipv4Addr::LOCALHOST, api_port)),
            api_secret: "e2e".into(),
            log_level: "warn".into(),
            host_ipv6: crate::engine::host_has_ipv6(),
        },
    )
    .unwrap();
    // Everything else (endpoint, DNS, route rules, auto_detect_interface)
    // stays exactly as generated.
    config["inbounds"] = json!([{ "type": "mixed", "tag": "socks", "listen": "127.0.0.1", "listen_port": socks_port }]);
    let client = start(&exe, &config);
    wait_for_port(socks_port);

    let mut stream = socks_connect(socks_port, echo_addr);
    stream.write_all(b"hello through the tunnel\n").unwrap();
    let mut buf = [0u8; 25];
    stream.read_exact(&mut buf).expect("echo over WireGuard");
    assert_eq!(&buf, b"hello through the tunnel\n");

    let api = ClashApi::new(SocketAddr::from((Ipv4Addr::LOCALHOST, api_port)), "e2e");
    let snapshot = api.connections().unwrap();
    let ours = snapshot
        .connections
        .iter()
        .find(|c| c.port == echo_addr.port())
        .expect("connection listed by the engine API");
    assert_eq!(ours.route, Route::Tunnel);
    assert!(ours.upload >= 25 && ours.download >= 25, "byte counters: {ours:?}");

    // The health probe the service runs: an HTTP request through the tunnel.
    // Needs internet access, so a failure here is reported, not fatal.
    match api.delay(TUNNEL_TAG, crate::engine::HEALTH_URL, crate::engine::HEALTH_TIMEOUT_MS) {
        Ok(ms) => eprintln!("health probe through the tunnel: {ms} ms"),
        Err(error) => eprintln!("health probe failed (offline?): {error:#}"),
    }

    // With the server gone, the probe must fail rather than report healthy.
    drop(server);
    std::thread::sleep(Duration::from_millis(300));
    assert!(api.delay(TUNNEL_TAG, crate::engine::HEALTH_URL, 2000).is_err(), "probe passed with the server down");

    drop(stream);
    drop(client);
}
