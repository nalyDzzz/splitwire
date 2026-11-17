//! Supervises the sing-box process: start and stop, bringing a dropped tunnel
//! back, the kill switch, traffic stats, and a health probe through the tunnel.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context};
use splitwire_core::consts::TUN_INTERFACE_NAME;
use splitwire_core::ipc::{
    Blocking, ConnectOptions, ConnectionState, Connection, EngineStatus, Health, LogEntry, LogLevel, LogSource,
    Reconnect, Route, RouteTraffic, Traffic,
};
use splitwire_core::model::Tunnel;
use splitwire_core::now_ms;
use splitwire_core::singbox::{self, EngineMode, EngineOptions, TUNNEL_TAG};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;

use crate::clash::{ClashApi, Snapshot};
use crate::firewall::{Firewall, Wfp};
use crate::logs::{parse_engine_line, LogBuffer};

const START_TIMEOUT: Duration = Duration::from_secs(20);
const STOP_TIMEOUT: Duration = Duration::from_secs(5);
const STATS_INTERVAL: Duration = Duration::from_secs(1);
const READY_MARKER: &str = "sing-box started";
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(10);

/// Fetched through the tunnel to prove traffic gets through. Returns an
/// empty 204, the same check browsers use for captive portals.
pub(crate) const HEALTH_URL: &str = "http://cp.cloudflare.com/generate_204";
pub(crate) const HEALTH_TIMEOUT_MS: u32 = 5000;
const HEALTH_INTERVAL: Duration = Duration::from_secs(30);
const HEALTH_RETRY: Duration = Duration::from_secs(5);
/// Consecutive failures before reporting the tunnel as failing.
const HEALTH_FAILURES: u32 = 2;

/// How a dropped tunnel is brought back.
#[derive(Debug, Clone)]
struct RetryPolicy {
    /// Waits between attempts. The first restart after a drop is immediate;
    /// the last wait repeats.
    delays: Vec<Duration>,
    /// An engine that stays up this long has recovered: if it drops again,
    /// the restart is immediate again.
    stable_uptime: Duration,
    /// With the kill switch on, waits at least this long run the blocked
    /// engine, so apps outside the tunnel keep working. Shorter waits keep
    /// everything blocked rather than start an engine just to stop it.
    blocked_engine_after: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            delays: [1, 2, 5, 10, 20, 30].map(Duration::from_secs).to_vec(),
            stable_uptime: Duration::from_secs(60),
            blocked_engine_after: Duration::from_secs(5),
        }
    }
}

impl RetryPolicy {
    /// Wait before the next attempt, given the failures so far.
    fn delay(&self, failures: u32) -> Duration {
        match failures {
            0 => Duration::ZERO,
            n => self.delays[(n as usize - 1).min(self.delays.len() - 1)],
        }
    }
}

pub struct Engine {
    core: Arc<Core>,
    /// Serializes connect and disconnect. Holds the session keeping the
    /// current tunnel up.
    control: tokio::sync::Mutex<Option<Session>>,
}

/// Everything a session needs, shared with its task.
struct Core {
    sing_box: PathBuf,
    engine_version: Option<String>,
    logs: Arc<LogBuffer>,
    shared: Arc<Shared>,
    firewall: Box<dyn Firewall>,
    retry: RetryPolicy,
    #[cfg(test)]
    test: TestHooks,
}

/// State read by many tasks; kept behind cheap std mutexes.
struct Shared {
    status: Mutex<EngineStatus>,
    connections: Mutex<Vec<Connection>>,
    /// Incremented per engine start so a stopped engine's tasks can't clobber
    /// newer state.
    generation: Mutex<u64>,
}

/// A running sing-box process.
struct Running {
    pid: u32,
    started: Instant,
    /// Log position at start, to find the engine's last error.
    log_start: u64,
    stopping: Arc<AtomicBool>,
    exited: watch::Receiver<bool>,
    exit_code: Arc<Mutex<Option<i32>>>,
    kill: Option<oneshot::Sender<()>>,
    /// Stats and health tasks, aborted on stop.
    tasks: Vec<JoinHandle<()>>,
    #[cfg(windows)]
    _job: crate::win::KillOnCloseJob,
}

