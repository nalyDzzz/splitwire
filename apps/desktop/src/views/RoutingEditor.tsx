import clsx from "clsx";
import { AnimatePresence, motion } from "motion/react";
import { ArrowDownUp, Plus, TriangleAlert, X } from "lucide-react";
import { useState, type ReactNode } from "react";

import { AppIcon } from "@/components/AppIcon";
import { Button, Segmented, Switch } from "@/components/ui";
import { plural } from "@/lib/format";
import { KIND_LABEL, conflictKeys, isCidr, ruleIconPath, ruleKey, ruleLabel, toCidr } from "@/lib/rules";
import type { AppMode, AppRule, Routing, Tunnel } from "@/lib/types";
import { useStore } from "@/store";

type ListName = "apps" | "bypassApps";

export function RoutingEditor({ tunnel }: { tunnel: Tunnel }) {
  const saveTunnel = useStore((s) => s.saveTunnel);
  const openDialog = useStore((s) => s.openDialog);
  const errors = useStore((s) => s.errors[tunnel.id]);
  const routing = tunnel.routing;
  const conflicts = conflictKeys(routing);

  const update = (fn: (r: Routing) => Routing) => void saveTunnel({ ...tunnel, routing: fn(routing) });

  const remove = (list: ListName, index: number) =>
    update((r) => ({ ...r, [list]: r[list].filter((_, i) => i !== index) }));

  const move = (from: ListName, index: number) => {
    const to: ListName = from === "apps" ? "bypassApps" : "apps";
    update((r) => {
      const rule = r[from][index]!;
      const target = r[to].some((x) => ruleKey(x) === ruleKey(rule)) ? r[to] : [...r[to], rule];
      return { ...r, [from]: r[from].filter((_, i) => i !== index), [to]: target };
    });
  };

  const keepInTunnel = () =>
    update((r) => ({ ...r, bypassApps: r.bypassApps.filter((x) => !conflicts.has(ruleKey(x))) }));

  const conflictRules = routing.apps.filter((r) => conflicts.has(ruleKey(r)));
  const fieldError = (list: ListName, index: number) =>
    errors?.find((e) => e.field === `routing.${list}.${index}`)?.message ?? null;

  return (
    <div className="flex max-w-[1080px] flex-col gap-9">
      <Section
        tone="lamp"
        title="Through the tunnel"
        description={
          routing.appMode === "selected"
            ? "Only these apps use the tunnel. Everything else stays on your normal connection."
            : "Every app uses the tunnel, except the ones under Always direct."
        }
        aside={
          <Segmented<AppMode>
            value={routing.appMode}
            onChange={(appMode) => update((r) => ({ ...r, appMode }))}
            options={[
              { value: "selected", label: "Only selected apps" },
              { value: "all", label: "All apps" },
            ]}
          />
        }
      >
        <AnimatePresence initial={false}>
          {routing.appMode === "selected" && routing.apps.length === 0 && (
            <Notice key="empty" tone="lamp">
              No apps are selected, so nothing will use the tunnel. Add the apps you want to route.
            </Notice>
          )}
          {routing.appMode === "selected" && conflictRules.length > 0 && (
            <Notice key="conflict" tone="signal">
              <span>
                {conflictRules.length === 1
                  ? `${ruleLabel(conflictRules[0]!, []).detail} is also under Always direct, so it won't use the tunnel.`
                  : `${plural(conflictRules.length, "app")} are also under Always direct, so they won't use the tunnel: ${conflictRules
                      .map((r) => ruleLabel(r, []).detail)
                      .join(", ")}.`}
              </span>
              <Button size="sm" variant="outline" onClick={keepInTunnel}>
                Keep in tunnel
              </Button>
            </Notice>
          )}
        </AnimatePresence>

        {routing.appMode === "selected" ? (
          <RuleGrid
            rules={routing.apps}
            list="apps"
            conflicts={conflicts}
            errorFor={(i) => fieldError("apps", i)}
            onRemove={(i) => remove("apps", i)}
            onMove={(i) => move("apps", i)}
            onAdd={() => openDialog({ kind: "picker", target: "apps" })}
          />
        ) : (
          routing.apps.length > 0 && (
            <p className="text-xs text-ink-3">
              Your {plural(routing.apps.length, "selected app")} are kept for when you switch back to Only selected apps.
            </p>
          )
        )}
      </Section>

      <Section
        tone="track"
        title="Always direct"
        description={
          routing.appMode === "selected"
            ? "These apps never use the tunnel, even if they're also selected above."
            : "These apps never use the tunnel."
        }
      >
        <RuleGrid
          rules={routing.bypassApps}
          list="bypassApps"
          conflicts={routing.appMode === "selected" ? conflicts : new Set()}
          errorFor={(i) => fieldError("bypassApps", i)}
          onRemove={(i) => remove("bypassApps", i)}
          onMove={(i) => move("bypassApps", i)}
          onAdd={() => openDialog({ kind: "picker", target: "bypassApps" })}
        />
      </Section>

      <Section
        tone="track"
        title="Direct destinations"
        description="Traffic to these addresses never uses the tunnel, whichever app sends it. Your local network already stays direct."
      >
        <CidrInput
          values={routing.bypassIps}
          onChange={(bypassIps) => update((r) => ({ ...r, bypassIps }))}
        />
      </Section>

      <Section
        title="DNS"
        description="Windows looks up names for every app in one place, so this applies to all apps, not only tunneled ones."
      >
        <div className="flex items-center justify-between gap-6 rounded-xl border border-rule bg-plate px-4 py-3">
          <div>
            <p className="text-[13px] font-medium">Send DNS lookups through the tunnel</p>
            <p className="mt-0.5 text-xs text-ink-3">
              {tunnel.interface.dns.length
                ? routing.dnsMode === "tunnel"
                  ? `Every lookup goes to ${tunnel.interface.dns[0]} through the tunnel.`
                  : `Lookups use your normal network's DNS. Turn on to use ${tunnel.interface.dns[0]} instead.`
                : "Add a DNS server in WireGuard settings to use this."}
            </p>
          </div>
          <Switch
            label="Send DNS lookups through the tunnel"
            checked={routing.dnsMode === "tunnel"}
            disabled={!tunnel.interface.dns.length}
            onChange={(on) => update((r) => ({ ...r, dnsMode: on ? "tunnel" : "system" }))}
          />
        </div>
      </Section>
    </div>
  );
}

