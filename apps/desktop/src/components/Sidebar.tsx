import clsx from "clsx";
import { AnimatePresence, motion } from "motion/react";
import { ClipboardPaste, FileUp, Plus, ScrollText, Settings2, Sparkles } from "lucide-react";
import { useState, type ReactNode } from "react";

import { api, errorMessage } from "@/lib/api";
import type { ServiceStatus } from "@/lib/types";
import { useEngine, useStore } from "@/store";

import { Lamp, useDismiss } from "./ui";

export function Sidebar() {
  const tunnels = useStore((s) => s.tunnels);
  const selectedId = useStore((s) => s.selectedId);
  const view = useStore((s) => s.view);
  const select = useStore((s) => s.select);
  const setView = useStore((s) => s.setView);
  const service = useStore((s) => s.service);
  const engine = useEngine();

  return (
    <aside className="flex w-[236px] shrink-0 flex-col border-r border-rule/70 bg-sunk/60">
      <div className="flex items-center justify-between px-4 pt-4 pb-2">
        <span className="engrave text-ink-3">Tunnels</span>
        <AddMenu />
      </div>

      <nav className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto px-2 pb-3">
        {!tunnels.length && <p className="px-2.5 py-2 text-xs text-ink-4">No tunnels yet</p>}
        {tunnels.map((tunnel) => {
          const selected = view === "tunnel" && tunnel.id === selectedId;
          const active = engine?.tunnelId === tunnel.id ? engine.state : null;
          const host = tunnel.peer.endpoint.replace(/:\d+$/, "").replace(/^\[|\]$/g, "");
          return (
            <button
              key={tunnel.id}
              type="button"
              onClick={() => select(tunnel.id)}
              className={clsx(
                "group relative flex items-center gap-3 rounded-lg px-2.5 py-2 text-left transition-colors",
                selected ? "bg-plate-2" : "hover:bg-plate/80",
              )}
            >
              {selected && (
                <motion.span
                  layoutId="sidebar-selected"
                  className="absolute top-2 bottom-2 left-0 w-[2px] rounded-full bg-ink"
                  transition={{ type: "spring", duration: 0.3, bounce: 0.1 }}
                />
              )}
              <TunnelGlyph state={active} />
              <span className="min-w-0 flex-1">
                <span
                  className={clsx(
                    "block truncate text-[13px] font-medium",
                    selected ? "text-ink" : "text-ink-2 group-hover:text-ink",
                  )}
                >
                  {tunnel.name}
                </span>
                <span className="block truncate font-mono text-2xs text-ink-3">{host}</span>
              </span>
              {active && active !== "disconnected" && (
                <span
                  className={clsx(
                    "engrave text-[9.5px]",
                    active === "failed" ? "text-signal" : "text-lamp",
                  )}
                >
                  {active === "connected" ? "Live" : active === "failed" ? "Failed" : "···"}
                </span>
              )}
            </button>
          );
        })}
      </nav>

      <div className="flex flex-col gap-0.5 border-t border-rule/70 px-2 py-2">
        <NavItem
          icon={<ScrollText size={15} strokeWidth={1.75} />}
          label="Logs"
          active={view === "logs"}
          onClick={() => setView("logs")}
        />
        <NavItem
          icon={<Settings2 size={15} strokeWidth={1.75} />}
          label="Settings"
          active={view === "settings"}
          onClick={() => setView("settings")}
        />
        <ServiceHealth service={service} onClick={() => setView("settings")} />
      </div>
    </aside>
  );
}

/** A tiny two-track switch that shows whether this tunnel is live. */
function TunnelGlyph({ state }: { state: string | null }) {
  const lit = state === "connected";
  const setting = state === "connecting" || state === "disconnecting";
  const failed = state === "failed";
  return (
    <svg width="22" height="22" viewBox="0 0 22 22" fill="none" aria-hidden className="shrink-0">
      <path d="M2 14H8" stroke="var(--color-ink-3)" strokeWidth="1.6" strokeLinecap="round" />
      <path d="M8 14H20" stroke="var(--color-ink-4)" strokeWidth="1.6" strokeLinecap="round" />
      <path
        d="M8 14C11 14 11 7 14 7H20"
        stroke={failed ? "var(--color-signal)" : lit || setting ? "var(--color-lamp)" : "var(--color-ink-4)"}
        strokeWidth="1.6"
        strokeLinecap="round"
        className={setting ? "anim-route-set" : undefined}
        style={lit ? { filter: "drop-shadow(0 0 3px rgba(255,174,52,.8))" } : undefined}
      />
      <circle cx="8" cy="14" r="1.8" fill="var(--color-sunk)" stroke="var(--color-ink-3)" strokeWidth="1.2" />
    </svg>
  );
}

