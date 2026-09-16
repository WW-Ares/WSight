#!/usr/bin/env node
/**
 * 发版后自检：确认这次发布"真的落地了"，而不是只看每条命令有没有报错。
 *
 *   node scripts/post-release-check.mjs          # 检查 package.json 的 version
 *   node scripts/post-release-check.mjs 0.4.0
 *
 * 查六件事：
 *   1. 工作区干净（发版完不该还有没提交的改动）
 *   2. 本地 HEAD 与远端 main 一致（推上去了、也没落后）
 *   3. 远端最后一条提交说明只有那两个词之一
 *   4. 标签 v<版本> 存在，且指向 HEAD
 *   5. Release 存在，附件里有 WSight.exe（且**只该有它**，夹带截图会提醒），并且附件的
 *      sha256 与本地 WSight.exe 一致 —— 名字对不代表内容对：v0.5.0 传上去的那个包
 *      名字没错、大小正常，前端却是上一个版本的，只查名字根本查不出来
 *   6. Release 说明覆盖了"上一个已发布标签 → 本次版本"区间里的**每一个**版本，
 *      并且是**归并稿**——按内容归类，不是逐版本罗列。只提交、没发过 Release 的
 *      中间版本必须跟着本次一起露脸；同一处功能的多轮改动（先加上、后修好）
 *      应该合并成一句，而不是把两轮都写上去。
 *
 * 有 ❌ 退出码 1 —— 可以直接接在发布流程末尾当守门员。
 */
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { changelogSectionsSince, sectionEntries } from "./lib/changelog-section.mjs";
import { previousReleaseTag } from "./lib/release-range.mjs";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const ALLOWED_MESSAGES = new Set(["Initial commit", "Update"]);
const REQUIRED_ASSET = "WSight.exe";
/**
 * 发布附件只上 exe，截图不上传。
 *
 * 截图留在仓库 `docs/` 里给 README 和文档用，Release 附件不带 —— 用户要的是
 * 能跑的程序，混着几张 png 只会让"下载哪个"变含糊。这条按大王 2026-09-16 的要求定下。
 */
const IMAGE_ASSET = /\.(png|jpe?g|gif|webp|bmp)$/i;

const version =
  process.argv[2] ?? JSON.parse(readFileSync(join(root, "package.json"), "utf8")).version;

/** @type {{level:"ok"|"warn"|"fail", label:string, detail:string}[]} */
const checks = [];
const add = (level, label, detail = "") => checks.push({ level, label, detail });

function tryRun(cmd, args) {
  try {
    const out = execFileSync(cmd, args, {
      cwd: root,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
      maxBuffer: 16 * 1024 * 1024,
    });
    return { ok: true, out: out.trim(), err: "" };
  } catch (error) {
    const err = `${error.stderr ?? ""}${error.message ?? ""}`.trim();
    return { ok: false, out: `${error.stdout ?? ""}`.trim(), err };
  }
}

function findGh() {
  const candidates = [
    process.env.GH_PATH,
    "gh",
    "C:/Program Files/GitHub CLI/gh.exe",
    "C:\\Program Files\\GitHub CLI\\gh.exe",
  ].filter(Boolean);
  for (const candidate of candidates) {
    if (tryRun(candidate, ["--version"]).ok) return candidate;
  }
  return null;
}

/** gh api；404 返回 null，其它错误直接抛出给人看 */
function ghApi(path) {
  const res = tryRun(gh, ["api", path]);
  if (res.ok) return JSON.parse(res.out);
  if (/Not Found|HTTP 404/i.test(res.err)) return null;
  throw new Error(`gh api ${path} 失败：${res.err.split("\n")[0]}`);
}

const gh = findGh();
const short = (sha) => (sha ? sha.slice(0, 7) : "(未知)");

/**
 * 一个版本段落里的"主题指纹"：各条目开头那个加粗短语的前两个字。
 *
 * 发布说明会把措辞重写、把同主题的条目并成一句，所以逐字比对行不通；但"这一版谈过
 * 磁吸、谈过天气、谈过设置"这种主题不该凭空消失 —— 拿它对一下，能发现"说明是另写的一份"。
 *
 * @param {string} section
 * @returns {string[]}
 */
function themeFingerprints(section) {
  const prints = [];
  for (const entry of sectionEntries(section)) {
    for (const piece of entry.lines.join(" ").match(/\*\*(.+?)\*\*/g) ?? []) {
      const core = piece.replace(/\*\*/g, "").replace(/[^\u4e00-\u9fa5A-Za-z0-9]/g, "");
      if (core.length >= 2) prints.push(core.slice(0, 2));
    }
  }
  return prints;
}

