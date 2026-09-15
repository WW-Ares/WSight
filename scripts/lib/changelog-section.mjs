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

  let to = versions.length; // exclusive
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
 * @param {{version: string, section: string}[]} sections
 * @returns {string}
 */
export function joinSections(sections) {
  return `${sections.map((item) => item.section).join("\n\n---\n\n")}\n`;
}

/**
 * 段落里的第一行"有内容的话"，用来粗判两份文案是不是同一个东西。
 * 跳过标题行、空行、`###` 小标题。
 *
 * @param {string} section
 * @returns {string|null}
 */
export function firstContentLine(section) {
  for (const raw of section.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    return line;
  }
  return null;
}
