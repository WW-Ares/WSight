//! In-place self-update, without an installer.
//!
//! WSight ships as one green `WSight.exe` and updates by replacing itself.
//! There is no NSIS package (the toolchain will not download here), no signing
//! key to lose, and no `latest.json` to keep in step - only GitHub Releases,
//! which the project already publishes by hand.
//!
//! The whole design turns on one Windows rule: **you cannot overwrite a running
//! executable, but you can rename it.** So the swap is
//!
//! 1. `WSight.exe`         -> `WSight-old-0.4.5.exe`
//! 2. `WSight.exe.new`     -> `WSight.exe`
//! 3. launch the new `WSight.exe`, then exit
//! 4. the new process deletes older `WSight-old-*.exe`, keeping one
//!
//! The path never changes, so the auto-start entry, any shortcut and the update
//! itself all keep talking about the same file.
//!
//! **The swap is started from two places, and the unattended one is the point.**
//! A button in settings ("重启更新") installs a staged build immediately. But a
//! desktop widget's settings window is opened roughly never, so on its own that
//! button means an update could sit on disk forever - downloaded, verified, and
//! never once installed. `main` therefore applies a staged build at launch,
//! before any window exists, and hands over to it; the user's next logon is
//! simply the new version. Nothing is lost by doing it there: at that moment
//! there is no window, no unsaved state, and no user watching.
//!
//! What that costs is a way back, which is why step 4 keeps one generation of
//! the previous build instead of deleting it. If the new build will not start
//! there is no window to offer a "roll back" button from, so the way back has
//! to be a file the user can double-click - hence a name Windows will run,
//! rather than the `.old-` suffix this used to use.
//!
//! Two things make step 3 non-obvious:
//!
//! * **The new process cannot claim the single-instance mutex while we are
//!   still alive** (`single_instance.rs` holds it until the process dies, and
//!   it must - that is what stops the logon double-launch). So the new binary
//!   starts with `--handover` and waits for the mutex to disappear instead of
//!   exiting on sight. That is why there is no helper executable.
//! * **The download is staged next to the exe, not in `%APPDATA%`.** Step 1-2
//!   are renames, and a rename cannot cross volumes - the app lives on `D:`
//!   and `%APPDATA%` is on `C:` on this very machine. When the folder is not
//!   writable (a portable install dropped into `Program Files`) the download
//!   falls back to `%APPDATA%` and gets copied into place instead, which the
//!   apply step handles.
//!
//! Nothing here runs a downloaded binary without a hash check first: GitHub
//! publishes a `digest` per asset and the file is only renamed into place when
//! it matches. A release without a digest still works - the size is checked
//! instead - but the UI says so rather than implying a check that did not
//! happen.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};

use crate::config::{self, AppConfig};
use crate::AppState;

/// GitHub wants a User-Agent and rejects the request without one.
const USER_AGENT: &str = concat!("WSight/", env!("CARGO_PKG_VERSION"));
const RELEASES_API: &str = "https://api.github.com/repos/WW-Ares/WSight/releases/latest";

/// Name of the release asset. Fixed, like the exe: the updater knows exactly
/// what it is looking for instead of trusting whatever is attached.
const ASSET_NAME: &str = "WSight.exe";

/// Downloading / downloaded names, both beside the exe.
const DOWNLOAD_SUFFIX: &str = ".download";
const STAGED_SUFFIX: &str = ".new";
/// What we rename ourselves to before letting the new build take our place.
///
/// It has to end in `.exe`. The one moment this file matters is the moment the
/// fresh build will not start, and then there is no window left to offer a
/// roll-back button from - the user has the file and nothing else. A name
/// Windows refuses to run is not a way back.
const OLD_STEM: &str = "WSight-old-";
const OLD_EXT: &str = ".exe";
/// The suffix this used before the name had to be runnable. Nothing writes it
/// any more, but a folder that still holds one should not keep it forever.
const LEGACY_OLD_PREFIX: &str = "WSight.exe.old-";

/// First check this long after launch - the same 15 s the weather panel waits,
/// so start-up is not competing with a network round trip.
const FIRST_CHECK_SECS: u64 = 15;
/// Then every six hours. Frequent enough that a fix reaches people the same
/// day, rare enough to be invisible in any traffic log.
const CHECK_INTERVAL_SECS: u64 = 6 * 60 * 60;
/// How long the swapped-in binary waits for its predecessor to let go of the
/// single-instance mutex. Generous: the old process is exiting, and giving up
/// early would leave the user with no window at all.
pub const HANDOVER_WAIT_SECS: u64 = 60;

