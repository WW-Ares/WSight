#!/usr/bin/env node
/**
 * 从 CHANGELOG.md 抽出某个版本的段落，写成文件交给 `gh release create --notes-file`。
 *
 *   node scripts/release-notes.mjs             # 版本号默认取 package.json
 *   node scripts/release-notes.mjs 0.4.1
 *   node scripts/release-notes.mjs 0.4.1 --out /tmp/notes.md
 *
 * 为什么需要它：发布说明只该有一个源头。CHANGELOG.md 是源头，Release 页面的
 * 说明由它派生；否则"仓库里的记录"和"发布页上的说明"会各写一份，慢慢就对不上了。
 * 抽不出段落就退出码 1 —— 版本没写进 CHANGELOG 就发版，就是要在这里被拦住。
 */
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { changelogSection } from "./lib/changelog-section.mjs";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

const USAGE = `用法：node scripts/release-notes.mjs [版本号] [--out 文件]

  版本号省略时取 package.json 的 version。
  默认输出到 .verify/release-notes.md（.verify 已被 gitignore）。`;

const argv = process.argv.slice(2);
let out = join(root, ".verify", "release-notes.md");
let version = null;

for (let i = 0; i < argv.length; i++) {
  const arg = argv[i];
  if (arg === "-h" || arg === "--help") {
    console.log(USAGE);
    process.exit(0);
  }
  if (arg === "--out") {
    out = argv[++i];
    if (!out) {
      console.error("--out 后面要跟一个文件路径\n\n" + USAGE);
      process.exit(2);
    }
    continue;
  }
  if (arg.startsWith("-")) {
    console.error(`不认识的参数：${arg}\n\n${USAGE}`);
    process.exit(2);
  }
  version ??= arg;
}

if (!version) {
  version = JSON.parse(readFileSync(join(root, "package.json"), "utf8")).version;
}

const changelog = readFileSync(join(root, "CHANGELOG.md"), "utf8");
const section = changelogSection(changelog, version);

if (!section) {
  console.error(`CHANGELOG.md 里找不到 ${version} 的段落。`);
  console.error("先把这一版写进 CHANGELOG.md，再发版 —— 发布页的说明是它的派生物。");
  process.exit(1);
}

const target = out.startsWith("/") || /^[A-Za-z]:[\\/]/.test(out) ? out : join(root, out);
mkdirSync(dirname(target), { recursive: true });
writeFileSync(target, section, "utf8");

const preview = section.split(/\r?\n/).slice(0, 6).join("\n");
console.log(`已写入 ${target}`);
console.log(`--- ${version} 发布说明（前 6 行）---`);
console.log(preview);
console.log("---");
console.log(`下一步：gh release create v${version} WSight.exe docs/*.png --notes-file "${target}"`);
