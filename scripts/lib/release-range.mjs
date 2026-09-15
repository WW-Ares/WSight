/**
 * 「这次发布要交代哪一段历史」——用 git 标签划定发布区间。
 *
 * 规则：发布说明覆盖 **上一个已打标签的版本** 到 **本次版本** 之间的全部 CHANGELOG
 * 段落。标签 = 发过 Release 的版本；中间那些只在本地提交、没打标签的版本，
 * 会跟着本次一起露脸。
 *
 * 为什么用标签而不是 package.json 里的上一个版本号：版本号是"我改到了第几版"，
 * 标签才是"我发出去过第几版"。用户能装到的只有后者，说明也该从后者开始算。
 */
import { execFileSync } from "node:child_process";

/** 数字段比较，够用且不引依赖。返回 -1 / 0 / 1。 */
export function compareVersions(a, b) {
  const pa = a.split(".").map(Number);
  const pb = b.split(".").map(Number);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const diff = (pa[i] ?? 0) - (pb[i] ?? 0);
    if (diff) return diff < 0 ? -1 : 1;
  }
  return 0;
}

/**
 * 比 `currentVersion` 小的最新一个标签，形如 "v0.4.2"；没有则 null。
 *
 * 有意把 `>= currentVersion` 的标签排除掉：脚本重跑时，本次的标签可能已经打上了，
 * 那时它自己就是"最新的标签"，拿它当区间起点会算出一段空的区间。
 *
 * @param {string} currentVersion 形如 "0.4.5"
 * @param {string} cwd            仓库根目录
 * @returns {string|null}
 */
export function previousReleaseTag(currentVersion, cwd) {
  let raw;
  try {
    raw = execFileSync("git", ["tag", "--list", "v*"], {
      cwd,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    });
  } catch {
    return null; // 不是 git 仓库 / 没有 git：退回"只发当前版本"
  }

  const candidates = raw
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean)
    .map((tag) => ({ tag, version: tag.replace(/^v/, "") }))
    .filter(({ version }) => /^\d+\.\d+\.\d+$/.test(version))
    .filter(({ version }) => compareVersions(version, currentVersion) < 0)
    .sort((a, b) => compareVersions(a.version, b.version));

  return candidates.length ? candidates[candidates.length - 1].tag : null;
}
