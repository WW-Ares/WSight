#!/usr/bin/env node
/**
 * 从 CHANGELOG.md 抽出"本次发布要交代"的内容，写成文件交给 `gh release create --notes-file`。
 *
 *   node scripts/release-notes.mjs             # 版本号默认取 package.json
 *   node scripts/release-notes.mjs 0.4.5
 *   node scripts/release-notes.mjs 0.4.5 --since v0.4.2   # 手动指定上次发布的标签
 *   node scripts/release-notes.mjs 0.4.5 --since none     # 只发这一个版本
 *   node scripts/release-notes.mjs 0.4.5 --out /tmp/notes.md
 *
 * 为什么要它：发布说明只该有一个源头。CHANGELOG.md 是源头，Release 页面的说明由它
 * 派生；否则"仓库里的记录"和"发布页上的说明"会各写一份，慢慢就对不上了。
 *
 * **区间口径**：从上一个已打标签的版本（= 上次发出去的版本）到本次版本，中间所有
 * 版本都要算进来 —— 只提交、没发过 Release 的中间版本必须跟着一起露脸，否则用户装上
 * 新版，看到的说明里缺了一整批他从未见过的功能。标签由 `lib/release-range.mjs` 探测，
 * `--since` 可以覆盖它。
 *
 * **归并口径**：读者要的是"从上次发版到现在一共变成了什么样"，不是"每个版本号底下各
 * 干了什么"。所以输出**不带版本标题**，区间内所有条目按 新增 / 改进 / 修复 三类并到
 * 一起；每条前面留一行 `<!-- vX.Y.Z -->` 注释标明出处（渲染时不显示）。
 *
 * 需要人再走一步：同一件事在多个版本里被反复改动的（v0.4.4 加上磁吸、v0.4.5 又修好），
 * 措辞层面要合并成一句"新增磁吸"。脚本只把材料摆整齐 —— 它会往 stderr 打出疑似同主题
 * 的条目对你，照着一改即可。
 *
 * 抽不出段落就退出码 1 —— 版本没写进 CHANGELOG 就发版，就是要在这里被拦住。
 */
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { changelogSectionsSince, likelyDuplicates, mergeSections } from "./lib/changelog-section.mjs";
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

// 归并成一份说明：不带版本标题，所有条目按 新增 / 改进 / 修复 并到一起。
// 多版本时顶上写一行"这次到底带了什么"，否则读者会以为漏了什么。
const versions = sections.map((item) => item.version);
const header =
  sections.length > 1 && fromTag
    ? `本版包含 \`${fromTag}\`（上次发布）之后的全部改动：` +
      `${versions.map((item) => `**v${item}**`).join(" → ")}（共 ${sections.length} 个版本）。` +
      `以下按内容归类，同一处功能的多轮改动已合并。\n\n---\n\n`
    : `本次发布 **v${version}**，更新内容如下。\n\n---\n\n`;
const notes = header + mergeSections(sections);
const target = out.startsWith("/") || /^[A-Za-z]:[\\/]/.test(out) ? out : join(root, out);
mkdirSync(dirname(target), { recursive: true });
writeFileSync(target, notes, "utf8");

// --- 汇报 ---------------------------------------------------------------------
console.log(`已写入 ${target}`);
if (fromVersion) {
  console.log(`发布区间：${fromTag} 之后 → v${version}`);
  console.log(`覆盖 ${sections.length} 个版本：${versions.map((item) => `v${item}`).join(" → ")}`);
} else {
  console.log("发布区间：无起点（首次发布或 --since none），只写当前版本");
}

// 同一件事被多个版本反复改的，措辞要合成一句再发（v0.4.4 加磁吸、v0.4.5 修磁吸
// → "新增磁吸"）。脚本认不出哪几条该合并，只把疑似同主题的挑出来给人看。
const suspects = likelyDuplicates(sections);
if (suspects.length) {
  console.warn("");
  console.warn("⚠️  下面这些条目可能讲的是同一件事，发之前请合并成一句：");
  for (const item of suspects) {
    console.warn(`    · ${item.a}  ×  ${item.b}   （共 "${item.shared}"）`);
  }
  console.warn("   （只影响措辞，不改就发也不会报错）");
}

console.log("");
console.log(`--- 归并稿前 12 行 ---`);
console.log(notes.split(/\r?\n/).slice(0, 12).join("\n"));
console.log("---");
console.log(
  `下一步：核对措辞并合并同主题条目，然后\n` +
    `  gh release create v${version} WSight.exe docs/*.png --notes-file "${target}"`,
);
