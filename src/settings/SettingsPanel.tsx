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
  DEFAULT_CONFIG,
  MAX_DRIVES,
  MAX_DISK_TILES,
  MAX_WIDGET_WIDTH,
  MIN_WIDGET_WIDTH,
  RING_DEFAULTS,
  type AppConfig,
  type GeoCity,
  type Snapshot,
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

function Row({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children?: ReactNode;
}) {
  return (
    <div className="st-row">
      <div className="st-row-label">
        <span>{label}</span>
        {hint ? <em>{hint}</em> : null}
      </div>
      <div className="st-row-ctl">{children}</div>
    </div>
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
    text: "改动会自动保存并立即生效",
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
      .then(() =>
        setStatus({ kind: "ok", text: "两个悬浮窗已回到默认位置" }),
      )
      .catch((e: unknown) =>
        setStatus({ kind: "err", text: `重置失败：${String(e)}` }),
      );
  }, []);

  // -------------------------------------------------------------- render

  const hostIsPreset = HOST_PRESETS.some((p) => p.value === draft.qweatherHost);

  return (
    <div className="st">
      <header className="st-head">
        <div>
          <h1>
            WSight 设置
            <span className="st-ver" title={`WSight ${APP_VERSION_LABEL}`}>
              {APP_VERSION_LABEL}
            </span>
          </h1>
          <p>硬件监控 + 天气 · 所有改动自动保存，并立即应用到两个悬浮窗</p>
        </div>
        <span className={`st-status ${status.kind}`}>{status.text}</span>
      </header>

      <div className="st-body">
        <Section title="外观" desc="拖动即可实时预览">
          <div
            className="st-preview"
            style={{ borderRadius: draft.radius }}
          >
            <MiniRing value={37} color={draft.colorCpu} label="CPU" />
            <MiniRing value={62} color={draft.colorMem} label="MEM" />
            <MiniRing value={18} color={draft.colorGpu} label="GPU" />
            <div className="st-preview-net">
              <span style={{ color: draft.colorNet }}>↓ 1.2 MB/s</span>
              <span style={{ color: "#ffd479" }}>↑ 240 KB/s</span>
            </div>
          </div>

          <Row label="主题">
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
          </Row>

          <Row label="不透明度" hint={`${Math.round(draft.opacity * 100)}% · 仅背景`}>
            <input
              type="range"
              min={25}
              max={100}
              value={Math.round(draft.opacity * 100)}
              onChange={(e) => update({ opacity: Number(e.target.value) / 100 })}
            />
          </Row>

          <p className="st-note">
            不透明度只作用于卡片背景：文字、圆环和图标始终是全对比度，不会跟着变淡。
          </p>

          <Row label="圆角" hint={`${Math.round(draft.radius)} px`}>
            <input
              type="range"
              min={0}
              max={24}
              value={Math.round(draft.radius)}
              onChange={(e) => update({ radius: Number(e.target.value) })}
            />
          </Row>

          <Row label="监控窗大小" hint={`${Math.round(draft.monitorWidth)} px 宽`}>
            <input
              type="range"
              min={MIN_WIDGET_WIDTH}
              max={MAX_WIDGET_WIDTH}
              step={2}
              value={Math.round(draft.monitorWidth)}
              onChange={(e) => setWidgetWidth("monitor", Number(e.target.value))}
            />
          </Row>

          <Row label="天气窗大小" hint={`${Math.round(draft.weatherWidth)} px 宽`}>
            <input
              type="range"
              min={MIN_WIDGET_WIDTH}
              max={MAX_WIDGET_WIDTH}
              step={2}
              value={Math.round(draft.weatherWidth)}
              onChange={(e) => setWidgetWidth("weather", Number(e.target.value))}
            />
          </Row>

          <Row label="窗口位置" hint="拖到哪儿，下次就开在哪儿">
            <button
              type="button"
              className="st-btn ghost small"
              onClick={resetPositions}
            >
              重置到默认位置
            </button>
          </Row>

          <p className="st-note">
            悬浮窗默认是锁定的：既不能拖动也不能缩放，避免误碰。在悬浮窗上
            <b>点右键</b>可以选「调整」——此时窗口边框会高亮，才能拖动和缩放；
            同菜单里还有「置顶」和「设置」。两个窗口共用同一套 300 px 版式，
            宽度变化时内容会等比放大，高度自动跟随，不会重排或截断。
          </p>

          <Row label="配色" hint="点击色块调整">
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
                默认配色
              </button>
            </div>
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
          <Row label="显示 GPU 环形" hint="非 NVIDIA 显卡无数据">
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

          <Row label="API Key" hint="在控制台创建的项目 Key">
            <div className="st-inline">
              <input
                type={showKey ? "text" : "password"}
                value={draft.qweatherKey}
                placeholder="粘贴和风天气 API Key"
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

          <Row label="城市" hint={draft.locationName || "未选择"}>
            <div className="st-inline">
              <input
                type="text"
                value={keyword}
                placeholder="输入城市名后回车，例如 杭州"
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

          <Row label="预报天数" hint={`${draft.weatherDays} 天`}>
            <input
              type="range"
              min={1}
              max={7}
              value={draft.weatherDays}
              onChange={(e) => update({ weatherDays: Number(e.target.value) })}
            />
          </Row>
        </Section>

        {testState ? (
          <p className={`st-note ${testState.kind === "err" ? "warn" : "ok"}`}>
            {testState.text}
          </p>
        ) : null}
      </div>

      <div className="st-about">
        <span className="st-about-line">
          WSight {APP_VERSION_LABEL} · MIT 开源
        </span>
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
          <button
            type="button"
            className="st-link"
            onClick={() => open(ISSUES_URL)}
          >
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
          {refreshing ? "刷新中…" : "立即刷新天气"}
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
