mod autostart;
mod collector;
mod config;
#[cfg(windows)]
mod desktop;
mod launchlog;
mod monitorcache;
mod native;
mod single_instance;
mod updater;
mod weather;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};

use collector::{Collector, Snapshot};
use config::AppConfig;
use monitorcache::MonitorCache;
use weather::{GeoCity, WeatherCache, WeatherPayload};

pub const SNAPSHOT_EVENT: &str = "monitor://snapshot";
pub const WEATHER_EVENT: &str = "weather://payload";
pub const WEATHER_ERROR_EVENT: &str = "weather://error";
pub const CONFIG_EVENT: &str = "config://changed";

pub struct AppState {
    pub config: Mutex<AppConfig>,
    pub snapshot: Mutex<Option<Snapshot>>,
}

/// Append one line to `%APPDATA%\WSight\launch.log`.
///
/// Called from `main` before Tauri exists, because the facts worth recording
/// (is this the auto-start launch, is another copy already running) are all
/// decided before - or instead of - the app booting.
pub fn log_launch(line: &str) {
    launchlog::log(line);
}

/// False when another WSight is already running; `main` then exits quietly
/// instead of opening a second, dead window.
pub fn claim_instance() -> bool {
    single_instance::claim()
}

/// Wait for the build the updater just replaced to finish exiting, then take
/// the single-instance mutex. False on timeout; `main` then exits quietly,
/// because starting a second copy beside a live one is the failure this whole
/// mechanism exists to prevent.
pub fn await_instance() -> bool {
    single_instance::wait_for_release(Duration::from_secs(updater::HANDOVER_WAIT_SECS))
}

/// Delete the builds earlier updates stepped aside, keeping the most recent.
pub fn clean_up_replaced(replaced: &str) {
    updater::clean_up(replaced);
}

/// Install an update that is already downloaded, before any window exists.
///
/// Returns true when the swap happened and this process should end, leaving the
/// new build to take over. This is the path that makes updates actually reach
/// people: the settings window has a 重启更新 button for a user who wants the
/// new build now, but a desktop widget's settings window is opened roughly
/// never, so on its own that button leaves a verified update sitting on disk
/// forever.
///
/// `startup` is forwarded to the new build so a logon-time swap keeps the logon
/// delay - which is the moment this runs most often.
pub fn auto_apply_staged(startup: bool) -> bool {
    if !updater::staged_ready() {
        return false;
    }
    if updater::is_previous_copy() {
        // A kept-back copy must not install anything - see `updater::check`.
        log_launch("auto-apply: kept-back copy - skipped");
        return false;
    }
    match updater::apply(startup) {
        Ok(()) => {
            log_launch("auto-apply: staged update installed, handing over");
            true
        }
        Err(message) => {
            // Falling through is right: the old build still works, the staged
            // file is still there, and the user can retry from the button.
            log_launch(&format!("auto-apply: failed - {message}"));
            false
        }
    }
}

// ---------------------------------------------------------------- helpers

fn read_config(app: &AppHandle) -> AppConfig {
    app.state::<AppState>()
        .config
        .lock()
        .map(|c| c.clone())
        .unwrap_or_default()
}

// ------------------------------------------------------------- window size

/// Inner size of a window in *logical* px - the same unit the webview reports
/// through `window.innerWidth` / `innerHeight`, so the two sides can compare
/// numbers without worrying about DPI.
fn logical_size(win: &tauri::WebviewWindow) -> Option<(f64, f64)> {
    let scale = win.scale_factor().unwrap_or(1.0);
    let size = win.inner_size().ok()?;
    Some((size.width as f64 / scale, size.height as f64 / scale))
}

fn widget_width(cfg: &AppConfig, label: &str) -> f64 {
    match label {
        "monitor" => cfg.monitor_width,
        _ => cfg.weather_width,
    }
}

/// A reported width within this many px of the stored one is treated as
/// logical<->physical rounding noise rather than a deliberate resize.
const WIDTH_NOISE_PX: f64 = 12.0;

/// Gap between the screen edge and a widget that has no remembered position.
const CORNER_MARGIN: f64 = 40.0;

// ------------------------------------------------------------------ snapping
//
// Two widgets that live on the same desktop are almost always meant to be
// aligned: stacked in a column, side by side, or sharing an axis so they read
// as one column of information. Doing that by hand is a pixel hunt, so a
// widget being moved is drawn to its neighbour's edges the way a window
// manager snaps to a screen edge.
//
// Five lines per axis, which covers everything the eye reads as "aligned":
//   - the two near edges (top/left with top/left, bottom/right with
//     bottom/right),
//   - the two far edges (this widget's top against its neighbour's bottom, and
//     the mirror image), which is "touching, flush, no gap",
//   - the centres, for a stack of two different-sized cards on one axis.
//
// Only while 调整 is on: at rest the widgets are locked, so nothing can be
// dragged into a position that would need snapping anyway.

/// How close an edge has to be, in logical px, before it is drawn in.
///
/// 8 sits at the light end of what published implementations use - the HarmonyOS
/// PC window-API walkthrough recommends 8-15px and ships 10, a Win32
/// magnetic-window sample uses 8, and Windows' own `SnapAssistDistance` defaults
/// to 16px (documented range 4-16). We are at the low end on purpose: those
/// systems can afford a wide field because the window there keeps following the
/// pointer and only the *preview* snaps, while here the window itself is pulled
/// onto the line, and every pixel of field is a pixel the widget does not move
/// under the pointer.
const SNAP_PX: f64 = 8.0;

/// Once a line has hold of the widget, the pointer has to travel this far from
/// it before the widget lets go - 1.5x the distance it took to catch.
///
/// This is the "排斥区" the same implementations insist on: with a single
/// threshold, a pointer resting on the edge of the field makes the widget
/// flicker between the line and the pointer. It only has to out-size the
/// wobble of a slow drag, though - not a deliberate pull - so it is kept near
/// the catching distance.
const SNAP_ESCAPE_PX: f64 = 12.0;

/// Guards the move we make *because* of a snap from being read as the user
/// moving the window again.
static SNAPPING: AtomicBool = AtomicBool::new(false);

/// What the magnet needs to remember between one call and the next.
///
/// Nothing here is bookkeeping for its own sake - the window move loop makes
/// it necessary. A `Moved` only says where the *window* went, and while a line
/// is holding the widget the window goes nowhere: the drag loop keeps asking
/// for the pointer's position, we keep putting the window back on the line.
/// Measuring the pointer's travel off the window therefore either freezes the
/// total (single-threshold magnets: "磁吸太紧，拖不动") or double-counts it.
/// So the running total is advanced by the *pointer's* own delta, and the
/// window position only serves as the value we correct.
#[derive(Clone, Copy, Default)]
struct SnapTrack {
    /// Where the widget would be if there were no magnet at all.
    free: Option<(f64, f64)>,
    /// Pointer position (logical px) at the previous call, so the next one can
    /// add a true pointer delta.
    cursor: Option<(f64, f64)>,
    /// Where the previous call left the window - the position this code asked
    /// for. Anything else showing up here next time came from outside.
    win: Option<(f64, f64)>,
    /// The line each axis is currently held by, in logical px.
    held: (Option<f64>, Option<f64>),
}

static SNAP: Mutex<SnapTrack> = Mutex::new(SnapTrack {
    free: None,
    cursor: None,
    win: None,
    held: (None, None),
});

/// Until this moment (ms since the epoch) a `Moved` event is not a drag.
///
/// The arrow-key nudge moves the widget on purpose, one pixel at a time; if the
/// magnet were listening it would swallow every step. A short quiet window
/// rather than a one-shot flag: a nudge can report more than one `Moved`, the
/// event may arrive on either side of the command returning, and holding the
/// key down repeats it every ~30ms - all three are covered by "no snapping for
/// a moment after the last nudge".
static NUDGE_UNTIL_MS: AtomicU64 = AtomicU64::new(0);
const NUDGE_QUIET_MS: u64 = 250;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Forget where the magnet thought the widget and the pointer were.
///
/// For the moments the widget is moved by something other than a drag - a
/// nudge, a reset, entering or leaving 调整 - where the next `Moved` is a
/// fresh start rather than a delta on top of an older one.
fn reset_snap_track() {
    if let Ok(mut track) = SNAP.lock() {
        *track = SnapTrack::default();
    }
}

/// A window's rectangle in logical px: `(x, y, w, h)`.
fn widget_rect(win: &tauri::WebviewWindow) -> Option<(f64, f64, f64, f64)> {
    let scale = win.scale_factor().ok()?;
    let pos = win.outer_position().ok()?;
    let size = win.outer_size().ok()?;
    Some((
        pos.x as f64 / scale,
        pos.y as f64 / scale,
        size.width as f64 / scale,
        size.height as f64 / scale,
    ))
}

/// Snap one axis: return `(coordinate, line it is held by)`.
///
/// `held` is the line this axis was on last time. A line that already has hold
/// keeps it until the pointer pulls past `SNAP_ESCAPE_PX`; only then is the
/// field consulted again from scratch. Deciding afresh on every event - which
/// is what a single threshold amounts to - is what makes a magnet feel stuck.
///
/// `lines` are the coordinates this axis may lock onto, in order of increasing
/// preference for ties: screen edges first, the neighbour's alignment lines
/// last (`<=` lets a later, equally close line win).
fn snap_axis(cur: f64, lines: &[f64], held: Option<f64>) -> (f64, Option<f64>) {
    if let Some(line) = held {
        if (line - cur).abs() <= SNAP_ESCAPE_PX {
            return (line, Some(line));
        }
    }

    let mut best = cur;
    let mut best_gap = SNAP_PX;
    let mut lock = None;
    for &candidate in lines {
        let gap = (candidate - cur).abs();
        if gap <= best_gap {
            best_gap = gap;
            best = candidate;
            lock = Some(candidate);
        }
    }
    (best, lock)
}

/// Pull a widget that is being moved onto its neighbour's alignment lines.
///
/// Called from the window event hook for every `Moved`. The guard matters:
/// snapping moves the window, which emits another `Moved`, and without the
/// flag that second event would run the whole calculation again from inside
/// the first - harmless in effect, but a recursive call for no reason.
///
/// The position reported here is *not* where the pointer is - see `SnapTrack` -
/// so the magnet decides on the tracked free position instead, and the reported
/// one only serves to advance it.
fn snap_widget(window: &tauri::Window) {
    // 自己转圈的拖动会直接调 `snap_apply`；这里再算一次就是两边抢同一个
    // 窗口位置（实测拖到一半卡住、或者冲出指针一大截）。
    if DRAG_ACTIVE.load(Ordering::Relaxed) {
        return;
    }
    snap_apply(window.app_handle(), window.label(), false);
}

