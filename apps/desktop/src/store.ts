import { create } from "zustand";

import { api, errorMessage, isValidationError } from "./lib/api";
import type {
  Connection,
  EngineStatus,
  ImportedTunnel,
  ProcessInfo,
  ServiceStatus,
  Settings,
  Tunnel,
  UpdateInfo,
  UpdateProgress,
  ValidationError,
} from "./lib/types";

export type View = "tunnel" | "logs" | "settings";
export type Tab = "routing" | "connections" | "wireguard";

export interface TrafficSample {
  tunnelDown: number;
  tunnelUp: number;
  directDown: number;
  directUp: number;
}

export type Dialog =
  | { kind: "import"; imported: ImportedTunnel | null; mode: "file" | "paste" }
  | { kind: "newTunnel" }
  | { kind: "picker"; target: "apps" | "bypassApps" }
  | { kind: "deleteTunnel"; id: string }
  | { kind: "update" }
  | null;

/** Where an update stands. "installing": the installer is starting and the app is about to exit. */
export type UpdatePhase = "idle" | "checking" | "upToDate" | "downloading" | "installing" | "error";

export interface Toast {
  id: number;
  message: string;
  tone: "info" | "error";
}

const HISTORY = 90;

interface State {
  ready: boolean;
  tunnels: Tunnel[];
  selectedId: string | null;
  view: View;
  tab: Tab;
  service: ServiceStatus | null;
  history: TrafficSample[];
  connections: Connection[];
  processes: ProcessInfo[];
  icons: Record<string, string | null>;
  settings: Settings | null;
  /** Tunnel ids whose routing changed while they were connected. */
  stale: Record<string, boolean>;
  /** Field errors from the last failed save, keyed by tunnel id. */
  errors: Record<string, ValidationError[]>;
  saving: Record<string, "saving" | "saved" | "error">;
  dialog: Dialog;
  toasts: Toast[];
  busy: boolean;
  appVersion: string | null;
  /** A newer release, once a check has found one. */
  update: UpdateInfo | null;
  updatePhase: UpdatePhase;
  updateProgress: UpdateProgress | null;
  updateError: string | null;

  init(): Promise<void>;
  select(id: string): void;
  setView(view: View): void;
  setTab(tab: Tab): void;
  openDialog(dialog: Dialog): void;
  toast(message: string, tone?: Toast["tone"]): void;
  dismissToast(id: number): void;
  saveTunnel(tunnel: Tunnel): Promise<boolean>;
  addTunnel(tunnel: Tunnel): Promise<boolean>;
  deleteTunnel(id: string): Promise<void>;
  connect(id: string): Promise<void>;
  disconnect(): Promise<void>;
  refreshProcesses(): Promise<void>;
  requestIcons(paths: (string | null | undefined)[]): void;
  updateSettings(patch: Partial<Settings>): Promise<void>;
  installService(): Promise<void>;
  checkForUpdate(): Promise<void>;
  installUpdate(): Promise<void>;
}

export function engineOf(service: ServiceStatus | null): EngineStatus | null {
  return service?.kind === "running" ? service.status : null;
}

let initialized: Promise<void> | null = null;
let toastId = 0;
let iconQueue = new Set<string>();
let iconTimer: ReturnType<typeof setTimeout> | null = null;
let connectionsTimer: ReturnType<typeof setInterval> | null = null;
let saveCounter = 0;

