/**
 * CHANGELOG.md 是唯一的发布说明源头，别的产物（Release 页面、发版自检）
 * 都从它派生。解析逻辑集中在这里，免得两处正则各自漂移。
 */

/**
 * 取出某个版本的段落（含标题行），找不到返回 null。
 *
 * 认得的标题写法：`## [0.4.1] - 2026-09-15` 或 `## 0.4.1`。
 * 段落结束于下一个 `## ` 标题；尾部的 `---` 分隔线会被去掉。
 *
 * @param {string} changelogText CHANGELOG.md 的全文
 * @param {string} version       形如 "0.4.1"
 * @returns {string|null}
 */
export function changelogSection(changelogText, version) {
  const lines = changelogText.split(/\r?\n/);
  const escaped = version.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const heading = new RegExp(`^##\\s+\\[?${escaped}\\]?`);

  const start = lines.findIndex((line) => heading.test(line.trim()));
  if (start === -1) return null;

  let end = lines.length;
  for (let i = start + 1; i < lines.length; i++) {
    if (/^##\s+/.test(lines[i].trim())) {
      end = i;
      break;
    }
  }

  const body = lines
    .slice(start, end)
    .join("\n")
    .trimEnd()
    .replace(/\n+-{3,}\s*$/, "")
    .trimEnd();

  return body ? `${body}\n` : null;
}

/**
 * 列出 CHANGELOG 里出现过的所有版本号，按文件里的先后顺序。
 *
 * CHANGELOG 是倒着写的（最新在最上面），所以返回值通常是从大到小 ——
 * 但这里不排序、只照抄文件顺序，因为"读者看到的第一段"才是发布页要顶上去的那段。
 *
 * @param {string} changelogText
 * @returns {string[]} 形如 ["0.4.5", "0.4.4", ...]
 */