/// `dragging` = 调用方是那条自绘拖动循环。
///
/// 只有一处区别：拖动**不吃**方向键的安静窗口。安静窗口是给"用户在用方向键
/// 瞄一个像素"准备的，而按住标题栏拖动期间它就是一段凭空的卡顿。
fn snap_apply(app_handle: &tauri::AppHandle, label: &str, dragging: bool) {
    if !is_adjusting(label) {
        return;
    }
    // Arrow keys are the user aiming at a pixel; the magnet stays out of it.
    if !dragging && now_ms() < NUDGE_UNTIL_MS.load(Ordering::SeqCst) {
        return;
    }
    if SNAPPING.swap(true, Ordering::SeqCst) {
        return;
    }

    let app = app_handle;
    let neighbour_label = if label == "monitor" { "weather" } else { "monitor" };
    if let Some(moving) = app.get_webview_window(label) {
        if let Some((x, y, w, h)) = widget_rect(&moving) {
            // 对齐线：**屏幕工作区边缘**（靠边吸附）+ **邻居卡片**的五种对齐。
            //
            // 工作区而不是整块屏幕：下边缘要让开任务栏，否则卡片会钻到任务栏
            // 底下，用户再也点不到它。屏幕线放前面、邻居线放后面 —— `snap_axis`
            // 里是 `<=`，平手时后面的赢，即"离哪条线一样近时优先对齐邻居"。
            let mut xs: Vec<f64> = Vec::new();
            let mut ys: Vec<f64> = Vec::new();
            if let Ok(Some(mon)) = moving.current_monitor() {
                let scale = mon.scale_factor() as f64;
                let wa = mon.work_area();
                let left = wa.position.x as f64 / scale;
                let top = wa.position.y as f64 / scale;
                let right = (wa.position.x as f64 + wa.size.width as f64) / scale;
                let bottom = (wa.position.y as f64 + wa.size.height as f64) / scale;
                xs.push(left);
                xs.push(right - w);
                ys.push(top);
                ys.push(bottom - h);
            }
            // A hidden neighbour is not on screen to align with - and its stored
            // rectangle would be the last place it was seen, which is worse than
            // no neighbour lines at all. The screen lines stay either way.
            if let Some(other) = app.get_webview_window(neighbour_label) {
                if other.is_visible().unwrap_or(false) {
                    if let Some((ox, oy, ow, oh)) = widget_rect(&other) {
                        xs.extend([ox, ox + ow - w, ox + ow, ox - w, ox + (ow - w) / 2.0]);
                        ys.extend([oy, oy + oh - h, oy + oh, oy - h, oy + (oh - h) / 2.0]);
                    }
                }
            }
            if !xs.is_empty() {
                // 指针此刻的位置（逻辑 px）。自由位置用它来累加，而不是用
                // 窗口位移 —— 窗口被吸附按住时位移是 0 或跳变，拿它积分一定失真。
                let scale = moving.scale_factor().unwrap_or(1.0);
                let cursor = moving.cursor_position().ok().map(|p| {
                    (p.x as f64 / scale as f64, p.y as f64 / scale as f64)
                });
                let (free, held_x, held_y) = SNAP
                    .lock()
                    .map(|track| {
                        let pointer_moved = match (track.cursor, cursor) {
                            (Some(prev), Some(cur)) => {
                                cur.0 != prev.0 || cur.1 != prev.1
                            }
                            _ => false,
                        };
                        // 窗口没待在我们上次放的地方 = 有别人动过它。
                        // 容差要容得下逻辑 ↔ 物理的取整（半个物理 px）。
                        let window_moved = match track.win {
                            Some((wx, wy)) => {
                                (wx - x).abs() > 1.5 || (wy - y).abs() > 1.5
                            }
                            None => false,
                        };
                        match (track.free, track.cursor, cursor) {
                            // 指针动过 = 真的在拖。按**指针位移**累加，因为窗口
                            // 被吸附线按住时它的位移是 0 或跳变，拿它积分一定失真。
                            (Some(free), Some(prev), Some(cur)) if pointer_moved => {
                                (
                                    (free.0 + (cur.0 - prev.0), free.1 + (cur.1 - prev.1)),
                                    track.held.0,
                                    track.held.1,
                                )
                            }
                            // 指针一寸没动、窗口却换了地方 —— 是**别的东西**挪的：
                            // 方向键微调、进出「调整」、换屏还原、外部 SetWindowPos。
                            // 自由位置改从窗口现在的位置起算，否则会拿上一次拖动
                            // 留下的老位置把窗口**拽回原地**（摆到 A 点自己弹回 B 点）。
                            // 吸附锁定**先留着**：`snap_axis` 自己会在超出 12 逻辑 px
                            // 时松手，而"吸住之后再拨开 10px 仍要吸住"正是那个排斥区。
                            // 拖动期间**不看窗口读数**：`set_position` 是异步的，
                            // 下一帧读到的往往是上一跳的位置，看起来就像"别人动过"。
                            // 拿它重置 free，等于把刚攒起来的指针行程扔掉（实测
                            // 窗口读数会稳定落后自由位置约一跳），偶发就出在这里。
                            // 拖动时自由位置只由指针说了算。
                            (Some(_), _, _) if window_moved && !dragging => {
                                ((x, y), track.held.0, track.held.1)
                            }
                            (Some(free), _, _) => (free, track.held.0, track.held.1),
                            _ => ((x, y), None, None),
                        }
                    })
                    .unwrap_or(((x, y), None, None));

                let (nx, lock_x) = snap_axis(free.0, &xs, held_x);
                let (ny, lock_y) = snap_axis(free.1, &ys, held_y);
                let moved = (nx - x).abs() >= 0.5 || (ny - y).abs() >= 0.5;
                if moved {
                    let _ = moving.set_position(tauri::LogicalPosition::new(nx, ny));
                }

                if let Ok(mut track) = SNAP.lock() {
                    track.free = Some(free);
                    track.cursor = cursor;
                    // 记"我们让它待的地方"，下次好认出外人插的手。没动就是原地。
                    track.win = Some(if moved { (nx, ny) } else { (x, y) });
                    track.held = (lock_x, lock_y);
                }
            }
        }
    }

    SNAPPING.store(false, Ordering::SeqCst);
}

// --------------------------------------------------------------- z-order
//
// Tauri can float a window above everything (`set_always_on_top`) but has no
// way to say the opposite, which is where a desktop widget actually wants to
// live: at the very bottom of the stack, behind every ordinary window.
// `SetWindowPos(HWND_BOTTOM)` is that missing call, and it lives in `desktop`
// together with the two other Win32-only traits a real desktop widget needs
// (tool-window style, no-activate). See `desktop.rs`.

/// Whether a widget is currently in "调整" mode. Tracked natively because the
/// settings window autosaves in the background, and a save must not drag a
/// widget the user is in the middle of positioning back down to the desktop.
static ADJUSTING_MONITOR: AtomicBool = AtomicBool::new(false);
static ADJUSTING_WEATHER: AtomicBool = AtomicBool::new(false);

/// Whether the user wants each widget on screen. Mirrors the config, because
/// the guard thread below must not read the config file every half second, and
/// must never disagree with a save that is still in flight.
static WANT_VISIBLE_MONITOR: AtomicBool = AtomicBool::new(false);
static WANT_VISIBLE_WEATHER: AtomicBool = AtomicBool::new(false);

/// 置顶 state, for the same reason: the guard has to know whether a widget is
/// allowed to sit above the desktop or belongs under every window.
static ON_TOP_MONITOR: AtomicBool = AtomicBool::new(false);
static ON_TOP_WEATHER: AtomicBool = AtomicBool::new(false);

/// "暂时置顶"：显示桌面期间卡片跟着桌面走（贴在桌面正上方），桌面收起后
/// 必须跟着落回。注意它**不是** `WS_EX_TOPMOST`——置顶层高于一切，最小化动画
/// 还没走完的窗口会被卡片压住；贴桌面正上方则任何可见窗口都必然在卡片之上。
static TEMP_TOP_MONITOR: AtomicBool = AtomicBool::new(false);
static TEMP_TOP_WEATHER: AtomicBool = AtomicBool::new(false);

/// 前端最近一次要求的高度（逻辑 px 的 f64 位模式，0 = 还没报过）。
///
/// 窗口高度归内容管：前端量出自然高度就发过来。可这是一条**没有回执**的
/// 命令 —— 原生拖拽循环正在跑的时候，`set_size` 会被它当场覆盖掉，而覆盖
/// 之后不一定还有新的 resize 事件来触发下一次上报。结果就是窗口停在用户
/// 拖到的高度上、内容被裁掉一截，而且再也不会自己长回来（"有时候高度不
/// 自动调整，显示不全"）。所以把要求值记下来，由守护线程对账。
static FIT_H_MONITOR: AtomicU64 = AtomicU64::new(0);
static FIT_H_WEATHER: AtomicU64 = AtomicU64::new(0);

/// 对账踩刹车用：(上次试过的目标高度, 时刻, 那一次看到的实际高度)。
///
/// 刹车只在**没有进展**的时候踩：实际高度和上次看到的一样 → 说明这一下多半
/// 白撞了（原生拖拽把 `set_size` 吞了，或者系统把窗口钉住了），歇半秒再说；
/// 高度变了 → 用户还在拖，值得立刻再对一次。
///
/// 半秒，不是两秒：拖拽刚结束的那一刻，"上一次尝试被吞掉"和"用户松手了、高度
/// 当然不变"长得一模一样，判据分不开。两秒的刹车会让收尾那次对账整整晚两秒，
/// 落在用户眼里就是"有时候高度不自动调整"（实测见 `verify_fit_height` 第 2/4 轮）。
static FIT_TRY_MONITOR: [AtomicU64; 3] =
    [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];
static FIT_TRY_WEATHER: [AtomicU64; 3] =
    [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];

/// Windows-only counterpart of the block above: there is no portable version of
/// "a tool window that ignores activation and still shows after 显示桌面", so
/// every other platform gets no-ops.
#[cfg(not(windows))]
mod desktop {
    use tauri::WebviewWindow;
    pub fn mark_as_tool(_win: &WebviewWindow) {}
    pub fn set_no_activate(_win: &WebviewWindow, _on: bool) {}
    pub fn hidden_by_shell(_win: &WebviewWindow) -> bool {
        false
    }
    pub fn is_minimized(_win: &WebviewWindow) -> bool {
        false
    }
    pub fn restore_window(_win: &WebviewWindow) {}
    pub fn covered_by_desktop(_win: &WebviewWindow) -> bool {
        false
    }
    pub fn set_topmost(_win: &WebviewWindow) {}
    pub fn is_topmost(_win: &WebviewWindow) -> bool {
        false
    }
    pub fn stick_to_desktop(_win: &WebviewWindow, _ignore: &[isize]) -> bool {
        false
    }
    pub fn stick_hwnd(_hwnd: isize, _ignore: &[isize]) -> bool {
        false
    }
    pub fn desktop_hwnd() -> Option<isize> {
        None
    }
    pub fn desktop_above(_hwnd: isize, _desktop: isize) -> bool {
        false
    }
    pub fn sits_on_desktop_hwnd(_hwnd: isize, _desktop: isize, _ignore: &[isize]) -> bool {
        false
    }
    pub fn desktop_on_top(_ignore: &[isize]) -> bool {
        false
    }
    pub fn desktop_showing(_ignore: &[isize]) -> bool {
        false
    }
    pub fn foreground_class() -> String {
        String::new()
    }
    pub fn to_bottom(_win: &WebviewWindow) {}
    pub fn left_button_down() -> bool {
        false
    }
    pub fn watch_foreground(_sink: fn(bool)) {}
    pub fn watch_click_outside(_sink: fn(isize)) {}
}

