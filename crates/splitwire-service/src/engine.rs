//! Supervises the sing-box process: start, stop, crash detection, traffic
//! stats, and a health probe through the tunnel.

use std::collections::HashMap;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context};
use splitwire_core::consts::TUN_INTERFACE_NAME;
use splitwire_core::ipc::{
    ConnectionState, Connection, EngineStatus, Health, LogEntry, LogLevel, LogSource, Route, RouteTraffic, Traffic,
};
use splitwire_core::model::Tunnel;
use splitwire_core::now_ms;
use splitwire_core::singbox::{self, EngineOptions, TUNNEL_TAG};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;

use crate::clash::{ClashApi, Snapshot};
use crate::logs::{parse_engine_line, LogBuffer};

const START_TIMEOUT: Duration = Duration::from_secs(20);
const STOP_TIMEOUT: Duration = Duration::from_secs(5);
const STATS_INTERVAL: Duration = Duration::from_secs(1);
const READY_MARKER: &str = "sing-box started";

/// Fetched through the tunnel to prove traffic gets through. Returns an
/// empty 204, the same check browsers use for captive portals.
pub(crate) const HEALTH_URL: &str = "http://cp.cloudflare.com/generate_204";
pub(crate) const HEALTH_TIMEOUT_MS: u32 = 5000;
const HEALTH_INTERVAL: Duration = Duration::from_secs(30);
const HEALTH_RETRY: Duration = Duration::from_secs(5);
/// Consecutive failures before reporting the tunnel as failing.
const HEALTH_FAILURES: u32 = 2;

pub struct Engine {
    sing_box: PathBuf,
    engine_version: Option<String>,
    logs: Arc<LogBuffer>,
    /// Serializes connect/disconnect.
    control: tokio::sync::Mutex<Option<Running>>,
    shared: Arc<Shared>,
}

/// State read by many tasks; kept behind cheap std mutexes.
struct Shared {
    status: Mutex<EngineStatus>,
    connections: Mutex<Vec<Connection>>,
    /// Incremented per start so stale tasks can't clobber newer state.
    generation: Mutex<u64>,
}

struct Running {
    pid: u32,
    stopping: Arc<AtomicBool>,
    exited: watch::Receiver<bool>,
    kill: Option<oneshot::Sender<()>>,
    /// Stats and health tasks, aborted on stop.
    tasks: Vec<JoinHandle<()>>,
    #[cfg(windows)]
    _job: crate::win::KillOnCloseJob,
}

impl Engine {
    pub fn new(sing_box: PathBuf) -> Self {
        let engine_version = read_engine_version(&sing_box);
        Self {
            sing_box,
            engine_version,
            logs: Arc::new(LogBuffer::new()),
            control: tokio::sync::Mutex::new(None),
            shared: Arc::new(Shared {
                status: Mutex::new(EngineStatus::disconnected()),
                connections: Mutex::new(Vec::new()),
                generation: Mutex::new(0),
            }),
        }
    }

    pub fn sing_box_path(&self) -> &Path {
        &self.sing_box
    }

    pub fn engine_version(&self) -> Option<&str> {
        self.engine_version.as_deref()
    }

    pub fn status(&self) -> EngineStatus {
        self.shared.status.lock().unwrap().clone()
    }

    pub fn connections(&self) -> Vec<Connection> {
        self.shared.connections.lock().unwrap().clone()
    }

    pub fn logs(&self, after: u64) -> Vec<LogEntry> {
        self.logs.since(after)
    }

    pub async fn connect(&self, tunnel: Tunnel) -> Result<(), String> {
        if let Err(errors) = tunnel.validate() {
            let message = errors.iter().map(ToString::to_string).collect::<Vec<_>>().join("; ");
            return Err(format!("Invalid tunnel: {message}"));
        }

        let mut control = self.control.lock().await;
        if let Some(running) = control.take() {
            self.stop(running).await;
        }

        let generation = {
            let mut generation = self.shared.generation.lock().unwrap();
            *generation += 1;
            *generation
        };
        self.set_status(|s| {
            *s = EngineStatus {
                state: ConnectionState::Connecting,
                tunnel_id: Some(tunnel.id.clone()),
                tunnel_name: Some(tunnel.name.clone()),
                ..EngineStatus::disconnected()
            }
        });
        self.logs.info(format!("Connecting \"{}\" ({})", tunnel.name, tunnel.peer.endpoint));

        match self.start(&tunnel, generation).await {
            Ok(running) => {
                *control = Some(running);
                let probe = probes_internet(&tunnel);
                self.set_status(|s| {
                    s.state = ConnectionState::Connected;
                    s.connected_at = Some(now_ms());
                    // The probe may already have reported.
                    if s.health == Health::Unknown {
                        s.health = if probe { Health::Checking } else { Health::Unsupported };
                    }
                });
                self.logs.info(format!("Connected \"{}\"", tunnel.name));
                Ok(())
            }
            Err(error) => {
                let message = format!("{error:#}");
                self.logs.error(format!("Couldn't connect \"{}\": {message}", tunnel.name));
                self.set_status(|s| {
                    s.state = ConnectionState::Failed;
                    s.error = Some(message.clone());
                });
                Err(message)
            }
        }
    }

