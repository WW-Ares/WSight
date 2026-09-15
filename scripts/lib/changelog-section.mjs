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
