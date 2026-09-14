// Shared data contracts between the Rust backend and both widget windows.

export interface CpuInfo {
  /** total load percentage, 0..100 */
  load: number;
  /** logical core count */
  cores: number;
  /** per-core load percentages, 0..100 */
  perCore: number[];
  /**
   * Rated clock in MHz. Constant - it is what `freqLiveMhz` is expressed
   * against, and stands in for it when the live figure is unavailable.
   */
  freqMhz: number;
  /**
   * Clock the CPU is actually running at, in MHz, from the PDH
   * "% Processor Performance" counter. Boosts above `freqMhz`, and falls back
   * to 0 when the platform does not implement the counter.
   */
  freqLiveMhz: number;
  brand: string;
}

export interface MemInfo {
  /** bytes */
  total: number;
  /** bytes */
  used: number;
  /** bytes */
  free: number;
  /** 0..100 */
  percent: number;
  /** bytes */
  swapTotal: number;
  /** bytes */
  swapUsed: number;
  /** installed DDR speed in MHz, 0 when the firmware table is unreadable */
  speedMhz: number;
}

export interface GpuInfo {
  name: string;
  /** 0..100, -1 when unavailable */
  load: number;
  /** bytes, 0 when unknown */
  memTotal: number;
  /** bytes, 0 when unknown */
  memUsed: number;
  /** celsius, null when unavailable */
  tempC: number | null;
  /** 0..100, -1 when unavailable */
  fanPercent: number;
  /** board power draw in watts, null when the driver hides it */
  powerW: number | null;
}

export interface NetInfo {
  name: string;
  /** bytes per second, smoothed */
  rxSec: number;
  txSec: number;
  /** cumulative bytes since boot */
  rxTotal: number;
  txTotal: number;
}

export interface DiskInfo {
  /** volume label, e.g. "Win11" / "软件"; empty when the volume has none */
  name: string;
  mountPoint: string;
  /** drive letter, e.g. "C:" - always present, letterless partitions are filtered out */
  letter: string;
  total: number;
  used: number;
  percent: number;
  /** live throughput in bytes per second, for this volume alone */
  readSec: number;
  writeSec: number;
  /** true = solid state, false = spinning, null = the bus would not say */
  isSsd: boolean | null;
  /** physical disk behind this volume; volumes sharing it are one drive */
  device: number;
}

/**
 * One *physical* drive. The OS counters are per volume, so a drive's figures
 * are the sum over its volumes - which is what "the SSD's throughput" means
 * when one 932 GB NVMe is split into C: and D:.
 */
export interface DriveInfo {
  device: number;
  isSsd: boolean | null;
  /** drive letters on this disk, smallest first, e.g. "C/D" or "E" */
  letters: string;
  /** shortest non-empty volume label on the drive, for the tooltip */
  label: string;
  readSec: number;
  writeSec: number;
}

export interface Snapshot {
  /** epoch millis when the sample was produced */
  ts: number;
  uptimeSec: number;
  cpu: CpuInfo;
  mem: MemInfo;
  gpu: GpuInfo | null;
  nets: NetInfo[];
  /** one entry per volume with a drive letter, ordered by letter */
  disks: DiskInfo[];
  /** one entry per physical drive, ordered by its first letter */
  drives: DriveInfo[];
}

export interface WeatherNow {
  obsTime: string;
  temp: string;
  feelsLike: string;
  icon: string;
  text: string;
  windDir: string;
  windScale: string;
  humidity: string;
  precip: string;
  pressure: string;
  vis: string;
}

export interface WeatherDaily {
  fxDate: string;
  tempMax: string;
  tempMin: string;
  iconDay: string;
  textDay: string;
  iconNight: string;
  textNight: string;
  sunrise: string;
  sunset: string;
  precip: string;
  uvIndex: string;
}

export interface WeatherAdvice {
  /** e.g. 交通指数 */
  name: string;
  /** e.g. 良好 */
  category: string;
  /** the full sentence shown under the current conditions */
  text: string;
}

export interface WeatherPayload {
  locationName: string;
  now: WeatherNow;
  daily: WeatherDaily[];
  updateTime: string;
  /** optional lifestyle advice line; null when the plan has no access to it */
  advice: WeatherAdvice | null;
}

