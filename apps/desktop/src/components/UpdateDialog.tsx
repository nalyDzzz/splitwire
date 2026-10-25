import { CircleArrowUp, RotateCw, TriangleAlert } from "lucide-react";
import type { ReactNode } from "react";

import { formatBytes } from "@/lib/format";
import { useEngine, useStore } from "@/store";

import { Button, Modal, Spinner } from "./ui";

export function UpdateDialog() {
  const open = useStore((s) => s.dialog?.kind === "update");
  const openDialog = useStore((s) => s.openDialog);
  const update = useStore((s) => s.update);
  const phase = useStore((s) => s.updatePhase);
  const progress = useStore((s) => s.updateProgress);
  const error = useStore((s) => s.updateError);
  const installUpdate = useStore((s) => s.installUpdate);
  const engine = useEngine();

  const working = phase === "downloading" || phase === "installing";
  const live = engine && (engine.state === "connected" || engine.state === "connecting") ? engine.tunnelName : null;
  const fraction = progress?.total ? Math.min(1, progress.downloaded / progress.total) : null;

  if (!update) return null;

  return (
    <Modal
      open={open}
      onClose={() => !working && openDialog(null)}
      title={`Splitwire ${update.version} is available`}
      description={
        <>
          You have {update.currentVersion}
          {update.date && <> · released {releasedAgo(update.date)}</>}
        </>
      }
      width={560}
      footer={
        <>
          <Button variant="ghost" onClick={() => openDialog(null)} disabled={working}>
            Later
          </Button>
          <Button
            variant="primary"
            onClick={installUpdate}
            disabled={working}
            icon={working ? <Spinner /> : phase === "error" ? <RotateCw size={14} /> : <CircleArrowUp size={15} />}
          >
            {phase === "installing" ? "Restarting…" : working ? "Downloading…" : phase === "error" ? "Try again" : "Install and restart"}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-4">
        {update.notes ? (
          <div className="max-h-[260px] overflow-y-auto rounded-xl border border-rule bg-sunk/60 px-4 py-3">
            <ReleaseNotes markdown={update.notes} />
          </div>
        ) : (
          <p className="text-sm text-ink-2">This release has no notes.</p>
        )}

        {working ? (
          <div>
            <div className="h-1.5 overflow-hidden rounded-full bg-plate-3">
              <div
                className={fraction === null ? "h-full w-1/3 animate-pulse rounded-full bg-lamp" : "h-full rounded-full bg-lamp transition-[width] duration-150"}
                style={fraction === null ? undefined : { width: `${fraction * 100}%` }}
              />
            </div>
            <p className="mt-2 font-mono text-xs text-ink-3 tabular">
              {phase === "installing"
                ? "Verified. Starting the installer; Splitwire will restart by itself."
                : progress
                  ? `Downloading ${formatBytes(progress.downloaded)}${progress.total ? ` of ${formatBytes(progress.total)}` : ""}`
                  : "Starting download…"}
            </p>
          </div>
        ) : (
          <ul className="flex flex-col gap-1.5 text-xs text-ink-3">
            <li>Windows asks for permission, since the update also replaces the background service.</li>
            {live && <li>{live} disconnects for a few seconds while it installs, then reconnects.</li>}
            <li>The download is checked against Splitwire's signing key before anything runs.</li>
          </ul>
        )}

        {phase === "error" && error && (
          <p className="flex items-start gap-2 rounded-lg border border-signal/35 bg-signal-deep/60 px-3 py-2 text-sm text-[#ffc9c2]">
            <TriangleAlert size={15} className="mt-0.5 shrink-0 text-signal" />
            {error}
          </p>
        )}
      </div>
    </Modal>
  );
}

function releasedAgo(date: string): string {
  const days = Math.floor((Date.now() - new Date(date).getTime()) / 86_400_000);
  if (Number.isNaN(days)) return "recently";
  if (days <= 0) return "today";
  if (days === 1) return "yesterday";
  if (days < 30) return `${days} days ago`;
  return new Date(date).toLocaleDateString();
}

/**
 * Just enough Markdown for GitHub's generated release notes: headings,
 * bullets, bold and links (shown as text, since the app can't open URLs).
 */
function ReleaseNotes({ markdown }: { markdown: string }) {
  const blocks: ReactNode[] = [];
  let bullets: string[] = [];
  const flush = () => {
    if (!bullets.length) return;
    blocks.push(
      <ul key={blocks.length} className="my-1 flex list-disc flex-col gap-1 pl-4 text-sm text-ink-2 marker:text-ink-4">
        {bullets.map((b, i) => (
          <li key={i}>{inline(b)}</li>
        ))}
      </ul>,
    );
    bullets = [];
  };

  for (const raw of markdown.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line || line.startsWith("<!--")) {
      flush();
      continue;
    }
    const bullet = line.match(/^[-*]\s+(.*)$/);
    if (bullet) {
      bullets.push(bullet[1]!);
      continue;
    }
    flush();
    const heading = line.match(/^#{1,6}\s+(.*)$/);
    blocks.push(
      heading ? (
        <h3 key={blocks.length} className="engrave mt-2 mb-1 text-ink-3 first:mt-0">
          {inline(heading[1]!)}
        </h3>
      ) : (
        <p key={blocks.length} className="my-1 text-sm text-ink-2">
          {inline(line)}
        </p>
      ),
    );
  }
  flush();
  return <div>{blocks}</div>;
}

function inline(text: string): string {
  return text
    .replace(/\[([^\]]+)\]\([^)]+\)/g, "$1")
    .replace(/\*\*([^*]+)\*\*/g, "$1")
    .replace(/`([^`]+)`/g, "$1")
    .replace(/https:\/\/github\.com\/[^/\s]+\/[^/\s]+\/(pull|compare)\/(\S+)/g, (_, kind: string, rest: string) =>
      kind === "pull" ? `#${rest}` : rest,
    );
}
