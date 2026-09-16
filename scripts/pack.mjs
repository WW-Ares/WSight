#!/usr/bin/env node
/**
 * Stage the built binary in the repository root.
 *
 * The name stays `WSight.exe` on purpose. WSight replaces itself in place, so
 * the file it replaces has to be the file it is running as; keeping the name
 * fixed means the auto-start entry, any shortcut and the update swap all talk
 * about one path, and the swap is the classic "rename the running exe aside,
 * put the new one in its place" (see `src-tauri/src/updater.rs`).
 *
 * The SHA-256 is printed because GitHub reports its own `digest` for each
 * release asset and the updater verifies the download against it. Comparing the
 * two is how a release gets checked end to end without downloading it again.
 *
 * Usage: node scripts/pack.mjs        (after `pnpm build` and `tauri build`)
 */
import { createHash } from "node:crypto";
import { copyFileSync, readFileSync, statSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

function version() {
  const m = /"version"\s*:\s*"([^"]+)"/.exec(readFileSync(join(root, "package.json"), "utf8"));
  if (!m) throw new Error("package.json 里找不到 version");
  return m[1];
}

const v = version();
const built = join(root, "src-tauri", "target", "release", "wsight.exe");
const staged = join(root, "WSight.exe");

try {
  statSync(built);
} catch {
  console.error(`找不到构建产物：${built}`);
  console.error("先跑 pnpm build，再跑 pnpm tauri build --no-bundle。");
  process.exit(1);
}

copyFileSync(built, staged);

const bytes = statSync(staged).size;
const sha = createHash("sha256").update(readFileSync(staged)).digest("hex");

console.log(`已就绪  WSight.exe  (v${v})`);
console.log(`  大小    ${(bytes / 1024 / 1024).toFixed(2)} MB`);
console.log(`  路径    ${staged}`);
console.log(`  sha256  ${sha}`);
console.log("");
console.log("发布：");
console.log(`  gh release create v${v} WSight.exe docs/*.png --notes-file .verify/release-notes.md`);
