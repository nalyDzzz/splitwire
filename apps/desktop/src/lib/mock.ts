// In-memory backend for building the UI in a plain browser (`pnpm dev:web`).
//
// URL switches:
//   ?service=notInstalled|stopped|unreachable   start with the service in that state
//   ?empty                                       start with no tunnels
//   ?connect=<tunnel name>                       connect to a tunnel on load
//   ?health=failing                              connected, but nothing gets through
//   ?update[=fail]                               a newer release is available (installing it fails)
// The tunnel named "lab" always fails to connect, to show the failed state.
// `window.splitwireMock.dropConf()` simulates dropping a .conf on the window.

import type { Backend } from "./api";
import { basename } from "./format";
import { isCidr, ruleMatchesPath } from "./rules";
import type {
  AppRule,
  CommandError,
  Connection,
  EngineStatus,
  ImportedTunnel,
  LogEntry,
  LogLevel,
  LogSource,
  ProcessInfo,
  Route,
  ServiceStatus,
  Settings,
  Tunnel,
  UpdateInfo,
  UpdateProgress,
  ValidationError,
} from "./types";

const params = new URLSearchParams(globalThis.location?.search ?? "");

// ---- seed data ----

function randomKey(): string {
  const bytes = new Uint8Array(32);
  crypto.getRandomValues(bytes);
  return btoa(String.fromCharCode(...bytes));
}

function decodeKey(key: string): Uint8Array | null {
  try {
    const raw = atob(key.trim());
    if (raw.length !== 32) return null;
    return Uint8Array.from(raw, (c) => c.charCodeAt(0));
  } catch {
    return null;
  }
}

async function derivePublicKey(privateKey: string): Promise<string> {
  const bytes = decodeKey(privateKey);
  if (!bytes) throw { kind: "other", message: "Private key must be a base64 WireGuard key" } satisfies CommandError;
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes as BufferSource));
  return btoa(String.fromCharCode(...digest));
}

const names = (list: string[]): AppRule[] =>
  list.map((value) => ({ kind: "name", value: `${value}.exe`, pathHint: null }));

const SEED: Tunnel[] = [
  {
    id: "t-work",
    name: "work",
    interface: { privateKey: randomKey(), addresses: ["10.8.0.2/24"], dns: ["1.1.1.1"], mtu: null },
    peer: {
      publicKey: randomKey(),
      presharedKey: randomKey(),
      endpoint: "vpn.example.com:51820",
      allowedIps: ["0.0.0.0/0", "::/0"],
      persistentKeepalive: 25,
    },
    routing: {
      appMode: "selected",
      apps: [
        ...names([
          "Code", "node", "java", "webstorm64", "datagrip64", "goland64", "pycharm64", "python",
          "php", "httpd", "MySQLWorkbench", "Beekeeper Studio",
        ]),
        { kind: "keyword", value: "docker", pathHint: null },
      ],
      // httpd is in both lists, to show the conflict warning.
      bypassApps: names(["chrome", "Discord", "slack", "ms-teams", "Zoom", "steam", "Spotify", "httpd"]),
      bypassIps: [],
      dnsMode: "system",
    },
  },
  {
    id: "t-home",
    name: "home",
    interface: { privateKey: randomKey(), addresses: ["10.66.0.2/32"], dns: ["10.66.0.1"], mtu: null },
    peer: {
      publicKey: randomKey(),
      presharedKey: null,
      endpoint: "203.0.113.24:51820",
      allowedIps: ["0.0.0.0/0", "::/0"],
      persistentKeepalive: 25,
    },
    routing: {
      appMode: "all",
      apps: [],
      bypassApps: names(["Discord", "steam", "Spotify"]),
      bypassIps: ["192.168.0.0/16"],
      dnsMode: "tunnel",
    },
  },
  {
    id: "t-lab",
    name: "lab",
    interface: { privateKey: randomKey(), addresses: ["10.13.13.4/32"], dns: [], mtu: 1380 },
    peer: {
      publicKey: randomKey(),
      presharedKey: null,
      endpoint: "lab.example.net:51820",
      allowedIps: ["10.13.13.0/24", "192.168.1.0/24"],
      persistentKeepalive: 25,
    },
    routing: {
      appMode: "selected",
      apps: [
        { kind: "name", value: "Code.exe", pathHint: null },
        { kind: "keyword", value: "docker", pathHint: null },
        { kind: "folder", value: "C:\\Program Files\\JetBrains", pathHint: null },
      ],
      bypassApps: [],
      bypassIps: [],
      dnsMode: "system",
    },
  },
];

