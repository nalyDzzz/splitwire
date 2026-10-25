// Builds the Windows installer: fetches sing-box, builds the service in
// release mode, stages both next to the app, and runs `tauri build`.
// Output: target/release/bundle/nsis/Splitwire_<version>_x64-setup.exe

import { spawnSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const desktop = join(root, "apps", "desktop");
const staging = join(desktop, "src-tauri", "binaries");

function run(command, args, cwd = root) {
  console.log(`\n> ${command} ${args.join(" ")}`);
  const result = spawnSync(command, args, { cwd, stdio: "inherit", shell: process.platform === "win32" });
  if (result.status !== 0) process.exit(result.status ?? 1);
}

run("node", ["scripts/fetch-sing-box.mjs"]);
run("cargo", ["build", "--release", "-p", "splitwire-service"]);

mkdirSync(staging, { recursive: true });
copyFileSync(join(root, "target", "release", "splitwire-service.exe"), join(staging, "splitwire-service.exe"));
copyFileSync(join(root, "vendor", "sing-box", "sing-box.exe"), join(staging, "sing-box.exe"));
copyFileSync(join(root, "vendor", "sing-box", "LICENSE"), join(staging, "sing-box-LICENSE.txt"));

run("pnpm", ["tauri", "build", "--config", "src-tauri/tauri.bundle.json"], desktop);
