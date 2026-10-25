import clsx from "clsx";
import { ArrowDownToLine, Eraser } from "lucide-react";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";

import { Button, Segmented } from "@/components/ui";
import { api } from "@/lib/api";
import { formatClock } from "@/lib/format";
import type { LogEntry, LogLevel } from "@/lib/types";
import { useStore } from "@/store";

type LevelFilter = "all" | "info" | "warn" | "error";
const RANK: Record<LogLevel, number> = { debug: 0, info: 1, warn: 2, error: 3 };
const MAX = 3000;

export function LogsView() {
  const serviceRunning = useStore((s) => s.service?.kind === "running");
  const [entries, setEntries] = useState<LogEntry[]>([]);
  const [level, setLevel] = useState<LevelFilter>("info");
  const [follow, setFollow] = useState(true);
  const after = useRef(0);
  const scroller = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!serviceRunning) return;
    let stopped = false;
    const poll = async () => {
      try {
        const next = await api.getLogs(after.current);
        if (stopped || !next.length) return;
        after.current = next[next.length - 1]!.seq;
        setEntries((prev) => [...prev, ...next].slice(-MAX));
      } catch {
        // The service status already explains why.
      }
    };
    void poll();
    const t = setInterval(poll, 1000);
    return () => {
      stopped = true;
      clearInterval(t);
    };
  }, [serviceRunning]);

  const shown = useMemo(
    () => entries.filter((e) => level === "all" || RANK[e.level] >= RANK[level]),
    [entries, level],
  );

  useLayoutEffect(() => {
    const el = scroller.current;
    if (follow && el) el.scrollTop = el.scrollHeight;
  }, [shown, follow]);

  return (
    <div className="flex min-h-0 flex-1 flex-col px-8 pt-6 pb-6">
      <div className="flex items-end justify-between gap-6">
        <div>
          <h1 className="font-expanded text-[22px] leading-tight font-semibold tracking-[-0.02em]">Logs</h1>
          <p className="mt-1 text-sm text-ink-3">What the service and the sing-box engine are doing. Include these when reporting a problem.</p>
        </div>
        <div className="flex items-center gap-2">
          <Segmented<LevelFilter>
            size="sm"
            value={level}
            onChange={setLevel}
            options={[
              { value: "all", label: "Debug" },
              { value: "info", label: "Info" },
              { value: "warn", label: "Warnings" },
              { value: "error", label: "Errors" },
            ]}
          />
          <Button
            size="sm"
            variant={follow ? "secondary" : "ghost"}
            icon={<ArrowDownToLine size={13} />}
            onClick={() => setFollow((f) => !f)}
            aria-pressed={follow}
          >
            Follow
          </Button>
          <Button size="sm" variant="ghost" icon={<Eraser size={13} />} onClick={() => setEntries([])}>
            Clear
          </Button>
        </div>
      </div>

      <div
        ref={scroller}
        onScroll={(e) => {
          const el = e.currentTarget;
          const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
          if (!atBottom && follow) setFollow(false);
        }}
        className="selectable mt-5 min-h-0 flex-1 overflow-y-auto rounded-2xl border border-rule bg-sunk px-1 py-2 font-mono text-[11.5px] leading-[1.65]"
      >
        {shown.map((e) => (
          <div key={e.seq} className="grid grid-cols-[72px_58px_44px_1fr] gap-3 rounded px-3 hover:bg-plate/60">
            <span className="text-ink-4 tabular">{formatClock(e.time)}</span>
            <span className={clsx("engrave pt-[5px] text-[9.5px]", e.source === "engine" ? "text-ink-3" : "text-ink-2")}>
              {e.source === "engine" ? "sing-box" : "service"}
            </span>
            <span
              className={clsx(
                "engrave pt-[5px] text-[9.5px]",
                e.level === "error" && "text-signal",
                e.level === "warn" && "text-lamp",
                e.level === "info" && "text-ink-3",
                e.level === "debug" && "text-ink-4",
              )}
            >
              {e.level}
            </span>
            <span
              className={clsx(
                "break-words whitespace-pre-wrap",
                e.level === "error" ? "text-[#ffb3ab]" : e.level === "debug" ? "text-ink-3" : "text-ink-2",
              )}
            >
              {e.message}
            </span>
          </div>
        ))}
        {!shown.length && (
          <p className="px-3 py-10 text-center font-sans text-xs text-ink-3">
            {serviceRunning ? "Nothing logged yet." : "Logs appear once the service is running."}
          </p>
        )}
      </div>
    </div>
  );
}
