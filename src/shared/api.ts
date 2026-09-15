import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AppConfig,
  GeoCity,
  MonitorCache,
  Snapshot,
  WeatherCache,
  WeatherPayload,
} from "./types";

/** Event name emitted by the Rust collector thread on every successful sample. */
export const SNAPSHOT_EVENT = "monitor://snapshot";
export const WEATHER_EVENT = "weather://payload";
export const WEATHER_ERROR_EVENT = "weather://error";
export const CONFIG_EVENT = "config://changed";

export const api = {
  /** latest cached snapshot (null until the first sample lands) */
  getSnapshot: () => invoke<Snapshot | null>("get_snapshot"),

  /**
   * Hardware facts from the previous session - null before the app has ever
   * sampled. Lets the monitor panel draw rated clocks, installed memory and
   * the disk layout while the live figures are still 0.
   */
  getMonitorCache: () => invoke<MonitorCache | null>("get_monitor_cache"),

  getConfig: () => invoke<AppConfig>("get_config"),
  /** persist + apply; resolves with the sanitised config the app actually uses */
  saveConfig: (config: AppConfig) => invoke<AppConfig>("save_config", { config }),

  /** last payload written to disk - null on a first run */
  getCachedWeather: () => invoke<WeatherCache | null>("get_cached_weather"),

  /** force-refresh weather from QWeather */
  fetchWeather: () => invoke<WeatherPayload>("fetch_weather"),
  /** try an unsaved key / host / city combo without storing it */
  probeWeather: (host: string, key: string, locationId: string) =>
    invoke<WeatherPayload>("probe_weather", { host, key, locationId }),
  /** search a city by keyword -> LocationID list */
  lookupCity: (keyword: string, key?: string) =>
    invoke<GeoCity[]>("lookup_city", { keyword, key: key ?? null }),

  openSettings: () => invoke<void>("open_settings_window"),

  /**
   * Fit a widget window's height to its scaled content. Called by `useStage`
   * whenever the measured content height changes; the backend only touches the
   * height so it can never perturb the width the zoom is derived from.
   */
  fitWidgetHeight: (label: "monitor" | "weather", height: number) =>
    invoke<void>("fit_widget_height", { label, height }),
  /** remember a width the user dragged to, without resizing anything */
  rememberWidgetWidth: (label: "monitor" | "weather", width: number) =>
    invoke<void>("remember_widget_width", { label, width }),
  /** set the width from the settings slider: resizes *and* persists */
  setWidgetWidth: (label: "monitor" | "weather", width: number) =>
    invoke<void>("set_widget_width", { label, width }),

  /** forget the remembered positions and park both widgets back in the corner */
  resetWidgetPositions: () => invoke<void>("reset_widget_positions"),

  /**
   * Float a widget above every other window, or drop it back into the desktop
   * layer. Persisted per window.
   */
  setWidgetAlwaysOnTop: (label: "monitor" | "weather", onTop: boolean) =>
    invoke<void>("set_widget_always_on_top", { label, onTop }),

  /**
   * "调整" mode. Widgets are locked (not movable, not resizable) by default;
   * this unlocks both and lifts the window so it can be worked on.
   */
  setWidgetAdjust: (label: "monitor" | "weather", adjusting: boolean) =>
    invoke<void>("set_widget_adjust", { label, adjusting }),

  /**
   * Shift a widget by a few logical pixels - the arrow-key nudge. Distances
   * are logical, so a step is a step at any display scaling.
   */
  nudgeWidget: (label: "monitor" | "weather", dx: number, dy: number) =>
    invoke<void>("nudge_widget", { label, dx, dy }),

  toggleWidget: (label: string, visible: boolean) =>
    invoke<void>("toggle_widget", { label, visible }),
  quitApp: () => invoke<void>("quit_app"),

  /**
   * Open a URL in the user's default browser. The backend rejects anything
   * that is not `http(s)`, so a corrupted config can never turn this into a
   * way to launch an arbitrary local program.
   */
  openUrl: (url: string) => invoke<void>("open_url", { url }),

  /** Launch both widgets on login (Windows per-user `Run` entry). */
  setAutostart: (enabled: boolean) => invoke<boolean>("set_autostart", { enabled }),
  getAutostart: () => invoke<boolean>("get_autostart"),
};

export function onSnapshot(cb: (s: Snapshot) => void): Promise<UnlistenFn> {
  return listen<Snapshot>(SNAPSHOT_EVENT, (e) => cb(e.payload));
}

export function onWeather(cb: (w: WeatherPayload) => void): Promise<UnlistenFn> {
  return listen<WeatherPayload>(WEATHER_EVENT, (e) => cb(e.payload));
}

export function onWeatherError(cb: (msg: string) => void): Promise<UnlistenFn> {
  return listen<string>(WEATHER_ERROR_EVENT, (e) => cb(e.payload));
}

export function onConfig(cb: (c: AppConfig) => void): Promise<UnlistenFn> {
  return listen<AppConfig>(CONFIG_EVENT, (e) => cb(e.payload));
}