fn adjusting_flag(label: &str) -> &'static AtomicBool {
    match label {
        "monitor" => &ADJUSTING_MONITOR,
        _ => &ADJUSTING_WEATHER,
    }
}

fn is_adjusting(label: &str) -> bool {
    adjusting_flag(label).load(Ordering::Relaxed)
}

fn widget_always_on_top(cfg: &AppConfig, label: &str) -> bool {
    match label {
        "monitor" => cfg.monitor_always_on_top,
        _ => cfg.weather_always_on_top,
    }
}

fn on_top_flag(label: &str) -> &'static AtomicBool {
    match label {
        "monitor" => &ON_TOP_MONITOR,
        _ => &ON_TOP_WEATHER,
    }
}

fn wants_visible(label: &str) -> &'static AtomicBool {
    match label {
        "monitor" => &WANT_VISIBLE_MONITOR,
        _ => &WANT_VISIBLE_WEATHER,
    }
}

fn temp_top_flag(label: &str) -> &'static AtomicBool {
    match label {
        "monitor" => &TEMP_TOP_MONITOR,
        _ => &TEMP_TOP_WEATHER,
    }
}

/// Put a widget at the level its 置顶 setting asks for: floating above
/// everything, or tucked under every ordinary window.
///
/// Also re-stamps the furniture traits after the level change, because tao
/// rewrites the whole style block behind every flag change: `set_resizable`
/// (调整 mode) silently drops `WS_EX_TOOLWINDOW`, which is how a widget ends up
/// back in the taskbar after the user has adjusted it once.
///
/// The level itself is applied with raw `SetWindowPos` rather than
/// `set_always_on_top`: tao invalidates the whole window after changing that
/// flag, i.e. a full WebView2 repaint the user sees as a blink.
fn apply_widget_level(win: &tauri::WebviewWindow, on_top: bool) {
    let label = win.label().to_string();
    if label == "monitor" || label == "weather" {
        on_top_flag(&label).store(on_top, Ordering::Relaxed);
        // 用户（或配置）已经明确指定层级了，临时置顶那一页翻过去。
        temp_top_flag(&label).store(false, Ordering::Relaxed);
    }
    desktop::mark_as_tool(win);
    // 调整 mode needs the keyboard, so activation is allowed only there.
    desktop::set_no_activate(win, !is_adjusting(&label));
    if on_top {
        desktop::set_topmost(win);
    } else {
        // 落点是"紧贴桌面上方"，不是"全栈最底"：桌面自己的 z 序会变（Win+D
        // 时被抬到最上面），压到最底的窗口会在它回落的那几帧里被盖住。
        place_widget_level(win);
    }
}

/// 把一张卡片贴回桌面层，顺手把另一张卡片从插入候选里排掉。
///
/// "桌面层"就是"桌面窗口正上方"：平时 = 窗口栈最底（所有普通窗口之下）；
/// 桌面被 Win+D 抬起来时卡片跟着上去（`stick_to_desktop`），所以这一下同时
/// 是"抬升"和"落回"。两张卡片的句柄互相当排除表：`sits_on_desktop` 把同样
/// 贴桌面的兄弟当邻居跳过，`stick_to_desktop` 插入时也不会拿兄弟当锚点 ——
/// 否则它们会把彼此当成"挡在桌面之上的那个窗口"来回搬运，每次搬运都是一次
/// 重合成，连着几十次就是肉眼可见的持续轻闪。
fn place_widget_level(win: &tauri::WebviewWindow) -> bool {
    let mine = widget_hwnds(win.app_handle());
    desktop::stick_to_desktop(win, &mine)
}

/// Apply the remembered 置顶 choice and the locked-by-default state.
///
/// Widgets start locked: `resizable(false)` plus no drag region means a stray
/// click or drag on the desktop cannot move or reshape them. 调整 in the
/// right-click menu is the only way in.
fn apply_widget_behaviour(app: &AppHandle, cfg: &AppConfig) {
    for label in ["monitor", "weather"] {
        let Some(win) = app.get_webview_window(label) else {
            continue;
        };
        let _ = win.set_resizable(false);
        apply_widget_level(&win, widget_always_on_top(cfg, label));
    }
}

/// 进程级 `AppHandle`，给 C ABI 的事件回调用（那条签名带不了闭包）。
static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();

/// z 序调整日志的节流：同一张卡片 2 秒内只写一条。
///
/// 这条日志只在"真挪了"时触发，正常情况下切换一次就一条。可一旦哪天又出现
/// "清不掉"的现场，250ms 一轮会把 launch.log 刷满，反而看不到更早的线索。
fn z_log_due(label: &str) -> bool {
    static LAST: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
    let slot = if label == "monitor" { 0 } else { 1 };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let last = LAST[slot].load(Ordering::Relaxed);
    if now.saturating_sub(last) < 2000 {
        return false;
    }
    LAST[slot].store(now, Ordering::Relaxed);
    true
}

/// 把一张卡片贴到「桌面正上方」—— 跟随桌面，而不是压到一切之上。
///
/// 只在"桌面真的占上来"时调用（调用方见 `on_desktop_foreground`）。
/// 全程只用原生 `SetWindowPos`，不碰 tao 的 `set_always_on_top`：后者切完 z 序
/// 会 `InvalidateRgn` 让整窗重绘，`Win+D` 两头都会因此闪一帧。
fn stick_for_desktop(win: &tauri::WebviewWindow) {
    desktop::mark_as_tool(win);
    desktop::set_no_activate(win, true);
    // `stick_to_desktop` 对"已经坐在桌面上"的卡片是幂等 no-op，所以连点两下
    // 桌面、守护线程每 250ms 兜底都不会多出一次重合成。真挪了才记日志。
    if place_widget_level(win) && z_log_due(win.label()) {
        launchlog::log(&format!(
            "z: {} 贴到桌面正上方（前台={}）",
            win.label(),
            desktop::foreground_class(),
        ));
    }
}

/// 桌面收起，让窗口跟着桌面落回桌面层。
///
/// 落点不是"全栈最底"，更不是"插一次就完事"：恢复动画里**桌面自己**也在移动
/// （实测 z 序从栈顶附近滑到最底，约 110ms），所以任何单次插入都会把卡片留在
/// 半空——要么被桌面盖住（消失一帧），要么压在恢复中的程序上面。
///
/// 这里用一个 ~260ms 的短跟踪：每 12ms 重新贴到"桌面正上方"。桌面移动时真的
/// `SetWindowPos`，不动时 `stick_to_desktop` 认出来是同一位置直接返回，
/// 所以既跟得上动画，也不会制造多余的重合成。
fn drop_temp_topmost(win: tauri::WebviewWindow) {
    std::thread::spawn(move || {
        for _ in 0..22 {
            // 桌面又回到前台了（`Win+D` 连按、桌面动画回弹、钩子给了两拍相反的
            // 前台事件）：这一步**必须放手**。落回会把卡片重新插到（可能又在
            // 升高的）桌面正上方，倒也无害；但按旧剧本"桌面在前台"时卡片该
            // 保持跟着桌面，这里续上账本再由抬高路径接管。
            let mine = widget_hwnds(win.app_handle());
            if desktop::desktop_showing(&mine) {
                temp_top_flag(win.label()).store(true, Ordering::Relaxed);
                return;
            }
            place_widget_level(&win);
            std::thread::sleep(Duration::from_millis(12));
        }
    });
}

/// 两个悬浮窗的顶层句柄。
///
/// z 序判定要拿它当排除表：卡片自己被抬高之后会成为栈里的一员，不排掉的话
/// "第一个普通窗口是谁"永远答错。
fn widget_hwnds(app: &AppHandle) -> Vec<isize> {
    let mut mine = Vec::new();
    for label in ["monitor", "weather"] {
        if let Some(win) = app.get_webview_window(label) {
            if let Ok(hwnd) = win.hwnd() {
                mine.push(hwnd.0 as isize);
            }
        }
    }
    mine
}

/// 鼠标按下的通知事件：payload 是"点中的那个悬浮窗的 label"，点别处是空串。
const CLICK_OUTSIDE_EVENT: &str = "widget-click-outside";

/// 用户在任何地方按下鼠标 —— 前端据此收起右击菜单。
///
/// 为什么要绕一圈到原生层：菜单收起原本靠窗口失焦（`blur` / `onFocusChanged`），
/// 而悬浮窗带 `WS_EX_NOACTIVATE`，点桌面、点任务栏、点别的程序都不会让它失焦，
/// 于是菜单一直挂在屏幕上。`WH_MOUSE_LL` 是唯一能看到"所有点击"的地方。
///
/// 事件带着点中窗口的 label：点在自己身上（菜单项、卡片内）时前端不该关 ——
/// 那一下由前端自己的 `onMouseDown` / 按钮负责。
fn on_widget_click_outside(root: isize) {
    let Some(app) = APP_HANDLE.get() else {
        return;
    };
    let hit = ["monitor", "weather"].into_iter().find(|label| {
        app.get_webview_window(label)
            .and_then(|win| win.hwnd().ok())
            .map(|h| h.0 as isize)
            == Some(root)
    });
    let _ = app.emit(CLICK_OUTSIDE_EVENT, hit.unwrap_or(""));
}

/// 把一张卡片临时贴到桌面正上方（桌面正被抬起来）。调用方必须先确认"桌面
/// 真的盖住了它"，见 `on_desktop_foreground`。
fn raise_for_desktop(app: &AppHandle, label: &str) {
    if !wants_visible(label).load(Ordering::Relaxed) || is_adjusting(label) {
        return;
    }
    // 用户自己在菜单里选了「置顶」，层级归他管，这里别去翻。
    if on_top_flag(label).load(Ordering::Relaxed) {
        return;
    }
    if let Some(win) = app.get_webview_window(label) {
        stick_for_desktop(&win);
        temp_top_flag(label).store(true, Ordering::Relaxed);
    }
}