const U = "C:\\Users\\alex\\AppData\\Local";
const R = "C:\\Users\\alex\\AppData\\Roaming";
const PF = "C:\\Program Files";
const JB = `${PF}\\JetBrains`;

const PROCESSES: ProcessInfo[] = [
  { name: "webstorm64.exe", path: `${JB}\\WebStorm 2026.2\\bin\\webstorm64.exe`, description: "WebStorm", instances: 1 },
  { name: "phpstorm64.exe", path: `${JB}\\PhpStorm 2026.2\\bin\\phpstorm64.exe`, description: "PhpStorm", instances: 1 },
  { name: "goland64.exe", path: `${JB}\\GoLand 2026.2\\bin\\goland64.exe`, description: "GoLand", instances: 1 },
  { name: "datagrip64.exe", path: `${JB}\\DataGrip 2026.2\\bin\\datagrip64.exe`, description: "DataGrip", instances: 1 },
  { name: "pycharm64.exe", path: `${JB}\\PyCharm 2026.2\\bin\\pycharm64.exe`, description: "PyCharm", instances: 1 },
  { name: "java.exe", path: `${PF}\\Eclipse Adoptium\\jdk-21.0.6.7-hotspot\\bin\\java.exe`, description: "OpenJDK Platform binary", instances: 3 },
  { name: "node.exe", path: `${PF}\\nodejs\\node.exe`, description: "Node.js JavaScript Runtime", instances: 6 },
  { name: "Code.exe", path: `${U}\\Programs\\Microsoft VS Code\\Code.exe`, description: "Visual Studio Code", instances: 14 },
  { name: "python.exe", path: `${U}\\Programs\\Python\\Python313\\python.exe`, description: "Python", instances: 2 },
  { name: "php.exe", path: "C:\\tools\\php\\php.exe", description: "CLI", instances: 1 },
  { name: "httpd.exe", path: "C:\\Apache24\\bin\\httpd.exe", description: "Apache HTTP Server", instances: 2 },
  { name: "Postman.exe", path: `${U}\\Postman\\Postman.exe`, description: "Postman", instances: 3 },
  { name: "MySQLWorkbench.exe", path: `${PF}\\MySQL\\MySQL Workbench 8.0\\MySQLWorkbench.exe`, description: "MySQL Workbench", instances: 1 },
  { name: "Docker Desktop.exe", path: `${PF}\\Docker\\Docker\\Docker Desktop.exe`, description: "Docker Desktop", instances: 4 },
  { name: "com.docker.backend.exe", path: `${PF}\\Docker\\Docker\\resources\\com.docker.backend.exe`, description: "Docker Desktop Backend", instances: 1 },
  { name: "Beekeeper Studio.exe", path: `${U}\\Programs\\beekeeper-studio\\Beekeeper Studio.exe`, description: "Beekeeper Studio", instances: 3 },
  { name: "chrome.exe", path: `${PF}\\Google\\Chrome\\Application\\chrome.exe`, description: "Google Chrome", instances: 31 },
  { name: "Discord.exe", path: `${U}\\Discord\\app-1.0.9213\\Discord.exe`, description: "Discord", instances: 6 },
  { name: "slack.exe", path: `${U}\\slack\\app-4.47.69\\slack.exe`, description: "Slack", instances: 5 },
  { name: "ms-teams.exe", path: `${PF}\\WindowsApps\\MSTeams_25290.205\\ms-teams.exe`, description: "Microsoft Teams", instances: 4 },
  { name: "Zoom.exe", path: `${R}\\Zoom\\bin\\Zoom.exe`, description: "Zoom Workplace", instances: 1 },
  { name: "steam.exe", path: "C:\\Program Files (x86)\\Steam\\steam.exe", description: "Steam", instances: 1 },
  { name: "Spotify.exe", path: `${R}\\Spotify\\Spotify.exe`, description: "Spotify", instances: 5 },
  { name: "obs64.exe", path: `${PF}\\obs-studio\\bin\\64bit\\obs64.exe`, description: "OBS Studio", instances: 1 },
  { name: "WindowsTerminal.exe", path: `${PF}\\WindowsApps\\Microsoft.WindowsTerminal_1.23\\WindowsTerminal.exe`, description: "Windows Terminal", instances: 1 },
  { name: "explorer.exe", path: "C:\\Windows\\explorer.exe", description: "Windows Explorer", instances: 1 },
  { name: "mstsc.exe", path: "C:\\Windows\\System32\\mstsc.exe", description: "Remote Desktop Connection", instances: 1 },
];