/// The task that keeps a connected tunnel up.
struct Session {
    cancel: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl Session {
    fn spawn(core: Arc<Core>, tunnel: Tunnel, probe: bool, peer: PeerAddress, running: Running) -> Self {
        let (cancel, cancelled) = watch::channel(false);
        let task = tokio::spawn(supervise(core, tunnel, probe, peer, running, cancelled));
        Self { cancel, task }
    }

    /// Ends the session, stopping its engine and lifting any block.
    async fn stop(self) {
        let _ = self.cancel.send(true);
        if let Err(error) = self.task.await {
            tracing::error!("tunnel session task failed: {error}");
        }
    }
}

impl Engine {
    pub fn new(sing_box: PathBuf) -> Self {
        let firewall = Box::new(Wfp::new(sing_box.clone()));
        Self::with_parts(sing_box, firewall, RetryPolicy::default())
    }

    fn with_parts(sing_box: PathBuf, firewall: Box<dyn Firewall>, retry: RetryPolicy) -> Self {
        let engine_version = read_engine_version(&sing_box);
        Self {
            core: Arc::new(Core {
                sing_box,
                engine_version,
                logs: Arc::new(LogBuffer::new()),
                shared: Arc::new(Shared {
                    status: Mutex::new(EngineStatus::disconnected()),
                    connections: Mutex::new(Vec::new()),
                    generation: Mutex::new(0),
                }),
                firewall,
                retry,
                #[cfg(test)]
                test: TestHooks::default(),
            }),
            control: tokio::sync::Mutex::new(None),
        }
    }

    pub fn sing_box_path(&self) -> &Path {
        &self.core.sing_box
    }

    pub fn engine_version(&self) -> Option<&str> {
        self.core.engine_version.as_deref()
    }

    pub fn status(&self) -> EngineStatus {
        self.core.shared.status.lock().unwrap().clone()
    }

    pub fn connections(&self) -> Vec<Connection> {
        self.core.shared.connections.lock().unwrap().clone()
    }

    pub fn logs(&self, after: u64) -> Vec<LogEntry> {
        self.core.logs.since(after)
    }

    /// Starts `tunnel`, replacing any running one. A failure here is
    /// reported, not retried; once the tunnel is up, drops are retried until
    /// [`Engine::disconnect`].
    pub async fn connect(&self, tunnel: Tunnel, options: ConnectOptions) -> Result<(), String> {
        if let Err(errors) = tunnel.validate() {
            let message = errors.iter().map(ToString::to_string).collect::<Vec<_>>().join("; ");
            return Err(format!("Invalid tunnel: {message}"));
        }

        let mut control = self.control.lock().await;
        if let Some(session) = control.take() {
            session.stop().await;
        }

        let core = &self.core;
        core.set_status(|s| {
            *s = EngineStatus {
                state: ConnectionState::Connecting,
                tunnel_id: Some(tunnel.id.clone()),
                tunnel_name: Some(tunnel.name.clone()),
                ..EngineStatus::disconnected()
            }
        });
        core.logs.info(format!("Connecting \"{}\" ({})", tunnel.name, tunnel.peer.endpoint));

        let probe = options.health_check && probes_internet(&tunnel);
        let mut peer = PeerAddress::new(&tunnel)?;
        let started = match peer.resolve(true, &core.logs).await {
            Ok(address) => core.start(&tunnel, EngineMode::Tunnel, Some(address), probe).await,
            Err(message) => Err(anyhow!(message)),
        };
        match started {
            Ok(running) => {
                core.set_connected(probe, Some(now_ms()));
                core.logs.info(format!("Connected \"{}\"", tunnel.name));
                *control = Some(Session::spawn(self.core.clone(), tunnel, probe, peer, running));
                Ok(())
            }
            Err(error) => {
                let message = format!("{error:#}");
                core.logs.error(format!("Couldn't connect \"{}\": {message}", tunnel.name));
                core.set_status(|s| {
                    s.state = ConnectionState::Failed;
                    s.error = Some(message.clone());
                });
                Err(message)
            }
        }
    }