/// 桌面被抬到最前 / 又收起的即时反应。
///
/// 这是 `Win+D` 不闪的关键：轮询最快也要等一个间隔，中间那几帧卡片是被桌面
/// 盖住的；而前台事件在桌面被抬起的同一刻就到，遮挡和贴位几乎同时发生。
///
/// 触发条件不能只看"前台窗口是桌面"——**点一下桌面就算**，哪怕卡片在另一块
/// 屏上正被一个最大化的程序盖着。那种时候把卡片贴到桌面正上方没问题（桌面
/// 没占上栈顶，贴了也等于贴在栈底），但为省一次无谓的重排，仍先确认桌面
/// **真的在栈顶、或者真的盖住了这张卡片**，两个判据任一成立才动。
fn on_desktop_foreground(desktop_in_front: bool) {
    let Some(app) = APP_HANDLE.get() else {
        return;
    };

    if !desktop_in_front {
        for label in ["monitor", "weather"] {
            if temp_top_flag(label).swap(false, Ordering::Relaxed) {
                if let Some(win) = app.get_webview_window(label) {
                    drop_temp_topmost(win);
                }
            }
        }
        return;
    }

    let mine = widget_hwnds(app);
    let desktop_up = desktop::desktop_on_top(&mine);
    let mut undecided = false;

    for label in ["monitor", "weather"] {
        let covers = desktop_up
            || app
                .get_webview_window(label)
                .map(|win| desktop::covered_by_desktop(&win))
                .unwrap_or(false);
        if covers {
            raise_for_desktop(app, label);
        } else if wants_visible(label).load(Ordering::Relaxed) && !is_adjusting(label) {
            undecided = true;
        }
    }

    if undecided {
        // Win+D 的时序是「先抬桌面、再切前台」，钩子几乎总赶在前半步：实测
        // （`.verify/diag_wind4_timeline.py`）桌面在事件之后约 6ms 才升到普通窗口
        // 栈顶。所以这里**不能**固定等一个时长再复检 —— 等 45ms 就是让卡片被
        // 桌面实实在在地盖住 40ms（两三帧），用户看到的就是"闪一下"。
        // 改成 2ms 一探：桌面一上来同一帧就抬手，遮挡压到一帧以内；如果用户只是
        // 点了下桌面（不是显示桌面），桌面永远不上升，这一轮什么都不做。
        let app = app.clone();
        std::thread::spawn(move || {
            for _ in 0..30 {
                if desktop_is_up(&app) {
                    for label in ["monitor", "weather"] {
                        raise_for_desktop(&app, label);
                    }
                    return;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        });
    }
}

/// 桌面此刻是不是盖在卡片之上（要么它升到了普通窗口栈顶，要么它真的把卡片
/// 盖住了）。
///
/// 两个判据任一成立即可：Win+D 的一瞬间是前者先成立，而桌面换壁纸/动态壁纸
/// 那类场景里 `Progman` 可能压根进不了我们的 z 序枚举，只能用像素命中判。
fn desktop_is_up(app: &AppHandle) -> bool {
    let mine = widget_hwnds(app);
    if desktop::desktop_on_top(&mine) {
        return true;
    }
    ["monitor", "weather"].into_iter().any(|label| {
        app.get_webview_window(label)
            .map(|win| desktop::covered_by_desktop(&win))
            .unwrap_or(false)
    })
}

/// Win+D 的前台事件有一个盲区：**桌面已经占着前台**（用户刚点过桌面）时再按
/// Win+D，前台没有变化，`EVENT_SYSTEM_FOREGROUND` 根本不发，等 250ms 守护线程
/// 兜底就是肉眼可见的一闪（大王实测：这种状态下必然闪现）。
///
/// 这个线程只做一件极轻的事：每 4ms 看一眼"桌面是不是压到卡片上面了"，是就
/// 同一帧贴回桌面正上方。常态开销 ≈ 每卡 2 次 `GetWindow`（`sits_on_desktop`
/// 快检两格命中），只有发现异常才沿 z 序上溯，不查类名只比句柄整数。
///
/// 它只管"被桌面盖住"这一种扰动：点击浮起有 `settle_widget`、层级错杂有
/// 250ms 守护线程，各管各的，互不打架。
fn spawn_desktop_shadow(app: AppHandle) {
    std::thread::spawn(move || {
        // (label, hwnd)；窗口重建后句柄会变，~1s 刷新一次。
        let mut cards: [(&'static str, isize); 2] = [("monitor", 0), ("weather", 0)];
        let mut desk: Option<isize> = None;
        let mut fails = [0u8; 2];
        let mut tick = 0u32;
        loop {
            std::thread::sleep(Duration::from_millis(4));
            tick = tick.wrapping_add(1);
            if tick % 250 == 1 || desk.is_none() {
                desk = desktop::desktop_hwnd();
                for slot in &mut cards {
                    slot.1 = app
                        .get_webview_window(slot.0)
                        .and_then(|w| w.hwnd().ok())
                        .map(|h| h.0 as isize)
                        .unwrap_or(0);
                }
            }
            let Some(d) = desk else { continue };
            let mine: Vec<isize> = cards
                .iter()
                .map(|&(_, h)| h)
                .filter(|&h| h != 0)
                .collect();
            for (i, &(label, h)) in cards.iter().enumerate() {
                if h == 0 || fails[i] >= 3 {
                    // 连贴 3 次还是被压（桌面进了置顶层那类形态）：放手，
                    // 交给 250ms 守护线程 + stick 的保险丝，别每 4ms 白挪一次。
                    continue;
                }
                if !wants_visible(label).load(Ordering::Relaxed)
                    || is_adjusting(label)
                    || on_top_flag(label).load(Ordering::Relaxed)
                {
                    continue;
                }
                // 常态快检：已经贴在桌面上就什么都不用做。
                if desktop::sits_on_desktop_hwnd(h, d, &mine) {
                    fails[i] = 0;
                    continue;
                }
                if desktop::desktop_above(h, d) {
                    desktop::stick_hwnd(h, &mine);
                    // 记账"被桌面盖过"：桌面收起前 `settle_widget` 不该把它压回去。
                    temp_top_flag(label).store(true, Ordering::Relaxed);
                    if desktop::desktop_above(h, d) {
                        fails[i] += 1;
                    } else {
                        fails[i] = 0;
                    }
                } else {
                    fails[i] = 0;
                }
            }
        }
    });
}

/// Keep the widgets where the user expects them, a few times a second.
///
/// Windows does two things behind an app's back, and both are handled here:
///
/// * **显示桌面 (`Win+D`)** raises the desktop window above every ordinary
///   window. Measured on Win11 26100 (`.verify/exp_follow_desktop.py`): the
///   fix is to keep a widget glued to the slot **immediately above the
///   desktop window** - it rises with the desktop and drops back with it,
///   so it is never above any ordinary window (the old
///   raise-to-`HWND_TOPMOST` approach floated the card above the windows
///   still playing their minimize animation, which read as "预览器被压在
///   卡片下面").
///   This loop is *not* what makes that fast enough to be invisible - the
///   foreground hook in `desktop::watch_foreground` is. This is the fallback
///   for the case where the hook event is lost, and the place that puts a
///   widget back when a shell hides or minimizes it behind our back.
/// * Some shells minimize or hide a non-activating window anyway. Put it back.
fn spawn_widget_guard(app: AppHandle) {
    std::thread::spawn(move || loop {
        // 250ms: 兜底要快得让人察觉不到，又不值得为此每秒烧掉 CPU。
        std::thread::sleep(Duration::from_millis(250));

        // 自己的两个窗口要从"谁是第一个普通窗口"的判定里排掉。
        let mut mine: Vec<isize> = Vec::new();
        for label in ["monitor", "weather"] {
            if let Some(win) = app.get_webview_window(label) {
                if let Ok(hwnd) = win.hwnd() {
                    mine.push(hwnd.0 as isize);
                }
            }
        }
        // 一次算好，两个卡片共用：桌面是不是真的占着前台（前台是桌面 + 桌面在栈顶）。
        let desktop_front = desktop::desktop_showing(&mine);

        for label in ["monitor", "weather"] {
            if !wants_visible(label).load(Ordering::Relaxed) {
                continue;
            }
            // 高度归内容管（见 `FIT_H_*`）：**调整模式下也要对账** —— 用户拖底边
            // 只是改了窗口高度，内容高度没变，它必须自己长回来。这一条要是放到
            // `is_adjusting` 后面，就正好把大王报的那个 bug 留在原地。
            reassert_fit_height(&app, label);
            if is_adjusting(label) {
                continue;
            }
            let Some(win) = app.get_webview_window(label) else {
                continue;
            };
            // 有些 Windows 版本（Win10 时代）的"显示桌面"是真最小化，而不是抬
            // 桌面。卡片没有最小化入口，`IsIconic` 为真就只能是系统干的：显式
            // `SW_RESTORE` 叫回来。**不能只靠下面的 `win.show()`** —— `SW_SHOW`
            // 不恢复最小化状态，会一直卡在图标化（`verify_restore_minimized.py`）。
            if desktop::is_minimized(&win) {
                desktop::restore_window(&win);
                launchlog::log(&format!(
                    "z: {label} 被系统最小化，已恢复（前台={}）",
                    desktop::foreground_class(),
                ));
            }
            if desktop::hidden_by_shell(&win) {
                let _ = win.show();
                apply_widget_level(&win, on_top_flag(label).load(Ordering::Relaxed));
                continue;
            }
            if on_top_flag(label).load(Ordering::Relaxed) {
                continue;
            }

            // 只有两种情况卡片该跟着桌面走（贴到桌面正上方）：
            //   ① 桌面**真的盖住了它**（像素命中，硬证据）；
            //   ② 桌面占着前台（前台是桌面，且桌面在普通窗口栈顶）。
            // ② 里的"前台"这一条是 2026-11-11 补的：全屏预览器会把自己抬成置顶、
            // 标题还可能为空，`desktop_on_top` 把它们整排跳过，只按 z 序会得出
            // "桌面还在最前面"的假答案，于是卡片被一直续着跟桌面，压在用户刚恢复的
            // 预览器上。前台窗口不会骗人 —— 用户正在用的那个窗口才在前台。
            // `stick_to_desktop` 幂等：已就位时是 no-op，不会制造重合成。
            if desktop::covered_by_desktop(&win) || desktop_front {
                temp_top_flag(label).store(true, Ordering::Relaxed);
                stick_for_desktop(&win);
                continue;
            }

            // 其余情况：卡片必须贴在桌面层（= 窗口栈最底）。`stick_to_desktop`
            // 是幂等的（已经在那儿就什么都不做），所以这里不用再看"高不高"
            // "要不要等两轮"——那种延时正是"卡片压着程序过了半秒"的来源。残留的
            // WS_EX_TOPMOST（保险丝路径留下的）、被点击顶上去、恢复动画留下的
            // 半空位置，都会在这一步被一起收回来（先显式 `HWND_NOTOPMOST` 摘位，
            // 再插到桌面正上方 —— 只插不摘是摘不掉的）。
            temp_top_flag(label).store(false, Ordering::Relaxed);
            if place_widget_level(&win) && z_log_due(label) {
                launchlog::log(&format!(
                    "z: {label} 落回桌面层（前台={} 置顶={} 桌面在前={})",
                    desktop::foreground_class(),
                    desktop::is_topmost(&win),
                    desktop_front,
                ));
            }
        }
    });
}

/// Undo the raise that a click causes.
///
/// `WS_EX_NOACTIVATE` stops the click from *activating* the window, but WebView2
/// still brings it to the top of the z-order, which leaves a desktop widget in
/// front of the program the user was working in. The click is visible to the
/// front end, so it calls this and we put the window back down - a few times,
/// because the raise is WebView2's own work and its timing is not ours to know.
#[tauri::command]
fn settle_widget(app: AppHandle, label: String) {
    if label != "monitor" && label != "weather" {
        return;
    }
    std::thread::spawn(move || {
        for _ in 0..3 {
            std::thread::sleep(Duration::from_millis(160));
            if is_adjusting(&label)
                || on_top_flag(&label).load(Ordering::Relaxed)
                || temp_top_flag(&label).load(Ordering::Relaxed)
            {
                return;
            }
            if let Some(win) = app.get_webview_window(&label) {
                place_widget_level(&win);
            }
        }
    });
}

/// Vertical gap used when stacking the two widgets on a first run.
const STACK_GAP: f64 = 200.0;

/// Counter behind the debounced config write; see `schedule_config_save`.
static SAVE_GENERATION: AtomicU64 = AtomicU64::new(0);

fn widget_position(cfg: &AppConfig, label: &str) -> Option<(f64, f64)> {
    let pair = match label {
        "monitor" => (cfg.monitor_x, cfg.monitor_y),
        _ => (cfg.weather_x, cfg.weather_y),
    };
    match pair {
        (Some(x), Some(y)) if x.is_finite() && y.is_finite() => Some((x, y)),
        _ => None,
    }
}

/// Pull a widget back onto a screen.
///
/// A widget has no taskbar entry and only a tiny right-click menu, so once its
/// rectangle leaves every monitor there is nothing on screen left to grab: the
/// only way back is editing `config.json` by hand. That is not hypothetical -
/// the 监控 panel was once lost that way, sitting at y=1229 on a 1304-tall
/// desktop with just 83 physical px still inside.
///
/// So *every* position goes through here - the restored one and the saved one -
/// and the rule is "keep the whole window inside the monitor it overlaps most".
/// A widget that overlaps none (dragged fully off the edge) snaps to the first
/// monitor, which is the primary one.
fn fit_on_screen(app: &AppHandle, x: f64, y: f64, w: f64, h: f64) -> (f64, f64) {
    let monitors = app.available_monitors().unwrap_or_default();
    if monitors.is_empty() {
        return (x.max(0.0), y.max(0.0));
    }

    let mut best: Option<(f64, f64, f64, f64)> = None;
    let mut best_area = -1.0;
    for monitor in &monitors {
        let scale = monitor.scale_factor();
        let pos = monitor.position();
        let size = monitor.size();
        let left = pos.x as f64 / scale;
        let top = pos.y as f64 / scale;
        let right = left + size.width as f64 / scale;
        let bottom = top + size.height as f64 / scale;

        let overlap_x = (x + w).min(right) - x.max(left);
        let overlap_y = (y + h).min(bottom) - y.max(top);
        let area = if overlap_x > 0.0 && overlap_y > 0.0 {
            overlap_x * overlap_y
        } else {
            0.0
        };
        if best.is_none() || area > best_area {
            best = Some((left, top, right, bottom));
            best_area = area;
        }
    }

    let Some((left, top, right, bottom)) = best else {
        return (x, y);
    };
    // `.max(left)` matters when a widget is wider than its screen: clamping to
    // `right - w` alone would push its left edge back out of view.
    let max_x = (right - w).max(left);
    let max_y = (bottom - h).max(top);
    (x.clamp(left, max_x), y.clamp(top, max_y))
}

/// Where a widget goes the first time it is ever shown: the primary monitor's
/// top-left corner, which is where a desktop gadget is least in the way.
fn default_origin(app: &AppHandle) -> (f64, f64) {
    match app.primary_monitor().ok().flatten() {
        Some(monitor) => {
            let scale = monitor.scale_factor();
            let pos = monitor.position();
            (
                pos.x as f64 / scale + CORNER_MARGIN,
                pos.y as f64 / scale + CORNER_MARGIN,
            )
        }
        None => (CORNER_MARGIN, CORNER_MARGIN),
    }
}

/// Show / hide the two widget windows according to the config and tell every
/// webview about the new settings so they can repaint immediately.
///
/// Deliberately does NOT touch window sizes. `useStage` owns the geometry and
/// writes a dragged width straight back into the config, so re-applying the
/// size here would fight a widget the user just resized using a stale value
/// from a settings draft.
fn apply_config(app: &AppHandle, cfg: &AppConfig) {
    for (label, visible) in [("monitor", cfg.show_monitor), ("weather", cfg.show_weather)] {
        // The guard thread reads this instead of the config file, so it has to
        // be told *before* the window is shown or hidden - otherwise a hide the
        // user just asked for would be undone 700 ms later.
        wants_visible(label).store(visible, Ordering::Relaxed);
        if let Some(win) = app.get_webview_window(label) {
            if visible {
                let _ = win.show();
            } else {
                let _ = win.hide();
            }
        }
    }
    // 托盘关闭 + 两个窗口都隐藏 = 这个程序在桌面上再无入口，只能去任务管理器。
    // 所以"没有可见窗口"时无视设置，把图标放回来当最后一条退路。
    let widgets_visible = cfg.show_monitor || cfg.show_weather;
    set_tray_visible(app, cfg.tray_icon || !widgets_visible);
    // Re-apply 置顶: it is edited from the settings window as well as from each
    // widget's own menu. A widget being adjusted is left where it is.
    for label in ["monitor", "weather"] {
        if is_adjusting(label) {
            continue;
        }
        if let Some(win) = app.get_webview_window(label) {
            apply_widget_level(&win, widget_always_on_top(cfg, label));
        }
    }
    let _ = app.emit(CONFIG_EVENT, cfg);
}

/// Restore the geometry a previous session persisted. Start-up only.
///
/// Runs *before* the windows are shown (they are created hidden, see
/// tauri.conf.json) so a widget appears at its remembered spot instead of
/// jumping there from wherever the platform parked it.
///
/// A width is always restored. A position is restored too, but clamped onto a
/// screen we still have - see `fit_on_screen`.
fn apply_saved_geometry(app: &AppHandle, cfg: &AppConfig) {
    let (default_x, default_y) = default_origin(app);
    let mut next_y = default_y;

    for label in ["monitor", "weather"] {
        let Some(win) = app.get_webview_window(label) else {
            continue;
        };

        let want_w = widget_width(cfg, label).clamp(config::MIN_WIDTH, config::MAX_WIDTH);
        let height = logical_size(&win).map(|(_, h)| h).unwrap_or(130.0);
        let _ = win.set_size(tauri::LogicalSize::new(want_w, height));

        let (x, y) = match widget_position(cfg, label) {
            // A remembered spot is honoured, but never outside the desktop. A
            // config written before this clamp existed (or by a session on a
            // monitor that is now unplugged) can hold a position that is
            // metres off screen, which is exactly how a widget goes missing.
            Some((x, y)) => fit_on_screen(app, x, y, want_w, height),
            // never placed: tuck it under the widget above rather than on top
            _ => (default_x, next_y),
        };
        let _ = win.set_position(tauri::LogicalPosition::new(x, y));
        next_y = y + STACK_GAP;
    }
}

async fn refresh_weather(app: &AppHandle) -> Result<WeatherPayload, String> {
    let cfg = read_config(app);
    match weather::fetch(&cfg).await {
        Ok(payload) => {
            weather::save_cache(&config::weather_cache_path(), &payload);
            let _ = app.emit(WEATHER_EVENT, &payload);
            Ok(payload)
        }
        Err(err) => {
            let _ = app.emit(WEATHER_ERROR_EVENT, &err);
            Err(err)
        }
    }
}

fn show_settings(app: &AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("settings") {
        let _ = win.unminimize();
        win.show().map_err(|e| e.to_string())?;
        win.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
    }
    // Fallback: the window is normally declared in tauri.conf.json, but if it
    // was destroyed we can rebuild it on the fly.
    WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("settings.html".into()))
        .title(format!("{} 设置", config::APP_NAME))
        .inner_size(470.0, 680.0)
        .min_inner_size(420.0, 480.0)
        .center()
        .build()
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ---------------------------------------------------------------- commands

#[tauri::command]
fn get_snapshot(state: State<'_, AppState>) -> Option<Snapshot> {
    state.snapshot.lock().ok().and_then(|s| s.clone())
}

/// The hardware half of the last sample, read straight from disk. The monitor
/// window calls this on mount so it can draw rated clocks, installed memory
/// and the disk layout immediately, and show 0 for everything that has to be
/// measured.
#[tauri::command]
fn get_monitor_cache() -> Option<MonitorCache> {
    monitorcache::load(&config::monitor_cache_path())
}

#[tauri::command]
fn get_config(state: State<'_, AppState>) -> AppConfig {
    state
        .config
        .lock()
        .map(|c| c.clone())
        .unwrap_or_default()
}

/// Persist settings coming from the settings window, apply them live and kick
/// off a weather refresh when something weather related actually changed.
///
/// The settings window auto-saves on every edit (debounced), so calling the
/// QWeather API unconditionally would burn through the free quota while the
/// user drags an opacity slider.
#[tauri::command]
fn save_config(
    app: AppHandle,
    state: State<'_, AppState>,
    config: AppConfig,
) -> Result<AppConfig, String> {
    let mut next = config;
    config::sanitize(&mut next);

    let previous = {
        let Ok(guard) = state.config.lock() else {
            return Err("配置锁被占用".to_string());
        };
        guard.clone()
    };

    // Three keys in this struct belong to the backend, not to the settings
    // form: the updater writes them and no control edits them. The settings
    // window keeps a whole config in its draft and auto-saves it 450ms after
    // any edit, so a window left open across a background check would write
    // its stale copy back and wipe the record of a downloaded update - taking
    // the "重启更新" prompt with it, while the file sits on disk orphaned. The
    // same draft would just as quietly forget an ignored version, and the
    // release the user turned down would come straight back. Carry them over
    // instead of trusting the form with them.
    next.update_last_check = previous.update_last_check;
    next.update_staged_version = previous.update_staged_version;
    next.update_ignored_version = previous.update_ignored_version;

    let weather_touched = previous.qweather_key != next.qweather_key
        || previous.qweather_host != next.qweather_host
        || previous.location_id != next.location_id
        || previous.weather_days != next.weather_days;

    config::save(&next)?;
    if let Ok(mut guard) = state.config.lock() {
        *guard = next.clone();
    }
    apply_config(&app, &next);

    if weather_touched && !next.qweather_key.is_empty() && !next.location_id.is_empty() {
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            let _ = refresh_weather(&handle).await;
        });
    }

    // The switch is in the same auto-saving form as everything else, so only
    // touch the registry when the value really flipped.
    if previous.autostart != next.autostart {
        if let Err(e) = autostart::set(next.autostart) {
            eprintln!("[autostart] {e}");
        }
    }

    Ok(next)
}

#[tauri::command]
async fn fetch_weather(app: AppHandle) -> Result<WeatherPayload, String> {
    refresh_weather(&app).await
}

/// Last payload written to disk, if any.
///
/// The weather window calls this first: painting yesterday's reading straight
/// away is much better than a blank card while the first round trip finishes.
#[tauri::command]
fn get_cached_weather() -> Option<WeatherCache> {
    weather::load_cache(&config::weather_cache_path())
}

/// Turn the Windows `Run` entry on or off. Returns the state the registry
/// actually reports, so a blocked write cannot leave the switch lying.
#[tauri::command]
fn set_autostart(enabled: bool) -> Result<bool, String> {
    autostart::set(enabled)
}

#[tauri::command]
fn get_autostart() -> bool {
    autostart::is_enabled()
}

/// Test an unsaved key/host/city combo from the settings window.
#[tauri::command]
async fn probe_weather(
    state: State<'_, AppState>,
    host: String,
    key: String,
    location_id: String,
) -> Result<WeatherPayload, String> {
    // The settings window auto-saves on every edit, so the index the user just
    // picked is already in the persisted config - read it from there rather
    // than adding a fourth argument the front end has to keep in sync.
    let advice_type = {
        let Ok(guard) = state.config.lock() else {
            return Err("配置锁被占用".to_string());
        };
        guard.weather_advice_type
    };
    weather::probe(&host, &key, &location_id, advice_type).await
}

/// Search a city by keyword. Falls back to the saved key when the settings
/// window has not been filled in yet.
#[tauri::command]
async fn lookup_city(
    app: AppHandle,
    keyword: String,
    key: Option<String>,
) -> Result<Vec<GeoCity>, String> {
    let effective = match key {
        Some(k) if !k.trim().is_empty() => k,
        _ => read_config(&app).qweather_key,
    };
    weather::lookup_city(&effective, &keyword).await
}

#[tauri::command]
fn toggle_widget(app: AppHandle, label: String, visible: bool) {
    if let Some(win) = app.get_webview_window(&label) {
        if visible {
            let _ = win.show();
        } else {
            let _ = win.hide();
        }
        persist_visibility(&app, &label, visible);
    }
}

#[tauri::command]
fn open_settings_window(app: AppHandle) -> Result<(), String> {
    show_settings(&app)
}

/// Fit a widget window's height to its scaled content, leaving the width alone.
///
/// Only the height is touched on purpose. The webview derives its zoom from
/// `innerWidth`, so if this command also wrote the width back, every
/// logical<->physical rounding step would feed the next one and the widget
/// would creep a few percent larger on each launch. Setting just the height
/// cannot perturb the width, which makes the whole sizing loop immune to that.
///
/// The 1.5px guard stops our own `set_size` from coming back as a resize event
/// and starting a feedback loop.
#[tauri::command]
fn fit_widget_height(app: AppHandle, label: String, height: f64) {
    if label != "monitor" && label != "weather" {
        return;
    }
    let Some(win) = app.get_webview_window(&label) else {
        return;
    };
    let want_h = height.clamp(config::MIN_HEIGHT, config::MAX_HEIGHT);
    // 记下内容要的高度：`set_size` 可能被原生拖拽循环当场覆盖，由守护线程对账。
    fit_h_slot(&label).store(want_h.to_bits(), Ordering::Relaxed);
    let Some((cur_w, cur_h)) = logical_size(&win) else {
        return;
    };
    if (cur_h - want_h).abs() < 1.5 {
        return;
    }
    let _ = win.set_size(tauri::LogicalSize::new(cur_w, want_h));
}

fn fit_h_slot(label: &str) -> &'static AtomicU64 {
    match label {
        "monitor" => &FIT_H_MONITOR,
        _ => &FIT_H_WEATHER,
    }
}

fn fit_try_slot(label: &str) -> &'static [AtomicU64; 3] {
    match label {
        "monitor" => &FIT_TRY_MONITOR,
        _ => &FIT_TRY_WEATHER,
    }
}

