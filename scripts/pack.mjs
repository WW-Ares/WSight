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
import { copyFileSync, existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

/** Newest mtime anywhere under `dir`, or 0 if it is not there. */
function newestMtime(dir) {
  if (!existsSync(dir)) return 0;
  let newest = 0;
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    newest = Math.max(newest, entry.isDirectory() ? newestMtime(path) : statSync(path).mtimeMs);
  }
  return newest;
}

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

// The exe embeds `dist/` as it stood when the link step ran, so a dist newer
// than the binary means the binary predates the frontend - it is carrying the
// previous UI. That is how v0.5.0 went out claiming to be 0.5.0 while showing
// 0.4.5 in the settings window: `tauri build --config
// '{"build":{"beforeBuildCommand":""}}'` skips `pnpm build`, so the stale dist
// was embedded whole. A second of slack absorbs filesystem timestamp rounding.
const distAge = newestMtime(join(root, "dist"));
const exeAge = statSync(built).mtimeMs;
if (distAge > exeAge + 1000) {
  console.error("构建产物比前端旧：exe 里嵌的是上一次的前端。");
  console.error(`  dist/ 最新  ${new Date(distAge).toLocaleString()}`);
  console.error(`  ${built}  ${new Date(exeAge).toLocaleString()}`);
  console.error("");
  console.error("先跑 pnpm build（vite build），再跑 tauri build。");
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
console.log("发布（附件只传 exe，不传截图）：");
console.log(`  gh release create v${v} WSight.exe --notes-file .verify/release-notes.md`);