// ------------------------------------------------------------------ status

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    /// `idle` | `checking` | `uptodate` | `available` | `downloading` | `ready` | `error`
    pub state: String,
    /// The version this binary is.
    pub current: String,
    /// Version offered by the release we last looked at.
    pub latest: String,
    /// Release notes, shown in the settings window so nobody has to go read a
    /// browser page to find out what changed.
    pub notes: String,
    /// 0..100 while `state == "downloading"`.
    pub progress: f32,
    /// Human-readable reason for `state == "error"`. Never shown as a dialog.
    pub error: String,
    /// Version already staged and waiting for a restart.
    pub staged: String,
    /// Whether the download was checked against GitHub's `digest`.
    ///
    /// Tri-state on purpose. `Some(true)` and `Some(false)` are the two cases
    /// `download()` can produce; `None` means nothing was checked *in this
    /// session*, which is what a staged update carried over a restart looks
    /// like. Guessing either way there would put a false claim in front of the
    /// user - "未提供校验值" on a download that was in fact verified.
    pub verified: Option<bool>,
}

static STATUS: OnceLock<Mutex<UpdateStatus>> = OnceLock::new();

fn status() -> &'static Mutex<UpdateStatus> {
    STATUS.get_or_init(|| {
        Mutex::new(UpdateStatus {
            state: "idle".to_string(),
            current: current_version().to_string(),
            ..UpdateStatus::default()
        })
    })
}

/// Read the whole status. `poisoned` only if a thread panicked while holding
/// it, and the value is still perfectly readable - the process is built with
/// `panic = "abort"`, so this is belt and braces.
fn with_status<T>(f: impl FnOnce(&mut UpdateStatus) -> T) -> T {
    let mut guard = status().lock().unwrap_or_else(|e| e.into_inner());
    f(&mut guard)
}

/// One line into the app's log, prefixed so it can be told apart from the
/// start-up lines in the same file.
///
/// A background check that fails on purpose says nothing in the UI - a network
/// being down is not the user's problem - which leaves the feature with no
/// evidence at all when it does break. The log is where that evidence goes;
/// `%APPDATA%\WSight\launch.log` is the file support already asks for.
fn note(line: &str) {
    crate::launchlog::log(&format!("update: {line}"));
}

pub fn snapshot() -> UpdateStatus {
    with_status(|s| {
        // A staged update recorded in the config outlives the download's own
        // lifetime: on a restart the file is still there and the prompt has to
        // come back.
        let staged_file = staged_path();
        if !s.staged.is_empty() && !staged_file.exists() {
            s.staged.clear();
        }
        s.clone()
    })
}

/// Seed `staged` from the config at start-up, so "已就绪" survives a restart.
pub fn restore_staged(version: &str) {
    if version.is_empty() {
        return;
    }
    let staged = staged_path();
    if staged.exists() {
        with_status(|s| {
            s.staged = version.to_string();
            s.latest = version.to_string();
            // Deliberately left unknown: the digest check happened in an
            // earlier process and its result was never written down. The UI has
            // a wording for exactly this case rather than inventing one.
            s.verified = None;
            if s.state != "downloading" {
                s.state = "ready".to_string();
            }
        });
    }
}

/// Seed `ignored` from the config at start-up.
///
/// The decision is persisted, so the settings window has to remember it across
/// a restart: without this the page would rest at "启动后自动检查" while the
/// config still held an ignored version, and the obvious reading of that is
/// that the ignore was lost. The first check corrects the label either way -
/// if something newer has been published, the ignored version no longer
/// matches and the release is offered normally.
pub fn restore_ignored(version: &str) {
    if version.is_empty() {
        return;
    }
    with_status(|s| {
        // A staged build outranks this: it is newer than the ignored version
        // (that is why it was downloaded at all) and it has an action attached
        // to it, which "已忽略" does not.
        if s.state != "downloading" && s.staged.is_empty() {
            s.latest = version.to_string();
            s.state = "ignored".to_string();
        }
    });
}

/// True when a verified build is sitting on disk waiting to be installed.
///
/// The launch path asks this before any window exists, so it has to stay a
/// file check with no state behind it.
pub fn staged_ready() -> bool {
    staged_path().exists()
}

/// True when this process is one of the kept-back copies from an earlier
/// update.
///
/// Such a copy exists for exactly one purpose: to be double-clicked when the
/// new build will not start. If it updated itself it would walk the user
/// straight back into the build they were fleeing, silently, on the next
/// launch - so it does not use the updater at all.
pub fn is_previous_copy() -> bool {
    exe_path()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .is_some_and(|n| is_previous_name(&n))
}