/// 窗口高度和内容要求对不上时把它拉回去 —— 守护线程每轮调一次。
///
/// 只有一个场景会走到这里：`set_size` 被原生拖拽循环吞掉了（用户正拖着
/// 窗口边缘），吞掉之后又没有新的 resize 事件来触发前端重报，窗口就停在
/// 一个"内容放不下"的高度上。
///
/// 刹车按"有没有进展"下脚，不是按时间：同一个目标高度、实际高度也一动没动，
/// 半秒内不再重试（窗口被系统钉死时别每 250ms 白撞一回）；高度只要变过，说明
/// 用户还在拖、还有得商量，立刻再对一次。老版本一律 2 秒一次，结果"拖底边拉
/// 高"两下里的第二下常常落在刹车间歇里，看着就像"有时候不自动调整"。
fn reassert_fit_height(app: &AppHandle, label: &str) {
    let bits = fit_h_slot(label).load(Ordering::Relaxed);
    if bits == 0 {
        return;
    }
    let want = f64::from_bits(bits);
    let Some(win) = app.get_webview_window(label) else {
        return;
    };
    let Some((cur_w, cur_h)) = logical_size(&win) else {
        return;
    };
    if (cur_h - want).abs() < 1.5 {
        return;
    }
    let try_slot = fit_try_slot(label);
    let now = now_ms();
    let last_h = f64::from_bits(try_slot[2].load(Ordering::Relaxed));
    let no_progress = (last_h - cur_h).abs() < 1.0;
    if try_slot[0].load(Ordering::Relaxed) == bits
        && no_progress
        && now.saturating_sub(try_slot[1].load(Ordering::Relaxed)) < 500
    {
        return;
    }
    try_slot[0].store(bits, Ordering::Relaxed);
    try_slot[1].store(now, Ordering::Relaxed);
    try_slot[2].store(cur_h.to_bits(), Ordering::Relaxed);
    let _ = win.set_size(tauri::LogicalSize::new(cur_w, want));
    launchlog::log(&format!(
        "fit: {label} 高度对不上（{cur_h:.0} → {want:.0}，w={cur_w:.0}）已拉回"
    ));
}