function Section({
  title,
  description,
  aside,
  tone,
  children,
}: {
  title: string;
  description: string;
  aside?: ReactNode;
  tone?: "lamp" | "track";
  children: ReactNode;
}) {
  return (
    <section className="flex flex-col gap-3.5">
      <div className="flex items-end justify-between gap-6">
        <div className="min-w-0">
          <h3 className="flex items-center gap-2.5 font-semiexpanded text-[14.5px] font-semibold tracking-[-0.01em]">
            {tone && (
              <span
                aria-hidden
                className={clsx(
                  "h-[3px] w-4 rounded-full",
                  tone === "lamp" ? "bg-lamp shadow-[0_0_8px_rgba(255,174,52,.6)]" : "bg-track/70",
                )}
              />
            )}
            {title}
          </h3>
          <p className="mt-1 text-sm text-ink-3">{description}</p>
        </div>
        {aside}
      </div>
      {children}
    </section>
  );
}

function Notice({ tone, children }: { tone: "lamp" | "signal"; children: ReactNode }) {
  return (
    <motion.div
      initial={{ opacity: 0, height: 0 }}
      animate={{ opacity: 1, height: "auto" }}
      exit={{ opacity: 0, height: 0 }}
      className="overflow-hidden"
    >
      <div
        className={clsx(
          "flex items-center justify-between gap-4 rounded-xl border px-3.5 py-2.5 text-sm",
          tone === "lamp" ? "border-lamp/25 bg-lamp-deep/50 text-lamp-2" : "border-signal/30 bg-signal-deep/60 text-[#ffb3ab]",
        )}
      >
        <span className="flex items-center gap-2.5">
          <TriangleAlert size={15} className="shrink-0" />
          {children}
        </span>
      </div>
    </motion.div>
  );
}

