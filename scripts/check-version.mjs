#!/usr/bin/env node
/**
 * Fail the build if the four places that carry the app version disagree.
 *
 * The version is duplicated on purpose - `tauri.conf.json` for the installer,
 * `Cargo.toml` for `env!("CARGO_PKG_VERSION")`, `package.json` for npm, and
 * `src/shared/about.ts` to print it in the settings window. Duplication is
 * fine as long as something checks it; without this, a release can ship with
 * the UI claiming a version the binary does not have.
 */
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

function read(rel) {
  return readFileSync(join(root, rel), "utf8");
}

function match(text, re, label) {
  const m = re.exec(text);
  if (!m) throw new Error(`在 ${label} 里找不到版本号`);
  return m[1];
}

const found = [
  ["package.json", match(read("package.json"), /"version"\s*:\s*"([^"]+)"/, "package.json")],
  [
    "src-tauri/tauri.conf.json",
    match(read("src-tauri/tauri.conf.json"), /"version"\s*:\s*"([^"]+)"/, "tauri.conf.json"),
  ],
  ["src-tauri/Cargo.toml", match(read("src-tauri/Cargo.toml"), /^version\s*=\s*"([^"]+)"/m, "Cargo.toml")],
  [
    "src/shared/about.ts",
    match(read("src/shared/about.ts"), /APP_VERSION\s*=\s*"([^"]+)"/, "about.ts"),
  ],
];

const versions = new Set(found.map(([, v]) => v));
if (versions.size !== 1) {
  console.error("版本号不一致：");
  for (const [file, v] of found) console.error(`  ${v.padEnd(10)} ${file}`);
  process.exit(1);
}

console.log(`版本号一致：${[...versions][0]}`);
