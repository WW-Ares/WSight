// Shared data contracts between the Rust backend and both widget windows.

export interface CpuInfo {
  /** total load percentage, 0..100 */
  load: number;
  /** logical processors - the "20T" of a 10-core part */
  cores: number;
  /** physical cores, 0 when the platform will not say */
  physicalCores: number;
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
  /** "DDR4" and friends; empty when the firmware does not say */
  ddrType: string;
  /** how many slots are populated */
  stickCount: number;
  /** capacity of one module in MB */
  stickMb: number;
  vendor: string;
  partNo: string;
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
  /** core clock in MHz, 0 when the driver refuses the query */
  coreClockMhz: number;
  /** memory clock in MHz, 0 under the same caveat */
  memClockMhz: number;
}

export interface NetInfo {
  name: string;
  /** bytes per second, smoothed */
  rxSec: number;
  txSec: number;
  /** cumulative bytes since boot */
  rxTotal: number;
  txTotal: number;
  /** IPv4 of this adapter; empty when the link is down or unconfigured */
  ipv4: string;
  /** negotiated link speed in Mbit/s, 0 when the driver will not say */
  linkMbps: number;
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
  /** mainboard model from the firmware; empty when the table will not say */
  board: string;
  /**
   * Running process count. Refreshed every few seconds rather than every
   * sample - enumerating the process table at 1 Hz is not worth the cost.
   */
  procCount: number;
}

/**
 * What the caption line under each gauge may show. The ids are the contract
 * with the backend: `src-tauri/src/config.rs` holds the same lists and drops
 * anything it does not recognise back to the default.
 */
export const L2_CPU_OPTIONS = [
  { id: "model", name: "CPU 型号" },
  { id: "extremes", name: "核心高低占用" },
  { id: "cores", name: "核心 / 线程" },
  { id: "procs", name: "进程数" },
  { id: "board", name: "主板型号" },
] as const;

export const L2_MEM_OPTIONS = [
  { id: "ddr", name: "类型 + 频率" },
  { id: "speed", name: "DDR 频率" },
  { id: "sticks", name: "插槽 条数 × 单条" },
  // Not "内存型号": the caption only shows the part number when it fits, and
  // falls back to `金士顿 D4 3200` when it does not. The number stays on the
  // tooltip either way, so the label has to describe both.
  { id: "partno", name: "内存品牌 · 规格" },
  { id: "board", name: "主板型号" },
] as const;

export const L2_GPU_OPTIONS = [
  { id: "model", name: "GPU 型号" },
  { id: "thermal", name: "温度 · 功耗" },
  { id: "clocks", name: "核心 / 显存频率" },
  { id: "fan", name: "温度 · 风扇" },
  { id: "board", name: "主板型号" },
] as const;

export const L2_NET_OPTIONS = [
  { id: "total", name: "累计用量" },
  { id: "ip", name: "本机 IP" },
  { id: "link", name: "链路速率" },
  { id: "board", name: "主板型号" },
] as const;

/** The choice each column starts out on - the captions as they were before. */
export const L2_DEFAULTS = {
  cpu: "extremes",
  mem: "speed",
  gpu: "thermal",
  net: "total",
} as const;

/**
 * The half of a snapshot that survives a reboot, persisted to
 * `monitor-cache.json` so the panel can open on the machine it is running on
 * instead of on placeholders.
 *
 * Anything that has to be *measured* - load, free memory, throughput - is
 * absent by design; the panel fills those with 0 until the first sample.
 */
export interface MonitorCache {
  savedAtMs: number;
  cpuBrand: string;
  cpuCores: number;
  /** physical cores, for the `10C 20T` caption */
  cpuPhysicalCores: number;
  /** rated clock; the live one is a measurement, so it is never stored */
  cpuFreqMhz: number;
  memTotal: number;
  memSwapTotal: number;
  memSpeedMhz: number;
  memDdrType: string;
  memStickCount: number;
  memStickMb: number;
  memVendor: string;
  memPartNo: string;
  gpu: { name: string; memTotal: number } | null;
  /** the adapter that carried traffic last session */
  netName: string | null;
  /** its negotiated link speed - but never its address, which belongs to one
   *  session and would be worse than blank if it came back stale */
  netLinkMbps: number;
  /** mainboard model, for the "board" caption option */
  board: string;
  disks: {
    letter: string;
    name: string;
    mountPoint: string;
    total: number;
    /** kept: a volume's used space moves slowly enough to stay honest */
    used: number;
    percent: number;
    isSsd: boolean | null;
    device: number;
  }[];
  drives: {
    device: number;
    isSsd: boolean | null;
    letters: string;
    label: string;
  }[];
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

/**
 * A payload plus the moment it was stored. The weather window paints this
 * before the first network round trip comes back, and greys the timestamp
 * once it is old enough to be misleading.
 */
export interface WeatherCache {
  cachedAtMs: number;
  payload: WeatherPayload;
}

/** Older than this and the observation time is dimmed as a warning. */
export const WEATHER_STALE_MS = 30 * 60 * 1000;

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
  /** QWeather lifestyle index id rendered under the temperature, 1..16 */
  weatherAdviceType: number;
  /** forecast cells on one row before it scrolls sideways, 3..5 */
  weatherForecastCols: number;
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
  /**
   * Draw the caption line under each gauge. Off drops the whole row - including
   * the network column's "用量" total, which is a caption rather than a
   * second figure - and the panel gets shorter.
   */
  monitorSecondLine: boolean;
  /** What each column's caption says; see `L2_*_OPTIONS`. */
  monitorL2Cpu: string;
  monitorL2Mem: string;
  monitorL2Gpu: string;
  monitorL2Net: string;

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
  /**
   * Look for a new release in the background and download it ready to install.
   *
   * Staging the download is all this does - nothing is ever replaced without
   * the user pressing 重启更新 in the settings window, because swapping the exe
   * restarts the app and that is not something to do behind someone's back.
   */
  updateAuto: boolean;
  /**
   * Unix seconds of the last check, so a restart does not re-check inside the
   * six-hour window. Written by the backend only.
   */
  updateLastCheck: number;
  /**
   * Version downloaded and waiting for a restart. Survives a relaunch, which
   * is what keeps the "已就绪" prompt on screen until it is acted on.
   */
  updateStagedVersion: string;
  /**
   * Diagnostic: puts the real rendering fps and the observed snapshot
   * interval into the monitor window's title. Flip `debugFps` in config.json
   * by hand; deliberately absent from the settings UI.
   */
  debugFps: boolean;
}

