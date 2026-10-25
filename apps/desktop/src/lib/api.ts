// The UI's only door to the backend. In the Tauri app every call goes to a
// Rust command in apps/desktop/src-tauri/src/commands.rs; in a plain browser
// (`pnpm dev:web`) it goes to an in-memory mock so the UI can be built
// without Rust or admin rights.

import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { createMockBackend } from "./mock";
import type {
  CommandError,
  Connection,
  ImportedTunnel,
  KeyPair,
  LogEntry,
  PickedApp,
  ProcessInfo,
  ServiceStatus,
  Settings,
  Tunnel,
} from "./types";

export interface Backend {
  listTunnels(): Promise<Tunnel[]>;
  /** Validates, then saves (insert or replace by id). Rejects with a "validation" CommandError. */
  saveTunnel(tunnel: Tunnel): Promise<Tunnel>;
  deleteTunnel(id: string): Promise<void>;
  /** Opens a file picker for .conf files. Resolves null if cancelled. Does not save. */
  importTunnelFile(): Promise<ImportedTunnel | null>;
  /** Parses pasted .conf text. Does not save. */
  importTunnelText(text: string, name: string): Promise<ImportedTunnel>;
  /** Opens a save dialog and writes the tunnel as a .conf. Resolves false if cancelled. */
  exportTunnelFile(id: string): Promise<boolean>;
  generateKeypair(): Promise<KeyPair>;
  publicKey(privateKey: string): Promise<string>;

  getServiceStatus(): Promise<ServiceStatus>;
  /** Runs the elevated service installer (shows a UAC prompt). */
  installService(): Promise<void>;
  connect(tunnelId: string): Promise<void>;
  disconnect(): Promise<void>;
  getConnections(): Promise<Connection[]>;
  /** Log entries with seq greater than `after`. */
  getLogs(after: number): Promise<LogEntry[]>;

  listProcesses(): Promise<ProcessInfo[]>;
  /** Exe paths to PNG data URLs (null when the exe has no icon). */
  getIcons(paths: string[]): Promise<Record<string, string | null>>;
  /** Opens a file picker for .exe files. */
  pickExecutable(): Promise<PickedApp | null>;
  pickFolder(): Promise<string | null>;

  getSettings(): Promise<Settings>;
  updateSettings(settings: Settings): Promise<Settings>;

  /** Called once the UI has rendered; the backend then shows the window (unless launched to the tray). */
  frontendReady(): Promise<void>;

  /** Pushed about once a second, and whenever something changes. */
  onServiceStatus(handler: (status: ServiceStatus) => void): Promise<UnlistenFn>;
  /** Tunnels changed outside the UI (e.g. from the tray). */
  onTunnelsChanged(handler: (tunnels: Tunnel[]) => void): Promise<UnlistenFn>;
  /** A .conf file was dropped on the window or opened with the app. */
  onImport(handler: (imported: ImportedTunnel) => void): Promise<UnlistenFn>;
}

const tauriBackend: Backend = {
  listTunnels: () => invoke("list_tunnels"),
  saveTunnel: (tunnel) => invoke("save_tunnel", { tunnel }),
  deleteTunnel: (id) => invoke("delete_tunnel", { id }),
  importTunnelFile: () => invoke("import_tunnel_file"),
  importTunnelText: (text, name) => invoke("import_tunnel_text", { text, name }),
  exportTunnelFile: (id) => invoke("export_tunnel_file", { id }),
  generateKeypair: () => invoke("generate_keypair"),
  publicKey: (privateKey) => invoke("public_key", { privateKey }),

  getServiceStatus: () => invoke("get_service_status"),
  installService: () => invoke("install_service"),
  connect: (tunnelId) => invoke("connect", { tunnelId }),
  disconnect: () => invoke("disconnect"),
  getConnections: () => invoke("get_connections"),
  getLogs: (after) => invoke("get_logs", { after }),

  listProcesses: () => invoke("list_processes"),
  getIcons: (paths) => invoke("get_icons", { paths }),
  pickExecutable: () => invoke("pick_executable"),
  pickFolder: () => invoke("pick_folder"),

  getSettings: () => invoke("get_settings"),
  updateSettings: (settings) => invoke("update_settings", { settings }),

  frontendReady: () => invoke("frontend_ready"),

  onServiceStatus: (handler) => listen<ServiceStatus>("service-status", (e) => handler(e.payload)),
  onTunnelsChanged: (handler) => listen<Tunnel[]>("tunnels-changed", (e) => handler(e.payload)),
  onImport: (handler) => listen<ImportedTunnel>("import-tunnel", (e) => handler(e.payload)),
};

export const api: Backend = isTauri() ? tauriBackend : createMockBackend();

export const isMockBackend = !isTauri();

/** Turns anything a command rejects with into a readable message. */
export function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error && typeof error === "object" && "kind" in error) {
    const e = error as CommandError;
    if (e.kind === "validation") return e.errors.map((v) => v.message).join("\n");
    return e.message;
  }
  if (error instanceof Error) return error.message;
  return String(error);
}

export function isValidationError(
  error: unknown,
): error is Extract<CommandError, { kind: "validation" }> {
  return !!error && typeof error === "object" && (error as CommandError).kind === "validation";
}
