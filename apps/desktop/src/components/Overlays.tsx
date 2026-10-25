import clsx from "clsx";
import { AnimatePresence, motion } from "motion/react";
import { TriangleAlert, X } from "lucide-react";
import { useState } from "react";

import { useEngine, useStore } from "@/store";

import { AppPicker } from "./AppPicker";
import { ImportDialog } from "./ImportDialog";
import { NewTunnelDialog } from "./NewTunnelDialog";
import { Button, Modal } from "./ui";
import { UpdateDialog } from "./UpdateDialog";

/** Every dialog and toast, mounted once. */
export function Overlays() {
  return (
    <>
      <ImportDialog />
      <NewTunnelDialog />
      <AppPicker />
      <DeleteDialog />
      <UpdateDialog />
      <Toasts />
    </>
  );
}

function DeleteDialog() {
  const dialog = useStore((s) => s.dialog);
  const openDialog = useStore((s) => s.openDialog);
  const deleteTunnel = useStore((s) => s.deleteTunnel);
  const tunnel = useStore((s) =>
    s.dialog?.kind === "deleteTunnel" ? s.tunnels.find((t) => t.id === (s.dialog as { id: string }).id) : undefined,
  );
  const engine = useEngine();
  const [busy, setBusy] = useState(false);
  const live = !!tunnel && engine?.tunnelId === tunnel.id && engine.state === "connected";

  return (
    <Modal
      open={dialog?.kind === "deleteTunnel" && !!tunnel}
      onClose={() => openDialog(null)}
      title={`Delete ${tunnel?.name ?? "tunnel"}?`}
      width={440}
      footer={
        <>
          <Button variant="ghost" onClick={() => openDialog(null)}>
            Cancel
          </Button>
          <Button
            variant="danger"
            disabled={busy}
            onClick={async () => {
              if (!tunnel) return;
              setBusy(true);
              await deleteTunnel(tunnel.id);
              setBusy(false);
            }}
          >
            {busy ? "Deleting…" : "Delete tunnel"}
          </Button>
        </>
      }
    >
      <p className="text-sm text-ink-2">
        This removes the tunnel and its private key from this PC. Export it first if you might need it again.
        {live && " It's connected right now, so it will disconnect."}
      </p>
    </Modal>
  );
}

function Toasts() {
  const toasts = useStore((s) => s.toasts);
  const dismiss = useStore((s) => s.dismissToast);
  return (
    <div className="pointer-events-none fixed right-5 bottom-5 z-[60] flex w-[360px] flex-col items-end gap-2">
      <AnimatePresence initial={false}>
        {toasts.map((t) => (
          <motion.div
            key={t.id}
            layout
            initial={{ opacity: 0, y: 12, scale: 0.97 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, x: 24 }}
            transition={{ type: "spring", duration: 0.35, bounce: 0.15 }}
            role="status"
            className={clsx(
              "pointer-events-auto flex max-w-full items-start gap-2.5 rounded-xl border px-3.5 py-2.5 text-sm shadow-[0_18px_40px_-12px_rgba(0,0,0,.8)]",
              t.tone === "error"
                ? "border-signal/35 bg-[#241312] text-[#ffc9c2]"
                : "border-rule-2 bg-plate-2 text-ink",
            )}
          >
            {t.tone === "error" && <TriangleAlert size={15} className="mt-0.5 shrink-0 text-signal" />}
            <span className="min-w-0 flex-1 leading-snug">{t.message}</span>
            <button
              type="button"
              aria-label="Dismiss"
              onClick={() => dismiss(t.id)}
              className="-mr-1 inline-flex size-5 shrink-0 items-center justify-center rounded text-ink-3 hover:text-ink"
            >
              <X size={13} />
            </button>
          </motion.div>
        ))}
      </AnimatePresence>
    </div>
  );
}