/// Persist a width the user dragged the window to, without resizing anything.
///
/// `useStage` reports `innerWidth`, which can sit a couple of pixels away from
/// the width the platform actually granted because of DPI rounding. Storing
/// those few pixels would move the window a hair on the next launch, and the
/// next reading would move it again - so anything below the threshold is
/// treated as noise. A real drag is always far above it.
#[tauri::command]
fn remember_widget_width(
    app: AppHandle,
    state: State<'_, AppState>,
    label: String,
    width: f64,
) -> Result<(), String> {
    if label != "monitor" && label != "weather" {
        return Err(format!("未知窗口 {label}"));
    }
    let want = width.clamp(config::MIN_WIDTH, config::MAX_WIDTH);

    let snapshot = {
        let Ok(mut guard) = state.config.lock() else {
            return Err("配置锁被占用".to_string());
        };
        let current = widget_width(&guard, &label);
        if (want - current).abs() < WIDTH_NOISE_PX {
            return Ok(());
        }
        match label.as_str() {
            "monitor" => guard.monitor_width = want,
            "weather" => guard.weather_width = want,
            _ => unreachable!(),
        }
        config::save(&guard)?;
        guard.clone()
    };

    // Keeps the settings window's size sliders in sync without touching the
    // rest of its draft.
    let _ = app.emit(CONFIG_EVENT, &snapshot);
    Ok(())
}

/// Set a widget width from the settings sliders: resize *and* persist.
///
/// The webview picks the new width up from its own resize event and follows up
/// with the matching height, so there is nothing else to do here.
#[tauri::command]
fn set_widget_width(    app: AppHandle,
    state: State<'_, AppState>,
    label: String,
    width: f64,
) -> Result<(), String> {
    if label != "monitor" && label != "weather" {
        return Err(format!("未知窗口 {label}"));
    }
    let want = width.clamp(config::MIN_WIDTH, config::MAX_WIDTH);

    let snapshot = {
        let Ok(mut guard) = state.config.lock() else {
            return Err("配置锁被占用".to_string());
        };
        match label.as_str() {
            "monitor" => guard.monitor_width = want,
            "weather" => guard.weather_width = want,
            _ => unreachable!(),
        }
        config::save(&guard)?;
        guard.clone()
    };

    if let Some(win) = app.get_webview_window(&label) {
        if let Some((cur_w, cur_h)) = logical_size(&win) {
            if (cur_w - want).abs() > 1.5 {
                let _ = win.set_size(tauri::LogicalSize::new(want, cur_h));
            }
        }
    }

    let _ = app.emit(CONFIG_EVENT, &snapshot);
    Ok(())
}

/// Forget the remembered positions and park both widgets in the default
/// corner again.
///
/// The way back from a widget that ended up somewhere awkward - or on a
/// monitor that is no longer attached and therefore cannot be seen at all.
#[tauri::command]
fn reset_widget_positions(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    {
        let Ok(mut guard) = state.config.lock() else {
            return Err("配置锁被占用".to_string());
        };
        guard.monitor_x = None;
        guard.monitor_y = None;
        guard.weather_x = None;
        guard.weather_y = None;
        config::save(&guard)?;
    }

    let (x, mut y) = default_origin(&app);
    reset_snap_track();
    for label in ["monitor", "weather"] {
        if let Some(win) = app.get_webview_window(label) {
            let _ = win.set_position(tauri::LogicalPosition::new(x, y));
            let height = logical_size(&win).map(|(_, h)| h).unwrap_or(130.0);
            y += height.max(120.0).min(STACK_GAP);
        }
    }
    Ok(())
}

/// Float a widget above everything - or drop it back into the desktop layer.
///
/// Per window on purpose: a weather card is worth pinning, a monitor panel
/// usually is not, and the two are configured independently.
#[tauri::command]
fn set_widget_always_on_top(
    app: AppHandle,
    state: State<'_, AppState>,
    label: String,
    on_top: bool,
) -> Result<(), String> {
    if label != "monitor" && label != "weather" {
        return Err(format!("未知窗口 {label}"));
    }
    let snapshot = {
        let Ok(mut guard) = state.config.lock() else {
            return Err("配置锁被占用".to_string());
        };
        match label.as_str() {
            "monitor" => guard.monitor_always_on_top = on_top,
            _ => guard.weather_always_on_top = on_top,
        }
        config::save(&guard)?;
        guard.clone()
    };
    if let Some(win) = app.get_webview_window(&label) {
        apply_widget_level(&win, on_top);
    }
    // The widget's own menu draws its tick from the live config, so it has to
    // hear about the change - otherwise the next click toggles from a stale
    // value and the switch appears stuck on.
    let _ = app.emit(CONFIG_EVENT, &snapshot);
    Ok(())
}