    pub async fn disconnect(&self) {
        let mut control = self.control.lock().await;
        let Some(running) = control.take() else {
            // Clear a previous failure.
            if self.status().state == ConnectionState::Failed {
                self.set_status(|s| *s = EngineStatus::disconnected());
            }
            return;
        };
        self.set_status(|s| s.state = ConnectionState::Disconnecting);
        self.stop(running).await;
        self.set_status(|s| *s = EngineStatus::disconnected());
        self.logs.info("Disconnected");
    }

    async fn start(&self, tunnel: &Tunnel, generation: u64) -> anyhow::Result<Running> {
        let api_listen = free_local_port().context("no free local port for the engine API")?;
        let api_secret = random_hex(16);
        // Checked before the engine starts: once its adapter is up, every
        // address looks routable.
        let host_ipv6 = host_has_ipv6();
        if !host_ipv6 {
            self.logs.info("No IPv6 on this network; the tunnel adapter will capture IPv4 only");
        }
        let options = EngineOptions {
            interface_name: TUN_INTERFACE_NAME.to_string(),
            api_listen,
            api_secret: api_secret.clone(),
            log_level: "info".into(),
            host_ipv6,
        };
        let config = singbox::build_config(tunnel, &options).map_err(|e| anyhow!(e))?;

        #[cfg(windows)]
        let job = crate::win::KillOnCloseJob::new()?;

        let mut command = Command::new(&self.sing_box);
        // The config is passed on stdin so private keys never touch disk.
        command
            .args(["run", "-c", "stdin", "--disable-color"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        {
            // Own process group so Ctrl+Break reaches only sing-box.
            const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
            command.creation_flags(CREATE_NEW_PROCESS_GROUP);
        }

        let log_start = self.logs.last_seq();
        let mut child = command.spawn().with_context(|| format!("couldn't start {}", self.sing_box.display()))?;
        let pid = child.id().context("engine exited immediately")?;
        #[cfg(windows)]
        if let Some(handle) = child.raw_handle() {
            job.assign(handle)?;
        }

        let mut stdin = child.stdin.take().context("engine stdin unavailable")?;
        stdin.write_all(config.to_string().as_bytes()).await?;
        drop(stdin);

        let (ready_tx, mut ready_rx) = watch::channel(false);
        let ready_tx = Arc::new(ready_tx);
        if let Some(stdout) = child.stdout.take() {
            spawn_log_reader(stdout, self.logs.clone(), ready_tx.clone());
        }
        if let Some(stderr) = child.stderr.take() {
            spawn_log_reader(stderr, self.logs.clone(), ready_tx.clone());
        }

        let stopping = Arc::new(AtomicBool::new(false));
        let (exited_tx, mut exited_rx) = watch::channel(false);
        let (kill_tx, kill_rx) = oneshot::channel();
        spawn_monitor(
            child,
            kill_rx,
            exited_tx,
            stopping.clone(),
            generation,
            self.shared.clone(),
            self.logs.clone(),
            log_start,
        );

        let mut running = Running {
            pid,
            stopping,
            exited: exited_rx.clone(),
            kill: Some(kill_tx),
            tasks: Vec::new(),
            #[cfg(windows)]
            _job: job,
        };

        let outcome = tokio::time::timeout(START_TIMEOUT, async {
            tokio::select! {
                _ = ready_rx.wait_for(|ready| *ready) => Ok(()),
                _ = exited_rx.wait_for(|exited| *exited) => Err(()),
            }
        })
        .await;

        match outcome {
            Ok(Ok(())) => {
                let api = ClashApi::new(api_listen, &api_secret);
                running.tasks.push(tokio::spawn(poll_stats(api, self.shared.clone(), running.exited.clone(), generation)));
                if probes_internet(tunnel) {
                    let api = ClashApi::new(api_listen, &api_secret);
                    running.tasks.push(tokio::spawn(poll_health(
                        api,
                        self.shared.clone(),
                        self.logs.clone(),
                        running.exited.clone(),
                        generation,
                    )));
                }
                Ok(running)
            }
            Ok(Err(())) => {
                running.stopping.store(true, Ordering::SeqCst);
                let reason = self.logs.last_engine_error(log_start).unwrap_or_else(|| "the engine exited during startup".into());
                bail!(explain_start_error(&reason))
            }
            Err(_) => {
                self.stop(running).await;
                let reason = self.logs.last_engine_error(log_start);
                bail!("timed out waiting for the engine to start{}", reason.map(|r| format!(" ({r})")).unwrap_or_default())
            }
        }
    }

    /// Asks sing-box to exit cleanly (so it removes its routes and firewall
    /// rules), and kills it if it doesn't.
    async fn stop(&self, mut running: Running) {
        running.stopping.store(true, Ordering::SeqCst);
        for task in running.tasks.drain(..) {
            task.abort();
        }
        self.shared.connections.lock().unwrap().clear();
        if *running.exited.borrow() {
            return;
        }

        #[cfg(windows)]
        if let Err(error) = crate::win::send_ctrl_break(running.pid) {
            tracing::debug!("graceful stop unavailable: {error:#}");
        }
        #[cfg(not(windows))]
        let _ = running.pid;

        let exited = tokio::time::timeout(STOP_TIMEOUT, running.exited.wait_for(|e| *e)).await.is_ok();
        if !exited {
            self.logs.warn("Engine didn't stop in time; killing it");
            if let Some(kill) = running.kill.take() {
                let _ = kill.send(());
            }
            let _ = tokio::time::timeout(STOP_TIMEOUT, running.exited.wait_for(|e| *e)).await;
        }
    }

    fn set_status(&self, update: impl FnOnce(&mut EngineStatus)) {
        update(&mut self.shared.status.lock().unwrap());
    }
}

fn spawn_log_reader(stream: impl AsyncRead + Unpin + Send + 'static, logs: Arc<LogBuffer>, ready: Arc<watch::Sender<bool>>) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(stream).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if line.trim().is_empty() {
                continue;
            }
            let parsed = parse_engine_line(&line);
            if parsed.message.contains(READY_MARKER) {
                let _ = ready.send(true);
            }
            if parsed.per_connection {
                // Per-connection chatter is shown in the connections view instead.
                if matches!(parsed.level, LogLevel::Warn | LogLevel::Error) {
                    logs.push(LogLevel::Warn, LogSource::Engine, parsed.message);
                }
                continue;
            }
            logs.push(parsed.level, LogSource::Engine, parsed.message);
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn spawn_monitor(
    mut child: Child,
    kill: oneshot::Receiver<()>,
    exited: watch::Sender<bool>,
    stopping: Arc<AtomicBool>,
    generation: u64,
    shared: Arc<Shared>,
    logs: Arc<LogBuffer>,
    log_start: u64,
) {
    tokio::spawn(async move {
        let status = tokio::select! {
            status = child.wait() => status,
            _ = kill => {
                let _ = child.start_kill();
                child.wait().await
            }
        };
        let _ = exited.send(true);

        // Startup failures are reported by `Engine::start`; only handle the
        // engine dying on its own while connected.
        if stopping.load(Ordering::SeqCst) || *shared.generation.lock().unwrap() != generation {
            return;
        }
        let mut s = shared.status.lock().unwrap();
        if s.state != ConnectionState::Connected {
            return;
        }
        let code = status.ok().and_then(|s| s.code());
        let reason = logs.last_engine_error(log_start).unwrap_or_else(|| {
            format!("The engine stopped unexpectedly (exit code {})", code.map_or("unknown".into(), |c| c.to_string()))
        });
        logs.error(format!("Tunnel dropped: {reason}"));
        shared.connections.lock().unwrap().clear();
        s.state = ConnectionState::Failed;
        s.error = Some(reason);
        s.traffic = Traffic::default();
        s.health = Health::Unknown;
    });
}

/// The probe goes to a public address, so it only proves something when the
/// tunnel carries all traffic (a default route in its allowed IPs).
fn probes_internet(tunnel: &Tunnel) -> bool {
    tunnel.peer.allowed_ips.iter().any(|net| net.prefix_len() == 0 && net.addr().is_ipv4())
}

/// Periodically fetches [`HEALTH_URL`] through the tunnel. A "connected"
/// engine only means sing-box started; this checks the WireGuard server
/// actually answers and forwards traffic.
async fn poll_health(
    api: ClashApi,
    shared: Arc<Shared>,
    logs: Arc<LogBuffer>,
    mut exited: watch::Receiver<bool>,
    generation: u64,
) {
    let api = Arc::new(api);
    let mut failures = 0u32;
    let mut was_failing = false;
    loop {
        if *exited.borrow() || *shared.generation.lock().unwrap() != generation {
            break;
        }
        let probe = api.clone();
        let result = match tokio::task::spawn_blocking(move || probe.delay(TUNNEL_TAG, HEALTH_URL, HEALTH_TIMEOUT_MS)).await {
            Ok(result) => result,
            Err(_) => break,
        };

        let health = match result {
            Ok(latency_ms) => {
                if was_failing {
                    logs.info(format!("Tunnel is passing traffic again ({latency_ms} ms)"));
                }
                failures = 0;
                was_failing = false;
                Some(Health::Ok { latency_ms })
            }
            Err(error) => {
                failures += 1;
                if failures < HEALTH_FAILURES {
                    None
                } else {
                    if !was_failing {
                        logs.warn(format!("Nothing is getting through the tunnel: {error:#}"));
                    }
                    was_failing = true;
                    Some(Health::Failing {
                        message: "No reply through the tunnel. Check that the server is up, that the keys \
                                  match, and that the server forwards traffic."
                            .into(),
                    })
                }
            }
        };
        if let Some(health) = health {
            if *shared.generation.lock().unwrap() != generation {
                break;
            }
            let mut status = shared.status.lock().unwrap();
            if matches!(status.state, ConnectionState::Connecting | ConnectionState::Connected) {
                status.health = health;
            }
        }

        let wait = if failures > 0 { HEALTH_RETRY } else { HEALTH_INTERVAL };
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            _ = exited.wait_for(|e| *e) => break,
        }
    }
}

/// Polls the engine once a second and turns per-connection byte counters
/// into per-route totals and rates.
async fn poll_stats(api: ClashApi, shared: Arc<Shared>, exited: watch::Receiver<bool>, generation: u64) {
    let api = Arc::new(api);
    let mut interval = tokio::time::interval(STATS_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut counters = TrafficCounter::default();

    loop {
        interval.tick().await;
        if *exited.borrow() {
            break;
        }
        let api = api.clone();
        let snapshot = match tokio::task::spawn_blocking(move || api.connections()).await {
            Ok(Ok(snapshot)) => snapshot,
            Ok(Err(error)) => {
                tracing::debug!("stats poll failed: {error:#}");
                continue;
            }
            Err(_) => break,
        };
        if *shared.generation.lock().unwrap() != generation {
            break;
        }
        let traffic = counters.update(&snapshot);
        *shared.connections.lock().unwrap() = snapshot.connections;
        let mut status = shared.status.lock().unwrap();
        if status.state == ConnectionState::Connected {
            status.traffic = traffic;
        }
    }
}

#[derive(Default)]
struct TrafficCounter {
    /// Last seen (upload, download) per connection id, for tunneled connections.
    seen: HashMap<String, (u64, u64)>,
    tunnel_total: (u64, u64),
    engine_total: Option<(u64, u64)>,
    last_sample: Option<Instant>,
}

impl TrafficCounter {
    fn update(&mut self, snapshot: &Snapshot) -> Traffic {
        let elapsed = self.last_sample.map(|t| t.elapsed().as_secs_f64()).unwrap_or(1.0).max(0.001);
        self.last_sample = Some(Instant::now());

        let mut tunnel_delta = (0u64, 0u64);
        let mut counts = (0u32, 0u32);
        let mut seen = HashMap::with_capacity(snapshot.connections.len());
        for c in &snapshot.connections {
            match c.route {
                Route::Tunnel => {
                    counts.0 += 1;
                    let (up, down) = self.seen.get(&c.id).copied().unwrap_or((0, 0));
                    tunnel_delta.0 += c.upload.saturating_sub(up);
                    tunnel_delta.1 += c.download.saturating_sub(down);
                    seen.insert(c.id.clone(), (c.upload, c.download));
                }
                Route::Direct => counts.1 += 1,
                Route::Other => {}
            }
        }
        self.seen = seen;
        self.tunnel_total.0 += tunnel_delta.0;
        self.tunnel_total.1 += tunnel_delta.1;

        // Direct traffic is whatever the engine moved that wasn't tunneled.
        // Using engine-wide totals also counts connections too short to be
        // seen by a poll.
        let engine_now = (snapshot.upload_total, snapshot.download_total);
        let engine_delta = match self.engine_total {
            Some(prev) => (engine_now.0.saturating_sub(prev.0), engine_now.1.saturating_sub(prev.1)),
            None => engine_now,
        };
        self.engine_total = Some(engine_now);
        let direct_delta = (engine_delta.0.saturating_sub(tunnel_delta.0), engine_delta.1.saturating_sub(tunnel_delta.1));
        let direct_total = (
            engine_now.0.saturating_sub(self.tunnel_total.0),
            engine_now.1.saturating_sub(self.tunnel_total.1),
        );

        let rate = |bytes: u64| (bytes as f64 / elapsed).round() as u64;
        Traffic {
            tunnel: RouteTraffic {
                upload: self.tunnel_total.0,
                download: self.tunnel_total.1,
                upload_rate: rate(tunnel_delta.0),
                download_rate: rate(tunnel_delta.1),
                connections: counts.0,
            },
            direct: RouteTraffic {
                upload: direct_total.0,
                download: direct_total.1,
                upload_rate: rate(direct_delta.0),
                download_rate: rate(direct_delta.1),
                connections: counts.1,
            },
        }
    }
}

/// Turns the engine's most common startup failures into something a person
/// can act on. The raw message stays in the logs.
fn explain_start_error(raw: &str) -> String {
    if raw.contains("tun") && raw.contains("Access is denied") {
        return "Windows wouldn't let the service create the virtual network adapter. The service needs \
                administrator rights: install it from Settings, or run `splitwire-service run` in an \
                elevated terminal."
            .into();
    }
    raw.to_string()
}

/// Whether a global IPv6 destination is routable from this machine. A UDP
/// "connect" only does the route lookup; nothing is sent. Teredo
/// (2001::/32) doesn't count: it isn't a real IPv6 network adapter.
pub fn host_has_ipv6() -> bool {
    let Ok(socket) = std::net::UdpSocket::bind((Ipv6Addr::UNSPECIFIED, 0)) else {
        return false;
    };
    let cloudflare_dns = Ipv6Addr::new(0x2606, 0x4700, 0x4700, 0, 0, 0, 0, 0x1111);
    if socket.connect((cloudflare_dns, 53)).is_err() {
        return false;
    }
    match socket.local_addr() {
        Ok(SocketAddr::V6(local)) => {
            let segments = local.ip().segments();
            let teredo = segments[0] == 0x2001 && segments[1] == 0;
            let global = (segments[0] & 0xe000) == 0x2000;
            global && !teredo
        }
        _ => false,
    }
}

fn free_local_port() -> std::io::Result<SocketAddr> {
    TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?.local_addr()
}

fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).expect("the OS random number generator failed");
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// Parses `sing-box version 1.14.4` from `sing-box version`.
fn read_engine_version(exe: &Path) -> Option<String> {
    let mut command = std::process::Command::new(exe);
    command.arg("version");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let output = command.output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines().next()?.strip_prefix("sing-box version ").map(|v| v.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn(id: &str, route: Route, up: u64, down: u64) -> Connection {
        Connection {
            id: id.into(),
            process_path: None,
            process_name: None,
            network: "tcp".into(),
            host: "example.com".into(),
            port: 443,
            route,
            upload: up,
            download: down,
            started_at: None,
        }
    }

    #[test]
    fn traffic_counter_splits_routes() {
        let mut counter = TrafficCounter::default();
        let first = counter.update(&Snapshot {
            upload_total: 150,
            download_total: 300,
            connections: vec![conn("a", Route::Tunnel, 100, 200), conn("b", Route::Direct, 50, 100)],
        });
        assert_eq!((first.tunnel.upload, first.tunnel.download), (100, 200));
        assert_eq!((first.direct.upload, first.direct.download), (50, 100));
        assert_eq!((first.tunnel.connections, first.direct.connections), (1, 1));

        // "a" grows by 10/20, "b" closes, a short direct connection moved 5/5.
        let second = counter.update(&Snapshot {
            upload_total: 165,
            download_total: 325,
            connections: vec![conn("a", Route::Tunnel, 110, 220)],
        });
        assert_eq!((second.tunnel.upload, second.tunnel.download), (110, 220));
        assert_eq!((second.direct.upload, second.direct.download), (55, 105));
        assert_eq!(second.direct.connections, 0);
    }
}