// ---- icons: small generated tiles that look like app icons ----

const BRANDS: Record<string, [string, string, string]> = {
  webstorm64: ["#07c3f2", "#087cfa", "WS"],
  phpstorm64: ["#b345f1", "#765af8", "PS"],
  goland64: ["#3bea62", "#0d7bf7", "GO"],
  datagrip64: ["#22d88f", "#9775f8", "DG"],
  pycharm64: ["#21d789", "#fcf84a", "PC"],
  java: ["#e76f00", "#5382a1", "J"],
  node: ["#5fa04e", "#3c873a", "N"],
  code: ["#22a6f2", "#0065a9", "</>"],
  python: ["#3776ab", "#ffd343", "Py"],
  php: ["#8892bf", "#4f5b93", "php"],
  httpd: ["#d22128", "#6d2c8b", "A"],
  mysqlworkbench: ["#00758f", "#f29111", "WB"],
  "docker desktop": ["#2496ed", "#1d63ed", "D"],
  "com.docker.backend": ["#2496ed", "#0b3d91", "D"],
  "beekeeper studio": ["#fad83b", "#e2b100", "B"],
  chrome: ["#ea4335", "#fbbc05", "C"],
  discord: ["#5865f2", "#404eed", "D"],
  slack: ["#4a154b", "#e01e5a", "S"],
  "ms-teams": ["#7b83eb", "#4b53bc", "T"],
  zoom: ["#2d8cff", "#0b5cff", "Z"],
  obs64: ["#302e31", "#7b6bd6", "O"],
  steam: ["#2a475e", "#1b2838", "S"],
  spotify: ["#1ed760", "#169c46", "S"],
  windowsterminal: ["#2f2f2f", "#4cc2ff", ">_"],
  explorer: ["#ffca28", "#f2a500", "E"],
  mstsc: ["#0078d4", "#005a9e", "RD"],
  postman: ["#ff6c37", "#e0461b", "P"],
};

function iconFor(path: string): string | null {
  const stem = basename(path).replace(/\.exe$/i, "").toLowerCase();
  const brand = BRANDS[stem];
  if (!brand) return null;
  const [a, b, letters] = brand;
  const size = letters.length > 2 ? 18 : letters.length === 2 ? 22 : 28;
  const svg =
    `<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 64 64'>` +
    `<defs><linearGradient id='g' x1='0' y1='0' x2='1' y2='1'><stop offset='0' stop-color='${a}'/><stop offset='1' stop-color='${b}'/></linearGradient></defs>` +
    `<rect x='6' y='6' width='52' height='52' rx='13' fill='url(#g)'/>` +
    `<text x='32' y='${32 + size * 0.36}' font-family='Segoe UI, Arial, sans-serif' font-weight='800' font-size='${size}' fill='white' text-anchor='middle'>${letters
      .replace("<", "&lt;")
      .replace(">", "&gt;")}</text></svg>`;
  return `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`;
}

// ---- simulated engine ----

