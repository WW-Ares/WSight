import { useEffect, useState, type ReactNode } from "react";
import { api, onWeather, onWeatherError } from "../shared/api";
import { useLiveConfig } from "../shared/useLiveConfig";
import { useStage } from "../shared/uiScale";
import { WidgetFrame } from "../shared/WidgetFrame";
import type { AppConfig, WeatherDaily, WeatherPayload } from "../shared/types";
import { WeatherIcon } from "./WeatherIcon";
import "./weather.css";

function weekdayLabel(fxDate: string, index: number): string {
  if (index === 0) return "今天";
  if (index === 1) return "明天";
  if (index === 2) return "后天";
  const d = new Date(`${fxDate}T00:00:00`);
  if (Number.isNaN(d.getTime())) return fxDate;
  return ["周日", "周一", "周二", "周三", "周四", "周五", "周六"][d.getDay()];
}

/** "2026-09-14T17:13+08:00" -> "2026-09-14 17:13" */
function obsStamp(obsTime: string): string {
  if (!obsTime || obsTime.length < 16) return "";
  return `${obsTime.slice(0, 10)} ${obsTime.slice(11, 16)}`;
}

function PinIcon() {
  return (
    <svg width="10" height="12" viewBox="0 0 12 14" aria-hidden="true">
      <path
        fillRule="evenodd"
        fill="currentColor"
        d="M6 .8c-2.5 0-4.5 2-4.5 4.5 0 3.3 4.5 8 4.5 8s4.5-4.7 4.5-8C10.5 2.8 8.5.8 6 .8Zm0 6.1a1.7 1.7 0 1 1 0-3.4 1.7 1.7 0 0 1 0 3.4Z"
      />
    </svg>
  );
}

/** One forecast cell: weekday above, icon in the middle, low/high below. */
function DayCell({ item, index }: { item: WeatherDaily; index: number }) {
  return (
    <div className="wx-day">
      <span className="wx-day-name">{weekdayLabel(item.fxDate, index)}</span>
      <WeatherIcon code={item.iconDay} size={26} />
      <span className="wx-day-temp">
        {item.tempMin}/{item.tempMax}°
      </span>
    </div>
  );
}

function SetupHint({
  message,
  showConfigHint,
}: {
  message?: string | null;
  showConfigHint: boolean;
}) {
  return (
    <div className="wx-hint">
      <p>{message ?? "尚未获取到天气数据"}</p>
      {showConfigHint ? (
        <p className="hint-small">
          需要在设置里填入和风天气 API Key、API Host 与城市
        </p>
      ) : null}
      <button
        type="button"
        className="weather-setup-btn"
        onClick={() => {
          void api.openSettings();
        }}
      >
        打开设置
      </button>
    </div>
  );
}

export function WeatherPanel({ config }: { config: AppConfig }) {
  const cfg = useLiveConfig(config);
  const [data, setData] = useState<WeatherPayload | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  const days = data ? Math.max(1, cfg.weatherDays) : 0;
  const hasAdvice = Boolean(data?.advice?.text);
  // Re-fit whenever the panel's natural height changes for a non-resize
  // reason (forecast count, an advice line appearing, ...).
  const stageRef = useStage("weather", [days, hasAdvice]);

  useEffect(() => {
    let alive = true;
    let offWeather: (() => void) | undefined;
    let offError: (() => void) | undefined;

    api
      .fetchWeather()
      .then((w) => {
        if (!alive) return;
        setData(w);
        setError(null);
      })
      .catch((e: unknown) => {
        if (alive) setError(String(e));
      })
      .finally(() => {
        if (alive) setLoading(false);
      });

    onWeather((w) => {
      if (!alive) return;
      setData(w);
      setError(null);
      setLoading(false);
    }).then((u) => {
      offWeather = u;
    });

    onWeatherError((msg) => {
      if (alive) setError(msg);
    }).then((u) => {
      offError = u;
    });

    return () => {
      alive = false;
      offWeather?.();
      offError?.();
    };
  }, []);

  const shell = (title: string, sub: string, body: ReactNode) => (
    <WidgetFrame
      label="weather"
      variant="weather"
      stageRef={stageRef}
      title={
        <>
          <PinIcon />
          {title}
        </>
      }
      sub={sub}
      onTop={cfg.weatherAlwaysOnTop}
    >
      {body}
    </WidgetFrame>
  );

  if (loading && !data) {
    return shell(cfg.locationName || "天气", "加载中…", (
      <div className="wx-hint"><p>正在获取天气…</p></div>
    ));
  }

  // No data at all: explain why and offer a shortcut into the settings window.
  // A stale-but-valid payload keeps rendering; an error is informational only.
  if (!data) {
    const notConfigured = !cfg.qweatherKey || !cfg.locationId;
    return shell(
      cfg.locationName || "天气",
      notConfigured ? "未配置" : "获取失败",
      <SetupHint message={error} showConfigHint={notConfigured} />,
    );
  }

  const { now } = data;
  const daily = data.daily.slice(0, days);
  // One row up to four days, then split into two balanced rows so the widget
  // stays wide instead of growing into a long list.
  const cols = daily.length <= 4 ? Math.max(1, daily.length) : Math.ceil(daily.length / 2);

  return shell(
    data.locationName || "天气",
    error ? "更新失败" : obsStamp(now.obsTime),
    <>
      <div className="wx-now">
        <WeatherIcon code={now.icon} size={54} />
        <div className="wx-now-main">
          <div className="wx-now-line">
            <span className="wx-temp">{now.temp}°</span>
            <span className="wx-cond">{now.text}</span>
          </div>
          <div className="wx-feels">体感 {now.feelsLike}°</div>
        </div>
      </div>

      {hasAdvice ? <p className="wx-advice">{data.advice?.text}</p> : null}

      <div className="wx-stats">
        <span>湿度 {now.humidity}%</span>
        <span>
          {now.windDir} {now.windScale}级
        </span>
        <span>{now.pressure}hPa</span>
        <span>能见度 {now.vis}km</span>
      </div>

      {daily.length ? (
        <div className="wx-daily">
          {daily.map((d, i) => (
            <div
              key={d.fxDate}
              className="wx-day-slot"
              style={{ width: `calc(${100 / cols}% - 0.02%)` }}
            >
              <DayCell item={d} index={i} />
            </div>
          ))}
        </div>
      ) : null}
    </>,
  );
}