export interface AppConfig {
  // ------------------------------------------------------------- weather
  /** QWeather API key */
  qweatherKey: string;
  /** QWeather data API base URL, e.g. https://devapi.qweather.com */
  qweatherHost: string;
  /** QWeather LocationID (e.g. 101010100) */
  locationId: string;
  /** display name, resolved from lookup or user input */
  locationName: string;
  /** weather refresh interval in minutes */
  weatherRefreshMin: number;
  /** how many forecast cells the weather window shows, 1..7 */
  weatherDays: number;
  showWeather: boolean;

  // ------------------------------------------------------------- monitor
  /** monitor sampling interval in ms (min 500) */
  monitorIntervalMs: number;
  showMonitor: boolean;
  monitorShowGpu: boolean;
  monitorShowNet: boolean;
  /**
   * Show the disk block under the gauges: one tile per volume on the left,
   * the physical drives' throughput on the right.
   */
  monitorShowDisk: boolean;
  /** drive letters the disk block shows, in order; empty = first three */
  monitorDisks: string[];
  /** physical disk numbers the throughput block shows; empty = first two */
  monitorDrives: number[];

  // ---------------------------------------------------------- appearance
  /** UI theme: dark | light */
  theme: "dark" | "light";
  /** whole-panel opacity, 0.25..1 */
  opacity: number;
  /** panel corner radius in px, 0..24 */
  radius: number;
  colorCpu: string;
  colorMem: string;
  colorGpu: string;
  colorNet: string;

  // ---------------------------------------------------------------- size
  /** logical width of the monitor window; every widget shares one base width */
  monitorWidth: number;
  /** logical width of the weather window */
  weatherWidth: number;

  // ------------------------------------------------------------ position
  /**
   * Remembered top-left corner of each floating window, in logical px.
   * `null` means the widget has never been placed, so the app parks it in its
   * own default corner instead.
   */
  monitorX: number | null;
  monitorY: number | null;
  weatherX: number | null;
  weatherY: number | null;

  // ----------------------------------------------------------- behaviour
  /**
   * Float the window above everything else. `false` - the default - drops it
   * to the desktop layer, behind every ordinary window. Set per window: the
   * right-click menu of each widget toggles its own.
   */
  monitorAlwaysOnTop: boolean;
  weatherAlwaysOnTop: boolean;

  // ---------------------------------------------------------------- misc
  /** reserved: start both widgets on login */
  autostart: boolean;
}

/** Width bounds the settings sliders and the native clamps agree on. */
export const MIN_WIDGET_WIDTH = 210;
export const MAX_WIDGET_WIDTH = 640;
/** Window height bounds - must match MAX_HEIGHT in src-tauri/src/config.rs. */
export const MAX_WIDGET_HEIGHT = 900;

/** How many volume tiles the monitor's disk block holds. */
export const MAX_DISK_TILES = 3;
/** How many physical drives the monitor's throughput block holds. */
export const MAX_DRIVES = 2;

export const RING_DEFAULTS = {
  cpu: "#4aa8ff",
  mem: "#52d3a4",
  gpu: "#b98cff",
  net: "#67d3ff",
} as const;

export const DEFAULT_CONFIG: AppConfig = {
  qweatherKey: "",
  qweatherHost: "https://devapi.qweather.com",
  locationId: "",
  locationName: "",
  weatherRefreshMin: 15,
  weatherDays: 3,
  showWeather: true,

  monitorIntervalMs: 1000,
  showMonitor: true,
  monitorShowGpu: true,
  monitorShowNet: true,
  monitorShowDisk: true,
  monitorDisks: [],
  monitorDrives: [],

  theme: "dark",
  opacity: 0.92,
  radius: 14,
  colorCpu: RING_DEFAULTS.cpu,
  colorMem: RING_DEFAULTS.mem,
  colorGpu: RING_DEFAULTS.gpu,
  colorNet: RING_DEFAULTS.net,

  monitorWidth: 300,
  weatherWidth: 300,

  monitorX: null,
  monitorY: null,
  weatherX: null,
  weatherY: null,

  monitorAlwaysOnTop: false,
  weatherAlwaysOnTop: false,

  autostart: false,
};

export interface GeoCity {
  id: string;
  name: string;
  adm1: string;
  adm2: string;
  country: string;
}