const HOSTS: Record<string, [string, number, "tcp" | "udp"][]> = {
  "webstorm64.exe": [["api.jetbrains.ai", 443, "tcp"], ["plugins.jetbrains.com", 443, "tcp"]],
  "goland64.exe": [["proxy.golang.org", 443, "tcp"], ["sum.golang.org", 443, "tcp"]],
  "datagrip64.exe": [["10.8.0.12", 3306, "tcp"], ["db.internal.lan", 5432, "tcp"]],
  "java.exe": [["repo.maven.apache.org", 443, "tcp"], ["10.8.0.20", 8080, "tcp"]],
  "node.exe": [["registry.npmjs.org", 443, "tcp"], ["github.com", 443, "tcp"], ["10.8.0.20", 5173, "tcp"]],
  "Code.exe": [["api.github.com", 443, "tcp"], ["marketplace.visualstudio.com", 443, "tcp"], ["copilot-proxy.githubusercontent.com", 443, "tcp"]],
  "python.exe": [["pypi.org", 443, "tcp"], ["files.pythonhosted.org", 443, "tcp"]],
  "com.docker.backend.exe": [["registry-1.docker.io", 443, "tcp"], ["production.cloudflare.docker.com", 443, "tcp"]],
  "MySQLWorkbench.exe": [["10.8.0.12", 3306, "tcp"]],
  "Beekeeper Studio.exe": [["db.internal.lan", 5432, "tcp"]],
  "chrome.exe": [["www.youtube.com", 443, "udp"], ["mail.google.com", 443, "tcp"], ["news.ycombinator.com", 443, "tcp"]],
  "Discord.exe": [["gateway.discord.gg", 443, "tcp"], ["162.159.130.234", 50004, "udp"]],
  "slack.exe": [["wss-primary.slack.com", 443, "tcp"]],
  "Spotify.exe": [["spclient.wg.spotify.com", 443, "tcp"], ["audio-ak.spotifycdn.com", 443, "tcp"]],
  "steam.exe": [["steamcommunity.com", 443, "tcp"], ["cache4-iad1.steamcontent.com", 80, "tcp"]],
  "ms-teams.exe": [["teams.microsoft.com", 443, "tcp"]],
};

function routeFor(tunnel: Tunnel, path: string): Route {
  const r = tunnel.routing;
  if (r.bypassApps.some((rule) => ruleMatchesPath(rule, path))) return "direct";
  if (r.appMode === "all") return "tunnel";
  return r.apps.some((rule) => ruleMatchesPath(rule, path)) ? "tunnel" : "direct";
}

const DISCONNECTED: EngineStatus = {
  state: "disconnected",
  tunnelId: null,
  tunnelName: null,
  connectedAt: null,
  error: null,
  traffic: {
    tunnel: { upload: 0, download: 0, uploadRate: 0, downloadRate: 0, connections: 0 },
    direct: { upload: 0, download: 0, uploadRate: 0, downloadRate: 0, connections: 0 },
  },
  health: { state: "unknown" },
};

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
const clone = <T>(value: T): T => structuredClone(value);
const jitter = (value: number, min: number, max: number) =>
  Math.min(max, Math.max(min, value * (0.75 + Math.random() * 0.5) + (Math.random() - 0.5) * min));

function mockValidate(t: Tunnel): ValidationError[] {
  const errors: ValidationError[] = [];
  const push = (field: string, message: string) => errors.push({ field, message });
  if (!t.name.trim()) push("name", "Give the tunnel a name");
  if (!decodeKey(t.interface.privateKey)) push("interface.privateKey", "Private key must be a base64 WireGuard key");
  if (!t.interface.addresses.length) push("interface.addresses", "Add at least one address, like 10.8.0.2/32");
  if (t.interface.mtu !== null && (t.interface.mtu < 1280 || t.interface.mtu > 9000))
    push("interface.mtu", "MTU must be between 1280 and 9000");
  if (!decodeKey(t.peer.publicKey)) push("peer.publicKey", "Public key must be a base64 WireGuard key");
  if (t.peer.presharedKey && !decodeKey(t.peer.presharedKey))
    push("peer.presharedKey", "Preshared key must be a base64 WireGuard key");
  if (!/^(\[[0-9a-f:]+\]|[a-z0-9.-]+):\d{1,5}$/i.test(t.peer.endpoint.trim()))
    push("peer.endpoint", "Endpoint needs a port, like vpn.example.com:51820");
  if (!t.peer.allowedIps.length) push("peer.allowedIps", "Add at least one allowed IP, like 0.0.0.0/0");
  t.routing.apps.forEach((rule, i) => !rule.value.trim() && push(`routing.apps.${i}`, "App rule is empty"));
  return errors;
}

