// Mirrors the Rust types in crates/splitwire-core (model.rs, ipc.rs, wgconf.rs)
// and apps/desktop/src-tauri. Keep the two in sync.

// ---- Tunnel model (splitwire-core/src/model.rs) ----

export interface Tunnel {
  id: string;
  name: string;
  interface: TunnelInterface;
  peer: Peer;
  routing: Routing;
}

export interface TunnelInterface {
  /** Base64 WireGuard private key. */
  privateKey: string;
  /** CIDR strings, e.g. "10.8.0.2/24". */
  addresses: string[];
  /** IP addresses. */
  dns: string[];
  mtu: number | null;
}

export interface Peer {
  publicKey: string;
  presharedKey: string | null;
  /** "host:port", "ip:port" or "[ipv6]:port". */
  endpoint: string;
  /** CIDR strings. Destinations that go through the tunnel. */
  allowedIps: string[];
  /** Seconds; null when off. */
  persistentKeepalive: number | null;
}

/**
 * A connection is tunneled when its app is in scope (every app, or one of
 * `apps`), it isn't in `bypassApps`, its destination is inside the peer's
 * allowed IPs, and its destination isn't in `bypassIps`. Bypass rules win.
 */
export interface Routing {
  appMode: AppMode;
  /** Apps to tunnel when appMode is "selected". */
  apps: AppRule[];
  /** Apps that never use the tunnel. */
  bypassApps: AppRule[];
  /** CIDR strings that never use the tunnel. */
  bypassIps: string[];
  dnsMode: DnsMode;
}

/** "all": every app except bypassed ones. "selected": only `apps`. */
export type AppMode = "all" | "selected";

/**
 * Windows resolves DNS for all apps in one system service, so this is
 * all-or-nothing. "system": normal network DNS. "tunnel": every lookup goes
 * to the tunnel's DNS servers through the tunnel.
 */
export type DnsMode = "system" | "tunnel";

export interface AppRule {
  kind: AppRuleKind;
  value: string;
  /** Exe path this rule was created from; only used to show an icon. */
  pathHint: string | null;
}

/**
 * name: exact exe name ("node.exe"; ".exe" optional)
 * path: exact exe path ("C:\Tools\node.exe")
 * folder: every exe inside a folder, including subfolders
 * keyword: exe name contains the text ("docker")
 */
export type AppRuleKind = "name" | "path" | "folder" | "keyword";

export interface ValidationError {
  /** camelCase field path, e.g. "peer.endpoint" or "routing.apps.2". */
  field: string;
  message: string;
}

export interface ImportedTunnel {
  /** Not saved yet; call saveTunnel to keep it. */
  tunnel: Tunnel;
  warnings: string[];
}

// ---- Service status (splitwire-core/src/ipc.rs) ----

export type ConnectionState =
  | "disconnected"
  | "connecting"
  | "connected"
  | "disconnecting"
  | "failed";

export interface EngineStatus {
  state: ConnectionState;
  tunnelId: string | null;
  tunnelName: string | null;
  /** Unix ms when the tunnel came up. */
  connectedAt: number | null;
  error: string | null;
  traffic: Traffic;
  /** Whether traffic actually gets through the tunnel. Only meaningful while connected. */
  health: Health;
}

/**
 * Result of periodically fetching a test URL through the tunnel.
 * "unsupported": not probed (turned off in settings, or the tunnel only
 * carries some networks so a public test page proves nothing).
 */
export type Health =
  | { state: "unknown" }
  | { state: "checking" }
  | { state: "ok"; latencyMs: number }
  | { state: "failing"; message: string }
  | { state: "unsupported" };

export interface Traffic {
  tunnel: RouteTraffic;
  direct: RouteTraffic;
}

export interface RouteTraffic {
  /** Bytes since the tunnel came up. */
  upload: number;
  download: number;
  /** Bytes per second. */
  uploadRate: number;
  downloadRate: number;
  /** Open connections right now. */
  connections: number;
}

export type Route = "tunnel" | "direct" | "other";

export interface Connection {
  id: string;
  processPath: string | null;
  processName: string | null;
  network: "tcp" | "udp" | string;
  /** Domain when known, otherwise IP. */
  host: string;
  port: number;
  route: Route;
  upload: number;
  download: number;
  /** RFC 3339. */
  startedAt: string | null;
}

export type LogLevel = "debug" | "info" | "warn" | "error";
export type LogSource = "service" | "engine";

export interface LogEntry {
  seq: number;
  /** Unix ms. */
  time: number;
  level: LogLevel;
  source: LogSource;
  message: string;
}

// ---- Desktop app (apps/desktop/src-tauri) ----

/** Whether the background service can be reached. */
export type ServiceStatus =
  | { kind: "notInstalled" }
  | { kind: "stopped" }
  | { kind: "unreachable"; message: string }
  | {
      kind: "running";
      serviceVersion: string;
      engineVersion: string | null;
      status: EngineStatus;
    };

export interface ProcessInfo {
  /** Exe file name, e.g. "Code.exe". */
  name: string;
  /** Full exe path. */
  path: string;
  /** FileDescription from the exe's version info, e.g. "Visual Studio Code". */
  description: string | null;
  /** How many running processes use this exe. */
  instances: number;
}

export interface PickedApp {
  name: string;
  path: string;
  description: string | null;
}

export interface KeyPair {
  privateKey: string;
  publicKey: string;
}

export interface Settings {
  /** Closing the window hides it to the tray instead of quitting. */
  closeToTray: boolean;
  launchAtLogin: boolean;
  /** Connect to lastTunnelId when the app starts. */
  connectOnLaunch: boolean;
  lastTunnelId: string | null;
  /** Check GitHub for new releases in the background. */
  autoCheckUpdates: boolean;
  /** Fetch a test page through the tunnel every 30 s (applies on next connect). */
  healthCheck: boolean;
}

export interface UpdateInfo {
  version: string;
  currentVersion: string;
  /** Release notes, Markdown. */
  notes: string | null;
  /** RFC 3339. */
  date: string | null;
}

export interface UpdateProgress {
  /** Bytes so far. */
  downloaded: number;
  total: number | null;
}

/** Shape of a rejected command promise. */
export type CommandError =
  | { kind: "validation"; errors: ValidationError[] }
  | { kind: "service"; message: string }
  | { kind: "other"; message: string };