/// Throw away a staged build the user does not want.
///
/// The automatic path never needs this - a newer release supersedes the one on
/// disk and the old file is overwritten. It is for the manual one: someone who
/// pressed 检查更新 on a metered connection, or who wants to stay on the version
/// they have, should not have to keep a 10 MB exe on disk as the price.
pub fn discard(app: &AppHandle) -> Result<(), String> {
    let staged = staged_path();
    if staged.exists() {
        std::fs::remove_file(&staged).map_err(|e| format!("无法删除更新包: {e}"))?;
    }
    // Which version is being turned down? The one the prompt was about.
    let wanted = with_status(|s| {
        if s.staged.is_empty() {
            s.latest.clone()
        } else {
            s.staged.clone()
        }
    });

    touch_config(app, |cfg| {
        cfg.update_staged_version.clear();
        // Empty is a legitimate value and means "nothing is ignored", so this
        // needs no branch. What it must not be is *missing*: deleting the file
        // alone was the old behaviour, and the next check simply fetched the
        // same release again - six hours later, or the moment 检查更新 was
        // pressed, and then launch-time install would put it in place anyway.
        cfg.update_ignored_version = wanted;
    });

    with_status(|s| {
        s.notes.clear();
        s.staged.clear();
        s.progress = 0.0;
        s.verified = None;
        s.error.clear();
        // `latest` is deliberately kept: "v0.5.2 已忽略" is the only proof the
        // click landed and the only place the decision can be unmade. With no
        // version to name there is nothing to say, so the page goes back to
        // resting instead of claiming an ignore that never happened.
        s.state = if s.latest.is_empty() { "idle" } else { "ignored" }.to_string();
    });
    Ok(())
}

/// Undo 忽略这个版本.
///
/// Only forgets: looking again is the caller's next move - the settings window
/// presses 检查更新 straight afterwards - so this stays a state edit with no
/// network in it, and cannot fail on a flaky connection.
pub fn unignore(app: &AppHandle) -> Result<(), String> {
    touch_config(app, |cfg| cfg.update_ignored_version.clear());
    with_status(|s| {
        if s.state == "ignored" {
            s.state = "idle".to_string();
            s.latest.clear();
        }
    });
    Ok(())
}

// ------------------------------------------------------------- version math

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// `v0.5.0` / `0.5.0` -> `[0, 5, 0]`. Anything else - a moving tag, a date
/// stamp - is refused rather than guessed at, because this decides whether a
/// download happens.
fn parse_version(text: &str) -> Option<Vec<u32>> {
    let core = text.trim().trim_start_matches('v');
    let core = core.split(['-', '+']).next()?;
    let parts = core
        .split('.')
        .map(|p| p.parse::<u32>().ok())
        .collect::<Option<Vec<u32>>>()?;
    if parts.is_empty() {
        None
    } else {
        Some(parts)
    }
}

fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(a), Some(b)) => {
            // Compare numerically, so 0.10.0 beats 0.9.0 - a string compare
            // gets that backwards and would silently stop offering updates.
            for i in 0..a.len().max(b.len()) {
                let x = a.get(i).copied().unwrap_or(0);
                let y = b.get(i).copied().unwrap_or(0);
                if x != y {
                    return x > y;
                }
            }
            false
        }
        _ => false,
    }
}

/// Is this release one the user asked us to stop offering?
///
/// "Ignored" is a statement about one version, not about updates in general, so
/// anything newer clears the objection on its own - no bookkeeping, and no way
/// to end up permanently cut off by a click made months ago.
///
/// Both sides have to be readable versions. `is_newer` answers `false` for two
/// tags it cannot parse, and negating that would read as "this release is
/// ignored" - the one answer that *hides* a release from the user. So an
/// unreadable value on either side means "not ignored", which fails towards
/// offering something they turned down rather than towards silence. `sanitize`
/// already drops an unparseable `ignored`; this is the second line of defence.
fn is_ignored(release: &str, ignored: &str) -> bool {
    if ignored.is_empty() || parse_version(release).is_none() || parse_version(ignored).is_none() {
        return false;
    }
    !is_newer(release, ignored)
}

// ------------------------------------------------------------------- paths

/// `<exe folder>/WSight.exe` - the file an update replaces.
fn exe_path() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

fn staged_path() -> PathBuf {
    let exe = exe_path().unwrap_or_else(|| PathBuf::from("WSight.exe"));
    let dir = exe.parent().map(Path::to_path_buf).unwrap_or_default();
    dir.join(format!("WSight.exe{STAGED_SUFFIX}"))
}