/**
 * What the updater knows right now, as `update_status` reports it.
 *
 * `state` is the whole state machine: `idle` -> `checking` -> `uptodate` |
 * `available` | `error`, and `downloading` -> `ready` when a build is staged.
 */
export interface UpdateStatus {
  state:
    | "idle"
    | "checking"
    | "uptodate"
    | "available"
    | "downloading"
    | "ready"
    | "error";
  /** the version this build is */
  current: string;
  /** version offered by the last release we looked at */
  latest: string;
  /** release notes, shown in the settings window */
  notes: string;
  /** 0..100 while `state === "downloading"` */
  progress: number;
  /** reason for `state === "error"` */
  error: string;
  /** version already staged and waiting for a restart */
  staged: string;
  /** false when the release carried no digest and only the size was checked */
  verified: boolean;
}

/** Width bounds the settings sliders and the native clamps agree on. */
export const MIN_WIDGET_WIDTH = 210;
export const MAX_WIDGET_WIDTH = 640;
/** Window height bounds - must match MAX_HEIGHT in src-tauri/src/config.rs. */
export const MAX_WIDGET_HEIGHT = 900;

/** How many forecast days the weather card may show - the setting's range. */
export const WEATHER_DAYS_MIN = 3;
export const WEATHER_DAYS_MAX = 7;
/** Forecast cells on one row before the row scrolls sideways. */
export const FORECAST_COLS_DEFAULT = 3;
export const FORECAST_COLS_MIN = 3;
export const FORECAST_COLS_MAX = 5;

/** How many volume tiles the monitor's disk block holds. */
export const MAX_DISK_TILES = 3;
/** How many physical drives the monitor's throughput block holds. */
export const MAX_DRIVES = 2;

export const RING_DEFAULTS = {
  cpu: "#ff9f57",
  mem: "#52d3a4",
  gpu: "#b98cff",
  net: "#67d3ff",
} as const;

/**
 * Fill colour of the disk capacity bars. Deliberately *not* the CPU accent:
 * sharing one hue made the disk block read as another CPU figure.
 */
export const DISK_BAR_COLOR = "#4aa8ff";

/**
 * The lifestyle indices QWeather exposes at `/v7/indices/1d?type=`, in id
 * order. The user picks one; its `text` is the sentence under the temperature.
 */
export const ADVICE_TYPES: ReadonlyArray<{ id: number; name: string }> = [
  { id: 1, name: "运动指数" },
  { id: 2, name: "洗车指数" },
  { id: 3, name: "穿衣指数" },
  { id: 4, name: "钓鱼指数" },
  { id: 5, name: "紫外线指数" },
  { id: 6, name: "旅游指数" },
  { id: 7, name: "花粉过敏指数" },
  { id: 8, name: "舒适度指数" },
  { id: 9, name: "感冒指数" },
  { id: 10, name: "空气污染扩散指数" },
  { id: 11, name: "空调开启指数" },
  { id: 12, name: "太阳镜指数" },
  { id: 13, name: "化妆指数" },
  { id: 14, name: "晾晒指数" },
  { id: 15, name: "交通指数" },
  { id: 16, name: "防晒指数" },
];

export const DEFAULT_CONFIG: AppConfig = {
  qweatherKey: "",
  qweatherHost: "https://devapi.qweather.com",
  locationId: "",
  locationName: "",
  weatherRefreshMin: 15,
  weatherDays: 3,
  weatherAdviceType: 8,
  weatherForecastCols: 3,
  showWeather: true,

  monitorIntervalMs: 1000,
  showMonitor: true,
  monitorShowGpu: true,
  monitorShowNet: true,
  monitorShowDisk: true,
  monitorDisks: [],
  monitorDrives: [],
  monitorSecondLine: true,
  monitorL2Cpu: L2_DEFAULTS.cpu,
  monitorL2Mem: L2_DEFAULTS.mem,
  monitorL2Gpu: L2_DEFAULTS.gpu,
  monitorL2Net: L2_DEFAULTS.net,

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

  updateAuto: true,
  updateLastCheck: 0,
  updateStagedVersion: "",

  debugFps: false,
};

export interface GeoCity {
  id: string;
  name: string;
  adm1: string;
  adm2: string;
  country: string;
}
