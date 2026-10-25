import { CircleArrowUp, RefreshCw, ShieldAlert } from "lucide-react";
import type { ReactNode } from "react";

import { Mark } from "@/components/Mark";
import { Button, Lamp, Spinner, Switch } from "@/components/ui";
import { isMockBackend } from "@/lib/api";
import type { Settings } from "@/lib/types";
import { useStore } from "@/store";

export function SettingsView() {
  const settings = useStore((s) => s.settings);
  const updateSettings = useStore((s) => s.updateSettings);
  const service = useStore((s) => s.service);
  const installService = useStore((s) => s.installService);
  const busy = useStore((s) => s.busy);
  const tunnels = useStore((s) => s.tunnels);

  const toggle = (key: keyof Settings) => (value: boolean) => void updateSettings({ [key]: value });
  const last = tunnels.find((t) => t.id === settings?.lastTunnelId);

  return (
    <div className="min-h-0 flex-1 overflow-y-auto px-8 pt-6 pb-12">
      <h1 className="font-expanded text-[22px] leading-tight font-semibold tracking-[-0.02em]">Settings</h1>

      <div className="mt-6 flex max-w-[720px] flex-col gap-8">
        <Group title="App">
          <Row
            title="Keep running in the tray when closed"
            detail="Closing the window leaves your tunnel up. Quit from the tray icon."
          >
            <Switch label="Keep running in the tray" checked={!!settings?.closeToTray} onChange={toggle("closeToTray")} disabled={!settings} />
          </Row>
          <Row title="Start with Windows" detail="Opens quietly in the tray when you sign in.">
            <Switch label="Start with Windows" checked={!!settings?.launchAtLogin} onChange={toggle("launchAtLogin")} disabled={!settings} />
          </Row>
          <Row
            title="Connect when Splitwire starts"
            detail={last ? `Reconnects to ${last.name}, the tunnel you used last.` : "Reconnects to the tunnel you used last."}
          >
            <Switch label="Connect when Splitwire starts" checked={!!settings?.connectOnLaunch} onChange={toggle("connectOnLaunch")} disabled={!settings} />
          </Row>
        </Group>

        <UpdatesGroup />

        <Group title="Privacy">
          <Row
            title="Tunnel health check"
            detail="Every 30 seconds, fetches a tiny test page from Cloudflare (cp.cloudflare.com) through the tunnel to confirm traffic gets through. Applies the next time you connect."
          >
            <Switch label="Tunnel health check" checked={!!settings?.healthCheck} onChange={toggle("healthCheck")} disabled={!settings} />
          </Row>
          <p className="px-4 py-3.5 text-xs text-ink-3">
            Splitwire doesn't collect or send anything about you. Its only requests of its own are the health check
            and update checks (to GitHub), and both can be turned off.
          </p>
        </Group>

        <Group title="Background service">
          <Row
            title={
              <span className="flex items-center gap-2.5">
                <Lamp
                  tone={
                    service?.kind === "running" ? "go" : service?.kind === "unreachable" ? "signal" : "off"
                  }
                />
                {service?.kind === "running"
                  ? "Running"
                  : service?.kind === "notInstalled"
                    ? "Not installed"
                    : service?.kind === "stopped"
                      ? "Stopped"
                      : service?.kind === "unreachable"
                        ? "Can't reach it"
                        : "Checking…"}
              </span>
            }
            detail={
              service?.kind === "running"
                ? "Runs tunnels with administrator rights, so this app doesn't need them."
                : service?.kind === "notInstalled"
                  ? "Splitwire needs its service to connect. Windows asks for permission once."
                  : service?.kind === "stopped"
                    ? "Start “Splitwire Tunnel Service” in Windows Services."
                    : service?.kind === "unreachable"
                      ? service.message
                      : ""
            }
          >
            {service?.kind === "notInstalled" && (
              <Button variant="primary" onClick={installService} disabled={busy} icon={busy ? <Spinner /> : <ShieldAlert size={14} />}>
                {busy ? "Installing…" : "Install service"}
              </Button>
            )}
          </Row>
          {service?.kind === "running" && (
            <div className="grid grid-cols-[140px_1fr] gap-y-2 px-4 py-3.5 text-sm">
              <span className="text-ink-3">Service</span>
              <span className="selectable font-mono text-xs leading-5 text-ink-2">{service.serviceVersion}</span>
              <span className="text-ink-3">sing-box engine</span>
              <span className="selectable font-mono text-xs leading-5 text-ink-2">{service.engineVersion ?? "unknown"}</span>
            </div>
          )}
        </Group>

        <Group title="About">
          <div className="flex gap-4 px-4 py-4">
            <span className="flex size-11 shrink-0 items-center justify-center rounded-xl border border-rule-2 bg-sunk">
              <Mark size={26} />
            </span>
            <div className="text-sm text-ink-2">
              <p>
                <span className="font-expanded font-semibold text-ink">Splitwire</span> sends the apps you choose
                through WireGuard and leaves everything else on your normal connection.
              </p>
              <p className="mt-2 text-xs text-ink-3">
                Free and open source under GPL-3.0. Routing runs on the sing-box engine.
                {isMockBackend && " You're looking at the browser preview with sample data."}
              </p>
            </div>
          </div>
        </Group>
      </div>
    </div>
  );
}

function UpdatesGroup() {
  const settings = useStore((s) => s.settings);
  const updateSettings = useStore((s) => s.updateSettings);
  const appVersion = useStore((s) => s.appVersion);
  const update = useStore((s) => s.update);
  const phase = useStore((s) => s.updatePhase);
  const error = useStore((s) => s.updateError);
  const checkForUpdate = useStore((s) => s.checkForUpdate);
  const openDialog = useStore((s) => s.openDialog);

  const detail =
    phase === "checking"
      ? "Checking GitHub for a newer release…"
      : update
        ? `Version ${update.version} is ready to install.`
        : phase === "upToDate"
          ? "You're on the latest version."
          : phase === "error"
            ? error
            : "Updates download from GitHub Releases and are verified before they install.";

  return (
    <Group title="Updates">
      <Row title={`Splitwire ${appVersion ?? ""}`.trim()} detail={detail}>
        {update ? (
          <Button variant="primary" onClick={() => openDialog({ kind: "update" })} icon={<CircleArrowUp size={14} />}>
            Update to {update.version}
          </Button>
        ) : (
          <Button onClick={checkForUpdate} disabled={phase === "checking"} icon={phase === "checking" ? <Spinner /> : <RefreshCw size={13} />}>
            Check for updates
          </Button>
        )}
      </Row>
      <Row title="Check for updates automatically" detail="Looks for a new release a little after Splitwire starts, then every few hours.">
        <Switch
          label="Check for updates automatically"
          checked={!!settings?.autoCheckUpdates}
          onChange={(value) => void updateSettings({ autoCheckUpdates: value })}
          disabled={!settings}
        />
      </Row>
    </Group>
  );
}

function Group({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section>
      <h2 className="engrave mb-2.5 text-ink-3">{title}</h2>
      <div className="divide-y divide-rule overflow-hidden rounded-2xl border border-rule bg-plate">{children}</div>
    </section>
  );
}

function Row({ title, detail, children }: { title: ReactNode; detail: ReactNode; children?: ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-6 px-4 py-3.5">
      <div className="min-w-0">
        <div className="text-[13px] font-medium text-ink">{title}</div>
        {detail && <p className="mt-0.5 text-xs text-ink-3">{detail}</p>}
      </div>
      {children}
    </div>
  );
}