/// Put a widget back where it can be seen and grabbed, and remember that.
///
/// Used when 调整 ends: a drag may have left most of the card past the edge of
/// the screen, and a widget has no other UI to bring it back. Safe to run here
/// because `is_adjusting` is already false, so the magnet has let go.
fn pull_into_screen(app: &AppHandle, label: &str, win: &tauri::WebviewWindow) {
    let Ok(pos) = win.outer_position() else {
        return;
    };
    let scale = win.scale_factor().unwrap_or(1.0);
    let (rw, rh) = win
        .outer_size()
        .map(|s| (s.width as f64 / scale, s.height as f64 / scale))
        .unwrap_or((0.0, 0.0));
    let (rx, ry) = (pos.x as f64 / scale, pos.y as f64 / scale);
    let (x, y) = fit_on_screen(app, rx, ry, rw, rh);
    if (x - rx).abs() < 0.5 && (y - ry).abs() < 0.5 {
        return;
    }
    let _ = win.set_position(tauri::LogicalPosition::new(x, y));

    let snapshot = {
        let state = app.state::<AppState>();
        let Ok(mut guard) = state.config.lock() else {
            return;
        };
        match label {
            "monitor" => {
                guard.monitor_x = Some(x);
                guard.monitor_y = Some(y);
            }
            _ => {
                guard.weather_x = Some(x);
                guard.weather_y = Some(y);
            }
        }
        let _ = config::save(&guard);
        guard.clone()
    };
    let _ = app.emit(CONFIG_EVENT, &snapshot);
}

/// Enter / leave "调整" mode, the only way to move or resize a widget.
///
/// Locked - which is the default - the window is not resizable and has no drag
/// region, so it cannot be shifted by accident. 调整 unlocks both and lifts the
/// window out of the desktop layer so the user can actually see what they are
/// positioning; leaving the mode locks it again and hands the window back to
/// its own 置顶 setting.
#[tauri::command]
fn set_widget_adjust(app: AppHandle, label: String, adjusting: bool) -> Result<(), String> {
    if label != "monitor" && label != "weather" {
        return Err(format!("未知窗口 {label}"));
    }
    let Some(win) = app.get_webview_window(&label) else {
        return Err(format!("窗口 {label} 不存在"));
    };
    win.set_resizable(adjusting).map_err(|e| e.to_string())?;
    adjusting_flag(&label).store(adjusting, Ordering::Relaxed);
    reset_snap_track();
    if adjusting {
        // 调整 needs the keyboard (arrow-key nudging) and needs to be reachable
        // above whatever the user has open, so activation is re-enabled for the
        // duration - `apply_widget_level` reads `is_adjusting` and lets it back
        // in. The styles have to be stamped after `set_resizable`, which is
        // what dropped them.
        apply_widget_level(&win, true);
        // Arrow-key nudging needs the keyboard, and the widgets are built
        // without focus so they never steal it on launch. In 调整 mode the
        // user has just asked to work on this window, so taking it is right.
        let _ = win.set_focus();
    } else {
        pull_into_screen(&app, &label, &win);
        let cfg = read_config(&app);
        apply_widget_level(&win, widget_always_on_top(&cfg, &label));
    }
    Ok(())
}

/// Shift a widget by a few logical pixels - the arrow-key nudge in 调整 mode.
///
/// Native rather than front-end driven: the position the webview can read is
/// affected by its own transform and by DPI rounding, while `outer_position`
/// is the real window rectangle. Going through it also means the move lands in
/// the same `Moved` hook as a drag, so where the widget ends up is remembered
/// the same way a drag is.
///
/// The magnet is *not* part of this: nudging is the user placing the widget on
/// an exact pixel, and a snap that undoes the step would make the keys useless
/// anywhere near the neighbour.
/// 正在拖动吗 —— `snap_widget`（Moved 事件那条路）看见它就别再动窗口，
/// 磁吸由拖动线程自己算，两个方向抢同一个窗口只会互相抵消。
static DRAG_ACTIVE: AtomicBool = AtomicBool::new(false);

/// 调整模式下按住卡片拖动。
///
/// **不用系统的标题栏拖拽**（Tauri 的 `data-tauri-drag-region` 走
/// `WM_NCLBUTTONDOWN + HTCAPTION`）：那是系统的移动循环，窗口一旦被拖到屏幕
/// 边缘，Aero Snap 就接管，把卡片改成半个屏幕 —— 实测 404x295 的卡片拖到副屏
/// 左沿后变成 640x994（= 580x900 逻辑），连 `monitorWidth` 都被一起改写，
/// 内容整块重排。所谓「屏幕边缘靠边磁吸有问题」就是这个。
///
/// 自己转圈只剩算术：每 4ms 让 `snap_apply` 拿指针位移累加一次自由位置、
/// 算完吸附、落一次位置。整跳只有一次 `SetWindowPos`，而且磁吸和位移是同
/// 一个数算出来的 —— 不会像系统拖拽那样把吸附抵消掉。
#[tauri::command]
fn drag_widget(app: AppHandle, label: String) {
    if label != "monitor" && label != "weather" {
        return;
    }
    if !is_adjusting(&label) {
        return;
    }
    if DRAG_ACTIVE.swap(true, Ordering::SeqCst) {
        return; // 已经在拖了（左键连点、标题栏和面板都发了一次）
    }
    if app.get_webview_window(&label).is_none() {
        DRAG_ACTIVE.store(false, Ordering::SeqCst);
        return;
    }
    reset_snap_track();
    std::thread::spawn(move || {
        // 每一跳只落一次位置：`snap_apply` 拿指针位移累加出自由位置、算完吸附
        // 再 `set_position` 一次。早先的写法是"先按位移挪一次、再被磁吸挪回来"，
        // 一跳两次 `SetWindowPos`，跨线程，追不上快速拖动（实测尾巴差 6px）。
        while desktop::left_button_down() {
            snap_apply(&app, &label, true);
            std::thread::sleep(Duration::from_millis(4));
        }
        DRAG_ACTIVE.store(false, Ordering::SeqCst);
        // 最后一次 set_position 是异步的：松手后等它真正落到窗口上再对账，
        // 否则读到的是倒数第二跳，磁吸会拿旧的 free/held 把窗口拽歪。
        std::thread::sleep(Duration::from_millis(25));
        // 松手后再对一次：磁吸收紧最后那一步没走完的几像素。
        snap_apply(&app, &label, true);
        // 之后还会有迟到的 Moved 事件；让它从窗口实际位置重新起算，别沿用
        // 拖动期间那套自由位置。
        reset_snap_track();
    });
}

#[tauri::command]
fn nudge_widget(app: AppHandle, label: String, dx: f64, dy: f64) -> Result<(), String> {
    if label != "monitor" && label != "weather" {
        return Err(format!("未知窗口 {label}"));
    }
    if !dx.is_finite() || !dy.is_finite() {
        return Err("位移无效".to_string());
    }
    let Some(win) = app.get_webview_window(&label) else {
        return Err(format!("窗口 {label} 不存在"));
    };
    let Some((x, y, _, _)) = widget_rect(&win) else {
        return Err("无法读取窗口位置".to_string());
    };
    NUDGE_UNTIL_MS.store(now_ms() + NUDGE_QUIET_MS, Ordering::SeqCst);
    reset_snap_track();
    win.set_position(tauri::LogicalPosition::new(x + dx, y + dy))
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

// ---------------------------------------------------------------- updates

/// What the updater knows right now. Cheap and lock-free enough to poll while
/// a download is in flight.
#[tauri::command]
fn update_status() -> updater::UpdateStatus {
    updater::snapshot()
}

/// Look for a new release. `manual` marks the check the user asked for with the
/// button, which is allowed to report a failure and will download even when the
/// automatic switch is off - stopping at "there is one, go find it" would be a
/// strange reading of "check now".
///
/// `auto` has to be a parameter and not two commands, because it is the same
/// code path: the difference is only who is allowed to complain.
#[tauri::command]
async fn check_update(app: AppHandle, manual: Option<bool>) -> updater::UpdateStatus {
    updater::check(app, manual.unwrap_or(true)).await;
    updater::snapshot()
}

/// Swap in the staged build and restart. On success this process is gone before
/// the call returns - the reply is for the failure case only.
#[tauri::command]
fn apply_update(app: AppHandle) -> Result<(), String> {
    // `false`: the user asked for this from a settings window, so it is not a
    // logon launch and the new build has no start-up delay to inherit.
    updater::apply(false)?;
    // The replacement is running and is waiting on our mutex, which we release
    // by dying. Nothing else here is worth finishing.
    app.exit(0);
    Ok(())
}

/// Turn down an update: delete the staged file and remember the version, so the
/// next check does not offer it again.
#[tauri::command]
fn discard_update(app: AppHandle) -> Result<(), String> {
    updater::discard(&app)
}

/// Undo 忽略这个版本. The frontend follows up with a check of its own.
#[tauri::command]
fn unignore_update(app: AppHandle) -> Result<(), String> {
    updater::unignore(&app)
}

/// Open a URL in whatever the user has registered as their browser.
///
/// Only `http`/`https` get through. This command is reachable from a clickable
/// link in the settings window, so accepting `file:` or a scheme-less path
/// would turn one click into arbitrary local execution.
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    // `CommandExt` lives on Windows only, and this app is Windows only - it
    // is imported here rather than at the top so the trait's scope stays next
    // to the one call that needs it.
    use std::os::windows::process::CommandExt as _;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let target = url.trim();
    if !target.starts_with("http://") && !target.starts_with("https://") {
        return Err("只允许打开 http/https 链接".to_string());
    }

    // `start` is a cmd builtin, hence the shell. The empty "" is its required
    // window-title argument; without it a quoted URL is read as the title.
    std::process::Command::new("cmd")
        .args(["/c", "start", "", target])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("打开链接失败: {e}"))?;
    Ok(())
}

// ------------------------------------------------------------- background

/// Write the config to disk a moment after the last change.
///
/// Dragging a window fires `Moved` dozens of times per second, and each of
/// those positions is already in memory - only the resting place is worth a
/// file write. Every call bumps a generation counter; a saver that wakes up to
/// find it is no longer the newest one simply goes back to sleep.
fn schedule_config_save(app: &AppHandle) {
    let generation = SAVE_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(700)).await;
        if SAVE_GENERATION.load(Ordering::SeqCst) != generation {
            return;
        }
        let cfg = read_config(&handle);
        let _ = config::save(&cfg);
    });
}

