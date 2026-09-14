//! Persisted application settings.
//!
//! The file lives in `%APPDATA%\WSight\config.json` and is edited
//! through the in-app settings window (never by hand). Every field carries
//! `#[serde(default)]` semantics via the struct-level attribute so an older
//! config file keeps loading after new fields are introduced.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

pub const DEFAULT_HOST: &str = "https://devapi.qweather.com";
pub const COLOR_CPU: &str = "#4aa8ff";
pub const COLOR_MEM: &str = "#52d3a4";
pub const COLOR_GPU: &str = "#b98cff";
pub const COLOR_NET: &str = "#67d3ff";

/// Width the front-end lays every widget out at, before `useStage` scales it.
/// Must stay in sync with `BASE_WIDTH` in `src/shared/uiScale.ts`.
pub const BASE_WIDTH: f64 = 300.0;

/// Bounds shared by the settings sliders and the native clamps.
pub const MIN_WIDTH: f64 = 210.0;
pub const MAX_WIDTH: f64 = 640.0;
pub const MIN_HEIGHT: f64 = 90.0;
pub const MAX_HEIGHT: f64 = 900.0;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppConfig {
    // ------------------------------------------------------------ weather
    pub qweather_key: String,
    /// Full base URL of the QWeather data API, e.g. https://devapi.qweather.com
    pub qweather_host: String,
    pub location_id: String,
    pub location_name: String,
    pub weather_refresh_min: u64,
    /// how many forecast rows the weather window shows (1..=7)
    pub weather_days: u32,
    pub show_weather: bool,

    // ------------------------------------------------------------ monitor
    pub monitor_interval_ms: u64,
    pub show_monitor: bool,
    pub monitor_show_gpu: bool,
    pub monitor_show_net: bool,
    /// show the disk block under the gauges: one tile per volume on the left,
    /// the per-drive throughput on the right
    pub monitor_show_disk: bool,
    /// Which volumes get a tile in the disk block, as drive letters (`"C:"`),
    /// in display order. Empty = the first three volumes by letter.
    pub monitor_disks: Vec<String>,
    /// Which *physical* drives the throughput block shows, as disk numbers.
    /// Empty = the first two. A number that no longer exists is ignored, and a
    /// selection that resolves to nothing falls back to the automatic pair.
    pub monitor_drives: Vec<u32>,

    // --------------------------------------------------------- appearance
    /// dark | light
    pub theme: String,
    /// whole-panel opacity, 0.25..=1
    pub opacity: f32,
    /// panel corner radius in px, 0..=24
    pub radius: f32,
    pub color_cpu: String,
    pub color_mem: String,
    pub color_gpu: String,
    pub color_net: String,

    // --------------------------------------------------------------- size
    /// logical width of the monitor window, in px
    pub monitor_width: f64,
    /// logical width of the weather window, in px
    pub weather_width: f64,

    // ----------------------------------------------------------- position
    /// remembered top-left corner in logical px; `None` = never placed, so the
    /// app uses its own default corner. Kept as four plain `Option`s rather
    /// than a nested struct so an older config file stays readable.
    pub monitor_x: Option<f64>,
    pub monitor_y: Option<f64>,
    pub weather_x: Option<f64>,
    pub weather_y: Option<f64>,

    // ----------------------------------------------------------- behaviour
    /// float the monitor window above everything else. `false` (the default)
    /// drops it to the desktop layer instead - it is furniture, not a tool.
    pub monitor_always_on_top: bool,
    /// same, for the weather window - set independently on purpose
    pub weather_always_on_top: bool,

    // --------------------------------------------------------------- misc
    /// reserved: launch both widgets on login
    pub autostart: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            qweather_key: String::new(),
            qweather_host: DEFAULT_HOST.to_string(),
            location_id: String::new(),
            location_name: String::new(),
            weather_refresh_min: 15,
            weather_days: 3,
            show_weather: true,

            monitor_interval_ms: 1000,
            show_monitor: true,
            monitor_show_gpu: true,
            monitor_show_net: true,
            monitor_show_disk: true,
            monitor_disks: Vec::new(),
            monitor_drives: Vec::new(),

            theme: "dark".to_string(),
            opacity: 0.92,
            radius: 14.0,
            color_cpu: COLOR_CPU.to_string(),
            color_mem: COLOR_MEM.to_string(),
            color_gpu: COLOR_GPU.to_string(),
            color_net: COLOR_NET.to_string(),

            monitor_width: BASE_WIDTH,
            weather_width: BASE_WIDTH,

            monitor_x: None,
            monitor_y: None,
            weather_x: None,
            weather_y: None,

            monitor_always_on_top: false,
            weather_always_on_top: false,

            autostart: false,
        }
    }
}

