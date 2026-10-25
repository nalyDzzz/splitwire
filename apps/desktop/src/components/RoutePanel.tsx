// The signature element: an illuminated route diagram, like a signal-box
// panel. Traffic leaves "This PC" and is switched onto two tracks. The tunnel
// track flashes while the route is being set, then glows steady with traffic
// flowing along it. The apps riding each track sit on the line.

import clsx from "clsx";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { Power, RotateCw, ShieldAlert } from "lucide-react";
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode, type RefObject } from "react";

import { formatClock, formatDuration, plural, rateParts } from "@/lib/format";
import { ruleIconPath, ruleLabel } from "@/lib/rules";
import type { Connection, EngineStatus, ServiceStatus, Tunnel } from "@/lib/types";
import { useStore, type TrafficSample } from "@/store";

import { AppIcon } from "./AppIcon";
import { Button, Spinner } from "./ui";

const H = 216;
const Y_TUNNEL = 82;
const Y_DIRECT = 172;
const PAD = 28;

type PanelState = "disconnected" | "connecting" | "connected" | "disconnecting" | "failed" | "unavailable";

interface Rider {
  key: string;
  path: string | null;
  name: string;
  kind?: "folder" | "keyword";
}

export function RoutePanel({
  tunnel,
  engine,
  service,
}: {
  tunnel: Tunnel;
  engine: EngineStatus | null;
  service: ServiceStatus | null;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const width = useWidth(ref);
  const reduceMotion = useReducedMotion();
  const connections = useStore((s) => s.connections);
  const processes = useStore((s) => s.processes);
  const history = useStore((s) => s.history);

  const serviceReady = service?.kind === "running";
  const mine = engine?.tunnelId === tunnel.id;
  const state: PanelState = !serviceReady
    ? "unavailable"
    : mine && engine
      ? engine.state
      : "disconnected";
  const live = state === "connected";
  const traffic = live && engine ? engine.traffic : null;
  // Up, but the health probe can't get anything through.
  const failing = live && engine?.health.state === "failing";

  // Geometry.
  const W = width;
  const xJ = Math.round(Math.max(300, W * 0.4));
  const xCurveEnd = xJ + 120;
  const xE = W - PAD;
  const trunk = `M${PAD} ${Y_DIRECT} H${xJ}`;
  const tunnelBranch = `M${xJ} ${Y_DIRECT} C${xJ + 64} ${Y_DIRECT} ${xJ + 52} ${Y_TUNNEL} ${xCurveEnd} ${Y_TUNNEL} H${xE}`;
  const directBranch = `M${xJ} ${Y_DIRECT} H${xE}`;
  const tunnelRoute = `${trunk} C${xJ + 64} ${Y_DIRECT} ${xJ + 52} ${Y_TUNNEL} ${xCurveEnd} ${Y_TUNNEL} H${xE}`;
  const directRoute = `${trunk} H${xE}`;

  // Who's riding each track.
  const riders = useMemo(() => {
    if (live) {
      return {
        tunnel: ridersFromConnections(connections, "tunnel"),
        direct: ridersFromConnections(connections, "direct"),
      };
    }
    const fromRules = (rules: Tunnel["routing"]["apps"]): Rider[] =>
      rules.map((rule) => ({
        key: `${rule.kind}:${rule.value}`,
        path: ruleIconPath(rule, processes),
        name: ruleLabel(rule, processes).title,
        kind: rule.kind === "folder" || rule.kind === "keyword" ? rule.kind : undefined,
      }));
    return {
      tunnel: tunnel.routing.appMode === "selected" ? fromRules(tunnel.routing.apps) : [],
      direct: fromRules(tunnel.routing.bypassApps),
    };
  }, [live, connections, processes, tunnel.routing]);

  const tunnelSlots = Math.max(0, Math.floor((xE - 40 - (xCurveEnd + 24)) / 30));
  const directSlots = Math.max(0, Math.floor((xE - 40 - (xJ + 34)) / 30));

  const tunnelTone =
    state === "failed" || failing
      ? "failed"
      : state === "connected"
        ? "lit"
        : state === "connecting" || state === "disconnecting"
          ? "setting"
          : "off";

  return (
    <div
      ref={ref}
      className="relative overflow-hidden rounded-2xl border border-rule bg-sunk"
      style={{ height: H }}
    >
      {/* faint panel texture: the "unlit" lamp grid of a signal panel */}
      <div
        aria-hidden
        className="pointer-events-none absolute inset-0 opacity-[0.35]"
        style={{
          backgroundImage: "radial-gradient(circle at 1px 1px, rgba(155,166,170,.09) 1px, transparent 0)",
          backgroundSize: "14px 14px",
        }}
      />
      {width > 0 && (
        <svg className="absolute inset-0" width={W} height={H} aria-hidden>
          <defs>
            <filter id="glow" x="-10%" y="-50%" width="120%" height="200%">
              <feGaussianBlur stdDeviation="5" />
            </filter>
          </defs>

          {/* direct track: always carries traffic */}
          <path d={directBranch} stroke="var(--color-track)" strokeOpacity={state === "connected" ? 0.5 : 0.72} strokeWidth="3" fill="none" strokeLinecap="round" />

          {/* tunnel track */}
          {tunnelTone === "lit" && (
            <path d={tunnelBranch} stroke="var(--color-lamp)" strokeWidth="7" fill="none" opacity="0.55" filter="url(#glow)" />
          )}
          {tunnelTone === "failed" && (
            <path d={tunnelBranch} stroke="var(--color-signal)" strokeWidth="6" fill="none" opacity="0.35" filter="url(#glow)" />
          )}
          <path
            d={tunnelBranch}
            fill="none"
            strokeLinecap="round"
            strokeWidth={tunnelTone === "off" ? 3 : 3.5}
            className={tunnelTone === "setting" ? "anim-route-set" : undefined}
            stroke={
              tunnelTone === "off"
                ? "var(--color-rule-2)"
                : tunnelTone === "failed"
                  ? "var(--color-signal)"
                  : "var(--color-lamp)"
            }
            strokeDasharray={tunnelTone === "off" ? "1 9" : undefined}
            style={{ transition: "stroke 250ms" }}
          />

          {/* trunk from this PC */}
          <path d={trunk} stroke="var(--color-ink)" strokeOpacity="0.85" strokeWidth="3" fill="none" strokeLinecap="round" />

          {/* traffic */}
          {!reduceMotion && traffic && (
            <>
              <Particles path={tunnelRoute} rate={traffic.tunnel.downloadRate + traffic.tunnel.uploadRate} color="var(--color-lamp)" length={xE - PAD + 120} />
              <Particles path={directRoute} rate={traffic.direct.downloadRate + traffic.direct.uploadRate} color="var(--color-track)" length={xE - PAD} />
            </>
          )}

          {/* source and terminals */}
          <circle cx={PAD} cy={Y_DIRECT} r="5" fill="var(--color-sunk)" stroke="var(--color-ink)" strokeWidth="2" />
          <circle cx={xE} cy={Y_DIRECT} r="5" fill="var(--color-track)" fillOpacity={0.85} />
          <circle
            cx={xE}
            cy={Y_TUNNEL}
            r="5"
            fill={tunnelTone === "lit" ? "var(--color-lamp)" : tunnelTone === "failed" ? "var(--color-signal)" : "var(--color-sunk)"}
            stroke={tunnelTone === "off" ? "var(--color-rule-2)" : "none"}
            strokeWidth="2"
            className={tunnelTone === "setting" ? "anim-route-set" : undefined}
            style={tunnelTone === "setting" ? { fill: "var(--color-lamp)" } : undefined}
          />

          {/* the switch */}
          <rect
            x={xJ - 6}
            y={Y_DIRECT - 6}
            width="12"
            height="12"
            rx="2"
            transform={`rotate(45 ${xJ} ${Y_DIRECT})`}
            fill="var(--color-sunk)"
            stroke={tunnelTone === "lit" ? "var(--color-lamp)" : "var(--color-ink)"}
            strokeWidth="2"
            style={{ transition: "stroke 250ms" }}
          />
        </svg>
      )}

      {/* labels */}
      <span className="engrave absolute text-ink-3" style={{ left: PAD - 4, top: Y_DIRECT + 14 }}>
        This PC
      </span>

      <Terminal
        top={Y_TUNNEL - 52}
        right={PAD - 6}
        label="Tunnel"
        detail={tunnel.peer.endpoint}
        tone={tunnelTone === "failed" ? "signal" : "lamp"}
        active={live}
        down={traffic?.tunnel.downloadRate}
        up={traffic?.tunnel.uploadRate}
        series={history}
        pick={(s) => s.tunnelDown + s.tunnelUp}
        idle={
          state === "failed"
            ? "No route"
            : tunnel.routing.appMode === "all"
              ? "Every app, except bypassed ones"
              : plural(tunnel.routing.apps.length, "app rule")
        }
      />
      <Terminal
        top={Y_DIRECT - 52}
        right={PAD - 6}
        label="Direct"
        detail="your network"
        tone="track"
        active={live}
        down={traffic?.direct.downloadRate}
        up={traffic?.direct.uploadRate}
        series={history}
        pick={(s) => s.directDown + s.directUp}
        idle={state === "connected" ? "" : "Everything else"}
      />

      {width > 0 && (
        <>
          <Riders
            riders={riders.tunnel}
            slots={tunnelSlots}
            x={xCurveEnd + 24}
            y={Y_TUNNEL}
            ghost={!live}
            emptyLabel={
              tunnel.routing.appMode === "all" && !live
                ? "All apps"
                : live
                  ? "Waiting for traffic"
                  : "No apps selected"
            }
          />
          <Riders riders={riders.direct} slots={directSlots} x={xJ + 34} y={Y_DIRECT} ghost={!live} />
        </>
      )}

      <StateBlock tunnel={tunnel} engine={engine} service={service} state={state} />
    </div>
  );
}

function ridersFromConnections(connections: Connection[], route: "tunnel" | "direct"): Rider[] {
  const totals = new Map<string, { path: string | null; name: string; bytes: number }>();
  for (const c of connections) {
    if (c.route !== route) continue;
    const key = (c.processPath ?? c.processName ?? "unknown").toLowerCase();
    const entry = totals.get(key) ?? { path: c.processPath, name: c.processName ?? "Unknown app", bytes: 0 };
    entry.bytes += c.upload + c.download;
    totals.set(key, entry);
  }
  return [...totals.entries()]
    .sort((a, b) => b[1].bytes - a[1].bytes)
    .map(([key, v]) => ({ key, path: v.path, name: v.name }));
}

function Riders({
  riders,
  slots,
  x,
  y,
  ghost,
  emptyLabel,
}: {
  riders: Rider[];
  slots: number;
  x: number;
  y: number;
  ghost: boolean;
  emptyLabel?: string;
}) {
  if (!riders.length) {
    return emptyLabel ? (
      <span
        className="absolute rounded-md border border-rule bg-sunk px-2 py-0.5 text-2xs text-ink-3"
        style={{ left: x, top: y - 11 }}
      >
        {emptyLabel}
      </span>
    ) : null;
  }
  const shown = riders.length > slots ? riders.slice(0, Math.max(0, slots - 1)) : riders;
  const extra = riders.length - shown.length;
  return (
    <div className="absolute flex items-center gap-[10px]" style={{ left: x, top: y - 10 }}>
      <AnimatePresence initial={false} mode="popLayout">
        {shown.map((rider) => (
          <motion.span
            key={rider.key}
            layout
            initial={{ opacity: 0, scale: 0.6 }}
            animate={{ opacity: ghost ? 0.5 : 1, scale: 1 }}
            exit={{ opacity: 0, scale: 0.6 }}
            transition={{ type: "spring", duration: 0.4, bounce: 0.2 }}
            title={rider.name}
            className="relative flex size-5 items-center justify-center rounded-md bg-sunk ring-2 ring-sunk"
          >
            <AppIcon path={rider.path} name={rider.name} size={20} kind={rider.kind} />
          </motion.span>
        ))}
        {extra > 0 && (
          <motion.span
            key="more"
            layout
            className="flex h-5 items-center rounded-md border border-rule-2 bg-sunk px-1.5 font-mono text-2xs text-ink-2"
          >
            +{extra}
          </motion.span>
        )}
      </AnimatePresence>
    </div>
  );
}

function Terminal({
  top,
  right,
  label,
  detail,
  tone,
  active,
  down,
  up,
  series,
  pick,
  idle,
}: {
  top: number;
  right: number;
  label: string;
  detail: string;
  tone: "lamp" | "track" | "signal";
  active: boolean;
  down?: number;
  up?: number;
  series: TrafficSample[];
  pick: (s: TrafficSample) => number;
  idle: string;
}) {
  const color = tone === "lamp" ? "text-lamp" : tone === "signal" ? "text-signal" : "text-track";
  return (
    <div className="absolute flex flex-col items-end gap-1.5 text-right" style={{ top, right }}>
      <div className="flex items-baseline gap-2">
        <span className={clsx("engrave", color)}>{label}</span>
        <span className="max-w-[220px] truncate font-mono text-2xs text-ink-3">{detail}</span>
      </div>
      {active && down !== undefined && up !== undefined ? (
        <div className="flex items-center gap-3">
          <Sparkline values={series.map(pick)} tone={tone} />
          <Rate arrow="↓" value={down} strong />
          <Rate arrow="↑" value={up} />
        </div>
      ) : (
        <span className="h-[18px] text-xs text-ink-3">{idle}</span>
      )}
    </div>
  );
}

function Rate({ arrow, value, strong }: { arrow: string; value: number; strong?: boolean }) {
  const [num, unit] = rateParts(value);
  return (
    <span className="flex items-baseline gap-1 font-mono tabular">
      <span className="text-2xs text-ink-3">{arrow}</span>
      <span className={clsx("text-[13px]", strong ? "font-semibold text-ink" : "text-ink-2")}>{num}</span>
      <span className="text-2xs text-ink-3">{unit}</span>
    </span>
  );
}

function Sparkline({ values, tone }: { values: number[]; tone: "lamp" | "track" | "signal" }) {
  const w = 76;
  const h = 18;
  const data = values.slice(-38);
  if (data.length < 2) return <span style={{ width: w }} />;
  const max = Math.max(...data, 1);
  const step = w / (data.length - 1);
  const points = data.map((v, i) => `${(i * step).toFixed(1)},${(h - 1 - (v / max) * (h - 3)).toFixed(1)}`);
  const stroke = tone === "lamp" ? "var(--color-lamp)" : tone === "signal" ? "var(--color-signal)" : "var(--color-track)";
  return (
    <svg width={w} height={h} aria-hidden className="overflow-visible">
      <polygon points={`0,${h} ${points.join(" ")} ${w},${h}`} fill={stroke} fillOpacity="0.12" />
      <polyline points={points.join(" ")} fill="none" stroke={stroke} strokeOpacity="0.85" strokeWidth="1.25" strokeLinejoin="round" />
    </svg>
  );
}

/** Dots that travel along a route; more and faster with more traffic. */
function Particles({ path, rate, color, length }: { path: string; rate: number; color: string; length: number }) {
  // Bucket the rate so the animation doesn't restart every tick.
  const level = rate <= 0 ? 0 : rate < 200_000 ? 1 : rate < 2_000_000 ? 2 : rate < 8_000_000 ? 3 : 4;
  if (level === 0) return null;
  const count = [0, 2, 3, 5, 7][level]!;
  const speed = [0, 140, 210, 300, 400][level]!; // px per second
  const duration = Math.max(1.2, length / speed);
  return (
    <g>
      {Array.from({ length: count }, (_, i) => (
        <circle key={`${level}-${i}`} r="2.2" fill={color}>
          <animateMotion dur={`${duration}s`} begin={`-${(duration / count) * i}s`} repeatCount="indefinite" path={path} />
          <animate attributeName="opacity" values="0;1;1;0" keyTimes="0;0.08;0.9;1" dur={`${duration}s`} begin={`-${(duration / count) * i}s`} repeatCount="indefinite" />
        </circle>
      ))}
    </g>
  );
}

function StateBlock({
  tunnel,
  engine,
  service,
  state,
}: {
  tunnel: Tunnel;
  engine: EngineStatus | null;
  service: ServiceStatus | null;
  state: PanelState;
}) {
  const connect = useStore((s) => s.connect);
  const disconnect = useStore((s) => s.disconnect);
  const installService = useStore((s) => s.installService);
  const busy = useStore((s) => s.busy);
  const stale = useStore((s) => s.stale[tunnel.id]);
  const now = useNow(state === "connected");

  const other =
    engine && engine.tunnelId !== tunnel.id && engine.state !== "disconnected" && engine.state !== "failed"
      ? engine.tunnelName
      : null;

  let title: string;
  let tone: "ink" | "lamp" | "signal" | "dim" = "ink";
  let detail: ReactNode = null;
  let actions: ReactNode = null;

  const scope =
    tunnel.routing.appMode === "all"
      ? tunnel.routing.bypassApps.length
        ? `Every app except ${plural(tunnel.routing.bypassApps.length, "bypassed app")} will use this tunnel.`
        : "Every app will use this tunnel."
      : tunnel.routing.apps.length
        ? `${plural(tunnel.routing.apps.length, "app rule")} will send traffic through this tunnel.`
        : "No apps are selected yet, so nothing will use this tunnel.";

  switch (state) {
    case "unavailable": {
      tone = "dim";
      if (service?.kind === "notInstalled") {
        title = "Service not installed";
        detail = "Splitwire routes traffic through a small background service. Windows asks for permission once.";
        actions = (
          <Button variant="primary" size="lg" onClick={installService} disabled={busy} icon={busy ? <Spinner /> : <ShieldAlert size={16} />}>
            {busy ? "Installing…" : "Install service"}
          </Button>
        );
      } else if (service?.kind === "stopped") {
        title = "Service stopped";
        detail = "Start “Splitwire Tunnel Service” in Windows Services, then connect again.";
      } else if (service?.kind === "unreachable") {
        title = "Can't reach the service";
        detail = service.message;
      } else {
        title = "Checking the service…";
      }
      break;
    }
    case "connecting":
      title = "Connecting";
      tone = "lamp";
      detail = <>Setting the route through <span className="font-mono text-xs text-ink">{tunnel.peer.endpoint}</span></>;
      actions = (
        <Button variant="outline" size="lg" disabled icon={<Spinner />}>
          Connecting…
        </Button>
      );
      break;
    case "disconnecting":
      title = "Disconnecting";
      detail = "Switching every app back to your normal connection.";
      actions = (
        <Button variant="outline" size="lg" disabled icon={<Spinner />}>
          Disconnecting…
        </Button>
      );
      break;
    case "connected": {
      const health = engine?.health ?? { state: "unknown" };
      const failing = health.state === "failing";
      title = failing ? "Nothing getting through" : "Connected";
      tone = failing ? "signal" : "lamp";
      const since = engine?.connectedAt ?? now;
      detail = (
        <span className="flex flex-col gap-0.5">
          {failing ? (
            <span className="line-clamp-2">{health.message}</span>
          ) : (
            <span className="flex items-baseline gap-2">
              <span className="font-mono text-[15px] font-medium tabular text-ink">{formatDuration(now - since)}</span>
              <span className="text-ink-3">since {formatClock(since)}</span>
            </span>
          )}
          {health.state === "ok" && (
            <span className="flex items-center gap-1.5 text-xs text-ink-3">
              <span className="size-1.5 rounded-full bg-go" />
              Passing traffic · <span className="font-mono tabular text-ink-2">{health.latencyMs} ms</span>
            </span>
          )}
          {health.state === "checking" && <span className="text-xs text-ink-3">Checking the tunnel…</span>}
          {stale && <span className="text-xs text-lamp-2">You changed this tunnel. Reconnect to apply it.</span>}
        </span>
      );
      actions = (
        <>
          {failing && !stale && (
            <Button variant="primary" size="lg" onClick={() => connect(tunnel.id)} disabled={busy} icon={<RotateCw size={15} />}>
              Reconnect
            </Button>
          )}
          {stale && (
            <Button variant="primary" size="lg" onClick={() => connect(tunnel.id)} disabled={busy} icon={<RotateCw size={15} />}>
              Reconnect
            </Button>
          )}
          <Button variant="outline" size="lg" onClick={disconnect} disabled={busy} icon={<Power size={16} />}>
            Disconnect
          </Button>
        </>
      );
      break;
    }
    case "failed":
      title = "Couldn't connect";
      tone = "signal";
      detail = <span className="line-clamp-2">{engine?.error ?? "The tunnel stopped unexpectedly."}</span>;
      actions = (
        <Button variant="primary" size="lg" onClick={() => connect(tunnel.id)} disabled={busy} icon={<RotateCw size={15} />}>
          Try again
        </Button>
      );
      break;
    default:
      title = other ? `Using ${other}` : "Disconnected";
      tone = other ? "dim" : "ink";
      detail = other ? `Connecting here replaces ${other}. ${scope}` : scope;
      actions = (
        <Button variant="primary" size="lg" onClick={() => connect(tunnel.id)} disabled={busy} icon={<Power size={16} />}>
          {other ? `Switch to ${tunnel.name}` : "Connect"}
        </Button>
      );
  }

  return (
    <div className="absolute top-5 left-7 flex max-w-[min(46%,420px)] flex-col items-start">
      <motion.h2
        key={title}
        initial={{ opacity: 0, y: 4 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.25 }}
        className={clsx(
          "font-expanded text-[26px] leading-none font-[650] tracking-[-0.02em]",
          tone === "lamp" && "text-lamp",
          tone === "signal" && "text-signal",
          tone === "ink" && "text-ink",
          tone === "dim" && "text-ink-2",
        )}
      >
        {title}
      </motion.h2>
      <div className="mt-2 min-h-[38px] text-sm leading-snug text-ink-2">{detail}</div>
      <div className="mt-2 flex flex-wrap gap-2">{actions}</div>
    </div>
  );
}

function useWidth(ref: RefObject<HTMLElement | null>): number {
  const [width, setWidth] = useState(0);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    setWidth(el.clientWidth);
    const observer = new ResizeObserver(([entry]) => setWidth(Math.round(entry!.contentRect.width)));
    observer.observe(el);
    return () => observer.disconnect();
  }, [ref]);
  return width;
}

function useNow(ticking: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!ticking) return;
    setNow(Date.now());
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [ticking]);
  return now;
}
