// Sets the app version everywhere it lives.
// Usage: node scripts/bump-version.mjs 0.2.0

import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const version = process.argv[2];
if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(version ?? "")) {
  console.error("Usage: node scripts/bump-version.mjs <semver>, e.g. 0.2.0 or 0.2.0-beta.1");
  process.exit(1);
}

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

function edit(path, pattern) {
  const file = join(root, path);
  const text = readFileSync(file, "utf8");
  if (!pattern.test(text)) throw new Error(`No version found in ${path}`);
  writeFileSync(file, text.replace(pattern, `$1${version}$2`));
  console.log(`updated ${path}`);
}

// The workspace version is inherited by every crate.
edit("Cargo.toml", /(\[workspace\.package\][^[]*?\nversion\s*=\s*")[^"]+(")/);
edit("apps/desktop/src-tauri/tauri.conf.json", /("version"\s*:\s*")[^"]+(")/);
edit("apps/desktop/package.json", /("version"\s*:\s*")[^"]+(")/);

console.log(`\nVersion is now ${version}. Next:
  cargo check            # refreshes Cargo.lock
  git commit -am "Release v${version}"
  git tag v${version} && git push && git push --tags`);
