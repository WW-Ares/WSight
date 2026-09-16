import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { api, onConfig } from "../shared/api";
import { applyTheme } from "../shared/theme";
import {
  APP_VERSION_LABEL,
  ISSUES_URL,
  RELEASES_URL,
  REPO_URL,
} from "../shared/about";
import {
  ADVICE_TYPES,
  DEFAULT_CONFIG,
  L2_CPU_OPTIONS,
  L2_GPU_OPTIONS,
  L2_MEM_OPTIONS,
  L2_NET_OPTIONS,
  MAX_DRIVES,
  MAX_DISK_TILES,
  MAX_WIDGET_WIDTH,
  MIN_WIDGET_WIDTH,
  RING_DEFAULTS,
  type AppConfig,
  type GeoCity,
  type Snapshot,
  type UpdateStatus,
} from "../shared/types";
import "./settings.css";

const HOST_PRESETS = [
  { value: "https://devapi.qweather.com", label: "devapi.qweather.com（免费订阅）" },
  { value: "https://api.qweather.com", label: "api.qweather.com（商业/付费订阅）" },
];
const CUSTOM_HOST = "__custom__";

const INTERVAL_PRESETS = [500, 1000, 2000, 5000, 10000];
const REFRESH_PRESETS = [5, 10, 15, 30, 60, 120];

type Status = { kind: "idle" | "ok" | "err"; text: string };

/** One pickable caption option, as the four option lists declare them. */
type L2Option = { readonly id: string; readonly name: string };

// ------------------------------------------------------------- primitives

function Section({
  title,
  desc,
  children,
}: {
  title: string;
  desc?: string;
  children: ReactNode;
}) {
  return (
    <section className="st-card">
      <div className="st-card-head">
        <h2>{title}</h2>
        {desc ? <span>{desc}</span> : null}
      </div>
      <div className="st-card-body">{children}</div>
    </section>
  );
}

/**
 * A label on the left, its control on the right.
 *
 * Anything explanatory goes into the `ⓘ` beside the label rather than into a
 * line of its own: a settings window is read once to find a control and then
 * never again, so every sentence under a row is noise that pushes the next
 * control further down.
 */
function Row({
  label,
  hint,
  value,
  children,
}: {
  label: string;
  hint?: string;
  /** shown next to the label in dim figures, e.g. `76%` */
  value?: string;
  children?: ReactNode;
}) {
  return (
    <div className="st-row">
      <span className="st-row-label">
        {label}
        {value ? <b className="st-row-value">{value}</b> : null}
        {hint ? (
          <i className="st-info" title={hint}>
            i
          </i>
        ) : null}
      </span>
      <div className="st-row-ctl">{children}</div>
    </div>
  );
}

/** A row whose control is a range input - the common case in 外观. */
function SliderRow({
  label,
  hint,
  value,
  display,
  min,
  max,
  step = 1,
  onChange,
}: {
  label: string;
  hint?: string;
  value: number;
  display: string;
  min: number;
  max: number;
  step?: number;
  onChange: (value: number) => void;
}) {
  return (
    <Row label={label} hint={hint} value={display}>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(e) => onChange(Number(e.target.value))}
      />
    </Row>
  );
}

function Toggle({
  checked,
  onChange,
}: {
  checked: boolean;
  onChange: (value: boolean) => void;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      className={`st-switch${checked ? " on" : ""}`}
      onClick={() => onChange(!checked)}
    >
      <span className="st-switch-knob" />
    </button>
  );
}

