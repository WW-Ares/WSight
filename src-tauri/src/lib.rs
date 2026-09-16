mod autostart;
mod collector;
mod config;
mod launchlog;
mod monitorcache;
mod native;
mod single_instance;
mod updater;
mod weather;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
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

/// Delete the exe the updater stepped aside, plus any earlier leftovers.
pub fn clean_up_replaced(replaced: &str) {
    updater::clean_up(replaced);
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

/// What the magnet needs to remember between one `Moved` and the next.
///
/// Nothing here is bookkeeping for its own sake - the window move loop makes
/// all three necessary. It reports each `Moved` as *the position we last gave
/// the window* plus the pointer's own delta, which means a snap we apply
/// becomes the reference for the next event: the two pixels the pointer spent
/// pulling away are added to the line instead of to the pointer's travel, and
/// are gone. A widget under a real drag can therefore never escape a line, no
/// matter how far it is pulled - which is exactly what "磁吸太紧，拖不动" is.
/// So we keep the running total ourselves.
#[derive(Clone, Copy, Default)]
struct SnapTrack {
    /// The last position handed to the window - what the move loop measures from.
    reported: Option<(f64, f64)>,
    /// Where the widget would be if there were no magnet at all: `reported`
    /// plus every movement the pointer has made while we were holding a line.
    free: Option<(f64, f64)>,
    /// The line each axis is currently held by, in logical px.
    held: (Option<f64>, Option<f64>),
}

static SNAP: Mutex<SnapTrack> = Mutex::new(SnapTrack {
    reported: None,
    free: None,
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
fn snap_axis(
    cur: f64,
    len: f64,
    other: f64,
    other_len: f64,
    held: Option<f64>,
) -> (f64, Option<f64>) {
    if let Some(line) = held {
        if (line - cur).abs() <= SNAP_ESCAPE_PX {
            return (line, Some(line));
        }
    }

    let candidates = [
        other,
        other + other_len - len,
        other + other_len,
        other - len,
        other + (other_len - len) / 2.0,
    ];
    let mut best = cur;
    let mut best_gap = SNAP_PX;
    let mut lock = None;
    for candidate in candidates {
        let gap = (candidate - cur).abs();
        // `<=` so a later, equally close line wins - they are ordered
        // near-edge first, which is the alignment a user reaches for most.
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
    let label = window.label();
    if !is_adjusting(label) {
        return;
    }
    // Arrow keys are the user aiming at a pixel; the magnet stays out of it.
    if now_ms() < NUDGE_UNTIL_MS.load(Ordering::SeqCst) {
        return;
    }
    if SNAPPING.swap(true, Ordering::SeqCst) {
        return;
    }

    let app = window.app_handle();
    let neighbour_label = if label == "monitor" { "weather" } else { "monitor" };
    if let Some(moving) = app.get_webview_window(label) {
        if let Some(other) = app.get_webview_window(neighbour_label) {
            // A hidden neighbour is not on screen to align with - and its stored
            // rectangle would be the last place it was seen, which is worse than
            // no snapping at all.
            if other.is_visible().unwrap_or(false) {
                if let (Some((x, y, w, h)), Some((ox, oy, ow, oh))) =
                    (widget_rect(&moving), widget_rect(&other))
                {
                    let (free, held_x, held_y) = SNAP
                        .lock()
                        .map(|track| match (track.reported, track.free) {
                            (Some(prev), Some(free)) => (
                                (free.0 + (x - prev.0), free.1 + (y - prev.1)),
                                track.held.0,
                                track.held.1,
                            ),
                            _ => ((x, y), None, None),
                        })
                        .unwrap_or(((x, y), None, None));

                    let (nx, lock_x) = snap_axis(free.0, w, ox, ow, held_x);
                    let (ny, lock_y) = snap_axis(free.1, h, oy, oh, held_y);
                    let applied = if (nx - x).abs() >= 0.5 || (ny - y).abs() >= 0.5 {
                        if moving
                            .set_position(tauri::LogicalPosition::new(nx, ny))
                            .is_ok()
                        {
                            (nx, ny)
                        } else {
                            (x, y)
                        }
                    } else {
                        (x, y)
                    };

                    if let Ok(mut track) = SNAP.lock() {
                        track.reported = Some(applied);
                        track.free = Some(free);
                        track.held = (lock_x, lock_y);
                    }
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
// `SetWindowPos(HWND_BOTTOM)` is that missing call. Declared by hand because
// it is one function; pulling in the whole `windows` crate for it would add
// build time to every release.
#[cfg(windows)]
mod zorder {
    use std::ffi::c_void;

    const HWND_TOP: isize = 0;
    const HWND_BOTTOM: isize = 1;
    const SWP_NOSIZE: u32 = 0x0001;
    const SWP_NOMOVE: u32 = 0x0002;
    const SWP_NOACTIVATE: u32 = 0x0010;
    const SWP_NOOWNERZORDER: u32 = 0x0200;

    #[link(name = "user32")]
    extern "system" {
        fn SetWindowPos(
            hwnd: *mut c_void,
            insert_after: *mut c_void,
            x: i32,
            y: i32,
            cx: i32,
            cy: i32,
            flags: u32,
        ) -> i32;
    }

    fn place(win: &tauri::WebviewWindow, after: isize) {
        let Ok(hwnd) = win.hwnd() else {
            return;
        };
        // SAFETY: the handle comes from Tauri's own window, and the flags ask
        // user32 to change nothing except the z-order.
        unsafe {
            let _ = SetWindowPos(
                hwnd.0 as *mut c_void,
                after as *mut c_void,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
            );
        }
    }

    /// Behind every ordinary window - the desktop-furniture level.
    pub fn to_bottom(win: &tauri::WebviewWindow) {
        place(win, HWND_BOTTOM);
    }

    /// Top of the ordinary (non-topmost) band. Used while adjusting, so the
    /// widget the user is working on is not hidden behind something else.
    pub fn to_front(win: &tauri::WebviewWindow) {
        place(win, HWND_TOP);
    }
}

#[cfg(not(windows))]
mod zorder {
    pub fn to_bottom(_win: &tauri::WebviewWindow) {}
    pub fn to_front(_win: &tauri::WebviewWindow) {}
}

/// Whether a widget is currently in "调整" mode. Tracked natively because the
/// settings window autosaves in the background, and a save must not drag a
/// widget the user is in the middle of positioning back down to the desktop.
static ADJUSTING_MONITOR: AtomicBool = AtomicBool::new(false);
static ADJUSTING_WEATHER: AtomicBool = AtomicBool::new(false);

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

/// Put a widget at the level its 置顶 setting asks for: floating above
/// everything, or tucked under every ordinary window.
fn apply_widget_level(win: &tauri::WebviewWindow, on_top: bool) {
    let _ = win.set_always_on_top(on_top);
    if on_top {
        zorder::to_front(win);
    } else {
        zorder::to_bottom(win);
    }
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

/// Is there still a screen under this point?
///
/// Guards the restore path: unplugging the monitor a widget lived on would
/// otherwise reopen it somewhere off the desktop, where it cannot be grabbed
/// or even seen. Anything outside every monitor falls back to the default
/// corner instead.
fn position_visible(app: &AppHandle, x: f64, y: f64) -> bool {
    let monitors = app.available_monitors().unwrap_or_default();
    monitors.iter().any(|monitor| {
        let scale = monitor.scale_factor();
        let pos = monitor.position();
        let size = monitor.size();
        let left = pos.x as f64 / scale;
        let top = pos.y as f64 / scale;
        let right = left + size.width as f64 / scale;
        let bottom = top + size.height as f64 / scale;
        // the whole title strip has to be reachable, not just the corner pixel
        x >= left - 8.0 && y >= top - 8.0 && x <= right - 60.0 && y <= bottom - 30.0
    })
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
        if let Some(win) = app.get_webview_window(label) {
            if visible {
                let _ = win.show();
            } else {
                let _ = win.hide();
            }
        }
    }
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
/// A width is always restored. A position is restored only if it still lands
/// on a monitor we have - see `position_visible`.
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
            Some((x, y)) if position_visible(app, x, y) => (x, y),
            // never placed, or placed on a screen that is gone: tuck it under
            // the widget above rather than on top of it
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
    let Some((cur_w, cur_h)) = logical_size(&win) else {
        return;
    };
    if (cur_h - want_h).abs() < 1.5 {
        return;
    }
    let _ = win.set_size(tauri::LogicalSize::new(cur_w, want_h));
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
        zorder::to_front(&win);
        // Arrow-key nudging needs the keyboard, and the widgets are built
        // without focus so they never steal it on launch. In 调整 mode the
        // user has just asked to work on this window, so taking it is right.
        let _ = win.set_focus();
    } else {
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
    updater::apply()?;
    // The replacement is running and is waiting on our mutex, which we release
    // by dying. Nothing else here is worth finishing.
    app.exit(0);
    Ok(())
}

/// Forget a staged update: delete the file and clear the config key.
#[tauri::command]
fn discard_update(app: AppHandle) -> Result<(), String> {
    updater::discard(&app)
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
fn remember_position(window: &tauri::Window, pos: tauri::PhysicalPosition<i32>) {
    let label = window.label().to_string();
    if label != "monitor" && label != "weather" {
        return;
    }
    let scale = window.scale_factor().unwrap_or(1.0);
    let (x, y) = (pos.x as f64 / scale, pos.y as f64 / scale);

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
    if now_visible {
        let _ = win.show();
    } else {
        let _ = win.hide();
    }
    persist_visibility(app, label, now_visible);
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
            set_widget_always_on_top,
            set_widget_adjust,
            toggle_widget,
            open_settings_window,
            open_url,
            update_status,
            check_update,
            apply_update,
            discard_update,
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

            // An update downloaded before the last restart is still sitting
            // beside the exe, and the settings window has to go on offering it.
            updater::restore_staged(&cfg.update_staged_version);
            updater::start(handle);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running WSight");
}