    pub async fn disconnect(&self) {
        let mut control = self.control.lock().await;
        let Some(session) = control.take() else {
            // Clear a previous failure.
            if self.status().state == ConnectionState::Failed {
                self.core.set_status(|s| *s = EngineStatus::disconnected());
            }
            return;
        };
        self.core.set_status(|s| s.state = ConnectionState::Disconnecting);
        session.stop().await;
        self.core.set_status(|s| *s = EngineStatus::disconnected());
        self.core.logs.info("Disconnected");
    }
}

/// Keeps a tunnel up until the session is cancelled. When the engine stops on
/// its own, starts it again: at once, then backing off. With the kill switch
/// on, blocks the traffic that would have used the tunnel in the meantime.
async fn supervise(
    core: Arc<Core>,
    tunnel: Tunnel,
    probe: bool,
    mut peer: PeerAddress,
    mut running: Running,
    mut cancel: watch::Receiver<bool>,
) {
    let kill_switch = tunnel.routing.kill_switch;
    let mut failures = 0u32;
    loop {
        // Up: wait for the engine to stop on its own, or for the session to end.
        let mut exited = running.exited.clone();
        tokio::select! {
            _ = flagged(&mut exited) => {}
            _ = flagged(&mut cancel) => {
                core.stop(running).await;
                return;
            }
        }

        // Block first, explain after: until an engine is back, every
        // connection an app makes would go out directly.
        let mut hold = if kill_switch { core.block_all() } else { None };
        let mut last_error = core.exit_reason(&running);
        failures = if running.started.elapsed() >= core.retry.stable_uptime { 0 } else { failures + 1 };
        core.stop(running).await;
        core.logs.error(format!("Tunnel dropped: {last_error}"));
        if hold.is_some() {
            core.logs.warn("Kill switch: blocking all traffic until the engine is back");
        } else if !kill_switch {
            core.logs.warn("Apps that use the tunnel are on your normal connection until it's back");
        }

        // The kill switch's engine, while waiting between attempts.
        let mut fallback: Option<Running> = None;
        let restarted = loop {
            let delay = core.retry.delay(failures);
            if !delay.is_zero() {
                if kill_switch && fallback.is_none() && delay >= core.retry.blocked_engine_after {
                    match core.start(&tunnel, EngineMode::Blocked, None, false).await {
                        Ok(engine) => {
                            fallback = Some(engine);
                            if hold.take().is_some() {
                                core.logs.info("Kill switch: blocking only the apps that use the tunnel while waiting");
                            }
                        }
                        Err(error) => {
                            core.logs.warn(format!("Kill switch couldn't start its blocking engine: {error:#}"));
                            if hold.is_none() {
                                hold = core.block_all();
                            }
                        }
                    }
                }
                let retry_at = now_ms() + delay.as_millis() as i64;
                core.set_reconnecting(&last_error, failures, Some(retry_at), blocking(&hold, &fallback));
                core.logs.info(format!("Trying again in {}", describe(delay)));

                let fallback_exited = async {
                    match &fallback {
                        Some(engine) => {
                            let mut exited = engine.exited.clone();
                            let _ = exited.wait_for(|e| *e).await;
                        }
                        None => std::future::pending().await,
                    }
                };
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    _ = fallback_exited => {
                        // Retry now rather than leave a gap until the next attempt.
                        core.logs.warn("The kill switch's blocking engine stopped; blocking all traffic instead");
                        hold = hold.or_else(|| core.block_all());
                        if let Some(engine) = fallback.take() {
                            core.stop(engine).await;
                        }
                    }
                    _ = flagged(&mut cancel) => {
                        if let Some(engine) = fallback.take() {
                            core.stop(engine).await;
                        }
                        return;
                    }
                }
            }

            // Make way for the tunnel. Its adapter takes over from the
            // blocked engine's, so block everything during the swap.
            if let Some(engine) = fallback.take() {
                hold = hold.or_else(|| core.block_all());
                core.stop(engine).await;
            }
            core.set_reconnecting(&last_error, failures, None, blocking(&hold, &fallback));

            // While everything is blocked, a lookup could only time out.
            let lookup = hold.is_none();
            let attempt = async {
                let address = peer.resolve(lookup, &core.logs).await.map_err(|message| anyhow!(message))?;
                core.start(&tunnel, EngineMode::Tunnel, Some(address), probe).await
            };
            tokio::select! {
                result = attempt => match result {
                    Ok(engine) => break engine,
                    Err(error) => {
                        failures += 1;
                        last_error = format!("{error:#}");
                        core.logs.warn(format!("Reconnect attempt failed: {last_error}"));
                    }
                },
                _ = flagged(&mut cancel) => return,
            }
        };

        // The new engine routes or blocks from here.
        drop(hold);
        running = restarted;
        core.set_connected(probe, None);
        core.logs.info(format!("Reconnected \"{}\"", tunnel.name));
    }
}