/// The name this build steps aside to when a new one takes its place.
fn previous_name(version: &str) -> String {
    format!("{OLD_STEM}{version}{OLD_EXT}")
}

fn previous_path(version: &str) -> PathBuf {
    let exe = exe_path().unwrap_or_else(|| PathBuf::from("WSight.exe"));
    let dir = exe.parent().map(Path::to_path_buf).unwrap_or_default();
    dir.join(previous_name(version))
}

/// True for the kept-back copies this updater creates and nothing else, which
/// is what lets the sweep in `clean_up` delete by pattern without ever being
/// able to hit a file the user put in the folder.
fn is_previous_name(name: &str) -> bool {
    name.starts_with(OLD_STEM)
        && name.ends_with(OLD_EXT)
        && name.len() > OLD_STEM.len() + OLD_EXT.len()
}

/// Where a download lands. The exe's own folder when it can be written to, so
/// the final swap is a rename; `%APPDATA%` otherwise.
fn download_dir() -> PathBuf {
    if let Some(dir) = exe_path().and_then(|p| p.parent().map(Path::to_path_buf)) {
        // Probe with a name of our own. Probing with the real staged filename
        // would destroy an update that is already sitting there.
        let probe = dir.join(".wsight-write-probe");
        if std::fs::write(&probe, b"").is_ok() {
            let _ = std::fs::remove_file(&probe);
            return dir;
        }
    }
    let dir = config::data_dir().join("update");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn download_path() -> PathBuf {
    download_dir().join(format!("WSight.exe{DOWNLOAD_SUFFIX}"))
}

// ---------------------------------------------------------------- the fetch

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    #[serde(default)]
    size: u64,
    /// `sha256:<hex>` - present on modern releases, absent on older ones.
    #[serde(default)]
    digest: Option<String>,
    browser_download_url: String,
}

struct Release {
    version: String,
    notes: String,
    url: String,
    size: u64,
    digest: Option<String>,
}

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        // Deliberately no total-request deadline. reqwest's `timeout` covers
        // the whole request *including the body*, so a 5 MB download on a slow
        // link gets killed while it is still making progress. Measured against
        // the real release: 21 s for 5 MB, which is 70% of a 30 s deadline -
        // two earlier runs died on it and the only trace was "check ran but
        // nothing arrived".
        .connect_timeout(Duration::from_secs(20))
        // What a download does need guarding against is a connection that goes
        // quiet halfway, and that is a per-read question, not a total one.
        .read_timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("无法创建 HTTP 客户端: {e}"))
}

async fn latest_release(client: &reqwest::Client) -> Result<Release, String> {
    let response = client
        .get(RELEASES_API)
        // The API answer is ~12 KB, so here a total deadline is the right
        // shape: either it arrives quickly or something is wrong.
        .timeout(Duration::from_secs(30))
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|e| format!("连不上 GitHub: {e}"))?;

    if response.status().as_u16() == 404 {
        return Err("仓库还没有发布过正式版本".to_string());
    }
    if !response.status().is_success() {
        return Err(format!("GitHub 返回 {}", response.status()));
    }

    let release: GhRelease = response
        .json()
        .await
        .map_err(|e| format!("解析发布信息失败: {e}"))?;

    // A draft or a pre-release is not something to push at people: the assets
    // may still be moving, and the tag is not what the docs point at.
    if release.draft || release.prerelease {
        return Err("远端最新版本还是草稿或预发布版".to_string());
    }

    let version = release
        .tag_name
        .trim()
        .trim_start_matches('v')
        .to_string();
    if parse_version(&version).is_none() {
        return Err(format!("看不懂的版本号：{}", release.tag_name));
    }

    let asset = release
        .assets
        .iter()
        .find(|a| a.name == ASSET_NAME)
        .or_else(|| release.assets.iter().find(|a| a.name.ends_with(".exe")))
        .ok_or_else(|| "这个版本没有附带 exe".to_string())?;

    Ok(Release {
        version,
        notes: release.body.trim().to_string(),
        url: asset.browser_download_url.clone(),
        size: asset.size,
        digest: asset.digest.clone(),
    })
}

// -------------------------------------------------------------- the download

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `sha256:6e87f4…` -> `6e87f4…`.
///
/// GitHub's asset `digest` field carries the algorithm prefix; a hex string of
/// our own does not. Normalising rather than comparing the raw strings means a
/// digest that arrives with different case, or without the prefix, still
/// verifies - a hash that fails on formatting would look exactly like a
/// corrupted download.
fn normalize_digest(digest: &str) -> String {
    digest
        .trim()
        .trim_start_matches("sha256:")
        .to_ascii_lowercase()
}