export function changelogVersions(changelogText) {
  const versions = [];
  for (const raw of changelogText.split(/\r?\n/)) {
    const match = raw.trim().match(/^##\s+\[?(\d+\.\d+\.\d+[^\]\s]*)\]?/);
    if (match) versions.push(match[1]);
  }
  return versions;
}

/**
 * 取出一段"发布区间"的段落集合：从 `newest` 开始，往回取到 `oldestExclusive`
 * 的前一段为止（不含 `oldestExclusive` 自己）。
 *
 * 这是发布说明的正确口径：一次发布要交代的是**上一次发布到这一次之间**的全部
 * 改动，而不是最后那一个版本号底下的几行。中间那些只提交、没发过 Release 的
 * 版本（本地攒着的），必须跟着这次一起露脸 —— 否则用户装上新版，看到的说明里
 * 缺了一整批他从未见过的功能。
 *
 * `oldestExclusive` 传 null 表示只取 `newest` 一段（首次发布，或没有上一个 tag）。
 * `oldestExclusive` 在 CHANGELOG 里找不到时返回 null —— 宁可让调用者报错，也不要
 * 顺着文末一路把半年前的版本全带上。
 *
 * @param {string} changelogText
 * @param {string} newest          本次要发布的版本，如 "0.4.5"
 * @param {string|null} oldestExclusive 上一次发布过的版本，如 "0.4.2"
 * @returns {{version: string, section: string}[]|null}
 */
export function changelogSectionsSince(changelogText, newest, oldestExclusive = null) {
  const versions = changelogVersions(changelogText);
  const from = versions.indexOf(newest);
  if (from === -1) return null;

  let to = from + 1; // exclusive；无起点时只取 newest 自己
  if (oldestExclusive) {
    const stop = versions.indexOf(oldestExclusive);
    if (stop === -1) return null;
    if (stop <= from) return null; // 位置反了：上一个版本排在本次后面，不合常理
    to = stop;
  }

  const picked = versions.slice(from, to).map((version) => {
    const section = changelogSection(changelogText, version);
    return section ? { version, section: section.trimEnd() } : null;
  });
  return picked.every(Boolean) && picked.length ? picked : null;
}

/**
 * 把若干段落拼成一份发布说明：段落之间用 `---` 隔开，最新的在最前面。
 *
 * 注意：这是**逐版本**的拼法，已经不用于发布说明（见 `mergeSections`）。
 * 留着是因为"按版本看历史"这个视角本身还有用（例如人工核对区间）。
 *
 * @param {{version: string, section: string}[]} sections
 * @returns {string}
 */
export function joinSections(sections) {
  return `${sections.map((item) => item.section).join("\n\n---\n\n")}\n`;
}

/**
 * 类别顺序：发布说明里先讲得到了什么，再讲变好的地方，最后讲修掉的毛病。
 */
export const CATEGORY_ORDER = ["新增", "改进", "修复", "其它"];

const CATEGORY_RULES = [
  ["新增", /^(added|new|feature|新增|增加|新功能)/i],
  ["修复", /^(fixed|fix|bug|修复|修正)/i],
  ["改进", /^(changed|improved|perf|改进|优化|调整|变更|改善)/i],
];

/**
 * 小标题 → 类别。各版本用的小节名不统一（`### Added` 与 `### 修复：开机启动` 混着来），
 * 这里收拢成同一套类别，好让不同版本的同性质条目并到一起。
 *
 * @param {string} heading 去掉 `#` 之后的小标题文字
 * @returns {string}
 */
function categorize(heading) {
  for (const [category, pattern] of CATEGORY_RULES) {
    if (pattern.test(heading)) return category;
  }
  return "其它";
}

/**
 * 把一个版本段落拆成条目，每条带上它所属的类别。
 *
 * 条目是列表项（`- xxx` 或 `1. xxx`），其后的缩进行算它的续行 —— CHANGELOG 里的
 * 条目常常写好几段，拆散了读不通。小标题只用来定类别，本身不进条目。
 *
 * @param {string} section 含版本标题行的完整段落
 * @returns {{category: string, lines: string[]}[]}
 */
export function sectionEntries(section) {
  const entries = [];
  let category = null;
  let current = null;

  const flush = () => {
    if (current && current.lines.join("").trim()) entries.push(current);
    current = null;
  };

  for (const raw of section.split(/\r?\n/)) {
    const line = raw.trimEnd();
    const trimmed = line.trim();
    if (!trimmed) {
      if (current) current.lines.push("");
      continue;
    }
    // 版本标题（`## [0.4.5] - 2026-09-15`）：不属于任何一个类别
    if (/^##\s+\[?\d/.test(trimmed)) {
      flush();
      category = null;
      continue;
    }
    if (/^#{2,4}\s+/.test(trimmed)) {
      flush();
      category = categorize(trimmed.replace(/^#{2,4}\s+/, ""));
      continue;
    }
    const isItem = /^(?:[-*]|\d+\.)\s+/.test(trimmed);
    if (isItem) {
      flush();
      current = { category: category ?? "其它", lines: [trimmed] };
      continue;
    }
    // 缩进行是上一条的续行；顶格的散句自成一条
    if (current && line !== trimmed) {
      current.lines.push(line);
      continue;
    }
    flush();
    current = { category: category ?? "其它", lines: [trimmed] };
  }
  flush();

  return entries.map((entry) => ({ ...entry, lines: trimBlankEdges(entry.lines) }));
}

function trimBlankEdges(lines) {
  const out = [...lines];
  while (out.length && !out[0].trim()) out.shift();
  while (out.length && !out[out.length - 1].trim()) out.pop();
  return out;
}

/**
 * 把"发布区间"里的若干版本段落**归并成一份说明**。
 *
 * 这是发布说明的正确口径：读者要的是"从上次发版到现在，这东西一共变成了什么样"，
 * 不是"每个版本号底下各干了什么"。所以逐版本标题被去掉，条目按类别并到一起。
 *
 * 归并只做到"同类合并"这一步 —— 同一件事在多个版本里被反复改动的（先加上、后修好），
 * 措辞层面的合并要人来看，脚本只把材料摆整齐，并在每条前留一行
 * `<!-- vX.Y.Z -->` 注释标明出处（Markdown 渲染时不显示）。
 *
 * @param {{version: string, section: string}[]} sections
 * @returns {string}
 */
export function mergeSections(sections) {
  /** @type {Map<string, {version: string, text: string}[]>} */
  const groups = new Map(CATEGORY_ORDER.map((name) => [name, []]));

  for (const { version, section } of sections) {
    for (const entry of sectionEntries(section)) {
      const bucket = groups.get(entry.category) ?? groups.get("其它");
      bucket.push({ version, text: entry.lines.join("\n") });
    }
  }

  const parts = [];
  for (const name of CATEGORY_ORDER) {
    const items = groups.get(name);
    if (!items.length) continue;
    const body = items
      .map((item) => `<!-- v${item.version} -->\n${item.text}`)
      .join("\n");
    parts.push(`### ${name}\n\n${body}`);
  }

  return parts.length ? `${parts.join("\n\n")}\n` : "";
}

/**
 * 挑出"可能讲的是同一件事"的条目，供人合并时参考。
 *
 * 判据很土：把每条的第一个加粗短语当作主题，去掉标点与常见虚词后取字符交集，
 * 交集里有 2 个字以上就算疑似。宁可多报 —— 它只往 stderr 打提示，不影响输出。
 *
 * @param {{version: string, section: string}[]} sections
 * @returns {{a: string, b: string, shared: string}[]}
 */
export function likelyDuplicates(sections) {
  const STOP = new Set("的了不在与和之一款并及其以对为个");
  const themes = [];
  for (const { version, section } of sections) {
    for (const entry of sectionEntries(section)) {
      const bold = entry.lines.join(" ").match(/\*\*(.+?)\*\*/);
      if (!bold) continue;
      const chars = new Set(
        bold[1].replace(/[^\u4e00-\u9fa5A-Za-z0-9]/g, "").split(""),
      );
      themes.push({ version, chars, label: `${bold[1]}（v${version}）` });
    }
  }

  const pairs = [];
  for (let i = 0; i < themes.length; i++) {
    for (let j = i + 1; j < themes.length; j++) {
      if (themes[i].version === themes[j].version) continue;
      const shared = [...themes[i].chars].filter(
        (c) => themes[j].chars.has(c) && !STOP.has(c),
      );
      if (shared.length >= 2) {
        pairs.push({ a: themes[i].label, b: themes[j].label, shared: shared.join("") });
      }
    }
  }
  return pairs;
}
