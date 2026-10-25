# Contributing

Thanks for helping out. Bug reports, fixes, UI polish and new VPN types are all welcome.

## Setup

You need Windows 10/11, Rust (stable, MSVC toolchain), Node 22+ and pnpm.

```sh
pnpm install
pnpm fetch:engine      # pinned, checksum-verified sing-box into vendor/
```

### Working on the UI only

```sh
pnpm dev:web           # http://localhost:1420, backed by an in-memory mock
```

No Rust and no admin rights needed. The mock lives in `apps/desktop/src/lib/mock.ts`.

URL switches:

- `?service=notInstalled|stopped|unreachable` shows the service states.
- `?empty` starts with no tunnels.
- `?connect=work` connects on load.
- `?health=failing` shows a tunnel that's up but passing no traffic.
- Connecting to `lab` always fails, so you can see the error state.

### Running the real thing

```sh
# Terminal 1, elevated ("Run as administrator"):
pnpm service           # the background service, in the console

# Terminal 2, normal:
pnpm dev               # the Tauri app with hot reload
```

Turn off other split-tunneling tools (TunnlTo, WireSock) while testing.

## Layout

| Path | What |
|---|---|
| `crates/splitwire-core` | Tunnel model, `.conf` import/export, sing-box config generation, IPC types |
| `crates/splitwire-service` | The privileged Windows service: named pipe server, sing-box supervisor, stats, health probe |
| `apps/desktop/src-tauri` | Tauri host: tunnel storage (DPAPI), process list and icons, tray, commands |
| `apps/desktop/src` | React UI |
| `scripts/` | Engine download, installer build, version bump |

The UI talks to Rust only through `apps/desktop/src/lib/api.ts`. Its types (`src/lib/types.ts`) mirror the Rust ones, so change both together.

## Before opening a PR

```sh
pnpm test              # Rust tests, including a real WireGuard handshake end to end
pnpm check             # clippy + TypeScript
```

CI runs the same checks. If you change how the engine config is generated, add a case to `sing_box_accepts_generated_configs` in `crates/splitwire-core/src/singbox.rs`. It runs every generated config through the real `sing-box check`.

## Releasing

1. Run `node scripts/bump-version.mjs 0.2.0`, then `cargo check` to refresh `Cargo.lock`.
2. Commit, then `git tag v0.2.0 && git push && git push --tags`.
3. The **Release** workflow builds the installer and publishes it with checksums and generated notes.
   - Tags with a suffix (`v0.2.0-beta.1`) are published as prereleases.

## Upgrading sing-box

Update `VERSION` and both checksums in `scripts/fetch-sing-box.mjs` (the release's `.zip` digests), run `pnpm fetch:engine --force`, then `pnpm test`.