async fn download(
    client: &reqwest::Client,
    release: &Release,
    dest: &Path,
) -> Result<bool, String> {
    use tokio::io::AsyncWriteExt;

    let mut response = client
        .get(&release.url)
        .send()
        .await
        .map_err(|e| format!("下载失败: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("下载返回 {}", response.status()));
    }

    let total = response.content_length().unwrap_or(release.size).max(1);
    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|e| format!("无法写入 {}: {e}", dest.display()))?;

    let mut hasher = Sha256::new();
    let mut written: u64 = 0;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| format!("下载中断: {e}"))?
    {
        hasher.update(&chunk);
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("写入失败: {e}"))?;
        written += chunk.len() as u64;
        let percent = (written as f64 / total as f64 * 100.0).min(100.0) as f32;
        with_status(|s| s.progress = percent);
    }
    file.flush().await.map_err(|e| format!("写入失败: {e}"))?;
    drop(file);

    let actual = hex(&hasher.finalize());

    match release.digest.as_deref() {
        Some(digest) => {
            let expected = normalize_digest(digest);
            if expected != actual {
                let _ = std::fs::remove_file(dest);
                return Err(format!(
                    "下载的文件校验不过（期望 {expected}，实际 {actual}），已丢弃"
                ));
            }
            Ok(true)
        }
        None => {
            // No digest to check against. Say so in the UI rather than
            // pretending the download was verified.
            if release.size > 0 && written != release.size {
                let _ = std::fs::remove_file(dest);
                return Err(format!(
                    "文件大小不对（期望 {} 字节，实际 {written} 字节），已丢弃",
                    release.size
                ));
            }
            Ok(false)
        }
    }
}

// --------------------------------------------------------------- the checks

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn touch_config(app: &AppHandle, edit: impl FnOnce(&mut AppConfig)) {
    if let Ok(mut guard) = app.state::<AppState>().config.lock() {
        edit(&mut guard);
        if let Err(e) = config::save(&guard) {
            eprintln!("[updater] 保存配置失败: {e}");
        }
    }
}

/// One check, start to finish. `manual` marks a check the user asked for, which
/// is allowed to report failures in the UI; an automatic one stays quiet,
/// because a network that is down is not the user's problem.
pub async fn check(app: AppHandle, manual: bool) {
    if is_previous_copy() {
        // A kept-back copy exists to be run when the new build is broken.
        // Finding an update from here would install the very build the user
        // just stepped away from - and, with launch-time installs, do it on
        // the next start without asking. So it never looks.
        note("check skipped - running as a kept-back copy");
        with_status(|s| {
            if manual {
                s.state = "error".to_string();
                s.error = "当前运行的是回退副本，已停用更新检查".to_string();
            }
        });
        return;
    }
    let busy = with_status(|s| s.state == "checking" || s.state == "downloading");
    if busy {
        return;
    }
    with_status(|s| {
        s.state = "checking".to_string();
        s.error.clear();
    });
    note(&format!(
        "check ({}) running {} -> {}",
        if manual { "manual" } else { "auto" },
        current_version(),
        staged_or_dash(),
    ));

    let result = run_check(&app, manual).await;

    let summary = with_status(|s| match result {
        Ok(Outcome::UpToDate) => {
            s.state = "uptodate".to_string();
            s.latest.clear();
            s.notes.clear();
            s.staged.clear();
            "up to date".to_string()
        }
        Ok(Outcome::Ready { version, verified }) => {
            s.state = "ready".to_string();
            s.latest = version.clone();
            s.staged = s.latest.clone();
            // `None` when the build was already staged before this check ran.
            s.verified = verified;
            s.progress = 100.0;
            format!(
                "ready {version} ({})",
                match verified {
                    Some(true) => "digest verified",
                    Some(false) => "size only, no digest published",
                    None => "already staged",
                }
            )
        }
        Ok(Outcome::Ignored { version }) => {
            s.state = "ignored".to_string();
            s.latest = version.clone();
            s.staged.clear();
            s.notes.clear();
            format!("ignored {version} (skipped by request)")
        }
        Ok(Outcome::Available { version }) => {
            s.state = "available".to_string();
            s.latest = version.clone();
            format!("available {version} (auto download off)")
        }
        Err(message) => {
            // A background failure stays invisible. The network being down is
            // not a defect, and it must never bury a build that is already
            // sitting on disk waiting for a restart - that is the one thing on
            // this page the user can still act on.
            if manual {
                s.state = "error".to_string();
                s.error = message.clone();
            } else {
                s.state = if s.staged.is_empty() {
                    "idle".to_string()
                } else {
                    "ready".to_string()
                };
                s.error.clear();
            }
            format!("failed: {message}")
        }
    });
    note(&summary);

    touch_config(&app, |cfg| cfg.update_last_check = now_secs());
}

