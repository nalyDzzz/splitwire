import { useEffect } from "react";

import { Overlays } from "./components/Overlays";
import { Sidebar } from "./components/Sidebar";
import { TitleBar } from "./components/TitleBar";
import { api } from "./lib/api";
import { useSelectedTunnel, useStore } from "./store";
import { EmptyState } from "./views/EmptyState";
import { LogsView } from "./views/LogsView";
import { SettingsView } from "./views/SettingsView";
import { TunnelView } from "./views/TunnelView";

export function App() {
  const ready = useStore((s) => s.ready);
  const init = useStore((s) => s.init);
  const view = useStore((s) => s.view);
  const firstId = useStore((s) => s.tunnels[0]?.id);
  const hasTunnels = !!firstId;
  const tunnel = useSelectedTunnel();

  // Keep a tunnel selected if the current one disappears (deleted from the tray, say).
  useEffect(() => {
    if (ready && !tunnel && firstId) useStore.setState({ selectedId: firstId });
  }, [ready, tunnel, firstId]);

  useEffect(() => {
    void init().finally(() => {
      // Let the first frame paint before the window appears.
      requestAnimationFrame(() => void api.frontendReady().catch(() => {}));
    });
  }, [init]);

  return (
    <div className="flex h-full flex-col">
      <TitleBar />
      <div className="flex min-h-0 flex-1">
        <Sidebar />
        <main className="flex min-w-0 flex-1 flex-col">
          {!ready ? null : view === "logs" ? (
            <LogsView />
          ) : view === "settings" ? (
            <SettingsView />
          ) : tunnel ? (
            <TunnelView key={tunnel.id} tunnel={tunnel} />
          ) : hasTunnels ? null : (
            <EmptyState />
          )}
        </main>
      </div>
      <Overlays />
    </div>
  );
}