/** Enough of the .conf format to make the paste flow feel real. */
function mockParse(text: string, name: string): ImportedTunnel {
  if (!/\[interface\]/i.test(text)) {
    throw { kind: "other", message: "line 1: Settings must be inside [Interface] or [Peer]" } satisfies CommandError;
  }
  const get = (key: string) =>
    [...text.matchAll(new RegExp(`^\\s*${key}\\s*=\\s*(.+)$`, "gim"))].map((m) => m[1]!.split("#")[0]!.trim());
  const list = (key: string) => get(key).flatMap((v) => v.split(",").map((s) => s.trim()).filter(Boolean));
  const privateKey = get("PrivateKey")[0];
  const publicKey = get("PublicKey")[0];
  const endpoint = get("Endpoint")[0];
  if (!privateKey) throw { kind: "other", message: "line 0: Missing PrivateKey in [Interface]" } satisfies CommandError;
  if (!publicKey || !endpoint) throw { kind: "other", message: "line 0: Missing [Peer] section" } satisfies CommandError;
  const cidr = (v: string) => (v.includes("/") ? v : v.includes(":") ? `${v}/128` : `${v}/32`);
  const apps = names(list("AllowedApps").map((n) => n.replace(/\.exe$/i, "")));
  const bypassApps = names(list("DisallowedApps").map((n) => n.replace(/\.exe$/i, "")));
  const warnings: string[] = [];
  const known = /^(privatekey|address|dns|mtu|listenport|publickey|presharedkey|endpoint|allowedips|persistentkeepalive|allowedapps|disallowedapps|disallowedips)$/i;
  const ignored = [...text.matchAll(/^\s*([A-Za-z0-9]+)\s*=/gm)].map((m) => m[1]!).filter((k) => !known.test(k));
  if (ignored.length) warnings.push(`Ignored unsupported settings: ${[...new Set(ignored)].join(", ")}`);
  for (const app of apps) {
    if (bypassApps.some((b) => b.value.toLowerCase() === app.value.toLowerCase())) {
      warnings.push(`"${app.value}" is in both AllowedApps and DisallowedApps, so it will bypass the tunnel`);
    }
  }
  const dns = list("DNS").filter((d) => /^[0-9a-f.:]+$/i.test(d));
  const keepalive = Number(get("PersistentKeepalive")[0] ?? 0);
  return {
    tunnel: {
      id: `t-${crypto.randomUUID()}`,
      name: name.trim(),
      interface: { privateKey, addresses: list("Address").map(cidr), dns, mtu: Number(get("MTU")[0]) || null },
      peer: {
        publicKey,
        presharedKey: get("PresharedKey")[0] ?? null,
        endpoint,
        allowedIps: list("AllowedIPs").map(cidr),
        persistentKeepalive: keepalive > 0 ? keepalive : null,
      },
      routing: {
        appMode: apps.length ? "selected" : "all",
        apps,
        bypassApps,
        bypassIps: list("DisallowedIPs").map(cidr).filter(isCidr),
        dnsMode: dns.length && !apps.length ? "tunnel" : "system",
      },
    },
    warnings,
  };
}

function sampleConf(): string {
  return [
    "[Interface]",
    `PrivateKey = ${randomKey()}`,
    "Address = 10.8.0.3/24",
    "DNS = 1.1.1.1",
    "",
    "[Peer]",
    `PublicKey = ${randomKey()}`,
    "Endpoint = vpn.example.com:51820",
    "AllowedApps = Code, node, goland64, docker",
    "AllowedIPs = 0.0.0.0/0, ::/0",
    "DisallowedApps = chrome, Discord",
    "PostUp = netsh interface ip set dns ...",
  ].join("\n");
}