/// For the log line only: what is staged right now, or `-`.
fn staged_or_dash() -> String {
    with_status(|s| {
        if s.staged.is_empty() {
            "-".to_string()
        } else {
            s.staged.clone()
        }
    })
}

enum Outcome {
    UpToDate,
    /// The release on offer is the one the user told us to stop offering.
    Ignored { version: String },
    Available { version: String },
    /// `verified` is `None` when the build on disk had already been staged
    /// before this check ran: the digest result died with the process that
    /// made it, and the UI has a wording for exactly that rather than
    /// inventing one.
    Ready {
        version: String,
        verified: Option<bool>,
    },
}

async fn run_check(app: &AppHandle, manual: bool) -> Result<Outcome, String> {
    let client = http_client()?;
    let release = latest_release(&client).await?;

    // Already holding a build at least as new as anything published: keep it.
    //
    // This is not merely an optimisation. The comparison below is against the
    // version *running*, which is still the old one until the user restarts —
    // so without this the loop would re-download the same 10 MB every six
    // hours, and a release that has since been pulled would be reported as
    // "已是最新" and wipe the ready prompt the user was about to act on.
    let staged = read_config(app).update_staged_version;
    if !staged.is_empty() && staged_path().exists() && !is_newer(&release.version, &staged) {
        return Ok(Outcome::Ready {
            version: staged,
            verified: None,
        });
    }

    // 忽略这个版本 is remembered, not merely acted on once. Deliberately after
    // the `staged` check above: a build already on disk is there because the
    // user did *not* ignore it, and it keeps its prompt.
    if is_ignored(&release.version, &read_config(app).update_ignored_version) {
        return Ok(Outcome::Ignored {
            version: release.version,
        });
    }

    if !is_newer(&release.version, current_version()) {
        return Ok(Outcome::UpToDate);
    }

    with_status(|s| {
        s.latest = release.version.clone();
        s.notes = release.notes.clone();
    });

    let auto_download = read_config(app).update_auto;
    // A manual check on a machine with automatic updates off still downloads -
    // the user just asked for the update, and stopping at "there is one, go
    // find it yourself" would be a strange reading of that request.
    if !auto_download && !manual {
        return Ok(Outcome::Available {
            version: release.version,
        });
    }

    with_status(|s| {
        s.state = "downloading".to_string();
        s.progress = 0.0;
    });

    let dest = download_path();
    let verified = download(&client, &release, &dest).await?;

    // Only now does it become the staged file. A partial download never wears
    // the name the apply step trusts.
    let staged = staged_path();
    let _ = std::fs::remove_file(&staged);
    std::fs::rename(&dest, &staged).map_err(|e| {
        let _ = std::fs::remove_file(&dest);
        format!("无法就位更新包: {e}")
    })?;

    touch_config(app, |cfg| {
        cfg.update_staged_version = release.version.clone();
    });

    Ok(Outcome::Ready {
        version: release.version,
        verified: Some(verified),
    })
}

fn read_config(app: &AppHandle) -> AppConfig {
    app.state::<AppState>()
        .config
        .lock()
        .map(|c| c.clone())
        .unwrap_or_default()
}

/// The loop the app runs in the background. Reads the switch every pass, so
/// turning updates back on takes effect without a restart.
pub fn start(app: AppHandle) {
    if is_previous_copy() {
        note("background updater off - running as a kept-back copy");
        return;
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(FIRST_CHECK_SECS)).await;
        loop {
            let cfg = read_config(&app);
            let due = now_secs().saturating_sub(cfg.update_last_check) >= CHECK_INTERVAL_SECS;
            if cfg.update_auto && due {
                check(app.clone(), false).await;
            }
            tokio::time::sleep(Duration::from_secs(60 * 10)).await;
        }
    });
}

// ---------------------------------------------------------------- the swap

