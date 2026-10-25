import { Check, Copy } from "lucide-react";
import { useEffect, useState, type ChangeEvent } from "react";

import { api } from "@/lib/api";
import type { KeyPair, Tunnel, ValidationError } from "@/lib/types";
import { useStore } from "@/store";
import { fromDraft } from "@/views/WireGuardEditor";

import { Button, Field, Modal, TextInput, useCopy } from "./ui";

const EMPTY = {
  name: "",
  endpoint: "",
  publicKey: "",
  addresses: "10.0.0.2/32",
  dns: "",
  allowedIps: "0.0.0.0/0, ::/0",
};

const FIELD: Record<keyof typeof EMPTY, string> = {
  name: "name",
  endpoint: "peer.endpoint",
  publicKey: "peer.publicKey",
  addresses: "interface.addresses",
  dns: "interface.dns",
  allowedIps: "peer.allowedIps",
};

export function NewTunnelDialog() {
  const dialog = useStore((s) => s.dialog);
  const openDialog = useStore((s) => s.openDialog);
  const addTunnel = useStore((s) => s.addTunnel);
  const allErrors = useStore((s) => s.errors);
  const open = dialog?.kind === "newTunnel";

  const [form, setForm] = useState(EMPTY);
  const [keys, setKeys] = useState<KeyPair | null>(null);
  const [id, setId] = useState("");
  const [localErrors, setLocalErrors] = useState<ValidationError[]>([]);
  const [busy, setBusy] = useState(false);
  const [copied, copy] = useCopy();

  useEffect(() => {
    if (!open) return;
    setForm(EMPTY);
    setLocalErrors([]);
    setEdited(new Set());
    setId(`t-${crypto.randomUUID()}`);
    void api.generateKeypair().then(setKeys);
  }, [open]);

  // Errors for fields edited since the last attempt are hidden.
  const [edited, setEdited] = useState<Set<string>>(new Set());
  const errors = [...localErrors, ...(allErrors[id] ?? [])].filter((e) => !edited.has(e.field));
  const errorFor = (field: string) => errors.find((e) => e.field === field)?.message ?? null;
  const set = (key: keyof typeof EMPTY) => (e: ChangeEvent<HTMLInputElement>) => {
    setForm((f) => ({ ...f, [key]: e.target.value }));
    setEdited((s) => new Set(s).add(FIELD[key]));
  };

  const create = async () => {
    if (!keys) return;
    setEdited(new Set());
    const base: Tunnel = {
      id,
      name: "",
      interface: { privateKey: keys.privateKey, addresses: [], dns: [], mtu: null },
      peer: { publicKey: "", presharedKey: null, endpoint: "", allowedIps: [], persistentKeepalive: null },
      routing: { appMode: "selected", apps: [], bypassApps: [], bypassIps: [], dnsMode: "system" },
    };
    const { tunnel, errors: parseErrors } = fromDraft(base, {
      name: form.name,
      privateKey: keys.privateKey,
      addresses: form.addresses,
      dns: form.dns,
      mtu: "",
      publicKey: form.publicKey,
      presharedKey: "",
      endpoint: form.endpoint,
      allowedIps: form.allowedIps,
      keepalive: "25",
    });
    setLocalErrors(parseErrors);
    if (parseErrors.length) return;
    setBusy(true);
    await addTunnel(tunnel);
    setBusy(false);
  };

  return (
    <Modal
      open={open}
      onClose={() => openDialog(null)}
      title="Set up a tunnel"
      description="Splitwire made a key pair for this device. Add its public key to your WireGuard server, then fill in the server's details."
      width={560}
      footer={
        <>
          <Button variant="ghost" onClick={() => openDialog(null)}>
            Cancel
          </Button>
          <Button variant="primary" onClick={create} disabled={busy || !keys}>
            {busy ? "Adding…" : "Add tunnel"}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-4">
        <div className="flex items-center gap-3 rounded-xl border border-lamp/25 bg-lamp-deep/40 px-3.5 py-2.5">
          <span className="engrave shrink-0 text-[9.5px] text-lamp">Your public key</span>
          <span className="selectable min-w-0 flex-1 truncate font-mono text-xs text-ink">{keys?.publicKey ?? "…"}</span>
          {keys && (
            <Button size="sm" variant="outline" onClick={() => copy(keys.publicKey)} icon={copied ? <Check size={13} /> : <Copy size={13} />}>
              {copied ? "Copied" : "Copy"}
            </Button>
          )}
        </div>

        <Field label="Name" error={errorFor("name")} htmlFor="new-name">
          <TextInput id="new-name" autoFocus value={form.name} onChange={set("name")} placeholder="home" invalid={!!errorFor("name")} />
        </Field>
        <div className="grid grid-cols-2 gap-3">
          <Field label="Server endpoint" error={errorFor("peer.endpoint")}>
            <TextInput mono value={form.endpoint} onChange={set("endpoint")} placeholder="vpn.example.com:51820" invalid={!!errorFor("peer.endpoint")} />
          </Field>
          <Field label="Your address on the tunnel" error={errorFor("interface.addresses")}>
            <TextInput mono value={form.addresses} onChange={set("addresses")} invalid={!!errorFor("interface.addresses")} />
          </Field>
        </div>
        <Field label="Server public key" error={errorFor("peer.publicKey")}>
          <TextInput mono value={form.publicKey} onChange={set("publicKey")} placeholder="Base64 key from the server" invalid={!!errorFor("peer.publicKey")} />
        </Field>
        <div className="grid grid-cols-2 gap-3">
          <Field label="DNS servers" hint="Optional" error={errorFor("interface.dns")}>
            <TextInput mono value={form.dns} onChange={set("dns")} placeholder="1.1.1.1" invalid={!!errorFor("interface.dns")} />
          </Field>
          <Field label="Allowed IPs" error={errorFor("peer.allowedIps")}>
            <TextInput mono value={form.allowedIps} onChange={set("allowedIps")} invalid={!!errorFor("peer.allowedIps")} />
          </Field>
        </div>
      </div>
    </Modal>
  );
}
