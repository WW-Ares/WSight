#!/usr/bin/env node
/**
 * Fail if the version the app *claims* is not the version it *is*.
 *
 * The version is duplicated on purpose - `tauri.conf.json` for the binary's
 * version resource, `Cargo.toml` for `env!("CARGO_PKG_VERSION")`, `package.json`
 * for npm, and `src/shared/about.ts` to print it in the settings window.
 * Duplication is fine as long as something checks it; without this, a release
 * can ship with the UI claiming a version the binary does not have.
 *
 * There are two halves to that, and for a long time only the first was checked:
 *
 *   1. the four source files agree with each other;
 *   2. the *built frontend* agrees with them.
 *
 * (2) is the one that bit us. Building a release runs
 * `tauri build --no-bundle --config '{"build":{"beforeBuildCommand":""}}'`,
 * which deliberately skips `pnpm build` - and `pnpm build` is what runs
 * `vite build`. Skip it and whatever sits in `dist/` from the previous build
 * gets embedded verbatim: v0.5.0 shipped with a 0.4.5 frontend, so a successful
 * self-update still showed the old version number. Checking `dist/` here is
 * cheap and would have stopped it.
 *
 * Usage: node scripts/check-version.mjs   (npm: `pnpm check:version`)
 */
import { existsSync, readFileSync, readdirSync } from "node:fs";
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

// ------------------------------------------------- the four source files

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

const version = [...versions][0];
console.log(`源码四处一致：${version}`);

// ------------------------------------------------------ the built frontend

/**
 * The version baked into `dist/`.
 *
 * Minification renames every local, so the symbol is gone - but its *value*
 * survives, and `about.ts` declares `APP_VERSION` immediately above
 * `REPO_URL`, which is a literal unique to this app. Anchor on the URL and read
 * the nearest version-shaped string to its left.
 */
function distVersion() {
  const assets = join(root, "dist", "assets");
  if (!existsSync(assets)) return null;

  const repo = match(read("src/shared/about.ts"), /REPO_URL\s*=\s*"([^"]+)"/, "about.ts");

  for (const name of readdirSync(assets)) {
    if (!name.endsWith(".js")) continue;
    const js = readFileSync(join(assets, name), "utf8");
    const at = js.indexOf(repo);
    if (at < 0) continue;
    const near = js.slice(Math.max(0, at - 240), at);
    const hits = [...near.matchAll(/"(\d+\.\d+\.\d+(?:[-+][^"]*)?)"/g)];
    if (hits.length) return { file: name, version: hits[hits.length - 1][1] };
  }
  return { file: null, version: null };
}

const dist = distVersion();
if (dist === null) {
  console.log("前端产物：dist/ 还没构建过，跳过（发布前必须先跑 pnpm build）");
  process.exit(0);
}

if (!dist.version) {
  console.error("在 dist/ 里找不到内嵌的版本号。");
  console.error("锚点是 about.ts 的 REPO_URL，找不到它说明构建产物不完整或前端入口变了。");
  process.exit(1);
}

if (dist.version !== version) {
  console.error("前端产物与源码版本号不一致：");
  console.error(`  ${version.padEnd(10)} 源码（package.json / tauri.conf.json / Cargo.toml / about.ts）`);
  console.error(`  ${dist.version.padEnd(10)} dist/${dist.file}`);
  console.error("");
  console.error("dist/ 里是上一次构建留下的旧前端，而 tauri build 会把它原样嵌进 exe。");
  console.error("用 --config '{\"build\":{\"beforeBuildCommand\":\"\"}}' 打包会跳过 vite build，");
  console.error("结果就是新版后端配上旧版界面。先跑 pnpm build，再打 tauri build。");
  process.exit(1);
}

console.log(`前端产物一致：${dist.version}（dist/assets/${dist.file}）`);