/// Move the staged build into place and start it. Returns the error string to
/// show; on success the caller must exit the process.
///
/// `startup` says the swap was decided at launch rather than by the button. It
/// only changes one thing - whether `--startup` is passed on to the new build -
/// and that matters, because a launch-time swap happens during a logon, which
/// is the situation the delay exists for.
///
/// Every failure path puts `WSight.exe` back. A user whose update failed still
/// has a working app - what they must never get is a missing exe.
pub fn apply(startup: bool) -> Result<(), String> {
    let exe = exe_path().ok_or("无法定位当前程序")?;
    let dir = exe
        .parent()
        .ok_or("无法定位程序目录")?
        .to_path_buf();

    let staged = staged_path();
    if !staged.exists() {
        return Err("没有已经下载好的更新".to_string());
    }

    // When the download landed in %APPDATA% (the exe folder was read-only) it
    // has to be copied in, because the final rename cannot cross volumes.
    let beside = dir.join(format!("WSight.exe{STAGED_SUFFIX}"));
    if !beside.exists() {
        std::fs::copy(&staged, &beside).map_err(|e| format!("无法把更新包放到程序目录: {e}"))?;
    }

    // The build we step aside to, and the one our successor must leave alone.
    // Clearing the name first keeps one copy even if a stale one is somehow
    // there under our own version number.
    let previous = previous_path(current_version());
    let _ = std::fs::remove_file(&previous);

    // 1. Step aside. Until this succeeds nothing has changed.
    std::fs::rename(&exe, &previous)
        .map_err(|e| format!("无法把当前程序改名（可能被占用）: {e}"))?;

    // 2. Take the new one's place. On failure, undo step 1.
    if let Err(e) = std::fs::rename(&beside, &exe) {
        let _ = std::fs::rename(&previous, &exe);
        return Err(format!("替换失败，已恢复原程序: {e}"));
    }

    // 3. Start it. `--handover` tells it to wait for our mutex instead of
    //    exiting as a duplicate; `--replaced` names the build it inherits the
    //    job of cleaning up after.
    let mut spawn = std::process::Command::new(&exe);
    spawn.arg("--handover").arg("--replaced").arg(&previous);
    if startup {
        spawn.arg("--startup");
    }

    if let Err(e) = spawn.spawn() {
        // Put everything back the way it was.
        let _ = std::fs::rename(&exe, &beside);
        let _ = std::fs::rename(&previous, &exe);
        return Err(format!("无法启动新版本，已恢复原程序: {e}"));
    }

    // The staged version is the running version now, so keeping the record
    // would have the next launch reporting a download that is not there.
    // Harmless - every reader checks the file too - but it is a lie that costs
    // nothing to avoid. Written last, once nothing above can still be undone.
    let mut cfg = config::load();
    if !cfg.update_staged_version.is_empty() {
        cfg.update_staged_version.clear();
        let _ = config::save(&cfg);
    }

    Ok(())
}

/// Delete older builds that earlier updates left behind - all but one.
///
/// The exception is `replaced`, the build this process just took over from, and
/// it is kept deliberately: if this build will not start, that file is the only
/// way back the user has, since there is no window left to put a button in.
/// Keeping every leftover would grow the folder by a whole build per release,
/// so the rule is one generation - this runs at every handover, so the copy
/// kept by the update before last is deleted by the update after next.
///
/// Only ever touches names this updater invents: `WSight-old-<version>.exe`,
/// plus the `WSight.exe.old-` form it used before the name had to be runnable.
/// A sweep can therefore never hit a file the user put in the folder.
pub fn clean_up(replaced: &str) {
    let Some(dir) = exe_path().and_then(|p| p.parent().map(Path::to_path_buf)) else {
        return;
    };
    let keep = PathBuf::from(replaced)
        .file_name()
        .map(|n| n.to_os_string());

    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let text = name.to_string_lossy();
        if !(is_previous_name(&text) || text.starts_with(LEGACY_OLD_PREFIX)) {
            continue;
        }
        if keep.as_deref() == Some(name.as_os_str()) {
            continue;
        }
        // Fails while an older process is still shutting down; the next launch
        // tries again, so this is not worth reporting.
        let _ = std::fs::remove_file(entry.path());
    }
}

