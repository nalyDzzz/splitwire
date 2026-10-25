import clsx from "clsx";
import { Search, Unplug } from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import { AppIcon } from "@/components/AppIcon";
import { Segmented, TextInput } from "@/components/ui";
import { basename, formatAge, formatBytes } from "@/lib/format";
import type { Route } from "@/lib/types";
import { useStore } from "@/store";

type Filter = "all" | "tunnel" | "direct";

export function ConnectionsView({ live }: { live: boolean }) {
  const connections = useStore((s) => s.connections);
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<Filter>("all");
  const now = useTick();

  const rows = useMemo(() => {
    const q = query.trim().toLowerCase();
    return connections
      .filter((c) => filter === "all" || c.route === filter)
      .filter(
        (c) =>
          !q ||
          c.host.toLowerCase().includes(q) ||
          (c.processName ?? "").toLowerCase().includes(q) ||
          String(c.port).includes(q),
      )
      .sort((a, b) => b.download + b.upload - (a.download + a.upload));
  }, [connections, query, filter]);

  if (!live) {
    return (
      <div className="flex flex-col items-center justify-center rounded-2xl border border-dashed border-rule-2 py-16 text-center">
        <Unplug size={22} className="text-ink-3" />
        <p className="mt-3 text-[13px] font-medium text-ink-2">Connect to see live connections</p>
        <p className="mt-1 max-w-sm text-xs text-ink-3">
          Every connection your apps open shows up here, with the route it took.
        </p>
      </div>
    );
  }

  const counts = {
    tunnel: connections.filter((c) => c.route === "tunnel").length,
    direct: connections.filter((c) => c.route === "direct").length,
  };

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-3">
        <div className="relative w-72">
          <Search size={14} className="pointer-events-none absolute top-1/2 left-2.5 -translate-y-1/2 text-ink-3" />
          <TextInput
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Filter by app, host or port"
            className="pl-8"
          />
        </div>
        <Segmented<Filter>
          size="sm"
          value={filter}
          onChange={setFilter}
          options={[
            { value: "all", label: `All ${connections.length}` },
            { value: "tunnel", label: `Tunnel ${counts.tunnel}` },
            { value: "direct", label: `Direct ${counts.direct}` },
          ]}
        />
      </div>

      <div
        className="overflow-x-auto rounded-xl border border-rule"
        style={{ ["--conn-cols" as string]: "minmax(120px,1fr) minmax(150px,1.5fr) 76px 66px 66px 40px" }}
      >
        <div className="grid grid-cols-(--conn-cols) items-center gap-2.5 border-b border-rule bg-plate px-3.5 py-2">
          {["App", "Destination", "Route", "Down", "Up", "Age"].map((h, i) => (
            <span key={h} className={clsx("engrave text-ink-3", i >= 3 && "text-right")}>
              {h}
            </span>
          ))}
        </div>
        <div className="divide-y divide-rule/60">
          {rows.map((c) => {
            const name = c.processName ?? (c.processPath ? basename(c.processPath) : "Unknown app");
            return (
              <div
                key={c.id}
                className="grid grid-cols-(--conn-cols) items-center gap-2.5 px-3.5 py-1.5 text-[12.5px] hover:bg-plate/70"
              >
                <span className="flex min-w-0 items-center gap-2.5">
                  <AppIcon path={c.processPath} name={name} size={18} />
                  <span className="truncate text-ink">{name.replace(/\.exe$/i, "")}</span>
                </span>
                <span className="selectable flex min-w-0 items-baseline gap-1.5 font-mono text-xs text-ink-2">
                  {c.network !== "tcp" && (
                    <span className="shrink-0 text-[9.5px] text-ink-3 uppercase">{c.network}</span>
                  )}
                  <span className="truncate">
                    {c.host}
                    <span className="text-ink-4">:{c.port}</span>
                  </span>
                </span>
                <RouteBadge route={c.route} />
                <span className="text-right font-mono text-xs tabular text-ink-2">{formatBytes(c.download)}</span>
                <span className="text-right font-mono text-xs tabular text-ink-3">{formatBytes(c.upload)}</span>
                <span className="text-right font-mono text-2xs tabular text-ink-3">{formatAge(c.startedAt, now)}</span>
              </div>
            );
          })}
          {!rows.length && (
            <p className="px-3.5 py-6 text-center text-xs text-ink-3">
              {connections.length ? "No connections match this filter." : "No connections yet."}
            </p>
          )}
        </div>
      </div>
    </div>
  );
}

function RouteBadge({ route }: { route: Route }) {
  return (
    <span
      className={clsx(
        "inline-flex w-fit items-center gap-1.5 rounded-md px-1.5 py-0.5 text-2xs font-medium",
        route === "tunnel" && "bg-lamp-deep text-lamp",
        route === "direct" && "bg-plate-3 text-track/80",
        route === "other" && "bg-plate-3 text-ink-3",
      )}
    >
      <span
        className={clsx(
          "size-1.5 rounded-full",
          route === "tunnel" ? "bg-lamp" : route === "direct" ? "bg-track/70" : "bg-ink-3",
        )}
      />
      {route === "tunnel" ? "Tunnel" : route === "direct" ? "Direct" : "Other"}
    </span>
  );
}

function useTick(): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, []);
  return now;
}