export const useStore = create<State>((set, get) => ({
  ready: false,
  tunnels: [],
  selectedId: null,
  view: "tunnel",
  tab: "routing",
  service: null,
  history: [],
  connections: [],
  processes: [],
  icons: {},
  settings: null,
  stale: {},
  errors: {},
  saving: {},
  dialog: null,
  toasts: [],
  busy: false,
  appVersion: null,
  update: null,
  updatePhase: "idle",
  updateProgress: null,
  updateError: null,

  init() {
    // StrictMode mounts twice in development; only load and subscribe once.
    initialized ??= (async () => {
      const [tunnels, settings, service] = await Promise.all([
        api.listTunnels(),
        api.getSettings(),
        api.getServiceStatus().catch(
          (e): ServiceStatus => ({ kind: "unreachable", message: errorMessage(e) }),
        ),
      ]);
      const engine = engineOf(service);
      const preferred = engine?.tunnelId ?? settings.lastTunnelId;
      set({
        tunnels,
        settings,
        service,
        selectedId: tunnels.some((t) => t.id === preferred) ? preferred : (tunnels[0]?.id ?? null),
        ready: true,
      });

      await api.onServiceStatus((next) => applyStatus(next));
      await api.onTunnelsChanged((next) => set({ tunnels: next }));
      await api.onImport((imported) => set({ dialog: { kind: "import", imported, mode: "file" } }));
      await api.onUpdateAvailable((update) => set({ update }));
      await api.onUpdateProgress((progress) =>
        set({
          updateProgress: progress,
          updatePhase: progress.total !== null && progress.downloaded >= progress.total ? "installing" : "downloading",
        }),
      );
      void get().refreshProcesses();
      void Promise.all([api.getAppVersion(), api.getAvailableUpdate()]).then(([appVersion, update]) =>
        set((s) => ({ appVersion, update: s.update ?? update })),
      );
    })();
    return initialized;
  },

  select(id) {
    set({ selectedId: id, view: "tunnel" });
  },
  setView(view) {
    set({ view });
  },
  setTab(tab) {
    set({ tab });
  },
  openDialog(dialog) {
    set({ dialog });
  },

  toast(message, tone = "info") {
    const id = ++toastId;
    set((s) => ({ toasts: [...s.toasts, { id, message, tone }].slice(-4) }));
    setTimeout(() => get().dismissToast(id), tone === "error" ? 7000 : 3200);
  },
  dismissToast(id) {
    set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) }));
  },

  async saveTunnel(tunnel) {
    const previous = get().tunnels.find((t) => t.id === tunnel.id);
    const ticket = ++saveCounter;
    set((s) => ({
      tunnels: s.tunnels.map((t) => (t.id === tunnel.id ? tunnel : t)),
      saving: { ...s.saving, [tunnel.id]: "saving" },
    }));
    try {
      const saved = await api.saveTunnel(tunnel);
      if (ticket !== saveCounter) return true;
      const engine = engineOf(get().service);
      const live = engine?.tunnelId === tunnel.id && engine.state === "connected";
      const routingChanged =
        previous && JSON.stringify(previous.routing) !== JSON.stringify(saved.routing);
      const engineChanged =
        previous &&
        (JSON.stringify(previous.interface) !== JSON.stringify(saved.interface) ||
          JSON.stringify(previous.peer) !== JSON.stringify(saved.peer));
      set((s) => ({
        tunnels: s.tunnels.map((t) => (t.id === saved.id ? saved : t)),
        saving: { ...s.saving, [saved.id]: "saved" },
        errors: { ...s.errors, [saved.id]: [] },
        stale: live && (routingChanged || engineChanged) ? { ...s.stale, [saved.id]: true } : s.stale,
      }));
      return true;
    } catch (error) {
      if (isValidationError(error)) {
        set((s) => ({
          errors: { ...s.errors, [tunnel.id]: error.errors },
          saving: { ...s.saving, [tunnel.id]: "error" },
        }));
      } else {
        set((s) => ({ saving: { ...s.saving, [tunnel.id]: "error" } }));
        get().toast(`Couldn't save ${tunnel.name}: ${errorMessage(error)}`, "error");
      }
      return false;
    }
  },

  async addTunnel(tunnel) {
    try {
      const saved = await api.saveTunnel(tunnel);
      set((s) => ({
        tunnels: [...s.tunnels.filter((t) => t.id !== saved.id), saved],
        selectedId: saved.id,
        view: "tunnel",
        tab: "routing",
        dialog: null,
      }));
      get().toast(`Added ${saved.name}`);
      return true;
    } catch (error) {
      if (isValidationError(error)) {
        set((s) => ({ errors: { ...s.errors, [tunnel.id]: error.errors } }));
      } else {
        get().toast(`Couldn't add the tunnel: ${errorMessage(error)}`, "error");
      }
      return false;
    }
  },

  async deleteTunnel(id) {
    const tunnel = get().tunnels.find((t) => t.id === id);
    try {
      await api.deleteTunnel(id);
      set((s) => {
        const tunnels = s.tunnels.filter((t) => t.id !== id);
        return {
          tunnels,
          selectedId: s.selectedId === id ? (tunnels[0]?.id ?? null) : s.selectedId,
          dialog: null,
        };
      });
      get().toast(`Deleted ${tunnel?.name ?? "tunnel"}`);
    } catch (error) {
      get().toast(`Couldn't delete ${tunnel?.name}: ${errorMessage(error)}`, "error");
    }
  },

  async connect(id) {
    set((s) => ({ busy: true, stale: { ...s.stale, [id]: false }, history: [] }));
    try {
      await api.connect(id);
      const settings = get().settings;
      if (settings && settings.lastTunnelId !== id) {
        set({ settings: { ...settings, lastTunnelId: id } });
      }
    } catch (error) {
      // Failed connects show up in the route panel via the status; only toast
      // when the status can't carry the message.
      const engine = engineOf(get().service);
      if (engine?.state !== "failed") get().toast(errorMessage(error), "error");
    } finally {
      set({ busy: false });
    }
  },

  async disconnect() {
    set({ busy: true });
    try {
      await api.disconnect();
    } catch (error) {
      get().toast(`Couldn't disconnect: ${errorMessage(error)}`, "error");
    } finally {
      set({ busy: false });
    }
  },

  async refreshProcesses() {
    try {
      const processes = await api.listProcesses();
      set({ processes });
    } catch (error) {
      get().toast(`Couldn't list running apps: ${errorMessage(error)}`, "error");
    }
  },

  requestIcons(paths) {
    const known = get().icons;
    for (const path of paths) {
      if (path && !(path in known)) iconQueue.add(path);
    }
    if (!iconQueue.size || iconTimer) return;
    iconTimer = setTimeout(async () => {
      const batch = [...iconQueue];
      iconQueue = new Set();
      iconTimer = null;
      // Mark as pending so they aren't requested twice.
      set((s) => ({ icons: { ...Object.fromEntries(batch.map((p) => [p, null])), ...s.icons } }));
      try {
        const result = await api.getIcons(batch);
        set((s) => ({ icons: { ...s.icons, ...result } }));
      } catch {
        // Icons are decoration; fall back to monograms.
      }
    }, 16);
  },

  async updateSettings(patch) {
    const current = get().settings;
    if (!current) return;
    const next = { ...current, ...patch };
    set({ settings: next });
    try {
      set({ settings: await api.updateSettings(next) });
    } catch (error) {
      set({ settings: current });
      get().toast(`Couldn't save settings: ${errorMessage(error)}`, "error");
    }
  },

  async installService() {
    set({ busy: true });
    try {
      await api.installService();
      const service = await api.getServiceStatus();
      applyStatus(service);
      if (service.kind === "running") get().toast("Service installed");
    } catch (error) {
      get().toast(`Couldn't install the service: ${errorMessage(error)}`, "error");
    } finally {
      set({ busy: false });
    }
  },

  async checkForUpdate() {
    set({ updatePhase: "checking", updateError: null });
    try {
      const update = await api.checkForUpdate();
      set({ update, updatePhase: update ? "idle" : "upToDate" });
    } catch (error) {
      set({ updatePhase: "error", updateError: errorMessage(error) });
    }
  },

  async installUpdate() {
    set({ updatePhase: "downloading", updateProgress: null, updateError: null });
    try {
      await api.installUpdate();
      // Only reached in a dry run; a real install exits the app.
      set({ updatePhase: "idle", updateProgress: null });
      get().toast("Update downloaded and verified (dry run, not installed)");
    } catch (error) {
      set({ updatePhase: "error", updateProgress: null, updateError: errorMessage(error) });
    }
  },
}));