/// Display name of the application. Used for the config folder, the tray
/// tooltip and the window titles, so it lives in exactly one place.
pub const APP_NAME: &str = "WSight";

/// Config folder of the pre-WSight build. Read once so an existing install keeps
/// its API key and city after the rename instead of silently resetting.
const LEGACY_DIR: &str = "desktop-widgets";

fn roaming() -> PathBuf {
    std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir())
}

pub fn data_dir() -> PathBuf {
    let dir = roaming().join(APP_NAME);
    let _ = fs::create_dir_all(&dir);
    dir
}

pub fn config_path() -> PathBuf {
    data_dir().join("config.json")
}

fn legacy_config_path() -> PathBuf {
    roaming().join(LEGACY_DIR).join("config.json")
}

pub fn load() -> AppConfig {
    let path = config_path();
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(_) => {
            // No config under the new name yet - look for one written by the
            // old build. Copied rather than moved, so the old folder survives
            // and going back to the previous exe still works.
            match fs::read_to_string(legacy_config_path()) {
                Ok(old) => {
                    if fs::write(&path, &old).is_ok() {
                        eprintln!("[config] migrated legacy config into {}", path.display());
                    }
                    old
                }
                Err(_) => return AppConfig::default(),
            }
        }
    };

    // Notepad / PowerShell write a UTF-8 BOM. `serde_json` rejects it,
    // which used to make the whole config silently fall back to the
    // defaults (empty API key, no city). Strip it before parsing.
    let cleaned = text.trim_start_matches('\u{feff}');
    match serde_json::from_str::<AppConfig>(cleaned) {
        Ok(cfg) => cfg,
        Err(err) => {
            eprintln!("[config] parse failed ({err}), falling back to defaults");
            AppConfig::default()
        }
    }
}

pub fn save(cfg: &AppConfig) -> Result<(), String> {
    let path = config_path();
    let text = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    fs::write(&path, text).map_err(|e| format!("write {}: {e}", path.display()))
}

fn is_hex_color(value: &str) -> bool {
    let v = value.trim();
    v.len() == 7 && v.starts_with('#') && v[1..].chars().all(|c| c.is_ascii_hexdigit())
}

fn fix_color(slot: &mut String, fallback: &str) {
    if !is_hex_color(slot) {
        *slot = fallback.to_string();
    } else {
        *slot = slot.trim().to_ascii_lowercase();
    }
}

/// Turn any user supplied host into a usable base URL.
/// Accepts `devapi.qweather.com`, `https://api.qweather.com/`, ... and always
/// returns something without a trailing slash.
pub fn normalize_host(raw: &str) -> String {
    let trimmed = raw.trim().trim_end_matches('/').trim();
    if trimmed.is_empty() {
        return DEFAULT_HOST.to_string();
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    }
}

/// Clamp user supplied values into sane ranges so a bad config can never turn
/// the collector into a CPU hog or the UI into an unreadable mess.
pub fn sanitize(cfg: &mut AppConfig) {
    cfg.monitor_interval_ms = cfg.monitor_interval_ms.clamp(500, 60_000);
    cfg.weather_refresh_min = cfg.weather_refresh_min.clamp(5, 720);
    cfg.weather_days = cfg.weather_days.clamp(1, 7);
    cfg.opacity = cfg.opacity.clamp(0.25, 1.0);
    cfg.radius = cfg.radius.clamp(0.0, 24.0);
    cfg.monitor_width = cfg.monitor_width.clamp(MIN_WIDTH, MAX_WIDTH);
    cfg.weather_width = cfg.weather_width.clamp(MIN_WIDTH, MAX_WIDTH);

    cfg.qweather_key = cfg.qweather_key.trim().to_string();
    cfg.location_id = cfg.location_id.trim().to_string();
    cfg.location_name = cfg.location_name.trim().to_string();
    cfg.qweather_host = normalize_host(&cfg.qweather_host);

    if cfg.theme != "light" && cfg.theme != "dark" {
        cfg.theme = "dark".to_string();
    }

    fix_color(&mut cfg.color_cpu, COLOR_CPU);
    fix_color(&mut cfg.color_mem, COLOR_MEM);
    fix_color(&mut cfg.color_gpu, COLOR_GPU);
    fix_color(&mut cfg.color_net, COLOR_NET);
}