console.log(`WSight v${version} 发版自检\n`);

// --- 1. 工作区 ---------------------------------------------------------------
const status = tryRun("git", ["status", "--porcelain"]);
if (!status.ok) {
  add("fail", "工作区状态", `git status 失败：${status.err.split("\n")[0]}`);
} else if (status.out) {
  const files = status.out.split("\n");
  add(
    "fail",
    "工作区干净",
    `还有 ${files.length} 项未提交：${files.slice(0, 3).join(" / ")}${files.length > 3 ? " …" : ""}`,
  );
} else {
  add("ok", "工作区干净");
}

// --- 2~6. 远端侧 -------------------------------------------------------------
const headSha = tryRun("git", ["rev-parse", "HEAD"]).out;

if (!gh) {
  add("warn", "远端状态", "找不到 gh 命令，设置 GH_PATH 或安装 GitHub CLI 后重跑");
} else {
  const remote = tryRun("git", ["remote", "get-url", "origin"]);
  const match = remote.ok ? remote.out.match(/github\.com[:/]([^/]+)\/(.+?)(?:\.git)?$/) : null;

  if (!match) {
    add("fail", "远端仓库", `从 origin 解析不出 GitHub 仓库：${remote.out || remote.err}`);
  } else {
    const repo = `${match[1]}/${match[2]}`;

    // 2. 本地 vs 远端 main
    const commit = ghApi(`repos/${repo}/commits/main`);
    const remoteSha = commit?.sha ?? "";
    if (!commit) {
      add("fail", "远端 main", `gh api repos/${repo}/commits/main 返回 404`);
    } else if (headSha && remoteSha === headSha) {
      add("ok", "本地与远端 main 一致", short(headSha));
    } else {
      add("fail", "本地与远端 main 一致", `本地 ${short(headSha)} ≠ 远端 ${short(remoteSha)}`);
    }

    // 3. 远端最后一条提交说明
    const message = (commit?.commit?.message ?? "").split(/\r?\n/)[0].trim();
    if (!commit) {
      // 上一条已经报过了，不重复
    } else if (ALLOWED_MESSAGES.has(message)) {
      add("ok", "远端最后一条提交说明合规", `"${message}"`);
    } else {
      add(
        "fail",
        "远端最后一条提交说明合规",
        `是 "${message}"，只允许 Initial commit / Update`,
      );
    }

    // 4. 标签指向 HEAD
    const tagRef = ghApi(`repos/${repo}/git/ref/tags/v${version}`);
    if (!tagRef) {
      add("fail", `标签 v${version} 存在`, "远端没有这个标签 —— 还没打标签或没推上去");
    } else {
      let target = tagRef.object?.sha ?? "";
      if (tagRef.object?.type === "tag") {
        target = ghApi(`repos/${repo}/git/tags/${target}`)?.object?.sha ?? "";
      }
      if (target && headSha && target === headSha) {
        add("ok", `标签 v${version} 指向 HEAD`, short(target));
      } else {
        add(
          "fail",
          `标签 v${version} 指向 HEAD`,
          `标签指向 ${short(target)}，HEAD 是 ${short(headSha)}`,
        );
      }
    }

    // 5~6. Release、附件、说明
    const release = ghApi(`repos/${repo}/releases/tags/v${version}`);
    if (!release) {
      add("fail", `Release v${version} 存在`, "远端没有对应 Release —— 还没发，或发失败了");
    } else {
      const names = (release.assets ?? []).map((asset) => asset.name);
      if (names.includes(REQUIRED_ASSET)) {
        add("ok", "Release 附件含安装产物", names.join(" / "));
      } else {
        add(
          "fail",
          "Release 附件含安装产物",
          `缺少 ${REQUIRED_ASSET}，现有：${names.join(" / ") || "(空)"}`,
        );
      }

      // 附件是不是本地这一份？用户装的就是它，所以它必须和本地打出来的
      // 那个文件一模一样。只看名字会漏掉"传了别的构建产物"这种情况。
      const asset = (release.assets ?? []).find((a) => a.name === REQUIRED_ASSET);
      const local = join(root, REQUIRED_ASSET);
      if (asset) {
        if (!existsSync(local)) {
          add("warn", "附件与本地产物一致", `本地没有 ${REQUIRED_ASSET}，没法比对`);
        } else {
          const sha = createHash("sha256").update(readFileSync(local)).digest("hex");
          const digest = String(asset.digest ?? "").replace(/^sha256:/i, "").toLowerCase();
          if (!digest) {
            add("warn", "附件与本地产物一致", "GitHub 没给 digest，无法比对");
          } else if (digest === sha) {
            add("ok", "附件与本地产物一致", `sha256 ${sha.slice(0, 12)}…`);
          } else {
            add(
              "fail",
              "附件与本地产物一致",
              `附件 ${digest.slice(0, 12)}… ≠ 本地 ${sha.slice(0, 12)}…`,
            );
          }
        }
      }
      const images = names.filter((name) => IMAGE_ASSET.test(name));
      if (images.length) {
        add(
          "warn",
          "Release 附件只有 exe",
          `带了图片（${images.join(" / ")}）—— 截图留在仓库 docs/ 即可，不用上传`,
        );
      } else {
        add("ok", "Release 附件只有 exe", "未夹带截图");
      }

      const body = (release.body ?? "").trim();
      if (!body) {
        add("warn", "Release 说明非空", "说明是空的，别让发布页只剩一个标题");
      } else {
        // 说明该覆盖的区间：上一个已发布标签 → 本次版本。中间没发过版的版本
        // （本地攒着的）必须一起出现，否则用户看到的说明缺了他没见过的功能。
        const fromTag = previousReleaseTag(version, root);
        const fromVersion = fromTag ? fromTag.replace(/^v/, "") : null;
        const sections = changelogSectionsSince(
          readFileSync(join(root, "CHANGELOG.md"), "utf8"),
          version,
          fromVersion,
        );

        if (!sections) {
          add(
            "warn",
            "Release 说明覆盖全部改动",
            `CHANGELOG.md 里划不出区间（起点 ${fromTag ?? "无"}）`,
          );
        } else {
          const range = fromVersion ? `${fromTag} 之后 → v${version}` : `v${version}`;

          // 6a. 区间里每个版本都得在说明里露过面
          const mentioned = new Set([...body.matchAll(/v?(\d+\.\d+\.\d+)/g)].map((m) => m[1]));
          const missing = sections
            .filter((item) => !mentioned.has(item.version))
            .map((item) => `v${item.version}`);
          if (missing.length) {
            add(
              "fail",
              "Release 说明覆盖全部改动",
              `区间（${range}）里 ${missing.join(" / ")} 没在说明里露面` +
                ` —— 漏带了只提交、没发过 Release 的版本`,
            );
          } else {
            add("ok", "Release 说明覆盖全部改动", `${range}，共 ${sections.length} 个版本`);
          }

          // 6b. 说明要讲"一共改了什么"，不是"每个版本各干了什么"
          const headings = [...body.matchAll(/^#{2,3}\s+\[?(\d+\.\d+\.\d+)/gm)].map((m) => m[1]);
          if (headings.length >= 2) {
            add(
              "fail",
              "Release 说明已归并",
              `说明里还按版本分段（${headings.join(" / ")}）—— 读者要看的是这次一共改了什么，` +
                `同一处功能的多轮改动应合并成一句`,
            );
          } else {
            add("ok", "Release 说明已归并", "按内容归类，未逐版本罗列");
          }

          // 6c. 同源：措辞可以重写，主题不该丢
          const shortfalls = [];
          for (const item of sections) {
            const prints = themeFingerprints(item.section);
            if (!prints.length) continue;
            const hits = prints.filter((print) => body.includes(print)).length;
            if (hits * 2 < prints.length) {
              shortfalls.push(`v${item.version}（${hits}/${prints.length}）`);
            }
          }
          if (shortfalls.length) {
            add(
              "warn",
              "Release 说明与 CHANGELOG 同源",
              `${shortfalls.join(" / ")} 的主题词在说明里几乎找不到 —— 确认不是另写了一份`,
            );
          } else {
            add("ok", "Release 说明与 CHANGELOG 同源", "各版本谈过的主题都能在说明里对上");
          }
        }
      }
    }
  }
}

// --- 输出 --------------------------------------------------------------------
const ICON = { ok: "✅", warn: "⚠️ ", fail: "❌" };
for (const check of checks) {
  console.log(`  ${ICON[check.level]} ${check.label}${check.detail ? `  —— ${check.detail}` : ""}`);
}

const failed = checks.filter((c) => c.level === "fail").length;
const warned = checks.filter((c) => c.level === "warn").length;
const passed = checks.length - failed - warned;
console.log("");
console.log(
  failed
    ? `结论：${passed} 项通过，${failed} 项失败${warned ? `，${warned} 项警告` : ""}`
    : `结论：全部通过${warned ? `（${warned} 项警告）` : ""}`,
);

process.exit(failed ? 1 : 0);
