// UI-side helpers for app rules and IP ranges. Matching mirrors
// AppRule::to_path_regex in crates/splitwire-core/src/model.rs.

import { basename } from "./format";
import type { AppRule, AppRuleKind, ProcessInfo, Routing } from "./types";

const stripExe = (name: string) => name.replace(/\.exe$/i, "");
const normPath = (path: string) => path.trim().replace(/\//g, "\\").replace(/\\+$/, "");

/** A key that's equal for rules matching the same thing. */
export function ruleKey(rule: AppRule): string {
  const value = rule.value.trim();
  switch (rule.kind) {
    case "name":
      return `name:${stripExe(value).toLowerCase()}`;
    case "path":
    case "folder":
      return `${rule.kind}:${normPath(value).toLowerCase()}`;
    case "keyword":
      return `keyword:${value.toLowerCase()}`;
  }
}

export function ruleMatchesPath(rule: AppRule, path: string): boolean {
  const lower = path.toLowerCase();
  const value = rule.value.trim().toLowerCase();
  switch (rule.kind) {
    case "name":
      return stripExe(basename(lower)) === stripExe(value) && /\.exe$/i.test(lower);
    case "path":
      return lower === normPath(value);
    case "folder":
      return lower.startsWith(`${normPath(value)}\\`);
    case "keyword":
      return basename(lower).includes(value);
  }
}

/** Running process this rule would match, used for icons and descriptions. */
export function matchProcess(rule: AppRule, processes: ProcessInfo[]): ProcessInfo | undefined {
  return processes.find((p) => ruleMatchesPath(rule, p.path));
}

/** Exe whose icon represents this rule. Folder and keyword rules use a glyph instead. */
export function ruleIconPath(rule: AppRule, processes: ProcessInfo[]): string | null {
  if (rule.kind === "folder" || rule.kind === "keyword") return null;
  if (rule.pathHint) return rule.pathHint;
  if (rule.kind === "path") return rule.value;
  return matchProcess(rule, processes)?.path ?? null;
}

export interface RuleLabel {
  title: string;
  detail: string;
}

export function ruleLabel(rule: AppRule, processes: ProcessInfo[]): RuleLabel {
  const value = rule.value.trim();
  switch (rule.kind) {
    case "name": {
      const exe = /\.exe$/i.test(value) ? value : `${value}.exe`;
      const description = matchProcess(rule, processes)?.description;
      return { title: description || stripExe(value), detail: exe };
    }
    case "path": {
      const description = matchProcess(rule, processes)?.description;
      return { title: description || stripExe(basename(value)), detail: value };
    }
    case "folder":
      return { title: `${basename(value)} folder`, detail: `${normPath(value)}\\…` };
    case "keyword":
      return { title: `Names containing “${value}”`, detail: `*${value}*.exe` };
  }
}

export const KIND_LABEL: Record<AppRuleKind, string> = {
  name: "Exe name",
  path: "Exact file",
  folder: "Folder",
  keyword: "Name contains",
};

/** Keys of rules that appear in both lists. */
export function conflictKeys(routing: Routing): Set<string> {
  const bypass = new Set(routing.bypassApps.map(ruleKey));
  return new Set(routing.apps.map(ruleKey).filter((key) => bypass.has(key)));
}

export function ruleHasError(rule: AppRule): string | null {
  const value = rule.value.trim();
  if (!value) return "Enter a value";
  const hasSep = /[\\/]/.test(value);
  if ((rule.kind === "name" || rule.kind === "keyword") && hasSep) {
    return "Use an exact file or folder rule to match by location";
  }
  if ((rule.kind === "path" || rule.kind === "folder") && !/^([a-z]:[\\/]|\\\\|\/\/)/i.test(value)) {
    return "Use a full path, like C:\\Program Files\\App";
  }
  return null;
}

// ---- IP addresses and ranges ----

function isIpv4(s: string): boolean {
  const parts = s.split(".");
  return (
    parts.length === 4 &&
    parts.every((p) => /^\d{1,3}$/.test(p) && Number(p) <= 255 && (p === "0" || !p.startsWith("0")))
  );
}

function isIpv6(s: string): boolean {
  if (!s.includes(":") || !/^[0-9a-f:.]+$/i.test(s)) return false;
  const halves = s.split("::");
  if (halves.length > 2) return false;
  const groups = (part: string) => (part === "" ? [] : part.split(":"));
  const all = [...groups(halves[0] ?? ""), ...groups(halves[1] ?? "")];
  let count = all.length;
  // An embedded IPv4 address at the end counts as two groups.
  const last = all[all.length - 1];
  if (last?.includes(".")) {
    if (!isIpv4(last)) return false;
    all.pop();
    count += 1;
  }
  if (!all.every((g) => /^[0-9a-f]{1,4}$/i.test(g))) return false;
  return halves.length === 2 ? count < 8 : count === 8;
}

export function isIp(s: string): boolean {
  return isIpv4(s) || isIpv6(s);
}

export function isCidr(s: string): boolean {
  const [ip, prefix, ...rest] = s.trim().split("/");
  if (rest.length || !ip) return false;
  if (prefix === undefined) return isIp(ip);
  if (!/^\d{1,3}$/.test(prefix)) return false;
  const bits = Number(prefix);
  if (isIpv4(ip)) return bits <= 32;
  if (isIpv6(ip)) return bits <= 128;
  return false;
}

/** Adds /32 or /128 to bare addresses, as the backend expects CIDR strings. */
export function toCidr(s: string): string {
  const value = s.trim();
  if (value.includes("/")) return value;
  return value.includes(":") ? `${value}/128` : `${value}/32`;
}

/** Splits "a, b\nc" into trimmed items. */
export function splitList(s: string): string[] {
  return s
    .split(/[\s,]+/)
    .map((x) => x.trim())
    .filter(Boolean);
}
