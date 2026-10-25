import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import clsx from "clsx";
import { Copy, Minus, Square, X } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";

import { formatRate } from "@/lib/format";
import { useEngine } from "@/store";

import { Mark } from "./Mark";
import { Lamp } from "./ui";

const inTauri = isTauri();
const win = () => getCurrentWindow();

export function TitleBar() {
  const engine = useEngine();
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    if (!inTauri) return;
    let unlisten: (() => void) | undefined;
    void win().isMaximized().then(setMaximized);
    void win()
      .onResized(() => void win().isMaximized().then(setMaximized))
      .then((fn) => (unlisten = fn));
    return () => unlisten?.();
  }, []);

  const live = engine?.state === "connected";

  return (
    <header
      data-tauri-drag-region
      className="relative z-20 flex h-[38px] shrink-0 items-center border-b border-rule/70 bg-sunk pl-3.5"
    >
      <div data-tauri-drag-region className="pointer-events-none flex items-center gap-2">
        <Mark size={18} lit={live} />
        <span className="font-expanded text-[12.5px] font-semibold tracking-tight text-ink">Splitwire</span>
      </div>

      {engine && engine.state !== "disconnected" && (
        <div
          data-tauri-drag-region
          className="pointer-events-none absolute left-1/2 flex -translate-x-1/2 items-center gap-2 text-xs text-ink-2"
        >
          <Lamp
            tone={engine.state === "failed" ? "signal" : "lamp"}
            pulse={engine.state === "connecting" || engine.state === "disconnecting"}
          />
          <span className="font-medium text-ink">{engine.tunnelName}</span>
          {live && (
            <span className="font-mono text-2xs tabular text-ink-3">
              ↓ {formatRate(engine.traffic.tunnel.downloadRate)}
            </span>
          )}
          {engine.state === "connecting" && <span className="text-ink-3">connecting</span>}
          {engine.state === "failed" && <span className="text-signal">couldn't connect</span>}
        </div>
      )}

      <div className="ml-auto flex h-full">
        <WindowButton label="Minimize" onClick={() => inTauri && void win().minimize()}>
          <Minus size={15} strokeWidth={1.5} />
        </WindowButton>
        <WindowButton
          label={maximized ? "Restore" : "Maximize"}
          onClick={() => inTauri && void win().toggleMaximize()}
        >
          {maximized ? (
            <Copy size={12} strokeWidth={1.5} className="-scale-x-100" />
          ) : (
            <Square size={12} strokeWidth={1.5} />
          )}
        </WindowButton>
        <WindowButton label="Close" danger onClick={() => inTauri && void win().close()}>
          <X size={16} strokeWidth={1.5} />
        </WindowButton>
      </div>
    </header>
  );
}

function WindowButton({
  label,
  danger,
  onClick,
  children,
}: {
  label: string;
  danger?: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      onClick={onClick}
      className={clsx(
        "inline-flex h-full w-[46px] items-center justify-center text-ink-2 transition-colors",
        danger ? "hover:bg-[#c42b1c] hover:text-white" : "hover:bg-plate-2 hover:text-ink",
      )}
    >
      {children}
    </button>
  );
}
