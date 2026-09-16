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
//! 1. `WSight.exe`      -> `WSight.exe.old-0.4.5`
//! 2. `WSight.exe.new`  -> `WSight.exe`
//! 3. launch the new `WSight.exe`, then exit
//! 4. the new process deletes `WSight.exe.old-*`
//!
//! The path never changes, so the auto-start entry, any shortcut and the update
//! itself all keep talking about the same file.
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
const OLD_PREFIX: &str = ".old-";

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
    touch_config(app, |cfg| cfg.update_staged_version.clear());
    with_status(|s| {
        s.state = "idle".to_string();
        s.latest.clear();
        s.notes.clear();
        s.staged.clear();
        s.progress = 0.0;
        s.verified = None;
        s.error.clear();
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
/// Every failure path puts `WSight.exe` back. A user whose update failed still
/// has a working app - what they must never get is a missing exe.
pub fn apply() -> Result<(), String> {
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

    let previous = dir.join(format!("WSight.exe{OLD_PREFIX}{}", current_version()));
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
    //    exiting as a duplicate; `--replaced` names the file it should delete.
    let spawned = std::process::Command::new(&exe)
        .arg("--handover")
        .arg("--replaced")
        .arg(&previous)
        .spawn();

    if let Err(e) = spawned {
        // Put everything back the way it was.
        let _ = std::fs::rename(&exe, &beside);
        let _ = std::fs::rename(&previous, &exe);
        return Err(format!("无法启动新版本，已恢复原程序: {e}"));
    }

    Ok(())
}

/// Delete the build this process replaced, plus any leftovers from earlier
/// updates that never got the chance.
///
/// Only ever touches `WSight.exe.old-*` beside our own exe - names this updater
/// invents and nothing else produces, so a sweep cannot hit a file the user
/// put there. (A broader `*.exe` sweep could, which is why it is not done.)
pub fn clean_up(replaced: &str) {
    if !replaced.is_empty() {
        let path = PathBuf::from(replaced);
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
    }
    let Some(dir) = exe_path().and_then(|p| p.parent().map(Path::to_path_buf)) else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("WSight.exe.old-") {
            // Fails while an older process is still shutting down; the next
            // launch tries again, so this is not worth reporting.
            let _ = std::fs::remove_file(entry.path());
        }
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
    fn the_staged_name_is_beside_the_exe_and_never_the_exe_itself() {
        let staged = staged_path();
        assert!(staged.to_string_lossy().ends_with("WSight.exe.new"));
        assert_ne!(staged, exe_path().unwrap_or_default());
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