/// Remember where a widget was dragged to. Called from the window event hook.
///
/// Positions are stored in logical px, like every other geometry value in this
/// app, so they survive a change of display scaling.
///
/// Whatever the drag reports is clamped to a screen before it is stored. The
/// window itself is left where the drag put it (moving it from inside a `Moved`
/// handler would fight the magnet), but the value that survives a restart is
/// always a spot the user can reach - see `fit_on_screen`.
fn remember_position(window: &tauri::Window, pos: tauri::PhysicalPosition<i32>) {
    let label = window.label().to_string();
    if label != "monitor" && label != "weather" {
        return;
    }
    let scale = window.scale_factor().unwrap_or(1.0);
    let (raw_x, raw_y) = (pos.x as f64 / scale, pos.y as f64 / scale);
    let (w, h) = window
        .outer_size()
        .map(|s| (s.width as f64 / scale, s.height as f64 / scale))
        .unwrap_or((0.0, 0.0));
    let (x, y) = fit_on_screen(window.app_handle(), raw_x, raw_y, w, h);

    let changed = {
        let state = window.app_handle().state::<AppState>();
        let Ok(mut guard) = state.config.lock() else {
            return;
        };
        let same = match widget_position(&guard, &label) {
            Some((px, py)) => (px - x).abs() < 1.0 && (py - y).abs() < 1.0,
            None => false,
        };
        if same {
            return; // a 1px wobble from the platform, not a move
        }
        match label.as_str() {
            "monitor" => {
                guard.monitor_x = Some(x);
                guard.monitor_y = Some(y);
            }
            _ => {
                guard.weather_x = Some(x);
                guard.weather_y = Some(y);
            }
        }
        true
    };

    if changed {
        schedule_config_save(&window.app_handle().clone());
    }
}

fn persist_visibility(app: &AppHandle, label: &str, visible: bool) {
    let snapshot = {
        let state = app.state::<AppState>();
        let Ok(mut guard) = state.config.lock() else {
            return;
        };
        match label {
            "monitor" => guard.show_monitor = visible,
            "weather" => guard.show_weather = visible,
            _ => return,
        }
        let _ = config::save(&guard);
        guard.clone()
    };
    let _ = app.emit(CONFIG_EVENT, &snapshot);
}

/// Hardware sampling loop.
///
/// Runs on a plain OS thread (never a busy loop). It wakes up once per
/// configured interval, samples everything in one go, caches the result and
/// pushes it to the webviews. When both widgets are hidden the sample is
/// skipped entirely, so a minimised widget costs literally nothing.
fn spawn_collector(app: AppHandle) {
    std::thread::spawn(move || {
        let mut collector = Collector::new();
        let mut last = Instant::now();
        let cache_path = config::monitor_cache_path();
        // Refreshed on the first sample of the session, then at most once a
        // minute: what it holds barely moves, and it is only ever read during
        // the second before the next first sample arrives.
        let mut cache_written: Option<Instant> = None;

        loop {
            let interval_ms = read_config(&app).monitor_interval_ms;
            std::thread::sleep(Duration::from_millis(interval_ms));

            let any_visible = ["monitor", "weather"]
                .iter()
                .any(|label| match app.get_webview_window(label) {
                    Some(win) => win.is_visible().unwrap_or(false),
                    None => false,
                });
            if !any_visible {
                last = Instant::now();
                continue;
            }

            let elapsed_ms = last.elapsed().as_millis() as u64;
            last = Instant::now();

            let snap = collector.sample(elapsed_ms);

            if let Ok(mut guard) = app.state::<AppState>().snapshot.lock() {
                *guard = Some(snap.clone());
            }
            let _ = app.emit(SNAPSHOT_EVENT, &snap);

            if cache_written
                .map(|t| t.elapsed() >= Duration::from_secs(60))
                .unwrap_or(true)
            {
                monitorcache::save(&cache_path, &snap);
                cache_written = Some(Instant::now());
            }
        }
    });
}

/// Weather refresh loop - orders of magnitude slower than the monitor loop.
fn spawn_weather(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        // The weather window asks for its own refresh the moment it mounts, so
        // this loop deliberately waits: firing 1.2s after launch used to spend
        // a second QWeather call on data that had just arrived. The card is
        // never blank in the meantime - it paints the on-disk cache.
        tokio::time::sleep(Duration::from_secs(15)).await;

        loop {
            let cfg = read_config(&app);
            if !cfg.qweather_key.is_empty() && !cfg.location_id.is_empty() {
                let _ = refresh_weather(&app).await;
            }
            tokio::time::sleep(Duration::from_secs(cfg.weather_refresh_min.max(5) * 60)).await;
        }
    });
}

// ------------------------------------------------------------------ setup

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    // NOTE: item variables must not be named after the helper functions below
    // (`refresh_weather` / `show_settings`) or they would shadow them.
    let mi_monitor =
        MenuItem::with_id(app, "toggle_monitor", "显示/隐藏 监控面板", true, None::<&str>)?;
    let mi_weather =
        MenuItem::with_id(app, "toggle_weather", "显示/隐藏 天气窗口", true, None::<&str>)?;
    let mi_refresh = MenuItem::with_id(app, "refresh_weather", "立即刷新天气", true, None::<&str>)?;
    let mi_settings = MenuItem::with_id(app, "settings", "设置…", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let mi_quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[
            &mi_monitor,
            &mi_weather,
            &mi_refresh,
            &sep1,
            &mi_settings,
            &sep2,
            &mi_quit,
        ],
    )?;

    let mut builder = TrayIconBuilder::with_id("main")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip(config::APP_NAME)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "toggle_monitor" => flip_visibility(app, "monitor"),
            "toggle_weather" => flip_visibility(app, "weather"),
            "refresh_weather" => {
                let handle = app.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = refresh_weather(&handle).await;
                });
            }
            "settings" => {
                let _ = show_settings(app);
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let _ = show_settings(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }

    builder.build(app)?;
    Ok(())
}

fn flip_visibility(app: &AppHandle, label: &str) {
    let Some(win) = app.get_webview_window(label) else {
        return;
    };
    let now_visible = !win.is_visible().unwrap_or(false);
    // Told before the window moves: the guard thread would otherwise see a
    // hidden widget whose config still says "show" and put it straight back.
    wants_visible(label).store(now_visible, Ordering::Relaxed);
    if now_visible {
        let _ = win.show();
    } else {
        let _ = win.hide();
    }
    persist_visibility(app, label, now_visible);
}

/// Show or hide the notification-area icon.
///
/// The tray is always *built*; the switch only flips its visibility, so it can
/// come and go at runtime without touching the menu or the icon handle. With
/// the icon gone, 设置 is still reachable from each widget's right-click menu
/// and 退出程序 from the settings window itself.
/// Last value handed to `set_tray_visible`. The settings window saves on every
/// edit, and `apply_config` runs on each save - logging or re-applying every
/// time would either spam launch.log or flicker the icon.
static TRAY_VISIBLE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(2);

fn set_tray_visible(app: &AppHandle, visible: bool) {
    let want = if visible { 1 } else { 0 };
    if TRAY_VISIBLE.swap(want, Ordering::Relaxed) == want {
        return;
    }
    let Some(tray) = app.tray_by_id("main") else {
        launchlog::log("tray: icon was never built - visibility not applied");
        return;
    };
    match tray.set_visible(visible) {
        // launch.log is the only place a "the switch does nothing" report can
        // be answered from, so record the one line that changes state.
        Ok(()) => launchlog::log(if visible {
            "tray: icon shown"
        } else {
            "tray: icon hidden"
        }),
        Err(e) => launchlog::log(&format!("tray: set_visible({visible}) failed: {e}")),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut initial = config::load();
    config::sanitize(&mut initial);

    tauri::Builder::default()
        .manage(AppState {
            config: Mutex::new(initial),
            snapshot: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            get_monitor_cache,
            get_config,
            save_config,
            fetch_weather,
            get_cached_weather,
            set_autostart,
            get_autostart,
            probe_weather,
            lookup_city,
            fit_widget_height,
            remember_widget_width,
            set_widget_width,
            reset_widget_positions,
            nudge_widget,
            drag_widget,
            set_widget_always_on_top,
            settle_widget,
            set_widget_adjust,
            toggle_widget,
            open_settings_window,
            open_url,
            update_status,
            check_update,
            apply_update,
            discard_update,
            unignore_update,
            quit_app
        ])
        .on_window_event(|window, event| {
            match event {
                // The settings window is closed by hiding it, so the webview
                // stays warm and reopens instantly.
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    if window.label() == "settings" {
                        api.prevent_close();
                        let _ = window.hide();
                    }
                }
                // Where the user leaves a widget is configuration, not state.
                // Snapping first, so what gets remembered is where the widget
                // actually ended up rather than where the pointer left it.
                tauri::WindowEvent::Moved(pos) => {
                    snap_widget(window);
                    remember_position(window, *pos);
                }
                _ => {}
            }
        })
        .setup(|app| {
            let handle = app.handle().clone();
            build_tray(&handle)?;

            let cfg = read_config(&handle);
            // Geometry first, visibility second: the widget windows are created
            // hidden so they can be moved into place before their first frame
            // is ever painted.
            apply_saved_geometry(&handle, &cfg);
            apply_config(&handle, &cfg);
            apply_widget_behaviour(&handle, &cfg);

            // Re-assert the Run entry on every launch: the value stores an
            // absolute exe path, so moving the folder (or installing a new
            // build elsewhere) would otherwise leave it pointing at nothing.
            if let Err(e) = autostart::set(cfg.autostart) {
                eprintln!("[autostart] {e}");
            }

            // First run: there is no config file at all, so no city to show and
            // no key to call with - open the settings window instead of
            // leaving the user staring at an empty widget.
            //
            // Keyed on the *file*, not on `location_id`. A config that exists
            // but has no city is a state the user chose (or is in the middle of
            // choosing inside the settings window); re-opening the window on
            // every start - including the auto-start one - is how it ends up
            // parked on the taskbar with nothing to do.
            let city = if cfg.location_id.is_empty() {
                "none".to_string()
            } else {
                cfg.location_id.clone()
            };
            launchlog::log(&format!(
                "boot pid={} config={} city={} autostart={}",
                std::process::id(),
                config::config_path().display(),
                city,
                cfg.autostart
            ));
            if !config::config_path().exists() {
                launchlog::log("first run: no config file - opening settings");
                let _ = show_settings(&handle);
            }

            spawn_collector(handle.clone());
            spawn_weather(handle.clone());
            spawn_widget_guard(handle.clone());
            spawn_desktop_shadow(handle.clone());

            // 前台事件驱动的那一路（`Win+D` 不闪靠它），守护线程只做兜底。
            let _ = APP_HANDLE.set(handle.clone());
            desktop::watch_foreground(on_desktop_foreground);
            // 点别处收菜单：悬浮窗不抢焦点，`blur` 那条路走不通，只能看全局鼠标。
            desktop::watch_click_outside(on_widget_click_outside);
            // An update downloaded before the last restart is still sitting
            // beside the exe, and the settings window has to go on offering it.
            updater::restore_staged(&cfg.update_staged_version);
            // Same reasoning for the version the user turned down: it has to
            // still be written on the page after a restart, or the ignore looks
            // like it was forgotten.
            updater::restore_ignored(&cfg.update_ignored_version);
            updater::start(handle);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running WSight");
}
