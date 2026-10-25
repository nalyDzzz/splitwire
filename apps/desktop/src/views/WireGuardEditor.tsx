import clsx from "clsx";
import { AnimatePresence, motion } from "motion/react";
import { Check, Copy, Download, Eye, EyeOff, KeyRound, Trash2 } from "lucide-react";
import { useEffect, useMemo, useState, type ReactNode } from "react";

import { Button, Field, TextInput, useCopy } from "@/components/ui";
import { api, errorMessage } from "@/lib/api";
import { isCidr, isIp, splitList, toCidr } from "@/lib/rules";
import type { Tunnel, ValidationError } from "@/lib/types";
import { useEngine, useStore } from "@/store";

interface Draft {
  name: string;
  privateKey: string;
  addresses: string;
  dns: string;
  mtu: string;
  publicKey: string;
  presharedKey: string;
  endpoint: string;
  allowedIps: string;
  keepalive: string;
}

/** Draft key to the validation field path the backend reports. */
const DRAFT_FIELD: Record<keyof Draft, string> = {
  name: "name",
  privateKey: "interface.privateKey",
  addresses: "interface.addresses",
  dns: "interface.dns",
  mtu: "interface.mtu",
  publicKey: "peer.publicKey",
  presharedKey: "peer.presharedKey",
  endpoint: "peer.endpoint",
  allowedIps: "peer.allowedIps",
  keepalive: "peer.persistentKeepalive",
};

function toDraft(t: Tunnel): Draft {
  return {
    name: t.name,
    privateKey: t.interface.privateKey,
    addresses: t.interface.addresses.join(", "),
    dns: t.interface.dns.join(", "),
    mtu: t.interface.mtu?.toString() ?? "",
    publicKey: t.peer.publicKey,
    presharedKey: t.peer.presharedKey ?? "",
    endpoint: t.peer.endpoint,
    allowedIps: t.peer.allowedIps.join(", "),
    keepalive: t.peer.persistentKeepalive?.toString() ?? "",
  };
}

/** Builds a tunnel from the draft, or returns the fields that don't parse. */
export function fromDraft(base: Tunnel, d: Draft): { tunnel: Tunnel; errors: ValidationError[] } {
  const errors: ValidationError[] = [];
  const cidrs = (field: string, text: string) => {
    const items = splitList(text);
    const bad = items.find((i) => !isCidr(i));
    if (bad) errors.push({ field, message: `“${bad}” isn't an IP address or CIDR range` });
    return items.map(toCidr);
  };
  const number = (field: string, text: string, label: string) => {
    if (!text.trim()) return null;
    const n = Number(text);
    if (!Number.isInteger(n) || n < 0 || n > 65535) {
      errors.push({ field, message: `${label} must be a whole number` });
      return null;
    }
    return n;
  };
  const dns = splitList(d.dns);
  const badDns = dns.find((x) => !isIp(x));
  if (badDns) errors.push({ field: "interface.dns", message: `“${badDns}” isn't an IP address` });

  const keepalive = number("peer.persistentKeepalive", d.keepalive, "Keepalive");
  const tunnel: Tunnel = {
    ...base,
    name: d.name.trim(),
    interface: {
      privateKey: d.privateKey.trim(),
      addresses: cidrs("interface.addresses", d.addresses),
      dns,
      mtu: number("interface.mtu", d.mtu, "MTU"),
    },
    peer: {
      publicKey: d.publicKey.trim(),
      presharedKey: d.presharedKey.trim() || null,
      endpoint: d.endpoint.trim(),
      allowedIps: cidrs("peer.allowedIps", d.allowedIps),
      persistentKeepalive: keepalive ? keepalive : null,
    },
  };
  return { tunnel, errors };
}

