// Writes the latest.json the in-app updater reads from GitHub Releases.
// Usage:
//   node scripts/updater-manifest.mjs --version 0.2.0 --installer <setup.exe> \
//     --url <download url> [--notes-file <notes.md>] --out <latest.json>
// The installer's minisign signature must already exist at <setup.exe>.sig
// (made by `tauri signer sign --app-version <version>`).

import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { parseArgs } from "node:util";

const { values } = parseArgs({
  options: {
    version: { type: "string" },
    installer: { type: "string" },
    url: { type: "string" },
    "notes-file": { type: "string" },
    out: { type: "string" },
  },
});

for (const key of ["version", "installer", "url", "out"]) {
  if (!values[key]) {
    console.error(`Missing --${key}`);
    process.exit(1);
  }
}

const sigPath = `${values.installer}.sig`;
if (!existsSync(sigPath)) {
  console.error(`No signature at ${sigPath}. Sign the installer first.`);
  process.exit(1);
}

const manifest = {
  version: values.version,
  notes: values["notes-file"] ? readFileSync(values["notes-file"], "utf8").trim() : "",
  pub_date: new Date().toISOString(),
  platforms: {
    "windows-x86_64": {
      signature: readFileSync(sigPath, "utf8").trim(),
      url: values.url,
    },
  },
};

writeFileSync(values.out, `${JSON.stringify(manifest, null, 2)}\n`);
console.log(`wrote ${values.out} for ${values.version}`);
