import clsx from "clsx";
import { motion } from "motion/react";
import { Check, TriangleAlert } from "lucide-react";

import { RoutePanel } from "@/components/RoutePanel";
import { Spinner } from "@/components/ui";
import type { Tunnel } from "@/lib/types";
import { useEngine, useStore, type Tab } from "@/store";

import { ConnectionsView } from "./ConnectionsView";
import { RoutingEditor } from "./RoutingEditor";
import { WireGuardEditor } from "./WireGuardEditor";

export function TunnelView({ tunnel }: { tunnel: Tunnel }) {
  const engine = useEngine();
  const service = useStore((s) => s.service);
  const tab = useStore((s) => s.tab);
  const setTab = useStore((s) => s.setTab);
  const saving = useStore((s) => s.saving[tunnel.id]);
  const connectionCount = useStore((s) => s.connections.length);
  const live = engine?.tunnelId === tunnel.id && engine.state === "connected";

  const tabs: { id: Tab; label: string; badge?: number }[] = [
    { id: "routing", label: "Routing" },
    { id: "connections", label: "Connections", badge: live ? connectionCount : undefined },
    { id: "wireguard", label: "WireGuard" },
  ];

  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="px-8 pt-6">
        <div className="flex items-end justify-between gap-6">
          <div className="min-w-0">
            <h1 className="truncate font-expanded text-[22px] leading-tight font-semibold tracking-[-0.02em]">
              {tunnel.name}
            </h1>
            <p className="selectable mt-1 truncate font-mono text-xs text-ink-3">
              {tunnel.peer.endpoint}
              <span className="mx-2 text-ink-4">/</span>
              {tunnel.interface.addresses.join(", ")}
              {tunnel.interface.dns.length > 0 && (
                <>
                  <span className="mx-2 text-ink-4">/</span>
                  DNS {tunnel.interface.dns.join(", ")}
                </>
              )}
            </p>
          </div>
          <SaveState state={saving} />
        </div>

        <div className="mt-5">
          <RoutePanel tunnel={tunnel} engine={engine} service={service} />
        </div>
      </div>

      <div className="sticky top-0 z-10 bg-enamel/95 px-8 pt-3 backdrop-blur-sm">
        <div role="tablist" className="flex gap-6 border-b border-rule">
          {tabs.map((t) => (
            <button
              key={t.id}
              role="tab"
              type="button"
              aria-selected={tab === t.id}
              onClick={() => setTab(t.id)}
              className={clsx(
                "relative flex h-9 items-center gap-2 text-[13px] font-medium transition-colors",
                tab === t.id ? "text-ink" : "text-ink-3 hover:text-ink-2",
              )}
            >
              {t.label}
              {t.badge !== undefined && (
                <span className="rounded-full bg-lamp-deep px-1.5 font-mono text-2xs leading-[18px] text-lamp tabular">
                  {t.badge}
                </span>
              )}
              {tab === t.id && (
                <motion.span
                  layoutId="tab-underline"
                  className="absolute right-0 -bottom-px left-0 h-[2px] rounded-full bg-ink"
                  transition={{ type: "spring", duration: 0.3, bounce: 0.1 }}
                />
              )}
            </button>
          ))}
        </div>
      </div>

      <div className="px-8 pt-5 pb-12">
        {tab === "routing" && <RoutingEditor tunnel={tunnel} />}
        {tab === "connections" && <ConnectionsView live={live} />}
        {tab === "wireguard" && <WireGuardEditor key={tunnel.id} tunnel={tunnel} />}
      </div>
    </div>
  );
}

function SaveState({ state }: { state: "saving" | "saved" | "error" | undefined }) {
  if (!state) return null;
  return (
    <span
      className={clsx(
        "flex shrink-0 items-center gap-1.5 pb-1 text-xs",
        state === "error" ? "text-signal" : "text-ink-3",
      )}
    >
      {state === "saving" && <Spinner size={11} />}
      {state === "saved" && <Check size={13} />}
      {state === "error" && <TriangleAlert size={13} />}
      {state === "saving" ? "Saving" : state === "saved" ? "Saved" : "Not saved — check the highlighted fields"}
    </span>
  );
}