export function WireGuardEditor({ tunnel }: { tunnel: Tunnel }) {
  const saveTunnel = useStore((s) => s.saveTunnel);
  const openDialog = useStore((s) => s.openDialog);
  const toast = useStore((s) => s.toast);
  const serverErrors = useStore((s) => s.errors[tunnel.id]);
  const engine = useEngine();
  const [draft, setDraft] = useState(() => toDraft(tunnel));
  const [localErrors, setLocalErrors] = useState<ValidationError[]>([]);
  const [saving, setSaving] = useState(false);

  const original = useMemo(() => toDraft(tunnel), [tunnel]);
  const dirty = JSON.stringify(original) !== JSON.stringify(draft);
  const live = engine?.tunnelId === tunnel.id && engine.state === "connected";

  // Errors for fields edited since the last save attempt are hidden.
  const [edited, setEdited] = useState<Set<string>>(new Set());
  const errors = [...localErrors, ...(serverErrors ?? [])].filter((e) => !edited.has(e.field));
  const errorFor = (field: string) => errors.find((e) => e.field === field)?.message ?? null;

  const set = (key: keyof Draft) => (value: string) => {
    setDraft((d) => ({ ...d, [key]: value }));
    setEdited((s) => new Set(s).add(DRAFT_FIELD[key]));
  };

  const save = async () => {
    const { tunnel: next, errors: parseErrors } = fromDraft(tunnel, draft);
    setEdited(new Set());
    setLocalErrors(parseErrors);
    if (parseErrors.length) return;
    setSaving(true);
    const ok = await saveTunnel(next);
    setSaving(false);
    if (ok) toast(live ? "Saved. Reconnect to use the new settings." : "Saved");
  };

  const exportConf = async () => {
    try {
      if (await api.exportTunnelFile(tunnel.id)) toast(`Exported ${tunnel.name}.conf`);
    } catch (error) {
      toast(`Couldn't export: ${errorMessage(error)}`, "error");
    }
  };

  return (
    <div className="flex max-w-[1080px] flex-col gap-6 pb-16">
      <Group title="Tunnel">
        <div className="flex flex-wrap items-end justify-between gap-4">
          <div className="w-full max-w-sm">
            <Field label="Name" error={errorFor("name")} htmlFor="wg-name">
              <TextInput id="wg-name" value={draft.name} onChange={(e) => set("name")(e.target.value)} invalid={!!errorFor("name")} />
            </Field>
          </div>
          <div className="flex items-center gap-2">
            <Button
              icon={<Download size={14} />}
              onClick={exportConf}
              disabled={dirty}
              title={dirty ? "Save your changes to export them" : undefined}
            >
              Export .conf
            </Button>
            <Button variant="danger" icon={<Trash2 size={14} />} onClick={() => openDialog({ kind: "deleteTunnel", id: tunnel.id })}>
              Delete
            </Button>
          </div>
        </div>
      </Group>

      <div className="grid gap-6 lg:grid-cols-2">
        <Group title="This device" note="The [Interface] section of a .conf">
          <PrivateKeyField
            value={draft.privateKey}
            onChange={set("privateKey")}
            error={errorFor("interface.privateKey")}
          />
          <Field label="Addresses" hint="Comma-separated, like 10.8.0.2/24" error={errorFor("interface.addresses")}>
            <TextInput mono value={draft.addresses} onChange={(e) => set("addresses")(e.target.value)} invalid={!!errorFor("interface.addresses")} />
          </Field>
          <div className="grid grid-cols-[1fr_110px] gap-3">
            <Field label="DNS servers" hint="Optional" error={errorFor("interface.dns")}>
              <TextInput mono value={draft.dns} onChange={(e) => set("dns")(e.target.value)} placeholder="1.1.1.1" invalid={!!errorFor("interface.dns")} />
            </Field>
            <Field label="MTU" hint="Default 1420" error={errorFor("interface.mtu")}>
              <TextInput mono value={draft.mtu} onChange={(e) => set("mtu")(e.target.value)} placeholder="1420" invalid={!!errorFor("interface.mtu")} />
            </Field>
          </div>
        </Group>

        <Group title="Server" note="The [Peer] section of a .conf">
          <Field label="Endpoint" hint="Host or IP with port" error={errorFor("peer.endpoint")}>
            <TextInput mono value={draft.endpoint} onChange={(e) => set("endpoint")(e.target.value)} placeholder="vpn.example.com:51820" invalid={!!errorFor("peer.endpoint")} />
          </Field>
          <Field label="Public key" error={errorFor("peer.publicKey")}>
            <TextInput mono value={draft.publicKey} onChange={(e) => set("publicKey")(e.target.value)} invalid={!!errorFor("peer.publicKey")} />
          </Field>
          <Field label="Preshared key" hint="Optional" error={errorFor("peer.presharedKey")}>
            <SecretInput value={draft.presharedKey} onChange={set("presharedKey")} invalid={!!errorFor("peer.presharedKey")} />
          </Field>
          <div className="grid grid-cols-[1fr_110px] gap-3">
            <Field label="Allowed IPs" hint="Destinations the server can carry" error={errorFor("peer.allowedIps")}>
              <TextInput mono value={draft.allowedIps} onChange={(e) => set("allowedIps")(e.target.value)} invalid={!!errorFor("peer.allowedIps")} />
            </Field>
            <Field label="Keepalive" hint="Seconds" error={errorFor("peer.persistentKeepalive")}>
              <TextInput mono value={draft.keepalive} onChange={(e) => set("keepalive")(e.target.value)} placeholder="Off" invalid={!!errorFor("peer.persistentKeepalive")} />
            </Field>
          </div>
        </Group>
      </div>

      <AnimatePresence>
        {dirty && (
          <motion.div
            initial={{ opacity: 0, y: 16 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: 16 }}
            transition={{ type: "spring", duration: 0.3, bounce: 0.1 }}
            className="sticky bottom-4 z-10 flex items-center justify-between gap-4 rounded-xl border border-rule-2 bg-plate-2/95 px-4 py-2.5 shadow-[0_18px_40px_-12px_rgba(0,0,0,.8)] backdrop-blur"
          >
            <span className="text-sm text-ink-2">
              {live ? "Unsaved changes. Saving takes effect when you reconnect." : "Unsaved changes"}
            </span>
            <span className="flex gap-2">
              <Button
                variant="ghost"
                onClick={() => {
                  setDraft(original);
                  setLocalErrors([]);
                  setEdited(new Set(Object.values(DRAFT_FIELD)));
                }}
              >
                Discard
              </Button>
              <Button variant="primary" onClick={save} disabled={saving}>
                {saving ? "Saving…" : "Save changes"}
              </Button>
            </span>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

function Group({ title, note, children }: { title: string; note?: string; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-4 rounded-2xl border border-rule bg-plate p-5">
      <div className="flex items-baseline justify-between gap-3">
        <h3 className="font-semiexpanded text-[13.5px] font-semibold">{title}</h3>
        {note && <span className="font-mono text-2xs text-ink-4">{note}</span>}
      </div>
      {children}
    </section>
  );
}

function SecretInput({
  value,
  onChange,
  invalid,
}: {
  value: string;
  onChange: (value: string) => void;
  invalid?: boolean;
}) {
  const [shown, setShown] = useState(false);
  return (
    <div className="relative">
      <TextInput
        mono
        type={shown ? "text" : "password"}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        invalid={invalid}
        className="pr-9"
      />
      <button
        type="button"
        aria-label={shown ? "Hide" : "Show"}
        title={shown ? "Hide" : "Show"}
        onClick={() => setShown((v) => !v)}
        className="absolute top-1/2 right-1.5 inline-flex size-6 -translate-y-1/2 items-center justify-center rounded text-ink-3 hover:text-ink"
      >
        {shown ? <EyeOff size={14} /> : <Eye size={14} />}
      </button>
    </div>
  );
}

function PrivateKeyField({
  value,
  onChange,
  error,
}: {
  value: string;
  onChange: (value: string) => void;
  error: string | null;
}) {
  const [publicKey, setPublicKey] = useState<string | null>(null);
  const [confirming, setConfirming] = useState(false);
  const [copied, copy] = useCopy();

  useEffect(() => {
    let cancelled = false;
    const t = setTimeout(() => {
      api
        .publicKey(value)
        .then((key) => !cancelled && setPublicKey(key))
        .catch(() => !cancelled && setPublicKey(null));
    }, 200);
    return () => {
      cancelled = true;
      clearTimeout(t);
    };
  }, [value]);

  const regenerate = async () => {
    const pair = await api.generateKeypair();
    onChange(pair.privateKey);
    setConfirming(false);
  };

  return (
    <Field
      label="Private key"
      error={error}
      aside={
        confirming ? (
          <span className="flex items-center gap-2 text-2xs">
            <span className="text-ink-3">The server needs the new public key.</span>
            <button type="button" className="font-medium text-lamp hover:underline" onClick={regenerate}>
              Generate
            </button>
            <button type="button" className="text-ink-3 hover:text-ink" onClick={() => setConfirming(false)}>
              Cancel
            </button>
          </span>
        ) : (
          <button
            type="button"
            onClick={() => setConfirming(true)}
            className="flex items-center gap-1 text-2xs text-ink-3 transition-colors hover:text-ink"
          >
            <KeyRound size={11} />
            New key pair
          </button>
        )
      }
    >
      <SecretInput value={value} onChange={onChange} invalid={!!error} />
      <div className="flex items-center gap-2 rounded-lg border border-rule bg-sunk/60 px-2.5 py-1.5">
        <span className="engrave shrink-0 text-[9.5px] text-ink-3">Public key</span>
        <span className={clsx("selectable min-w-0 flex-1 truncate font-mono text-2xs", publicKey ? "text-ink-2" : "text-ink-4")}>
          {publicKey ?? "Enter a valid private key"}
        </span>
        {publicKey && (
          <button
            type="button"
            aria-label="Copy public key"
            title="Copy public key"
            onClick={() => copy(publicKey)}
            className="inline-flex size-5 items-center justify-center rounded text-ink-3 hover:text-ink"
          >
            {copied ? <Check size={12} className="text-go" /> : <Copy size={12} />}
          </button>
        )}
      </div>
    </Field>
  );
}