function applyStatus(service: ServiceStatus) {
  const engine = engineOf(service);
  useStore.setState((s) => {
    const history =
      engine?.state === "connected"
        ? [
            ...s.history,
            {
              tunnelDown: engine.traffic.tunnel.downloadRate,
              tunnelUp: engine.traffic.tunnel.uploadRate,
              directDown: engine.traffic.direct.downloadRate,
              directUp: engine.traffic.direct.uploadRate,
            },
          ].slice(-HISTORY)
        : s.history;
    return { service, history };
  });
  syncConnectionsPolling(engine?.state === "connected");
}

function syncConnectionsPolling(connected: boolean) {
  if (connected && !connectionsTimer) {
    const poll = async () => {
      try {
        const connections = await api.getConnections();
        useStore.setState({ connections });
        useStore.getState().requestIcons(connections.map((c) => c.processPath));
      } catch {
        // Next tick will retry.
      }
    };
    void poll();
    connectionsTimer = setInterval(poll, 1500);
  } else if (!connected && connectionsTimer) {
    clearInterval(connectionsTimer);
    connectionsTimer = null;
    useStore.setState({ connections: [] });
  }
}

export function useEngine(): EngineStatus | null {
  return useStore((s) => engineOf(s.service));
}

export function useSelectedTunnel(): Tunnel | undefined {
  return useStore((s) => s.tunnels.find((t) => t.id === s.selectedId));
}