function MiniRing({
  value,
  color,
  label,
}: {
  value: number;
  color: string;
  label: string;
}) {
  const size = 46;
  const stroke = 5;
  const radius = (size - stroke) / 2;
  const circumference = 2 * Math.PI * radius;
  const pct = Math.max(0, Math.min(100, value));
  const dash = (pct / 100) * circumference;

  return (
    <div className="st-mini">
      <div className="st-mini-graphic" style={{ width: size, height: size }}>
        <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`}>
          <circle
            cx={size / 2}
            cy={size / 2}
            r={radius}
            fill="none"
            stroke="var(--ring-track)"
            strokeWidth={stroke}
          />
          <circle
            cx={size / 2}
            cy={size / 2}
            r={radius}
            fill="none"
            stroke={color}
            strokeWidth={stroke}
            strokeLinecap="round"
            strokeDasharray={`${dash} ${circumference - dash}`}
            transform={`rotate(-90 ${size / 2} ${size / 2})`}
          />
        </svg>
        <span className="st-mini-value">{pct.toFixed(0)}</span>
      </div>
      <span className="st-mini-label">{label}</span>
    </div>
  );
}

function ColorField({
  label,
  value,
  hint,
  onChange,
}: {
  label: string;
  value: string;
  hint: string;
  onChange: (value: string) => void;
}) {
  return (
    <div className="st-color" title={hint}>
      <input
        type="color"
        value={value}
        onChange={(e) => onChange(e.target.value)}
      />
      <span>{label}</span>
    </div>
  );
}

/** One column's caption picker, from one of the `L2_*_OPTIONS` lists. */
function L2Row({
  label,
  value,
  options,
  disabled,
  onChange,
}: {
  label: string;
  value: string;
  options: ReadonlyArray<L2Option>;
  disabled: boolean;
  onChange: (value: string) => void;
}) {
  return (
    <div className="st-row">
      <span className="st-row-label">{label}</span>
      <div className="st-row-ctl">
        <select
          value={value}
          disabled={disabled}
          onChange={(e) => onChange(e.target.value)}
        >
          {options.map((o) => (
            <option key={o.id} value={o.id}>
              {o.name}
            </option>
          ))}
        </select>
      </div>
    </div>
  );
}

function intervalOptions(current: number): number[] {
  return INTERVAL_PRESETS.includes(current)
    ? INTERVAL_PRESETS
    : [...INTERVAL_PRESETS, current].sort((a, b) => a - b);
}

function refreshOptions(current: number): number[] {
  return REFRESH_PRESETS.includes(current)
    ? REFRESH_PRESETS
    : [...REFRESH_PRESETS, current].sort((a, b) => a - b);
}

/**
 * One line describing what the updater is doing, in the user's terms.
 *
 * `auto` only matters in the resting states: with the switch off, "尚未检查"
 * would read as a fault where "自动更新已关闭" reads as a choice.
 */
function updateText(s: UpdateStatus | null, auto: boolean): string {
  if (!s) return auto ? "启动后自动检查" : "自动更新已关闭";
  switch (s.state) {
    case "checking":
      return "正在检查…";
    case "downloading":
      return `正在下载 v${s.latest} · ${Math.floor(s.progress)}%`;
    case "uptodate":
      return "已是最新版本";
    case "available":
      return `发现新版本 v${s.latest}——自动更新已关闭，点右边手动下载`;
    case "ready":
      // Three cases, not two: after a restart the digest result is gone, and
      // claiming either way would put a false statement in front of the user.
      if (s.verified === true) {
        return `v${s.staged} 已下载并校验通过，重启即可完成更新`;
      }
      if (s.verified === false) {
        return `v${s.staged} 已下载（该版本未提供校验值，仅核对文件大小）`;
      }
      return `v${s.staged} 已下载，重启即可完成更新`;
    case "error":
      return s.error;
    default:
      return auto ? "启动后自动检查" : "自动更新已关闭";
  }
}

/** Which colour the status dot wears. */
function updateTone(s: UpdateStatus | null): string {
  if (!s) return "idle";
  switch (s.state) {
    case "ready":
      return "ok";
    case "available":
      return "new";
    case "error":
      return "err";
    case "checking":
    case "downloading":
      return "busy";
    default:
      return "idle";
  }
}

function formatBytes(n: number): string {
  if (!Number.isFinite(n) || n <= 0) return "0G";
  const units = ["B", "K", "M", "G", "T"];
  let v = n;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i += 1;
  }
  const digits = i >= 3 ? (v >= 100 ? 0 : 1) : 0;
  return `${v.toFixed(digits)}${units[i]}`;
}

/**
 * A chip per real volume / physical disk, each one an independent switch.
 * Selection order is preserved because the widget renders the tiles in that
 * same order. Picking a fresh item while already at `limit` drops the oldest
 * one instead of silently doing nothing - a full picker still feels alive.
 */
function DiskPickers({
  snap,
  mode,
  selected,
  limit,
  onChange,
}: {
  snap: Snapshot | null;
  mode: "volume" | "drive";
  selected: string[];
  limit: number;
  onChange: (next: string[]) => void;
}) {
  const items = (() => {
    if (!snap) return [];
    if (mode === "volume") {
      return snap.disks.map((d) => ({
        key: d.letter,
        title: d.letter,
        sub: d.name || "本地磁盘",
        extra: d.total > 0 ? formatBytes(d.total) : "",
      }));
    }
    return snap.drives.map((d) => ({
      key: String(d.device),
      title: `磁盘 ${d.device}`,
      sub: d.letters
        ? `${d.isSsd === true ? "固态" : d.isSsd === false ? "机械" : "磁盘"} · ${d.letters}`
        : "物理硬盘",
      extra: "",
    }));
  })();

  if (!snap) {
    return <span className="st-chips-empty">正在读取磁盘…</span>;
  }
  if (!items.length) {
    return <span className="st-chips-empty">没有可用项</span>;
  }

  // An empty config means "automatic": the widget falls back to the first few
  // items. Mirror that here, otherwise the picker would look like nothing is
  // switched on while the panel happily shows three volumes.
  const effective = selected.length
    ? selected
    : items.slice(0, limit).map((item) => item.key);

  const toggle = (key: string) => {
    if (effective.includes(key)) {
      onChange(effective.filter((k) => k !== key));
      return;
    }
    const next = [...effective, key];
    onChange(next.length > limit ? next.slice(next.length - limit) : next);
  };

  return (
    <div className="st-chips">
      {items.map((item) => (
        <button
          key={item.key}
          type="button"
          className={`st-chip${effective.includes(item.key) ? " on" : ""}`}
          onClick={() => toggle(item.key)}
          title={`${item.sub}${item.extra ? ` · ${item.extra}` : ""}`}
        >
          <b>{item.title}</b>
          <span>{item.extra || item.sub}</span>
        </button>
      ))}
    </div>
  );
}

// ------------------------------------------------------------------ panel

export function SettingsPanel({ initial }: { initial: AppConfig }) {
  const [draft, setDraft] = useState<AppConfig>(initial);
  const draftRef = useRef<AppConfig>(initial);
  const timer = useRef<number | null>(null);

  const [status, setStatus] = useState<Status>({
    kind: "idle",
    text: "改动自动保存",
  });

  /**
   * Hand a URL to the OS browser. A Tauri webview has no way to open an
   * external link itself, and the failure is worth surfacing: if the shell
   * refuses, the button would otherwise look broken for no visible reason.
   */
  const open = (url: string) => {
    void api.openUrl(url).catch((e: unknown) => {
      setStatus({ kind: "err", text: `打开链接失败：${String(e)}` });
    });
  };

  const [showKey, setShowKey] = useState(false);
  // The collector caches the last sample; reusing it here gives us the real
  // volume / physical-drive inventory without a second probe path.
  const [snap, setSnap] = useState<Snapshot | null>(null);
  const [keyword, setKeyword] = useState("");
  const [cities, setCities] = useState<GeoCity[]>([]);
  const [searchState, setSearchState] = useState<string | null>(null);
  const [searching, setSearching] = useState(false);
  const [testState, setTestState] = useState<Status | null>(null);
  const [testing, setTesting] = useState(false);
  const [refreshing, setRefreshing] = useState(false);

  // ------------------------------------------------------------ updates

  const [upd, setUpd] = useState<UpdateStatus | null>(null);
  const [updBusy, setUpdBusy] = useState(false);

  /**
   * Read the status once on open. A build staged before the last restart is
   * still on disk - the backend seeds it from the config at start-up - so the
   * "重启更新" prompt comes back instead of the user having to check again.
   */
  useEffect(() => {
    let alive = true;
    api
      .updateStatus()
      .then((s) => {
        if (alive) setUpd(s);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);

  // A check or a download runs on the Rust side and can take a while; there is
  // no event stream for it, so poll while one is in flight and stop otherwise.
  // 400ms is fast enough for the progress bar to look alive and slow enough to
  // cost nothing (the status is a mutex read, no I/O).
  const updState = upd?.state ?? "";
  useEffect(() => {
    if (updState !== "checking" && updState !== "downloading") return;
    const id = window.setInterval(() => {
      api
        .updateStatus()
        .then(setUpd)
        .catch(() => {});
    }, 400);
    return () => window.clearInterval(id);
  }, [updState]);

  const checkNow = () => {
    setUpdBusy(true);
    api
      .checkUpdate(true)
      .then((s) => setUpd(s))
      .catch((e: unknown) =>
        setStatus({ kind: "err", text: `检查更新失败：${String(e)}` }),
      )
      .finally(() => setUpdBusy(false));
  };

  const restartNow = () => {
    const target = upd?.staged || upd?.latest || "新版本";
    if (
      !window.confirm(
        `现在重启并安装 ${target}？两个悬浮窗会关闭一下，然后自动回来。`,
      )
    ) {
      return;
    }
    setStatus({ kind: "idle", text: "正在重启…" });
    // On success the app is gone before this resolves - the promise only ever
    // settles when the swap failed.
    void api.applyUpdate().catch((e: unknown) => {
      setStatus({ kind: "err", text: `更新失败：${String(e)}` });
      api
        .updateStatus()
        .then(setUpd)
        .catch(() => {});
    });
  };

  const discardNow = () => {
    api
      .discardUpdate()
      .then(() => api.updateStatus())
      .then(setUpd)
      .catch((e: unknown) =>
        setStatus({ kind: "err", text: `取消失败：${String(e)}` }),
      );
  };

  // ------------------------------------------------------- auto save

  const scheduleSave = useCallback((next: AppConfig) => {
    if (timer.current !== null) window.clearTimeout(timer.current);
    setStatus({ kind: "idle", text: "保存中…" });
    timer.current = window.setTimeout(() => {
      timer.current = null;
      api
        .saveConfig(next)
        .then(() =>
          setStatus({
            kind: "ok",
            text: `已保存 ${new Date().toLocaleTimeString("zh-CN", { hour12: false })}`,
          }),
        )
        .catch((e: unknown) =>
          setStatus({ kind: "err", text: `保存失败：${String(e)}` }),
        );
    }, 450);
  }, []);

  const update = useCallback(
    (patch: Partial<AppConfig>) => {
      const next = { ...draftRef.current, ...patch };
      draftRef.current = next;
      setDraft(next);
      // instantly preview colours / radius inside the settings window itself
      applyTheme(next);
      scheduleSave(next);
    },
    [scheduleSave],
  );

  useEffect(
    () => () => {
      if (timer.current !== null) window.clearTimeout(timer.current);
    },
    [],
  );

  // Dragging a widget writes its width straight back into the config. Mirror
  // just those two fields so the sliders are never stale - and never touch
  // anything the user might be typing.
  useEffect(() => {
    let off: (() => void) | undefined;
    onConfig((cfg) => {
      const cur = draftRef.current;
      if (
        Math.abs(cfg.monitorWidth - cur.monitorWidth) < 1 &&
        Math.abs(cfg.weatherWidth - cur.weatherWidth) < 1
      ) {
        return;
      }
      const next: AppConfig = {
        ...cur,
        monitorWidth: cfg.monitorWidth,
        weatherWidth: cfg.weatherWidth,
      };
      draftRef.current = next;
      setDraft(next);
    })
      .then((u) => {
        off = u;
      })
      .catch(() => {});
    return () => off?.();
  }, []);

  // The tray can toggle widgets while this window is hidden - re-read on focus.
  useEffect(() => {
    const onFocus = () => {
      if (timer.current !== null) return;
      api
        .getConfig()
        .then((cfg) => {
          draftRef.current = cfg;
          setDraft(cfg);
          applyTheme(cfg);
        })
        .catch(() => {});
    };
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, []);

  // Snapshot once so the disk pickers can list what actually exists. The
  // collector may not have produced a sample yet at cold start, so retry a
  // couple of times before giving up.
  useEffect(() => {
    let alive = true;
    let tries = 0;
    const load = () => {
      api
        .getSnapshot()
        .then((s) => {
          if (!alive) return;
          if (s) setSnap(s);
          else if (++tries < 6) window.setTimeout(load, 700);
        })
        .catch(() => {
          if (alive && ++tries < 6) window.setTimeout(load, 700);
        });
    };
    load();
    return () => {
      alive = false;
    };
  }, []);

  // ------------------------------------------------------- city lookup

  const searchCity = useCallback(async () => {
    const q = keyword.trim();
    if (!q) {
      setSearchState("请输入城市名，例如：北京 / 杭州 / Dongcheng");
      return;
    }
    setSearching(true);
    setSearchState(null);
    setCities([]);
    try {
      const found = await api.lookupCity(q, draftRef.current.qweatherKey);
      setCities(found);
      if (!found.length) setSearchState("没有匹配的城市");
    } catch (e) {
      setSearchState(String(e));
    } finally {
      setSearching(false);
    }
  }, [keyword]);

  const testConnection = useCallback(async () => {
    const cfg = draftRef.current;
    setTesting(true);
    setTestState(null);
    try {
      const w = await api.probeWeather(
        cfg.qweatherHost,
        cfg.qweatherKey,
        cfg.locationId,
      );
      setTestState({
        kind: "ok",
        text: `连通成功：${w.locationName} ${w.now.temp}° ${w.now.text}`,
      });
    } catch (e) {
      setTestState({ kind: "err", text: String(e) });
    } finally {
      setTesting(false);
    }
  }, []);

  const refreshNow = useCallback(async () => {
    setRefreshing(true);
    try {
      const w = await api.fetchWeather();
      setTestState({
        kind: "ok",
        text: `已刷新：${w.locationName} ${w.now.temp}° ${w.now.text}`,
      });
    } catch (e) {
      setTestState({ kind: "err", text: String(e) });
    } finally {
      setRefreshing(false);
    }
  }, []);

  const resetAppearance = useCallback(() => {
    const keep = draftRef.current;
    update({
      ...DEFAULT_CONFIG,
      // credentials, visibility and placement are user intent, never wiped by
      // a reset
      qweatherKey: keep.qweatherKey,
      qweatherHost: keep.qweatherHost,
      locationId: keep.locationId,
      locationName: keep.locationName,
      showMonitor: keep.showMonitor,
      showWeather: keep.showWeather,
      monitorX: keep.monitorX,
      monitorY: keep.monitorY,
      weatherX: keep.weatherX,
      weatherY: keep.weatherY,
    });
    // window sizes are not re-applied by the backend (useStage owns them)
    void api.setWidgetWidth("monitor", DEFAULT_CONFIG.monitorWidth).catch(() => {});
    void api.setWidgetWidth("weather", DEFAULT_CONFIG.weatherWidth).catch(() => {});
  }, [update]);

  /**
   * Widths are applied immediately instead of waiting for the debounced save:
   * the backend resizes the window and the widget's own scale driver then
   * derives the matching height.
   */
  const setWidgetWidth = useCallback(
    (label: "monitor" | "weather", value: number) => {
      update(label === "monitor" ? { monitorWidth: value } : { weatherWidth: value });
      void api.setWidgetWidth(label, value).catch(() => {});
    },
    [update],
  );

  /** Escape hatch for a widget dragged somewhere awkward - or onto a monitor
      that is no longer plugged in, where it could not be reached at all. */
  const resetPositions = useCallback(() => {
    api
      .resetWidgetPositions()
      .then(() => setStatus({ kind: "ok", text: "两个悬浮窗已回到默认位置" }))
      .catch((e: unknown) =>
        setStatus({ kind: "err", text: `重置失败：${String(e)}` }),
      );
  }, []);

  // -------------------------------------------------------------- render

  const hostIsPreset = HOST_PRESETS.some((p) => p.value === draft.qweatherHost);
  const l2On = draft.monitorSecondLine;

  return (
    <div className="st">
      <header className="st-head">
        <h1>
          WSight 设置
          <span className="st-ver" title={`WSight ${APP_VERSION_LABEL}`}>
            {APP_VERSION_LABEL}
          </span>
        </h1>
        <span className={`st-status ${status.kind}`}>{status.text}</span>
      </header>

      <div className="st-body">
        <Section title="外观">
          <div className="st-preview" style={{ borderRadius: draft.radius }}>
            <MiniRing value={37} color={draft.colorCpu} label="CPU" />
            <MiniRing value={62} color={draft.colorMem} label="MEM" />
            <MiniRing value={18} color={draft.colorGpu} label="GPU" />
            <div className="st-preview-right">
              <div className="st-seg">
                <button
                  type="button"
                  className={draft.theme === "dark" ? "on" : ""}
                  onClick={() => update({ theme: "dark" })}
                >
                  深色
                </button>
                <button
                  type="button"
                  className={draft.theme === "light" ? "on" : ""}
                  onClick={() => update({ theme: "light" })}
                >
                  浅色
                </button>
              </div>
              <div className="st-preview-net">
                <span style={{ color: draft.colorNet }}>↓ 1.2 MB/s</span>
                <span style={{ color: "#ffd479" }}>↑ 240 KB/s</span>
              </div>
            </div>
          </div>

          <SliderRow
            label="不透明度"
            hint="只作用于卡片背景：文字、圆环和图标始终全对比度，不会跟着变淡"
            value={Math.round(draft.opacity * 100)}
            display={`${Math.round(draft.opacity * 100)}%`}
            min={25}
            max={100}
            onChange={(v) => update({ opacity: v / 100 })}
          />

          <SliderRow
            label="圆角"
            value={Math.round(draft.radius)}
            display={`${Math.round(draft.radius)} px`}
            min={0}
            max={24}
            onChange={(v) => update({ radius: v })}
          />

          <SliderRow
            label="监控窗宽度"
            value={Math.round(draft.monitorWidth)}
            display={`${Math.round(draft.monitorWidth)} px`}
            min={MIN_WIDGET_WIDTH}
            max={MAX_WIDGET_WIDTH}
            step={2}
            onChange={(v) => setWidgetWidth("monitor", v)}
          />

          <SliderRow
            label="天气窗宽度"
            value={Math.round(draft.weatherWidth)}
            display={`${Math.round(draft.weatherWidth)} px`}
            min={MIN_WIDGET_WIDTH}
            max={MAX_WIDGET_WIDTH}
            step={2}
            onChange={(v) => setWidgetWidth("weather", v)}
          />

          <Row
            label="配色"
            hint="点击色块调整；两个悬浮窗共用的环形与网速颜色"
          >
            <div className="st-colors">
              <ColorField
                label="CPU"
                hint="CPU 环形颜色"
                value={draft.colorCpu}
                onChange={(v) => update({ colorCpu: v })}
              />
              <ColorField
                label="内存"
                hint="内存环形颜色"
                value={draft.colorMem}
                onChange={(v) => update({ colorMem: v })}
              />
              <ColorField
                label="显卡"
                hint="GPU 环形颜色"
                value={draft.colorGpu}
                onChange={(v) => update({ colorGpu: v })}
              />
              <ColorField
                label="网络"
                hint="网速文字颜色"
                value={draft.colorNet}
                onChange={(v) => update({ colorNet: v })}
              />
              <button
                type="button"
                className="st-btn ghost small"
                onClick={() =>
                  update({
                    colorCpu: RING_DEFAULTS.cpu,
                    colorMem: RING_DEFAULTS.mem,
                    colorGpu: RING_DEFAULTS.gpu,
                    colorNet: RING_DEFAULTS.net,
                  })
                }
              >
                默认
              </button>
            </div>
          </Row>

          <Row
            label="窗口位置"
            hint={
              "右键悬浮窗选「调整」后才能拖动；调整中方向键可微调 1px，按住 Shift 是 10px。" +
              "两个悬浮窗靠近时自动磁吸：上下、左右边缘对齐与中心对齐都会吸附，方向键微调不受磁吸影响。" +
              "位置会被记住，下次开在原处。"
            }
          >
            <button
              type="button"
              className="st-btn ghost small"
              onClick={resetPositions}
            >
              重置到默认位置
            </button>
          </Row>
        </Section>

        <Section title="监控面板">
          <Row label="显示监控面板">
            <Toggle
              checked={draft.showMonitor}
              onChange={(v) => update({ showMonitor: v })}
            />
          </Row>
          <Row label="窗口置顶" hint="关闭时沉在桌面最下层">
            <Toggle
              checked={draft.monitorAlwaysOnTop}
              onChange={(v) => update({ monitorAlwaysOnTop: v })}
            />
          </Row>
          <Row label="显示 GPU 环形" hint="非 NVIDIA 显卡只能读型号与显存总量">
            <Toggle
              checked={draft.monitorShowGpu}
              onChange={(v) => update({ monitorShowGpu: v })}
            />
          </Row>
          <Row label="显示网络速率">
            <Toggle
              checked={draft.monitorShowNet}
              onChange={(v) => update({ monitorShowNet: v })}
            />
          </Row>
          <Row label="磁盘块" hint="圆环下方：卷占用 + 硬盘读写">
            <Toggle
              checked={draft.monitorShowDisk}
              onChange={(v) => update({ monitorShowDisk: v })}
            />
          </Row>
          <Row
            label="显示哪些卷"
            hint={`逐个开关，最多 ${MAX_DISK_TILES} 个，选中的顺序就是排列顺序`}
          >
            <DiskPickers
              snap={snap}
              mode="volume"
              selected={draft.monitorDisks}
              limit={MAX_DISK_TILES}
              onChange={(monitorDisks) => update({ monitorDisks })}
            />
          </Row>
          <Row
            label="读写显示哪些盘"
            hint={`按物理硬盘分组，最多 ${MAX_DRIVES} 个`}
          >
            <DiskPickers
              snap={snap}
              mode="drive"
              selected={draft.monitorDrives.map(String)}
              limit={MAX_DRIVES}
              onChange={(picked) =>
                update({ monitorDrives: picked.map((s) => Number(s)) })
              }
            />
          </Row>
          <Row label="采样间隔" hint="越小越灵敏，占用略增">
            <select
              value={draft.monitorIntervalMs}
              onChange={(e) =>
                update({ monitorIntervalMs: Number(e.target.value) })
              }
            >
              {intervalOptions(draft.monitorIntervalMs).map((v) => (
                <option key={v} value={v}>
                  {v < 1000 ? `${v} 毫秒` : `${v / 1000} 秒`}
                </option>
              ))}
            </select>
          </Row>
        </Section>

        <Section title="第二行内容" desc="圆环下方那一行，逐列定制">
          <Row
            label="显示第二行"
            hint="关闭后面板变矮，网络列的用量一并取消"
          >
            <Toggle
              checked={draft.monitorSecondLine}
              onChange={(v) => update({ monitorSecondLine: v })}
            />
          </Row>
          <div className={`st-l2${l2On ? "" : " off"}`}>
            <L2Row
              label="CPU"
              value={draft.monitorL2Cpu}
              options={L2_CPU_OPTIONS}
              disabled={!l2On}
              onChange={(v) => update({ monitorL2Cpu: v })}
            />
            <L2Row
              label="内存"
              value={draft.monitorL2Mem}
              options={L2_MEM_OPTIONS}
              disabled={!l2On}
              onChange={(v) => update({ monitorL2Mem: v })}
            />
            <L2Row
              label="显卡"
              value={draft.monitorL2Gpu}
              options={L2_GPU_OPTIONS}
              disabled={!l2On}
              onChange={(v) => update({ monitorL2Gpu: v })}
            />
            <L2Row
              label="网络"
              value={draft.monitorL2Net}
              options={L2_NET_OPTIONS}
              disabled={!l2On}
              onChange={(v) => update({ monitorL2Net: v })}
            />
          </div>
        </Section>

        <Section title="天气窗口" desc="和风天气 QWeather">
          <Row label="显示天气窗口">
            <Toggle
              checked={draft.showWeather}
              onChange={(v) => update({ showWeather: v })}
            />
          </Row>
          <Row label="窗口置顶" hint="关闭时沉在桌面最下层">
            <Toggle
              checked={draft.weatherAlwaysOnTop}
              onChange={(v) => update({ weatherAlwaysOnTop: v })}
            />
          </Row>

          <Row label="API Key" hint="在和风天气控制台创建的项目 Key">
            <div className="st-inline">
              <input
                type={showKey ? "text" : "password"}
                value={draft.qweatherKey}
                placeholder="粘贴 API Key"
                spellCheck={false}
                onChange={(e) => update({ qweatherKey: e.target.value })}
              />
              <button
                type="button"
                className="st-btn ghost small"
                onClick={() => setShowKey((v) => !v)}
              >
                {showKey ? "隐藏" : "显示"}
              </button>
            </div>
          </Row>

          <Row label="API Host" hint="公共域名或你的专属 API Host">
            <div className="st-inline">
              <select
                value={hostIsPreset ? draft.qweatherHost : CUSTOM_HOST}
                onChange={(e) => {
                  const v = e.target.value;
                  if (v === CUSTOM_HOST) update({ qweatherHost: "" });
                  else update({ qweatherHost: v });
                }}
              >
                {HOST_PRESETS.map((p) => (
                  <option key={p.value} value={p.value}>
                    {p.label}
                  </option>
                ))}
                <option value={CUSTOM_HOST}>自定义…</option>
              </select>
            </div>
          </Row>

          {!hostIsPreset ? (
            <Row label="自定义 Host">
              <input
                type="text"
                value={draft.qweatherHost}
                placeholder="例如 abc.def.qweatherapi.com"
                spellCheck={false}
                onChange={(e) => update({ qweatherHost: e.target.value })}
              />
            </Row>
          ) : null}

          <Row label="城市" value={draft.locationName || "未选择"}>
            <div className="st-inline">
              <input
                type="text"
                value={keyword}
                placeholder="输入城市名后回车"
                onChange={(e) => setKeyword(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") void searchCity();
                }}
              />
              <button
                type="button"
                className="st-btn ghost small"
                disabled={searching}
                onClick={() => void searchCity()}
              >
                {searching ? "查询中" : "搜索"}
              </button>
            </div>
          </Row>

          {searchState ? <p className="st-note">{searchState}</p> : null}

          {cities.length ? (
            <ul className="st-cities">
              {cities.map((c) => (
                <li key={c.id}>
                  <button
                    type="button"
                    onClick={() => {
                      update({ locationId: c.id, locationName: c.name });
                      setCities([]);
                      setKeyword("");
                      setSearchState(null);
                    }}
                  >
                    <b>{c.name}</b>
                    <span>
                      {c.adm1}
                      {c.adm2 && c.adm2 !== c.name ? ` · ${c.adm2}` : ""} ·{" "}
                      {c.id}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          ) : null}

          <Row label="刷新间隔">
            <select
              value={draft.weatherRefreshMin}
              onChange={(e) =>
                update({ weatherRefreshMin: Number(e.target.value) })
              }
            >
              {refreshOptions(draft.weatherRefreshMin).map((v) => (
                <option key={v} value={v}>
                  {v >= 60 && v % 60 === 0 ? `${v / 60} 小时` : `${v} 分钟`}
                </option>
              ))}
            </select>
          </Row>

          <Row label="预报天数" hint="多出来的日子横向滚动查看">
            <select
              value={draft.weatherDays}
              onChange={(e) => update({ weatherDays: Number(e.target.value) })}
            >
              {[3, 4, 5, 6, 7].map((v) => (
                <option key={v} value={v}>
                  {v} 天
                </option>
              ))}
            </select>
          </Row>

          <Row label="预报显示格数" hint="一行放几格，多出来的横向滚动">
            <select
              value={draft.weatherForecastCols}
              onChange={(e) =>
                update({ weatherForecastCols: Number(e.target.value) })
              }
            >
              {[3, 4, 5].map((v) => (
                <option key={v} value={v}>
                  {v} 格
                </option>
              ))}
            </select>
          </Row>

          <Row label="生活指数" hint="温度下方那句建议的类别">
            <select
              value={draft.weatherAdviceType}
              onChange={(e) =>
                update({ weatherAdviceType: Number(e.target.value) })
              }
            >
              {ADVICE_TYPES.map((t) => (
                <option key={t.id} value={t.id}>
                  {t.name}
                </option>
              ))}
            </select>
          </Row>
        </Section>

        <Section title="通用">
          <Row label="开机启动" hint="登录 Windows 后自动打开两个悬浮窗">
            <Toggle
              checked={draft.autostart}
              onChange={(v) => update({ autostart: v })}
            />
          </Row>
        </Section>

        <Section title="更新">
          <Row
            label="自动更新"
            hint="启动 15 秒后检查一次，之后每 6 小时一次；只下载，不会自己重启，安装要你点「重启更新」。"
          >
            <Toggle
              checked={draft.updateAuto}
              onChange={(v) => update({ updateAuto: v })}
            />
          </Row>

          <div className="st-upd">
            <div className="st-upd-head">
              <i className={`st-upd-dot ${updateTone(upd)}`} />
              <span className="st-upd-text">{updateText(upd, draft.updateAuto)}</span>
              <b className="st-upd-cur">{APP_VERSION_LABEL}</b>
            </div>

            {updState === "downloading" ? (
              <div className="st-bar">
                <span style={{ width: `${Math.min(100, Math.max(2, upd?.progress ?? 0))}%` }} />
              </div>
            ) : null}

            {upd?.notes && (updState === "ready" || updState === "available") ? (
              <details className="st-upd-notes">
                <summary>本次更新内容</summary>
                <pre>{upd.notes}</pre>
              </details>
            ) : null}

            <div className="st-upd-actions">
              {updState === "ready" ? (
                <button type="button" className="st-btn" onClick={restartNow}>
                  重启更新
                </button>
              ) : null}
              <button
                type="button"
                className="st-btn ghost"
                disabled={updBusy || updState === "downloading"}
                onClick={checkNow}
              >
                {updBusy || updState === "checking" ? "检查中…" : "检查更新"}
              </button>
              {updState === "ready" ? (
                <button type="button" className="st-link" onClick={discardNow}>
                  忽略这个版本
                </button>
              ) : null}
            </div>
          </div>
        </Section>

        {testState ? (
          <p className={`st-note ${testState.kind === "err" ? "warn" : "ok"}`}>
            {testState.text}
          </p>
        ) : null}
      </div>

      <div className="st-about">
        <span className="st-about-line">WSight {APP_VERSION_LABEL} · MIT</span>
        <div className="st-links">
          <button type="button" className="st-link" onClick={() => open(REPO_URL)}>
            开源主页
          </button>
          <button
            type="button"
            className="st-link"
            onClick={() => open(RELEASES_URL)}
          >
            更新日志
          </button>
          <button type="button" className="st-link" onClick={() => open(ISSUES_URL)}>
            反馈问题
          </button>
        </div>
      </div>

      <footer className="st-foot">
        <button type="button" className="st-btn ghost" onClick={resetAppearance}>
          恢复默认
        </button>
        <button
          type="button"
          className="st-btn ghost"
          disabled={testing}
          onClick={() => void testConnection()}
        >
          {testing ? "测试中…" : "测试连接"}
        </button>
        <button
          type="button"
          className="st-btn ghost"
          disabled={refreshing}
          onClick={() => void refreshNow()}
        >
          {refreshing ? "刷新中…" : "刷新天气"}
        </button>
        <button
          type="button"
          className="st-btn danger"
          onClick={() => {
            if (window.confirm("确定退出 WSight？两个悬浮窗会一起关闭。")) {
              void api.quitApp();
            }
          }}
        >
          退出程序
        </button>
      </footer>
    </div>
  );
}