export function createMockBackend(): Backend {
  let tunnels: Tunnel[] = params.has("empty") ? [] : clone(SEED);
  let settings: Settings = {
    closeToTray: true,
    launchAtLogin: false,
    connectOnLaunch: false,
    lastTunnelId: "t-work",
    autoCheckUpdates: true,
    healthCheck: true,
  };
  const VERSION = "0.2.0";
  // `?update` pretends a newer release is out; `?update=fail` makes installing it fail.
  const mockUpdate: UpdateInfo | null = params.has("update")
    ? {
        version: "0.3.0",
        currentVersion: VERSION,
        notes:
          "## What's Changed\n* Kill switch: block tunneled apps when the tunnel drops\n* Faster app picker with search by description\n* Fix tray icon not updating after sleep\n\n**Full Changelog**: https://github.com/nalyDzzz/splitwire/compare/v0.2.0...v0.3.0",
        date: new Date(Date.now() - 3 * 3600_000).toISOString(),
      }
    : null;
  let foundUpdate: UpdateInfo | null = null;
  const updateHandlers = new Set<(u: UpdateInfo) => void>();
  const progressHandlers = new Set<(p: UpdateProgress) => void>();
  let service = (params.get("service") ?? "running") as "running" | "notInstalled" | "stopped" | "unreachable";
  let engine: EngineStatus = clone(DISCONNECTED);
  let connections: Connection[] = [];
  const logs: LogEntry[] = [];
  let seq = 0;

  const statusHandlers = new Set<(s: ServiceStatus) => void>();
  const tunnelHandlers = new Set<(t: Tunnel[]) => void>();
  const importHandlers = new Set<(i: ImportedTunnel) => void>();

  const log = (level: LogLevel, source: LogSource, message: string) => {
    logs.push({ seq: ++seq, time: Date.now(), level, source, message });
    if (logs.length > 3000) logs.splice(0, logs.length - 3000);
  };

  const status = (): ServiceStatus => {
    switch (service) {
      case "running":
        return { kind: "running", serviceVersion: VERSION, engineVersion: "1.14.4", status: clone(engine) };
      case "unreachable":
        return { kind: "unreachable", message: "The pipe \\\\.\\pipe\\splitwire-service didn't answer within 5 seconds." };
      default:
        return { kind: service };
    }
  };
  const emit = () => statusHandlers.forEach((handler) => handler(status()));

  const requireService = () => {
    if (service !== "running") {
      throw { kind: "service", message: "The Splitwire service isn't running." } satisfies CommandError;
    }
  };

  const activeTunnel = () => tunnels.find((t) => t.id === engine.tunnelId);

  const newConnection = (tunnel: Tunnel): Connection => {
    const candidates = PROCESSES.filter((p) => HOSTS[p.name]);
    const proc = candidates[Math.floor(Math.random() * candidates.length)]!;
    const hosts = HOSTS[proc.name]!;
    const [host, port, network] = hosts[Math.floor(Math.random() * hosts.length)]!;
    return {
      id: crypto.randomUUID(),
      processPath: proc.path,
      processName: proc.name,
      network,
      host,
      port,
      route: routeFor(tunnel, proc.path),
      upload: Math.floor(Math.random() * 40_000),
      download: Math.floor(Math.random() * 400_000),
      startedAt: new Date(Date.now() - Math.random() * 600_000).toISOString(),
    };
  };

  // Engine tick: traffic, connections, the odd log line.
  setInterval(() => {
    if (engine.state === "connected") {
      const tunnel = activeTunnel();
      if (!tunnel) return;
      if (connections.length < 14 || Math.random() < 0.35) connections.push(newConnection(tunnel));
      if (connections.length > 26 || Math.random() < 0.25) connections.splice(Math.floor(Math.random() * connections.length), 1);
      for (const c of connections) {
        c.download += Math.floor(Math.random() * (c.route === "tunnel" ? 180_000 : 420_000));
        c.upload += Math.floor(Math.random() * 30_000);
      }
      const t = engine.traffic;
      t.tunnel.downloadRate = Math.floor(jitter(t.tunnel.downloadRate || 900_000, 40_000, 7_500_000));
      t.tunnel.uploadRate = Math.floor(jitter(t.tunnel.uploadRate || 90_000, 4_000, 900_000));
      t.direct.downloadRate = Math.floor(jitter(t.direct.downloadRate || 3_000_000, 120_000, 24_000_000));
      t.direct.uploadRate = Math.floor(jitter(t.direct.uploadRate || 240_000, 10_000, 2_000_000));
      for (const route of [t.tunnel, t.direct]) {
        route.download += route.downloadRate;
        route.upload += route.uploadRate;
      }
      t.tunnel.connections = connections.filter((c) => c.route === "tunnel").length;
      t.direct.connections = connections.filter((c) => c.route === "direct").length;
      if (Math.random() < 0.12) {
        const c = connections[connections.length - 1];
        if (c) log("debug", "engine", `dns: exchanged A ${c.host}`);
      }
      if (Math.random() < 0.04) log("info", "engine", "endpoint/wireguard[tunnel]: received handshake response");
    }
    emit();
  }, 1000);

  (globalThis as Record<string, unknown>).splitwireMock = {
    dropConf: () => importHandlers.forEach((h) => h(mockParse(sampleConf(), "dropped-config"))),
  };

  log("info", "service", "Splitwire service 0.1.0 starting");
  log("info", "service", "Using sing-box 1.14.4 at C:\\Program Files\\Splitwire\\engine\\sing-box.exe");
  log("info", "service", "Listening on \\\\.\\pipe\\splitwire-service");

  const backend: Backend = {
    async listTunnels() {
      await sleep(60);
      return clone(tunnels);
    },
    async saveTunnel(tunnel) {
      await sleep(120);
      const errors = mockValidate(tunnel);
      if (errors.length) throw { kind: "validation", errors } satisfies CommandError;
      const saved = clone(tunnel);
      const index = tunnels.findIndex((t) => t.id === saved.id);
      if (index >= 0) tunnels[index] = saved;
      else tunnels.push(saved);
      return clone(saved);
    },
    async deleteTunnel(id) {
      await sleep(80);
      if (engine.tunnelId === id) await backend.disconnect();
      tunnels = tunnels.filter((t) => t.id !== id);
    },
    async importTunnelFile() {
      await sleep(350);
      return mockParse(sampleConf(), "office");
    },
    async importTunnelText(text, name) {
      await sleep(120);
      return mockParse(text, name);
    },
    async exportTunnelFile() {
      await sleep(300);
      return true;
    },
    async generateKeypair() {
      const privateKey = randomKey();
      return { privateKey, publicKey: await derivePublicKey(privateKey) };
    },
    publicKey: derivePublicKey,

    async getServiceStatus() {
      await sleep(40);
      return status();
    },
    async installService() {
      await sleep(1600);
      service = "running";
      log("info", "service", "Service installed and started");
      emit();
    },
    async connect(tunnelId) {
      requireService();
      const tunnel = tunnels.find((t) => t.id === tunnelId);
      if (!tunnel) throw { kind: "other", message: "That tunnel no longer exists." } satisfies CommandError;
      if (engine.state === "connected" && engine.tunnelId !== tunnelId) {
        log("info", "service", `Stopping ${engine.tunnelName} to switch tunnels`);
      }
      connections = [];
      engine = { ...clone(DISCONNECTED), state: "connecting", tunnelId, tunnelName: tunnel.name };
      log("info", "service", `Connecting to ${tunnel.name} (${tunnel.peer.endpoint})`);
      log("info", "engine", "inbound/tun[tun-in]: started at Splitwire");
      emit();
      await sleep(1300);
      if (tunnel.name === "lab") {
        const error = "No handshake from lab.example.net:51820 after 5 seconds. Check that the server is reachable and its public key matches.";
        engine = { ...engine, state: "failed", error };
        log("error", "engine", "endpoint/wireguard[tunnel]: handshake did not complete after 5 seconds");
        log("error", "service", error);
        emit();
        throw { kind: "service", message: error } satisfies CommandError;
      }
      engine = { ...engine, state: "connected", connectedAt: Date.now(), health: { state: "checking" } };
      for (let i = 0; i < 12; i++) connections.push(newConnection(tunnel));
      log("info", "engine", "sing-box started (0.412s)");
      log("info", "service", `Connected to ${tunnel.name}`);
      settings = { ...settings, lastTunnelId: tunnelId };
      emit();
      setTimeout(() => {
        if (engine.state !== "connected" || engine.tunnelId !== tunnelId) return;
        engine = {
          ...engine,
          health: params.get("health") === "failing"
            ? {
                state: "failing",
                message: "No reply through the tunnel. Check that the server is up, that the keys match, and that the server forwards traffic.",
              }
            : { state: "ok", latencyMs: 18 + Math.round(Math.random() * 20) },
        };
        emit();
      }, 900);
    },
    async disconnect() {
      requireService();
      if (engine.state === "disconnected") return;
      const name = engine.tunnelName;
      engine = { ...engine, state: "disconnecting" };
      emit();
      await sleep(550);
      engine = clone(DISCONNECTED);
      connections = [];
      log("info", "service", `Disconnected from ${name}`);
      emit();
    },
    async getConnections() {
      await sleep(30);
      return engine.state === "connected" ? clone(connections) : [];
    },
    async getLogs(after) {
      await sleep(30);
      return logs.filter((entry) => entry.seq > after);
    },

    async listProcesses() {
      await sleep(180);
      return clone(PROCESSES);
    },
    async getIcons(paths) {
      await sleep(50);
      return Object.fromEntries(paths.map((path) => [path, iconFor(path)]));
    },
    async pickExecutable() {
      await sleep(400);
      return { name: "Postman.exe", path: `${U}\\Postman\\Postman.exe`, description: "Postman" };
    },
    async pickFolder() {
      await sleep(400);
      return `${PF}\\JetBrains`;
    },

    async getSettings() {
      return clone(settings);
    },
    async updateSettings(next) {
      await sleep(60);
      settings = clone(next);
      return clone(settings);
    },

    async frontendReady() {},

    async getAppVersion() {
      return VERSION;
    },
    async getAvailableUpdate() {
      return clone(foundUpdate);
    },
    async checkForUpdate() {
      await sleep(900);
      foundUpdate = mockUpdate;
      return clone(foundUpdate);
    },
    async installUpdate() {
      if (!foundUpdate) throw { kind: "other", message: "Splitwire is already up to date" } satisfies CommandError;
      const total = 22_180_000;
      for (let downloaded = 0; downloaded < total; downloaded += 1_400_000) {
        progressHandlers.forEach((h) => h({ downloaded, total }));
        await sleep(120);
      }
      progressHandlers.forEach((h) => h({ downloaded: total, total }));
      await sleep(400);
      if (params.get("update") === "fail") {
        throw { kind: "other", message: "Couldn't install the update: The administrator prompt was cancelled" } satisfies CommandError;
      }
      // The real app exits here and the installer relaunches it.
      await sleep(1500);
      location.reload();
    },

    async onServiceStatus(handler) {
      statusHandlers.add(handler);
      return () => statusHandlers.delete(handler);
    },
    async onTunnelsChanged(handler) {
      tunnelHandlers.add(handler);
      return () => tunnelHandlers.delete(handler);
    },
    async onImport(handler) {
      importHandlers.add(handler);
      return () => importHandlers.delete(handler);
    },
    async onUpdateAvailable(handler) {
      updateHandlers.add(handler);
      return () => updateHandlers.delete(handler);
    },
    async onUpdateProgress(handler) {
      progressHandlers.add(handler);
      return () => progressHandlers.delete(handler);
    },
  };

  // Like the real background check, a little after startup.
  if (mockUpdate) {
    setTimeout(() => {
      foundUpdate = mockUpdate;
      updateHandlers.forEach((h) => h(clone(mockUpdate)));
    }, 1500);
  }

  const autoConnect = params.get("connect");
  if (autoConnect) {
    const target = tunnels.find((t) => t.name === autoConnect);
    if (target) setTimeout(() => backend.connect(target.id).catch(() => {}), 300);
  }

  return backend;
}