// ------------------------------------------------------------------ tests
//
// Only the version comparison is unit-testable without a network or a second
// process. It is also the piece where a mistake is silent and expensive: get
// it wrong and the updater either stops offering releases or offers an older
// one, and both look like "no update available".
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_forms_github_produces() {
        assert_eq!(parse_version("0.5.0"), Some(vec![0, 5, 0]));
        assert_eq!(parse_version("v0.5.0"), Some(vec![0, 5, 0]));
        assert_eq!(parse_version(" v1.2.3 "), Some(vec![1, 2, 3]));
        assert_eq!(parse_version("0.5.0-beta.1"), Some(vec![0, 5, 0]));
        assert_eq!(parse_version("0.5.0+win"), Some(vec![0, 5, 0]));
        assert_eq!(parse_version("1"), Some(vec![1]));
    }

    #[test]
    fn refuses_a_tag_it_cannot_read() {
        // A moving tag or a date stamp is not a version, and guessing would
        // mean downloading something on a comparison nobody can explain.
        assert_eq!(parse_version("latest"), None);
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("nightly-2026-09-16"), None);
        assert_eq!(parse_version("v1.x.0"), None);
    }

    #[test]
    fn compares_numerically_not_as_text() {
        // The bug this guards: "0.10.0" < "0.9.0" as strings, so a string
        // compare would stop offering updates after the ninth patch.
        assert!(!is_newer("0.9.0", "0.10.0"));
        assert!(is_newer("0.10.0", "0.9.0"));
        assert!(is_newer("0.5.0", "0.4.5"));
        assert!(is_newer("1.0.0", "0.99.99"));
    }

    #[test]
    fn equal_and_shorter_versions_are_not_newer() {
        assert!(!is_newer("0.5.0", "0.5.0"));
        assert!(!is_newer("0.5", "0.5.0"));
        assert!(!is_newer("0.5.0.0", "0.5.0"));
        // A tag we cannot read is never an upgrade.
        assert!(!is_newer("latest", "0.5.0"));
        assert!(!is_newer("0.5.0", "latest"));
    }

    #[test]
    fn an_ignored_release_stays_ignored_but_only_its_own_version() {
        // The bug this guards: 忽略这个版本 used to delete the file and forget
        // the decision, so the next check offered the same release again.
        assert!(is_ignored("0.5.2", "0.5.2"));
        // A newer release supersedes the objection by itself - otherwise one
        // click would cut the user off from every future update.
        assert!(!is_ignored("0.5.3", "0.5.2"));
        assert!(!is_ignored("1.0.0", "0.5.2"));
        // Nothing ignored: nothing is skipped.
        assert!(!is_ignored("0.5.2", ""));
        // A release *older* than the ignored one was already superseded, and
        // re-offering it would be the same mistake in the other direction.
        assert!(is_ignored("0.5.1", "0.5.2"));
        // Unreadable on either side: never hide a release.
        assert!(!is_ignored("latest", "0.5.2"));
        assert!(!is_ignored("0.5.2", "latest"));
    }

    #[test]
    fn the_staged_name_is_beside_the_exe_and_never_the_exe_itself() {
        let staged = staged_path();
        assert!(staged.to_string_lossy().ends_with("WSight.exe.new"));
        assert_ne!(staged, exe_path().unwrap_or_default());
    }

    #[test]
    fn the_kept_back_copy_is_a_name_windows_will_run() {
        // The whole point of this file is that it can be double-clicked at the
        // one moment there is no window to offer a button from. `.exe` is not
        // decoration.
        assert_eq!(previous_name("0.5.0"), "WSight-old-0.5.0.exe");
        assert!(previous_name("0.5.0").ends_with(".exe"));
    }

    #[test]
    fn the_sweep_only_recognises_its_own_leftovers() {
        assert!(is_previous_name("WSight-old-0.5.0.exe"));
        assert!(is_previous_name("WSight-old-0.4.5.exe"));
        // Ours, from the naming this replaced - swept so it cannot pile up.
        assert!("WSight.exe.old-0.5.0".starts_with(LEGACY_OLD_PREFIX));
        // Not ours. A pattern that matched these would let an update delete a
        // file the user put in the folder.
        assert!(!is_previous_name("WSight.exe"));
        assert!(!is_previous_name("WSight.exe.new"));
        assert!(!is_previous_name("WSight-old-.exe"));
        assert!(!is_previous_name("WSight-old-0.5.0.exe.bak"));
        assert!(!is_previous_name("Backup-of-WSight.exe"));
    }

    #[test]
    fn hashes_the_way_github_writes_them() {
        // Known vector for "abc" - this pins our hex formatting to what the
        // GitHub API's `digest` field contains. A byte order or padding slip
        // here would reject every genuine download.
        let mut hasher = Sha256::new();
        hasher.update(b"abc");
        assert_eq!(
            hex(&hasher.finalize()),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn normalises_the_digest_field() {
        assert_eq!(normalize_digest("sha256:ABC123"), "abc123");
        assert_eq!(normalize_digest(" abc123 "), "abc123");
        // The real shape of the field: v0.4.5's `WSight.exe` asset, uppercase
        // and prefixed, exactly as GitHub reports it.
        let live = normalize_digest(
            "sha256:6E87F4FAB96E8C480AFE00A471B4A97030502164B90408EE7EBBECDE8F115A00",
        );
        assert_eq!(
            live,
            "6e87f4fab96e8c480afe00a471b4a97030502164b90408ee7ebbecde8f115a00"
        );
        assert_eq!(live.len(), 64);
    }
}