function NavItem({
  icon,
  label,
  active,
  onClick,
}: {
  icon: ReactNode;
  label: string;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={clsx(
        "flex h-8 items-center gap-2.5 rounded-lg px-2.5 text-[13px] transition-colors",
        active ? "bg-plate-2 text-ink" : "text-ink-2 hover:bg-plate/80 hover:text-ink",
      )}
    >
      {icon}
      {label}
    </button>
  );
}

function ServiceHealth({ service, onClick }: { service: ServiceStatus | null; onClick: () => void }) {
  const [tone, label] = !service
    ? (["off", "Checking service…"] as const)
    : service.kind === "running"
      ? (["go", "Service running"] as const)
      : service.kind === "notInstalled"
        ? (["off", "Service not installed"] as const)
        : service.kind === "stopped"
          ? (["off", "Service stopped"] as const)
          : (["signal", "Service unreachable"] as const);
  return (
    <button
      type="button"
      onClick={onClick}
      className="mt-1 flex h-8 items-center gap-2.5 rounded-lg px-2.5 text-xs text-ink-3 transition-colors hover:text-ink-2"
    >
      <span className="flex w-[15px] justify-center">
        <Lamp tone={tone} />
      </span>
      {label}
    </button>
  );
}

function AddMenu() {
  const [open, setOpen] = useState(false);
  const openDialog = useStore((s) => s.openDialog);
  const toast = useStore((s) => s.toast);
  const ref = useDismiss(open, () => setOpen(false));

  const importFile = async () => {
    setOpen(false);
    try {
      const imported = await api.importTunnelFile();
      if (imported) openDialog({ kind: "import", imported, mode: "file" });
    } catch (error) {
      toast(`Couldn't import that file: ${errorMessage(error)}`, "error");
    }
  };

  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        aria-label="Add tunnel"
        title="Add tunnel"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        className={clsx(
          "inline-flex size-6 items-center justify-center rounded-md border transition-colors",
          open ? "border-rule-2 bg-plate-3 text-ink" : "border-rule text-ink-2 hover:border-rule-2 hover:text-ink",
        )}
      >
        <Plus size={14} />
      </button>
      <AnimatePresence>
        {open && (
          <motion.div
            initial={{ opacity: 0, y: -4, scale: 0.98 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: -4, scale: 0.98 }}
            transition={{ duration: 0.12 }}
            className="absolute top-8 left-0 z-30 w-56 origin-top-left rounded-xl border border-rule-2 bg-plate-2 p-1 shadow-[0_18px_50px_-12px_rgba(0,0,0,.8)]"
          >
            <MenuItem icon={<FileUp size={15} />} label="Import .conf file" onClick={importFile} />
            <MenuItem
              icon={<ClipboardPaste size={15} />}
              label="Paste a config"
              onClick={() => {
                setOpen(false);
                openDialog({ kind: "import", imported: null, mode: "paste" });
              }}
            />
            <div className="my-1 h-px bg-rule" />
            <MenuItem
              icon={<Sparkles size={15} />}
              label="Set up from scratch"
              onClick={() => {
                setOpen(false);
                openDialog({ kind: "newTunnel" });
              }}
            />
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

function MenuItem({ icon, label, onClick }: { icon: ReactNode; label: string; onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex h-8 w-full items-center gap-2.5 rounded-lg px-2.5 text-[13px] text-ink-2 transition-colors hover:bg-plate-3 hover:text-ink"
    >
      <span className="text-ink-3">{icon}</span>
      {label}
    </button>
  );
}
