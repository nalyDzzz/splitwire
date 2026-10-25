import { motion, useReducedMotion } from "motion/react";
import { ClipboardPaste, FileUp, Sparkles } from "lucide-react";

import { Button } from "@/components/ui";
import { api, errorMessage } from "@/lib/api";
import { useStore } from "@/store";

export function EmptyState() {
  const openDialog = useStore((s) => s.openDialog);
  const toast = useStore((s) => s.toast);
  const reduce = useReducedMotion();

  const importFile = async () => {
    try {
      const imported = await api.importTunnelFile();
      if (imported) openDialog({ kind: "import", imported, mode: "file" });
    } catch (error) {
      toast(`Couldn't import that file: ${errorMessage(error)}`, "error");
    }
  };

  return (
    <div className="flex min-h-0 flex-1 items-center justify-center overflow-y-auto px-10 py-10">
      <div className="w-full max-w-[640px]">
        <svg viewBox="0 0 640 150" className="w-full" aria-hidden>
          <path d="M16 118H250" stroke="var(--color-ink)" strokeOpacity=".85" strokeWidth="3" strokeLinecap="round" />
          <path d="M250 118H624" stroke="var(--color-track)" strokeOpacity=".45" strokeWidth="3" strokeLinecap="round" />
          <motion.path
            d="M250 118C314 118 302 34 370 34H624"
            stroke="var(--color-lamp)"
            strokeWidth="3.5"
            strokeLinecap="round"
            fill="none"
            initial={reduce ? false : { pathLength: 0 }}
            animate={{ pathLength: 1 }}
            transition={{ duration: 1.1, ease: [0.2, 0.9, 0.1, 1], delay: 0.2 }}
            style={{ filter: "drop-shadow(0 0 6px rgba(255,174,52,.55))" }}
          />
          <rect x="244" y="112" width="12" height="12" rx="2" transform="rotate(45 250 118)" fill="var(--color-enamel)" stroke="var(--color-lamp)" strokeWidth="2" />
          <circle cx="16" cy="118" r="5" fill="var(--color-enamel)" stroke="var(--color-ink)" strokeWidth="2" />
          <circle cx="624" cy="34" r="5" fill="var(--color-lamp)" />
          <circle cx="624" cy="118" r="5" fill="var(--color-track)" fillOpacity=".8" />
          <text x="618" y="20" textAnchor="end" className="engrave" fill="var(--color-lamp)">Your apps · tunnel</text>
          <text x="618" y="104" textAnchor="end" className="engrave" fill="var(--color-ink-3)">Everything else · direct</text>
        </svg>

        <h1 className="mt-8 font-expanded text-[30px] leading-[1.08] font-[650] tracking-[-0.025em] text-ink">
          Pick the apps that take the tunnel.
        </h1>
        <p className="mt-3 max-w-[520px] text-[14px] leading-relaxed text-ink-2">
          Splitwire sends the apps you choose through WireGuard. Games, calls and everything else stay on your
          normal connection. Start by adding a tunnel.
        </p>

        <div className="mt-7 flex flex-wrap gap-2.5">
          <Button variant="primary" size="lg" icon={<FileUp size={16} />} onClick={importFile}>
            Import .conf file
          </Button>
          <Button
            size="lg"
            icon={<ClipboardPaste size={16} />}
            onClick={() => openDialog({ kind: "import", imported: null, mode: "paste" })}
          >
            Paste a config
          </Button>
          <Button size="lg" variant="ghost" icon={<Sparkles size={16} />} onClick={() => openDialog({ kind: "newTunnel" })}>
            Set up from scratch
          </Button>
        </div>

        <p className="mt-8 border-t border-rule pt-4 text-xs text-ink-3">
          Moving from TunnlTo or WireSock? Import the same .conf. Its AllowedApps and DisallowedApps lists carry over.
          TunnlTo keeps the tunnel you last used at{" "}
          <span className="selectable font-mono text-ink-2">%LOCALAPPDATA%\TunnlTo\tunnel.conf</span>.
        </p>
      </div>
    </div>
  );
}
