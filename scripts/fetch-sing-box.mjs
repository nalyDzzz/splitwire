// Downloads the pinned sing-box release into vendor/sing-box/ and verifies its checksum.
// Usage: node scripts/fetch-sing-box.mjs [--force]

import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, copyFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const VERSION = "1.14.4";
const ASSETS = {
  x64: {
    file: `sing-box-${VERSION}-windows-amd64.zip`,
    sha256: "99185d75d314a07dbba4a130c8ad986cbeb629a07927975636b341ca61f991df",
  },
  arm64: {
    file: `sing-box-${VERSION}-windows-arm64.zip`,
    sha256: "8cd0a4d0cd8cd480be5731ac31f17038969a42d12add9feb6e031fd43d85bbae",
  },
};

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const outDir = join(root, "vendor", "sing-box");
const exe = join(outDir, "sing-box.exe");
const stamp = join(outDir, "VERSION");

if (process.platform !== "win32") {
  console.error("Splitwire currently targets Windows only.");
  process.exit(1);
}

const asset = ASSETS[process.arch];
if (!asset) {
  console.error(`No sing-box build pinned for architecture ${process.arch}.`);
  process.exit(1);
}

const force = process.argv.includes("--force");
if (!force && existsSync(exe) && existsSync(stamp) && readFileSync(stamp, "utf8").trim() === VERSION) {
  console.log(`sing-box ${VERSION} already in ${outDir}`);
  process.exit(0);
}

const url = `https://github.com/SagerNet/sing-box/releases/download/v${VERSION}/${asset.file}`;
console.log(`Downloading ${url}`);
const response = await fetch(url);
if (!response.ok) {
  console.error(`Download failed: ${response.status} ${response.statusText}`);
  process.exit(1);
}
const bytes = Buffer.from(await response.arrayBuffer());

const actual = createHash("sha256").update(bytes).digest("hex");
if (actual !== asset.sha256) {
  console.error(`Checksum mismatch for ${asset.file}\n  expected ${asset.sha256}\n  actual   ${actual}`);
  process.exit(1);
}

const work = mkdtempSync(join(tmpdir(), "splitwire-sing-box-"));
try {
  const zip = join(work, asset.file);
  writeFileSync(zip, bytes);
  // bsdtar ships with Windows 10+ and reads zip files.
  execFileSync("tar", ["-xf", zip, "-C", work], { stdio: "inherit" });
  const extracted = join(work, asset.file.replace(/\.zip$/, ""));
  mkdirSync(outDir, { recursive: true });
  copyFileSync(join(extracted, "sing-box.exe"), exe);
  copyFileSync(join(extracted, "LICENSE"), join(outDir, "LICENSE"));
  writeFileSync(stamp, `${VERSION}\n`);
} finally {
  rmSync(work, { recursive: true, force: true });
}

console.log(`sing-box ${VERSION} ready at ${exe}`);
