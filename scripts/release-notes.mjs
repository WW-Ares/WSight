#!/usr/bin/env node
/**
 * 从 CHANGELOG.md 抽出"本次发布要交代"的段落，写成文件交给 `gh release create --notes-file`。
 *
 *   node scripts/release-notes.mjs             # 版本号默认取 package.json
 *   node scripts/release-notes.mjs 0.4.1
 *   node scripts/release-notes.mjs 0.4.1 --since v0.3.0   # 手动指定上次发布的标签
 *   node scripts/release-notes.mjs 0.4.1 --since none     # 只发这一个版本
 *   node scripts/release-notes.mjs 0.4.1 --out /tmp/notes.md
 *
 * 为什么要它：发布说明只该有一个源头。CHANGELOG.md 是源头，Release 页面的说明由它
 * 派生；否则"仓库里的记录"和"发布页上的说明"会各写一份，慢慢就对不上了。
 *
 * **区间口径**：从上一个已打标签的版本（= 上次发出去的版本）到本次版本，中间所有
 * 段落合并成一份说明 —— 只提交、没发过 Release 的中间版本必须跟着一起露脸，
 * 否则用户装上新版，看到的说明里缺了一整批他从未见过的功能。标签由
 * `lib/release-range.mjs` 探测，`--since` 可以覆盖它。
 *
 * 抽不出段落就退出码 1 —— 版本没写进 CHANGELOG 就发版，就是要在这里被拦住。
 */
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { changelogSectionsSince, joinSections } from "./lib/changelog-section.mjs";
import { previousReleaseTag } from "./lib/release-range.mjs";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

const USAGE = `用法：node scripts/release-notes.mjs [版本号] [--since 标签|none] [--out 文件]

  版本号省略时取 package.json 的 version。
  --since 指定上次发布的标签（如 v0.4.2）；省略时自动探测比本次小的最新标签。
          --since none 表示本次是首次发布，只写当前版本。
  默认输出到 .verify/release-notes.md（.verify 已被 gitignore）。`;

const argv = process.argv.slice(2);
let out = join(root, ".verify", "release-notes.md");
let version = null;
let since;
let sinceGiven = false;

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
  if (arg === "--since") {
    since = argv[++i];
    sinceGiven = true;
    if (!since) {
      console.error("--since 后面要跟一个标签，或 none\n\n" + USAGE);
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

// --- 区间起点 -----------------------------------------------------------------
const fromTag = sinceGiven
  ? since === "none"
    ? null
    : since
  : previousReleaseTag(version, root);
const fromVersion = fromTag ? fromTag.replace(/^v/, "") : null;

// --- 抽取 ---------------------------------------------------------------------
const changelog = readFileSync(join(root, "CHANGELOG.md"), "utf8");
const sections = changelogSectionsSince(changelog, version, fromVersion);

if (!sections) {
  if (fromVersion) {
    console.error(
      `CHANGELOG.md 里找不到 ${version} 或上次发布的 ${fromVersion} 的段落，无法划定发布区间。`,
    );
    console.error("确认这两个版本号都写在 CHANGELOG.md 里，或用 --since 指定起点。");
  } else {
    console.error(`CHANGELOG.md 里找不到 ${version} 的段落。`);
    console.error("先把这一版写进 CHANGELOG.md，再发版 —— 发布页的说明是它的派生物。");
  }
  process.exit(1);
}

// 多版本合并时，在顶上写一行"这次到底带了什么" —— 否则读者看到标题是
// v0.4.5、底下却跟着 v0.4.4 / v0.4.3 两段，会以为放错了。
const header =
  sections.length > 1 && fromTag
    ? `本版包含 \`${fromTag}\`（上次发布）之后的全部改动，共 ${sections.length} 个版本：` +
      `${sections.map((item) => `**v${item.version}**`).join(" → ")}。\n\n---\n\n`
    : "";
const notes = header + joinSections(sections);
const target = out.startsWith("/") || /^[A-Za-z]:[\\/]/.test(out) ? out : join(root, out);
mkdirSync(dirname(target), { recursive: true });
writeFileSync(target, notes, "utf8");

// --- 汇报 ---------------------------------------------------------------------
console.log(`已写入 ${target}`);
if (fromVersion) {
  console.log(`发布区间：${fromTag} 之后 → v${version}`);
  console.log(`覆盖 ${sections.length} 个版本：${sections.map((item) => `v${item.version}`).join(" → ")}`);
} else {
  console.log("发布区间：无起点（首次发布或 --since none），只写当前版本");
}
console.log(`--- v${version} 段落（前 6 行）---`);
console.log(sections[0].section.split(/\r?\n/).slice(0, 6).join("\n"));
console.log("---");
console.log(
  `下一步：gh release create v${version} WSight.exe docs/*.png --notes-file "${target}"`,
);