function RuleGrid({
  rules,
  list,
  conflicts,
  errorFor,
  onRemove,
  onMove,
  onAdd,
}: {
  rules: AppRule[];
  list: ListName;
  conflicts: Set<string>;
  errorFor: (index: number) => string | null;
  onRemove: (index: number) => void;
  onMove: (index: number) => void;
  onAdd: () => void;
}) {
  const processes = useStore((s) => s.processes);
  const seen = new Map<string, number>();
  return (
    <div className="grid grid-cols-[repeat(auto-fill,minmax(232px,1fr))] gap-2">
      <AnimatePresence initial={false}>
        {rules.map((rule, index) => {
          const key = ruleKey(rule);
          const occurrence = seen.get(key) ?? 0;
          seen.set(key, occurrence + 1);
          const label = ruleLabel(rule, processes);
          const conflict = conflicts.has(key);
          const error = errorFor(index);
          return (
            <motion.div
              key={occurrence ? `${key}#${occurrence}` : key}
              layout
              initial={{ opacity: 0, scale: 0.96 }}
              animate={{ opacity: 1, scale: 1 }}
              exit={{ opacity: 0, scale: 0.96 }}
              transition={{ duration: 0.18 }}
              className={clsx(
                "group relative flex h-[52px] items-center gap-3 overflow-hidden rounded-xl border bg-plate pr-2 pl-3.5 transition-colors hover:bg-plate-2",
                error ? "border-signal/60" : "border-rule hover:border-rule-2",
              )}
              title={error ?? (conflict ? "Also under Always direct — it won't use the tunnel" : label.detail)}
            >
              <span
                aria-hidden
                className={clsx(
                  "absolute top-3 bottom-3 left-0 w-[2px] rounded-r-full",
                  conflict || error ? "bg-signal" : list === "apps" ? "bg-lamp" : "bg-track/50",
                )}
              />
              <AppIcon
                path={ruleIconPath(rule, processes)}
                name={label.title}
                size={28}
                kind={rule.kind === "folder" || rule.kind === "keyword" ? rule.kind : undefined}
              />
              <span className="min-w-0 flex-1">
                <span className="flex items-center gap-1.5">
                  <span className="truncate text-[13px] font-medium text-ink">{label.title}</span>
                  {conflict && <TriangleAlert size={12} className="shrink-0 text-signal" />}
                </span>
                <span className="flex items-center gap-1.5">
                  {rule.kind !== "name" && (
                    <span className="engrave shrink-0 text-[9px] text-ink-3">{KIND_LABEL[rule.kind]}</span>
                  )}
                  <span className="truncate font-mono text-2xs text-ink-3">{error ?? label.detail}</span>
                </span>
              </span>
              <span className="flex shrink-0 opacity-0 transition-opacity group-focus-within:opacity-100 group-hover:opacity-100">
                <TileAction
                  label={list === "apps" ? "Move to Always direct" : "Move to the tunnel list"}
                  onClick={() => onMove(index)}
                >
                  <ArrowDownUp size={14} />
                </TileAction>
                <TileAction label="Remove" onClick={() => onRemove(index)}>
                  <X size={14} />
                </TileAction>
              </span>
            </motion.div>
          );
        })}
      </AnimatePresence>
      <button
        type="button"
        onClick={onAdd}
        className="flex h-[52px] items-center gap-3 rounded-xl border border-dashed border-rule-2 px-3.5 text-[13px] text-ink-2 transition-colors hover:border-ink-3 hover:bg-plate hover:text-ink"
      >
        <span className="flex size-7 items-center justify-center rounded-lg border border-rule-2">
          <Plus size={15} />
        </span>
        Add app
      </button>
    </div>
  );
}

function TileAction({ label, onClick, children }: { label: string; onClick: () => void; children: ReactNode }) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      onClick={onClick}
      className="inline-flex size-7 items-center justify-center rounded-md text-ink-3 transition-colors hover:bg-plate-3 hover:text-ink"
    >
      {children}
    </button>
  );
}

/** CIDR chips with inline validation. */
export function CidrInput({
  values,
  onChange,
  placeholder = "Add an address or range, like 10.0.0.0/8",
  invalid,
}: {
  values: string[];
  onChange: (values: string[]) => void;
  placeholder?: string;
  invalid?: boolean;
}) {
  const [draft, setDraft] = useState("");
  const [error, setError] = useState<string | null>(null);

  const commit = () => {
    const items = draft.split(/[\s,]+/).filter(Boolean);
    if (!items.length) return;
    const bad = items.find((item) => !isCidr(item));
    if (bad) {
      setError(`“${bad}” isn't an IP address or CIDR range`);
      return;
    }
    const next = [...values];
    for (const item of items.map(toCidr)) if (!next.includes(item)) next.push(item);
    onChange(next);
    setDraft("");
    setError(null);
  };

  return (
    <div>
      <div
        className={clsx(
          "flex min-h-10 flex-wrap items-center gap-1.5 rounded-xl border bg-sunk px-2 py-1.5 transition-colors focus-within:border-lamp/60",
          error || invalid ? "border-signal/60" : "border-rule",
        )}
      >
        {values.map((value) => (
          <span
            key={value}
            className="flex h-7 items-center gap-1 rounded-lg border border-rule-2 bg-plate-2 pr-1 pl-2.5 font-mono text-xs text-ink"
          >
            {value}
            <button
              type="button"
              aria-label={`Remove ${value}`}
              onClick={() => onChange(values.filter((v) => v !== value))}
              className="inline-flex size-5 items-center justify-center rounded text-ink-3 hover:bg-plate-3 hover:text-ink"
            >
              <X size={12} />
            </button>
          </span>
        ))}
        <input
          value={draft}
          spellCheck={false}
          onChange={(e) => {
            setDraft(e.target.value);
            setError(null);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === "," || e.key === " ") {
              e.preventDefault();
              commit();
            } else if (e.key === "Backspace" && !draft && values.length) {
              onChange(values.slice(0, -1));
            }
          }}
          onBlur={commit}
          placeholder={values.length ? "" : placeholder}
          className="h-7 min-w-[180px] flex-1 bg-transparent px-1.5 font-mono text-xs outline-none"
        />
      </div>
      {error && <p className="mt-1.5 text-xs text-signal">{error}</p>}
    </div>
  );
}
