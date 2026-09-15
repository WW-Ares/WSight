# 发布守则

这份文件管的是"谁能在什么时候发布"，以及"发完怎么确认它真的发出去了"。
命令细节、踩过的坑在别处；这里只写**规则**，因为命令会变，规则不该变。

## 一、授权边界（硬性）

| 动作 | 谁能决定 |
| --- | --- |
| 改代码、本地 `git commit`、构建、截图 | 代理可自行完成，不用问 |
| `git push`、打标签并推送、建/改 GitHub Release、上传附件 | **必须先问，得到明确同意才执行** |

"或至少通知一声"不算同意。没批准就不推。

## 二、提交信息

只有两个词：首次 `Initial commit`，之后一律 `Update`。

- 不写 `feat:` / `fix:` / `chore:` 这类前缀，不写正文，不写多行。
- 更新内容写在 `CHANGELOG.md` 里（见下条），不进提交信息。

## 三、版本号与更新说明

- 版本号在四处重复：`package.json`、`src-tauri/tauri.conf.json`、
  `src-tauri/Cargo.toml`、`src/shared/about.ts`。改完必须跑
  `pnpm check:version`，不一致就别构建。
- **`CHANGELOG.md` 是更新说明的唯一源头**。Release 页面的说明由它派生，
  不另写一份，避免两处慢慢说不到一块去。

## 四、发布流程

1. 先把功能改动 `git commit -m "Update"`（功能与版本号分开提交）。
2. 升版本号四处 + `CHANGELOG.md` 顶部加条目，跑 `pnpm check:version`。
3. 构建前端与 Rust 端，复制产物到根目录 `WSight.exe`。
4. **截图验收**：把 `opacity` 临时设为 1，启动程序截图，截完还原。
   编译通过不等于功能正常，看得见的这一遍不能省。
5. 提交版本变更，打标签 `vX.Y.Z`。
6. **征得同意后**推送 main 与标签。
7. 生成发布说明并建 Release：
   `pnpm release:notes` → `gh release create vX.Y.Z WSight.exe docs/*.png --notes-file …`。
8. **发版后自检**：`pnpm release:check`。全绿才算这一版发完了；
   有任何 ❌ 就当场处理，别留在下个版本。

## 五、自检查什么

`scripts/post-release-check.mjs` 会回查远端（不只看本地命令的退出码）：

工作区是否干净、本地 HEAD 与远端 main 是否一致、远端最后一条提交说明是否合规、
标签是否指向 HEAD、Release 是否存在且附件齐全、发布说明是否来自 `CHANGELOG.md`。

设计意图很直白：**命令没报错 ≠ 发布成功**。附件漏传、标签打歪、提交说明手滑写成别的词，
这些只有回头看一眼远端才能发现。

## 六、推送通道

只走 SSH，HTTPS 会被代理掐断（502）。推送时指定隧道配置，不要改全局 `~/.ssh/config`：

```bash
git -c core.sshCommand="ssh -F C:/Users/Administrator/.workbuddy/ssh_config_git" push origin main --tags
```

如果连不上，先确认 `~/.workbuddy/git-proxy-tunnel.py` 里的 `PROXY_PORT` 是系统代理端口
（`127.0.0.1:7897`），不要写沙箱注入的会话端口——那个端口每次会话都变。

## 七、已知隐患

`.github/workflows/release.yml` 在推送标签时会自动构建并尝试发布，但当前**每次都失败**：
`Resource not accessible by integration`（workflow 缺 `contents: write` 权限），
所以每个标签下面都留了一条红色的 Actions 记录。

**在它被修好或删掉之前，不要指望 CI 帮忙发版**——发布一律按上面第四节的流程手动做，
并且注意别让 CI 与手动发布抢同一个 Release。
