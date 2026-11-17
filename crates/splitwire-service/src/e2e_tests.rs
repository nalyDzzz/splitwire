//! End-to-end checks of the generated engine config and the engine
//! supervisor, without admin rights.
//!
//! Runs real sing-box processes: a WireGuard "server" peer, and a client
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
use splitwire_core::ipc::{Blocking, ConnectOptions, ConnectionState, Route};
use splitwire_core::keys;
use splitwire_core::model::{AppMode, DnsMode, Interface, Peer, Routing, Tunnel};
use splitwire_core::singbox::{build_config, EngineMode, EngineOptions, TUNNEL_TAG};

use crate::clash::ClashApi;
use crate::engine::Engine as Supervisor;
use crate::firewall::fake::FakeFirewall;

struct Process(Child);

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A TCP echo server on this machine's LAN address, and a WireGuard server
/// whose peer can reach it.
struct Fixture {
    exe: PathBuf,
    echo: SocketAddr,
    server: Option<Process>,
    tunnel: Tunnel,
}

impl Fixture {
    /// `None` (and a note) when sing-box or a network isn't available.
    fn new() -> Option<Self> {
        let Ok(exe) = crate::paths::sing_box_exe() else {
            eprintln!("skipping: sing-box not found");
            return None;
        };
        let Some(lan_ip) = lan_ipv4() else {
            eprintln!("skipping: no IPv4 network");
            return None;
        };

        // Not on loopback: the tunnel's userspace network stack keeps
        // 127.0.0.0/8 to itself.
        let listener = TcpListener::bind((lan_ip, 0)).unwrap();
        let echo = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
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
        Some(Self { exe, echo, server: Some(server), tunnel })
    }
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

fn socks_inbound(port: u16) -> Value {
    json!({ "type": "mixed", "tag": "socks", "listen": "127.0.0.1", "listen_port": port })
}

fn start(exe: &PathBuf, config: &Value) -> Process {
    let mut child = Command::new(exe)
        .args(["run", "-c", "stdin", "--disable-color"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start sing-box");
    child.stdin.take().unwrap().write_all(config.to_string().as_bytes()).unwrap();
    Process(child)
}

pub(crate) fn wait_for_port(port: u16) {
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
pub(crate) fn try_socks_connect(proxy_port: u16, target: SocketAddr) -> Result<TcpStream, String> {
    let SocketAddr::V4(target) = target else { panic!("IPv4 only") };
    let io = |e: std::io::Error| e.to_string();
    let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, proxy_port)).map_err(io)?;
    stream.set_read_timeout(Some(Duration::from_secs(10))).map_err(io)?;
    stream.write_all(&[5, 1, 0]).map_err(io)?;
    let mut reply = [0u8; 2];
    stream.read_exact(&mut reply).map_err(io)?;
    if reply != [5, 0] {
        return Err(format!("SOCKS auth negotiation failed: {reply:?}"));
    }
    let mut request = vec![5, 1, 0, 1];
    request.extend_from_slice(&target.ip().octets());
    request.extend_from_slice(&target.port().to_be_bytes());
    stream.write_all(&request).map_err(io)?;
    let mut reply = [0u8; 10];
    stream.read_exact(&mut reply).map_err(io)?;
    if reply[1] != 0 {
        return Err(format!("SOCKS connect refused (code {})", reply[1]));
    }
    Ok(stream)
}

fn socks_connect(proxy_port: u16, target: SocketAddr) -> TcpStream {
    try_socks_connect(proxy_port, target).unwrap()
}

/// Sends a line through the proxy and expects it back.
fn echo_through(proxy_port: u16, target: SocketAddr) -> Result<(), String> {
    let mut stream = try_socks_connect(proxy_port, target)?;
    stream.write_all(b"ping\n").map_err(|e| e.to_string())?;
    let mut buf = [0u8; 5];
    stream.read_exact(&mut buf).map_err(|e| format!("no echo: {e}"))?;
    if &buf != b"ping\n" {
        return Err(format!("unexpected echo {buf:?}"));
    }
    Ok(())
}

fn kill_process(pid: u32) {
    let status = Command::new("taskkill").args(["/F", "/PID", &pid.to_string()]).stdout(Stdio::null()).status().unwrap();
    assert!(status.success(), "couldn't kill engine {pid}");
}

async fn wait_until(engine: &Supervisor, what: &str, done: impl Fn(&Supervisor) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !done(engine) {
        assert!(Instant::now() < deadline, "timed out waiting for {what}; status: {:?}", engine.status());
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[test]
fn tcp_round_trips_through_a_real_wireguard_tunnel() {
    let Some(mut fixture) = Fixture::new() else { return };
    let api_port = free_port();
    let socks_port = free_port();
    let mut config = build_config(
        &fixture.tunnel,
        &EngineOptions {
            mode: EngineMode::Tunnel,
            interface_name: "unused".into(),
            api_listen: SocketAddr::from((Ipv4Addr::LOCALHOST, api_port)),
            api_secret: "e2e".into(),
            log_level: "warn".into(),
            host_ipv6: crate::engine::host_has_ipv6(),
            peer_address: None,
        },
    )
    .unwrap();
    // Everything else (endpoint, DNS, route rules, auto_detect_interface)
    // stays exactly as generated.
    config["inbounds"] = json!([socks_inbound(socks_port)]);
    let client = start(&fixture.exe, &config);
    wait_for_port(socks_port);

    let mut stream = socks_connect(socks_port, fixture.echo);
    stream.write_all(b"hello through the tunnel\n").unwrap();
    let mut buf = [0u8; 25];
    stream.read_exact(&mut buf).expect("echo over WireGuard");
    assert_eq!(&buf, b"hello through the tunnel\n");

    let api = ClashApi::new(SocketAddr::from((Ipv4Addr::LOCALHOST, api_port)), "e2e");
    let snapshot = api.connections().unwrap();
    let ours = snapshot
        .connections
        .iter()
        .find(|c| c.port == fixture.echo.port())
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
    fixture.server.take();
    std::thread::sleep(Duration::from_millis(300));
    assert!(api.delay(TUNNEL_TAG, crate::engine::HEALTH_URL, 2000).is_err(), "probe passed with the server down");

    drop(stream);
    drop(client);
}

/// The engine dies twice. The first time it comes straight back. The second
/// time the tunnel can't start for a while, and the kill switch's blocking
/// engine refuses the tunneled app instead of letting it out directly.
#[tokio::test(flavor = "multi_thread")]
async fn supervisor_brings_the_tunnel_back_and_blocks_while_down() {
    let Some(fixture) = Fixture::new() else { return };
    let socks_port = free_port();
    let firewall = FakeFirewall::default();
    let engine = Supervisor::for_tests(fixture.exe.clone(), Box::new(firewall.clone()), socks_inbound(socks_port));
    let echo = fixture.echo;
    let round_trip = move || tokio::task::spawn_blocking(move || echo_through(socks_port, echo));

    let mut tunnel = fixture.tunnel.clone();
    tunnel.routing.kill_switch = true;
    engine.connect(tunnel, ConnectOptions { health_check: false }).await.expect("connect");
    round_trip().await.unwrap().expect("echo through the tunnel");
    let connected_at = engine.status().connected_at;

    // 1. The engine dies and comes back on its own.
    let first = engine.last_engine_pid().unwrap();
    kill_process(first);
    wait_until(&engine, "the restart", |e| {
        e.status().state == ConnectionState::Connected && e.last_engine_pid() != Some(first)
    })
    .await;
    assert!(firewall.engaged() >= 1, "nothing was blocked while the engine was down");
    assert_eq!(firewall.active(), 0, "still blocking after the tunnel came back");
    assert_eq!(engine.status().connected_at, connected_at, "a reconnect isn't a new session");
    round_trip().await.unwrap().expect("echo after the restart");

    // 2. The engine dies and the tunnel won't start for a while.
    engine.fail_next_tunnel_starts(2);
    kill_process(engine.last_engine_pid().unwrap());
    wait_until(&engine, "the blocking engine", |e| {
        let status = e.status();
        status.state == ConnectionState::Reconnecting && status.blocking == Blocking::Apps
    })
    .await;
    let status = engine.status();
    assert!(status.reconnect.is_some() && status.error.is_some(), "status: {status:?}");
    let refused = round_trip().await.unwrap();
    assert!(refused.is_err(), "a tunneled app got out while the tunnel was down");

    wait_until(&engine, "the tunnel to come back", |e| e.status().state == ConnectionState::Connected).await;
    assert_eq!(firewall.active(), 0);
    assert_eq!(engine.status().blocking, Blocking::Off);
    round_trip().await.unwrap().expect("echo after coming back");

    engine.disconnect().await;
    assert_eq!(engine.status().state, ConnectionState::Disconnected);
    assert_eq!(firewall.active(), 0);
}

/// Without the kill switch, a drop is still repaired, and nothing is blocked.
#[tokio::test(flavor = "multi_thread")]
async fn supervisor_reconnects_without_blocking_when_the_kill_switch_is_off() {
    let Some(fixture) = Fixture::new() else { return };
    let socks_port = free_port();
    let firewall = FakeFirewall::default();
    let engine = Supervisor::for_tests(fixture.exe.clone(), Box::new(firewall.clone()), socks_inbound(socks_port));

    engine.connect(fixture.tunnel.clone(), ConnectOptions { health_check: false }).await.expect("connect");
    let first = engine.last_engine_pid().unwrap();
    kill_process(first);
    wait_until(&engine, "the restart", |e| {
        e.status().state == ConnectionState::Connected && e.last_engine_pid() != Some(first)
    })
    .await;
    assert_eq!(firewall.engaged(), 0);
    let waited = engine.logs(0).into_iter().any(|entry| entry.message.starts_with("Trying again"));
    assert!(!waited, "the first drop of a session should be restarted at once");
    let echo = fixture.echo;
    tokio::task::spawn_blocking(move || echo_through(socks_port, echo)).await.unwrap().expect("echo after the restart");

    // A disconnect during a retry wait ends the session cleanly.
    engine.fail_next_tunnel_starts(100);
    kill_process(engine.last_engine_pid().unwrap());
    wait_until(&engine, "a retry wait", |e| {
        e.status().reconnect.is_some_and(|r| r.failures >= 2 && r.retry_at.is_some())
    })
    .await;
    assert_eq!(engine.status().blocking, Blocking::Off);
    engine.disconnect().await;
    assert_eq!(engine.status().state, ConnectionState::Disconnected);
}
