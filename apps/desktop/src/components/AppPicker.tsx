import clsx from "clsx";
import { Check, FolderOpen, MousePointerClick, PenLine, Plus, RefreshCw, Search } from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import { errorMessage, api } from "@/lib/api";
import { basename, plural } from "@/lib/format";
import { KIND_LABEL, ruleHasError, ruleKey } from "@/lib/rules";
import type { AppRule, AppRuleKind, ProcessInfo } from "@/lib/types";
import { useSelectedTunnel, useStore } from "@/store";

import { AppIcon } from "./AppIcon";
import { Button, IconButton, Modal, Segmented, TextInput } from "./ui";

export function AppPicker() {
  const dialog = useStore((s) => s.dialog);
  const openDialog = useStore((s) => s.openDialog);
  const processes = useStore((s) => s.processes);
  const refreshProcesses = useStore((s) => s.refreshProcesses);
  const saveTunnel = useStore((s) => s.saveTunnel);
  const toast = useStore((s) => s.toast);
  const tunnel = useSelectedTunnel();
  const open = dialog?.kind === "picker" && !!tunnel;
  const target = dialog?.kind === "picker" ? dialog.target : "apps";

  const [query, setQuery] = useState("");
  const [refreshing, setRefreshing] = useState(false);
  const [manual, setManual] = useState(false);

  useEffect(() => {
    if (!open) return;
    setQuery("");
    setManual(false);
    void refreshProcesses();
  }, [open, refreshProcesses]);

  const list = tunnel?.routing[target] ?? [];
  const other = tunnel?.routing[target === "apps" ? "bypassApps" : "apps"] ?? [];
  const added = new Set(list.map(ruleKey));
  const inOther = new Set(other.map(ruleKey));

  const rows = useMemo(() => {
    const q = query.trim().toLowerCase();
    return [...processes]
      .filter(
        (p) =>
          !q ||
          p.name.toLowerCase().includes(q) ||
          (p.description ?? "").toLowerCase().includes(q) ||
          p.path.toLowerCase().includes(q),
      )
      .sort((a, b) => (a.description || a.name).localeCompare(b.description || b.name));
  }, [processes, query]);

  if (!tunnel) return null;

  const add = (rule: AppRule) => {
    if (added.has(ruleKey(rule))) return;
    void saveTunnel({ ...tunnel, routing: { ...tunnel.routing, [target]: [...list, rule] } });
  };

  const fromProcess = (p: ProcessInfo): AppRule => ({ kind: "name", value: p.name, pathHint: p.path });

  const browse = async () => {
    try {
      const picked = await api.pickExecutable();
      if (picked) add({ kind: "name", value: picked.name, pathHint: picked.path });
    } catch (error) {
      toast(errorMessage(error), "error");
    }
  };

  const browseFolder = async () => {
    try {
      const folder = await api.pickFolder();
      if (folder) add({ kind: "folder", value: folder, pathHint: null });
    } catch (error) {
      toast(errorMessage(error), "error");
    }
  };

  const title = target === "apps" ? "Add apps to the tunnel" : "Add apps that always go direct";

  return (
    <Modal
      open={open}
      onClose={() => openDialog(null)}
      title={title}
      description={`Choose from what's running now, or point at an app or folder. ${
        target === "apps" ? "They'll use the tunnel next time you connect." : "They'll never use the tunnel."
      }`}
      width={620}
      footer={
        <>
          <span className="mr-auto text-xs text-ink-3">{plural(list.length, "app rule")} in this list</span>
          <Button variant="primary" onClick={() => openDialog(null)}>
            Done
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3">
        <div className="flex gap-2">
          <div className="relative flex-1">
            <Search size={14} className="pointer-events-none absolute top-1/2 left-2.5 -translate-y-1/2 text-ink-3" />
            <TextInput
              autoFocus
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search running apps"
              className="pl-8"
            />
          </div>
          <IconButton
            label="Refresh running apps"
            className="size-8 border border-rule"
            onClick={async () => {
              setRefreshing(true);
              await refreshProcesses();
              setRefreshing(false);
            }}
          >
            <RefreshCw size={14} className={refreshing ? "anim-spin" : undefined} />
          </IconButton>
        </div>

        <div className="h-[300px] overflow-y-auto rounded-xl border border-rule bg-sunk/60 p-1">
          {rows.map((p) => {
            const rule = fromProcess(p);
            const isAdded = added.has(ruleKey(rule));
            const elsewhere = inOther.has(ruleKey(rule));
            return (
              <button
                key={p.path}
                type="button"
                onClick={() => add(rule)}
                disabled={isAdded}
                className={clsx(
                  "group flex w-full items-center gap-3 rounded-lg px-2.5 py-1.5 text-left transition-colors",
                  isAdded ? "opacity-60" : "hover:bg-plate-2",
                )}
              >
                <AppIcon path={p.path} name={p.name} size={26} />
                <span className="min-w-0 flex-1">
                  <span className="flex items-center gap-2">
                    <span className="truncate text-[13px] font-medium text-ink">
                      {p.description || p.name.replace(/\.exe$/i, "")}
                    </span>
                    {p.instances > 1 && (
                      <span className="font-mono text-2xs text-ink-4">×{p.instances}</span>
                    )}
                    {elsewhere && (
                      <span className="engrave text-[9px] text-signal">
                        {target === "apps" ? "In always direct" : "In tunnel list"}
                      </span>
                    )}
                  </span>
                  <span className="block truncate font-mono text-2xs text-ink-3">
                    {p.name}
                    <span className="text-ink-4"> · {p.path.slice(0, p.path.length - basename(p.path).length - 1)}</span>
                  </span>
                </span>
                {isAdded ? (
                  <span className="flex items-center gap-1 text-xs text-ink-3">
                    <Check size={14} /> Added
                  </span>
                ) : (
                  <span className="flex items-center gap-1 text-xs text-ink-3 opacity-0 transition-opacity group-hover:opacity-100">
                    <Plus size={14} /> Add
                  </span>
                )}
              </button>
            );
          })}
          {!rows.length && (
            <p className="px-3 py-10 text-center text-xs text-ink-3">
              {processes.length ? `Nothing running matches “${query}”.` : "Loading running apps…"}
            </p>
          )}
        </div>

        {manual ? (
          <ManualRule onAdd={(rule) => add(rule)} onCancel={() => setManual(false)} />
        ) : (
          <div className="flex flex-wrap gap-2">
            <Button variant="outline" size="sm" icon={<MousePointerClick size={13} />} onClick={browse}>
              Browse for an app…
            </Button>
            <Button variant="outline" size="sm" icon={<FolderOpen size={13} />} onClick={browseFolder}>
              Add a whole folder…
            </Button>
            <Button variant="outline" size="sm" icon={<PenLine size={13} />} onClick={() => setManual(true)}>
              Type a rule
            </Button>
          </div>
        )}
      </div>
    </Modal>
  );
}

const PLACEHOLDER: Record<AppRuleKind, string> = {
  name: "node.exe",
  path: "C:\\Tools\\node\\node.exe",
  folder: "C:\\Program Files\\JetBrains",
  keyword: "docker",
};

const HELP: Record<AppRuleKind, string> = {
  name: "Matches this exe wherever it's installed.",
  path: "Matches only this exact file.",
  folder: "Matches every exe in this folder and its subfolders.",
  keyword: "Matches any exe whose name contains this text.",
};

function ManualRule({ onAdd, onCancel }: { onAdd: (rule: AppRule) => void; onCancel: () => void }) {
  const [kind, setKind] = useState<AppRuleKind>("name");
  const [value, setValue] = useState("");
  const [touched, setTouched] = useState(false);
  const rule: AppRule = { kind, value: value.trim(), pathHint: null };
  const error = ruleHasError(rule);

  const submit = () => {
    setTouched(true);
    if (error) return;
    onAdd(rule);
    setValue("");
    setTouched(false);
  };

  return (
    <div className="flex flex-col gap-2.5 rounded-xl border border-rule bg-plate-2/50 p-3">
      <Segmented<AppRuleKind>
        size="sm"
        value={kind}
        onChange={setKind}
        options={(Object.keys(KIND_LABEL) as AppRuleKind[]).map((k) => ({ value: k, label: KIND_LABEL[k] }))}
      />
      <div className="flex gap-2">
        <TextInput
          autoFocus
          mono
          value={value}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && submit()}
          placeholder={PLACEHOLDER[kind]}
          invalid={touched && !!error}
        />
        <Button variant="primary" onClick={submit}>
          Add
        </Button>
        <Button variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
      </div>
      <p className={clsx("text-xs", touched && error ? "text-signal" : "text-ink-3")}>
        {touched && error ? error : HELP[kind]}
      </p>
    </div>
  );
}