/// Waits until a watch flag is set (or its sender is gone), without holding
/// the channel's lock afterwards.
async fn flagged(flag: &mut watch::Receiver<bool>) {
    let _ = flag.wait_for(|set| *set).await;
}

fn blocking(hold: &Option<Box<dyn Send>>, fallback: &Option<Running>) -> Blocking {
    if fallback.is_some() {
        Blocking::Apps
    } else if hold.is_some() {
        Blocking::All
    } else {
        Blocking::Off
    }
}

fn describe(delay: Duration) -> String {
    match delay.as_secs() {
        0 => format!("{} ms", delay.as_millis()),
        1 => "1 second".into(),
        n => format!("{n} seconds"),
    }
}

impl Core {
    async fn start(&self, tunnel: &Tunnel, mode: EngineMode, peer_address: Option<IpAddr>, probe: bool) -> anyhow::Result<Running> {
        #[cfg(test)]
        self.test.before_start(mode)?;

        let api_listen = free_local_port().context("no free local port for the engine API")?;
        let api_secret = random_hex(16);
        // Checked before the engine starts: once its adapter is up, every
        // address looks routable.
        let host_ipv6 = host_has_ipv6();
        if !host_ipv6 && mode == EngineMode::Tunnel {
            self.logs.info("No IPv6 on this network; the tunnel adapter will capture IPv4 only");
        }
        let options = EngineOptions {
            mode,
            interface_name: TUN_INTERFACE_NAME.to_string(),
            api_listen,
            api_secret: api_secret.clone(),
            log_level: "info".into(),
            host_ipv6,
            peer_address,
        };
        #[allow(unused_mut)]
        let mut config = singbox::build_config(tunnel, &options).map_err(|e| anyhow!(e))?;
        #[cfg(test)]
        if let Some(inbound) = &self.test.inbound {
            config["inbounds"] = serde_json::json!([inbound]);
        }

        let generation = {
            let mut generation = self.shared.generation.lock().unwrap();
            *generation += 1;
            *generation
        };

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
        #[cfg(test)]
        self.test.pids.lock().unwrap().push(pid);

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

        let (exited_tx, mut exited_rx) = watch::channel(false);
        let (kill_tx, kill_rx) = oneshot::channel();
        let exit_code = Arc::new(Mutex::new(None));
        spawn_monitor(child, kill_rx, exited_tx, exit_code.clone());

        let mut running = Running {
            pid,
            started: Instant::now(),
            log_start,
            stopping: Arc::new(AtomicBool::new(false)),
            exited: exited_rx.clone(),
            exit_code,
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
                running.started = Instant::now();
                if mode == EngineMode::Tunnel {
                    let api = ClashApi::new(api_listen, &api_secret);
                    running.tasks.push(tokio::spawn(poll_stats(api, self.shared.clone(), running.exited.clone(), generation)));
                    if probe {
                        let api = ClashApi::new(api_listen, &api_secret);
                        running.tasks.push(tokio::spawn(poll_health(
                            api,
                            self.shared.clone(),
                            self.logs.clone(),
                            running.exited.clone(),
                            generation,
                        )));
                    }
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
        let asked = match crate::win::send_ctrl_break(running.pid) {
            Ok(()) => true,
            Err(error) => {
                tracing::debug!("graceful stop unavailable: {error:#}");
                false
            }
        };
        #[cfg(not(windows))]
        let asked = {
            let _ = running.pid;
            false
        };

        let exited = asked && tokio::time::timeout(STOP_TIMEOUT, running.exited.wait_for(|e| *e)).await.is_ok();
        if !exited {
            if asked {
                self.logs.warn("Engine didn't stop in time; killing it");
            }
            if let Some(kill) = running.kill.take() {
                let _ = kill.send(());
            }
            let _ = tokio::time::timeout(STOP_TIMEOUT, running.exited.wait_for(|e| *e)).await;
        }
    }

    /// Why an engine stopped on its own, for the person reading the status.
    fn exit_reason(&self, running: &Running) -> String {
        self.logs.last_engine_error(running.log_start).unwrap_or_else(|| {
            let code = *running.exit_code.lock().unwrap();
            format!("The engine stopped unexpectedly (exit code {})", code.map_or("unknown".into(), |c| c.to_string()))
        })
    }

    /// Blocks everything but the engine. `None` if that failed, which is
    /// logged: the kill switch then has a gap until an engine is up.
    fn block_all(&self) -> Option<Box<dyn Send>> {
        match self.firewall.block_all() {
            Ok(guard) => Some(guard),
            Err(error) => {
                self.logs.error(format!("Kill switch couldn't block traffic: {error:#}"));
                None
            }
        }
    }

    /// `connected_at`: `None` keeps the time the session first connected.
    fn set_connected(&self, probe: bool, connected_at: Option<i64>) {
        self.set_status(|s| {
            // A disconnect is underway; it sets the final state.
            if s.state == ConnectionState::Disconnecting {
                return;
            }
            s.state = ConnectionState::Connected;
            if connected_at.is_some() {
                s.connected_at = connected_at;
            }
            s.error = None;
            s.blocking = Blocking::Off;
            s.reconnect = None;
            // The probe may already have reported.
            if s.health == Health::Unknown {
                s.health = if probe { Health::Checking } else { Health::Unsupported };
            }
        });
    }

    fn set_reconnecting(&self, error: &str, failures: u32, retry_at: Option<i64>, blocking: Blocking) {
        self.set_status(|s| {
            if s.state == ConnectionState::Disconnecting {
                return;
            }
            s.state = ConnectionState::Reconnecting;
            s.error = Some(error.to_string());
            s.traffic = Traffic::default();
            s.health = Health::Unknown;
            s.blocking = blocking;
            s.reconnect = Some(Reconnect { failures, retry_at });
        });
    }

    fn set_status(&self, update: impl FnOnce(&mut EngineStatus)) {
        update(&mut self.shared.status.lock().unwrap());
    }
}

/// The WireGuard server's address. The service looks it up itself, rather
/// than leaving it to the engine, and remembers it: a reconnect then works
/// even while DNS doesn't, and never waits on it.
struct PeerAddress {
    host: String,
    port: u16,
    last: Option<IpAddr>,
}

impl PeerAddress {
    fn new(tunnel: &Tunnel) -> Result<Self, String> {
        let endpoint = tunnel.endpoint()?;
        Ok(Self { last: None, host: endpoint.host, port: endpoint.port })
    }

    /// With `lookup` false, only the remembered address is used.
    async fn resolve(&mut self, lookup: bool, logs: &LogBuffer) -> Result<IpAddr, String> {
        if let Ok(ip) = self.host.parse::<IpAddr>() {
            return Ok(ip);
        }
        if !lookup {
            return self.last.ok_or_else(|| format!("Couldn't look up {} while traffic is blocked", self.host));
        }
        let error = match tokio::time::timeout(LOOKUP_TIMEOUT, tokio::net::lookup_host((self.host.as_str(), self.port))).await {
            Ok(Ok(addresses)) => {
                let addresses: Vec<IpAddr> = addresses.map(|a| a.ip()).collect();
                // IPv4 first: the engine binds the tunnel's socket to IPv4
                // unless the server only has an IPv6 address.
                match addresses.iter().find(|a| a.is_ipv4()).or(addresses.first()) {
                    Some(ip) => {
                        self.last = Some(*ip);
                        return Ok(*ip);
                    }
                    None => "no addresses found".to_string(),
                }
            }
            Ok(Err(error)) => error.to_string(),
            Err(_) => "timed out".to_string(),
        };
        match self.last {
            Some(ip) => {
                logs.warn(format!("Couldn't look up {} ({error}); using its last address, {ip}", self.host));
                Ok(ip)
            }
            None => Err(format!("Couldn't look up {}: {error}", self.host)),
        }
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

/// Waits for the engine to exit (or kills it on request) and records how.
fn spawn_monitor(mut child: Child, kill: oneshot::Receiver<()>, exited: watch::Sender<bool>, exit_code: Arc<Mutex<Option<i32>>>) {
    tokio::spawn(async move {
        let status = tokio::select! {
            status = child.wait() => status,
            // Also fires when the sender is dropped, e.g. a start that was cancelled.
            _ = kill => {
                let _ = child.start_kill();
                child.wait().await
            }
        };
        *exit_code.lock().unwrap() = status.ok().and_then(|s| s.code());
        let _ = exited.send(true);
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
            // While reconnecting, this probe belongs to the engine coming up.
            if matches!(
                status.state,
                ConnectionState::Connecting | ConnectionState::Connected | ConnectionState::Reconnecting
            ) {
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

/// Lets tests run the engine without admin rights and force failures.
#[cfg(test)]
#[derive(Default)]
struct TestHooks {
    /// Replaces the TUN inbound.
    inbound: Option<serde_json::Value>,
    /// Tunnel-mode starts that should fail before launching anything.
    fail_tunnel_starts: std::sync::atomic::AtomicU32,
    /// Every engine started, in order.
    pids: Mutex<Vec<u32>>,
}

#[cfg(test)]
impl TestHooks {
    fn before_start(&self, mode: EngineMode) -> anyhow::Result<()> {
        let fail = mode == EngineMode::Tunnel
            && self.fail_tunnel_starts.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1)).is_ok();
        if fail {
            bail!("simulated start failure");
        }
        Ok(())
    }
}

#[cfg(test)]
impl Engine {
    /// An engine that listens on `inbound` instead of a TUN adapter (so it
    /// needs no admin rights) and retries quickly.
    pub(crate) fn for_tests(sing_box: PathBuf, firewall: Box<dyn Firewall>, inbound: serde_json::Value) -> Self {
        let retry = RetryPolicy {
            delays: vec![Duration::from_millis(200), Duration::from_millis(1500)],
            stable_uptime: Duration::from_secs(60),
            blocked_engine_after: Duration::from_secs(1),
        };
        let mut engine = Self::with_parts(sing_box, firewall, retry);
        Arc::get_mut(&mut engine.core).expect("not shared yet").test.inbound = Some(inbound);
        engine
    }

    pub(crate) fn fail_next_tunnel_starts(&self, count: u32) {
        self.core.test.fail_tunnel_starts.store(count, Ordering::SeqCst);
    }

    pub(crate) fn last_engine_pid(&self) -> Option<u32> {
        self.core.test.pids.lock().unwrap().last().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_at_once_then_backs_off() {
        let policy = RetryPolicy::default();
        let secs: Vec<u64> = (0..9).map(|n| policy.delay(n).as_secs()).collect();
        assert_eq!(secs, [0, 1, 2, 5, 10, 20, 30, 30, 30]);
    }

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
