import { FileUp, TriangleAlert } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";

import { api, errorMessage } from "@/lib/api";
import { plural } from "@/lib/format";
import type { ImportedTunnel } from "@/lib/types";
import { useStore } from "@/store";

import { Button, Field, Modal, TextInput } from "./ui";

export function ImportDialog() {
  const dialog = useStore((s) => s.dialog);
  const openDialog = useStore((s) => s.openDialog);
  const addTunnel = useStore((s) => s.addTunnel);
  const allErrors = useStore((s) => s.errors);
  const open = dialog?.kind === "import";

  const [text, setText] = useState("");
  const [name, setName] = useState("");
  const [reviewed, setReviewed] = useState<ImportedTunnel | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (dialog?.kind !== "import") return;
    setReviewed(dialog.imported);
    setName(dialog.imported?.tunnel.name ?? "");
    setText("");
    setError(null);
  }, [dialog]);

  const close = () => openDialog(null);

  const read = async () => {
    if (!name.trim()) {
      setError("Give the tunnel a name first.");
      return;
    }
    setBusy(true);
    try {
      const result = await api.importTunnelText(text, name);
      setReviewed(result);
      setError(null);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const fromFile = async () => {
    try {
      const result = await api.importTunnelFile();
      if (result) {
        setReviewed(result);
        setName(result.tunnel.name);
      }
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const save = async () => {
    if (!reviewed) return;
    setBusy(true);
    await addTunnel({ ...reviewed.tunnel, name: name.trim() || reviewed.tunnel.name });
    setBusy(false);
  };

  const saveErrors = reviewed ? (allErrors[reviewed.tunnel.id] ?? []) : [];

  if (!reviewed) {
    return (
      <Modal
        open={open}
        onClose={close}
        title="Paste a WireGuard config"
        description="Paste the contents of a .conf file. WireSock and TunnlTo app lists (AllowedApps, DisallowedApps) carry over."
        width={600}
        footer={
          <>
            <Button variant="ghost" className="mr-auto" icon={<FileUp size={14} />} onClick={fromFile}>
              Choose a file instead
            </Button>
            <Button variant="ghost" onClick={close}>
              Cancel
            </Button>
            <Button variant="primary" onClick={read} disabled={!text.trim() || busy}>
              {busy ? "Reading…" : "Continue"}
            </Button>
          </>
        }
      >
        <div className="flex flex-col gap-4">
          <Field label="Name" htmlFor="import-name">
            <TextInput id="import-name" value={name} onChange={(e) => setName(e.target.value)} placeholder="office" className="max-w-xs" />
          </Field>
          <Field label="Config" error={error}>
            <textarea
              value={text}
              onChange={(e) => {
                setText(e.target.value);
                setError(null);
              }}
              spellCheck={false}
              placeholder={"[Interface]\nPrivateKey = …\nAddress = 10.8.0.2/24\n\n[Peer]\nPublicKey = …\nEndpoint = vpn.example.com:51820\nAllowedIPs = 0.0.0.0/0"}
              className="h-56 w-full resize-none rounded-xl border border-rule bg-sunk p-3 font-mono text-xs leading-relaxed text-ink outline-none focus:border-lamp/60"
            />
          </Field>
          <p className="text-xs text-ink-3">
            Coming from TunnlTo? It keeps the tunnel you last used at{" "}
            <span className="selectable font-mono text-ink-2">%LOCALAPPDATA%\TunnlTo\tunnel.conf</span>.
          </p>
        </div>
      </Modal>
    );
  }

  const t = reviewed.tunnel;
  return (
    <Modal
      open={open}
      onClose={close}
      title="Review the tunnel"
      description="Nothing is saved until you add it."
      width={560}
      footer={
        <>
          {dialog?.kind === "import" && dialog.mode === "paste" && (
            <Button variant="ghost" className="mr-auto" onClick={() => setReviewed(null)}>
              Back
            </Button>
          )}
          <Button variant="ghost" onClick={close}>
            Cancel
          </Button>
          <Button variant="primary" onClick={save} disabled={busy}>
            {busy ? "Adding…" : "Add tunnel"}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-4">
        <Field label="Name" htmlFor="review-name" error={saveErrors.find((e) => e.field === "name")?.message}>
          <TextInput id="review-name" value={name} onChange={(e) => setName(e.target.value)} className="max-w-xs" />
        </Field>

        <dl className="grid grid-cols-[130px_1fr] gap-x-4 gap-y-2.5 rounded-xl border border-rule bg-sunk/60 p-4 text-sm">
          <Row label="Server">{t.peer.endpoint}</Row>
          <Row label="Address">{t.interface.addresses.join(", ") || "—"}</Row>
          <Row label="DNS">{t.interface.dns.join(", ") || "Your network's"}</Row>
          <Row label="Through tunnel" plain>
            {t.routing.appMode === "all" ? "All apps" : plural(t.routing.apps.length, "selected app")}
          </Row>
          <Row label="Always direct" plain>
            {plural(t.routing.bypassApps.length, "app")}
            {t.routing.bypassIps.length > 0 && `, ${plural(t.routing.bypassIps.length, "address range")}`}
          </Row>
        </dl>

        {reviewed.warnings.length > 0 && (
          <div className="rounded-xl border border-lamp/25 bg-lamp-deep/40 p-3.5">
            <p className="flex items-center gap-2 text-sm font-medium text-lamp-2">
              <TriangleAlert size={14} />
              Worth knowing
            </p>
            <ul className="mt-2 flex list-disc flex-col gap-1 pl-9 text-xs text-ink-2">
              {reviewed.warnings.map((w) => (
                <li key={w}>{w}</li>
              ))}
            </ul>
          </div>
        )}

        {saveErrors.filter((e) => e.field !== "name").length > 0 && (
          <div className="rounded-xl border border-signal/30 bg-signal-deep/50 p-3.5 text-xs text-[#ffb3ab]">
            <p className="font-medium">This config can't be added yet. Fix these, then import it again:</p>
            <ul className="mt-1.5 list-disc pl-5">
              {saveErrors
                .filter((e) => e.field !== "name")
                .map((e) => (
                  <li key={e.field}>{e.message}</li>
                ))}
            </ul>
          </div>
        )}
      </div>
    </Modal>
  );
}

function Row({ label, plain, children }: { label: string; plain?: boolean; children: ReactNode }) {
  return (
    <>
      <dt className="text-ink-3">{label}</dt>
      <dd className={plain ? "text-ink" : "selectable truncate font-mono text-xs leading-5 text-ink"}>{children}</dd>
    </>
  );
}
