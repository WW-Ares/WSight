import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { api, onWeather, onWeatherError } from "../shared/api";
import { useLiveConfig } from "../shared/useLiveConfig";
import { useStage } from "../shared/uiScale";
import { WidgetFrame } from "../shared/WidgetFrame";
import type {
  AppConfig,
  WeatherCache,
  WeatherDaily,
  WeatherPayload,
} from "../shared/types";
import {
  FORECAST_COLS_DEFAULT,
  FORECAST_COLS_MAX,
  WEATHER_DAYS_MAX,
  WEATHER_DAYS_MIN,
  WEATHER_STALE_MS,
} from "../shared/types";
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

/**
 * Layout constants for the lifestyle sentence.
 *
 * The card reserves exactly `MAX_LINES` lines for it, so the window height is
 * the same whether the index returns one short clause or a long one.
 */
const ADVICE_MAX_LINES = 3;
/** Characters that fit on one line of the 300px card at 11.5px. */
const ADVICE_LINE_CHARS = 22;
/** Break opportunities - a line ends *before* one of these, never on it. */
const BREAK_CHARS = "，。、；：！？,.;:!?…";

function isBreakChar(ch: string): boolean {
  return BREAK_CHARS.includes(ch);
}

/**
 * Flatten the sentence the API sent.
 *
 * Some indices arrive with hard line breaks in the middle of a clause, which
 * used to render as a two-character first line. The card does its own
 * wrapping, so every break is removed and runs of blanks collapsed.
 */
function normalizeAdvice(text: string): string {
  return text
    .replace(/[\r\n]+/g, "")
    .replace(/[ \t　]+/g, " ")
    .trim();
}

/**
 * Break one sentence into at most `ADVICE_MAX_LINES` centred lines.
 *
 * Two rules the previous version broke:
 *  - a line never *ends* on punctuation - the break eats the comma, so the
 *    wrap reads as a pause rather than a stray mark hanging in the margin;
 *  - the cut is chosen at the punctuation closest to the end of the window,
 *    falling back to a hard cut, so a sentence with no comma still fills its
 *    lines instead of breaking after three characters.
 */
function wrapAdvice(text: string): string[] {
  const clean = normalizeAdvice(text);
  if (!clean) return [];
  if (clean.length <= ADVICE_LINE_CHARS) return [clean];

  const lines: string[] = [];
  let rest = clean;

  while (rest.length > ADVICE_LINE_CHARS && lines.length < ADVICE_MAX_LINES - 1) {
    // Prefer the last punctuation inside the window; it is the most natural
    // place to pause, and it keeps the next line a full one. The floor is low
    // (30% of the window) so a sentence like "稀释、扩散和清除…" can also cut
    // at the comma that opens the *next* window instead of falling through to
    // a hard, mid-word cut.
    let cut = -1;
    const from = Math.min(ADVICE_LINE_CHARS, rest.length - 1);
    const to = Math.max(4, Math.floor(ADVICE_LINE_CHARS * 0.3));
    for (let i = from; i >= to; i -= 1) {
      if (isBreakChar(rest[i])) {
        cut = i;
        break;
      }
    }

    let line: string;
    if (cut > 0) {
      line = rest.slice(0, cut);
      rest = rest.slice(cut + 1);
    } else {
      line = rest.slice(0, ADVICE_LINE_CHARS);
      rest = rest.slice(ADVICE_LINE_CHARS);
    }
    // A hard cut can still land on a mark, and the remainder can start with
    // one after a double punctuation ("…，"): trim both ends of the seam.
    line = line.replace(/[，。、；：！？,.;:!?…]+$/, "");
    rest = rest.replace(/^[，。、；：！？,.;:!?…]+/, "");
    if (line) lines.push(line);
  }

  // Whatever is left owns the last reserved line; an overlong tail is
  // clipped rather than allowed to grow the window.
  if (rest) {
    lines.push(
      rest.length > ADVICE_LINE_CHARS
        ? `${rest.slice(0, ADVICE_LINE_CHARS - 1)}…`
        : rest,
    );
  }
  return lines.slice(0, ADVICE_MAX_LINES);
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
      {/* 20 keeps the forecast glyph the same optical size it was when the
          icons carried padding; the frame is tighter now, so the old 26 would
          have drawn it a third larger. */}
      <WeatherIcon code={item.iconDay} size={20} />
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

/**
 * Placeholder with the same proportions as the real card.
 *
 * It is only ever shown before the first payload exists (no cache, slow
 * network). Matching the real layout keeps the window height steady, so the
 * card does not resize the moment data arrives.
 */
function WeatherSkeleton() {
  const line = (w: number | string, h = 11) => (
    <span className="wx-sk-line" style={{ width: w, height: h }} />
  );
  return (
    <div className="wx-skeleton">
      <div className="wx-now">
        <div className="wx-now-hero">
          <span className="wx-sk-circle" />
          <div className="wx-now-stack">
            {line(58, 24)}
            {line(40, 12)}
          </div>
        </div>
        {line(66, 11)}
      </div>
      <div className="wx-advice">
        {line("84%")}
        {line("62%")}
        {line("50%")}
      </div>
      {line("72%")}
      {line("72%")}
      <div className="wx-day-row">
        <span className="wx-sk-circle sm" />
        <span className="wx-sk-circle sm" />
        <span className="wx-sk-circle sm" />
      </div>
    </div>
  );
}

export function WeatherPanel({ config }: { config: AppConfig }) {
  const cfg = useLiveConfig(config);
  const [data, setData] = useState<WeatherPayload | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  /** when the payload on screen was stored/fetched, for the "old data" tint */
  const [cachedAt, setCachedAt] = useState(0);
  /** true while a network refresh is in flight (the header shows a dot) */
  const [refreshing, setRefreshing] = useState(true);

  /**
   * Advance width of the `°` glyph, measured rather than guessed.
   *
   * The condition text sits under the temperature and the two are meant to be
   * centred on each other - but on the *digits*, not on `32°`. The degree sign
   * has no width of its own to speak of and yet it still shifts the centre of
   * the line by half its advance, which was enough to make the word visibly
   * lean to the right. Measuring the glyph and reserving exactly that much
   * padding on the condition line moves the word back by half of it, while
   * the temperature block keeps the width it always had and does not move at
   * all.
   *
   * Measured instead of hard-coded because the value depends on the face the
   * system ends up picking (Segoe UI Variable, Segoe UI, a CJK fallback) - a
   * constant would be wrong on somebody's machine.
   */
  const [degWidth, setDegWidth] = useState(0);
  const degRef = useRef<HTMLSpanElement | null>(null);

  useLayoutEffect(() => {
    if (!data) return;
    const el = degRef.current;
    if (!el) return;
    const measure = () => {
      const w = el.offsetWidth;
      setDegWidth((prev) => (Math.abs(prev - w) > 0.4 ? w : prev));
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    const fonts = (document as Document & { fonts?: FontFaceSet }).fonts;
    fonts?.ready.then(measure).catch(() => {});
    return () => observer.disconnect();
  }, [data]);

  const days = data ? Math.min(Math.max(cfg.weatherDays || 3, WEATHER_DAYS_MIN), WEATHER_DAYS_MAX) : 0;
  const hasAdvice = Boolean(data?.advice?.text);
  // Re-fit whenever the panel's natural height changes for a non-resize
  // reason (forecast count, an advice line appearing, ...).
  const stageRef = useStage("weather", [
    days,
    hasAdvice,
    cfg.weatherForecastCols,
  ]);

  useEffect(() => {
    let alive = true;
    let offWeather: (() => void) | undefined;
    let offError: (() => void) | undefined;

    // Paint the stored payload first. This resolves from disk, so the card is
    // full in the same frame the window appears instead of waiting out a
    // QWeather round trip.
    api
      .getCachedWeather()
      .then((cache: WeatherCache | null) => {
        if (!alive || !cache?.payload) return;
        setData(cache.payload);
        setCachedAt(cache.cachedAtMs);
        setLoading(false);
      })
      .catch(() => {
        // no cache yet - the skeleton stays until the network answers
      });

    api
      .fetchWeather()
      .then((w) => {
        if (!alive) return;
        setData(w);
        setCachedAt(Date.now());
        setError(null);
      })
      .catch((e: unknown) => {
        if (alive) setError(String(e));
      })
      .finally(() => {
        if (alive) {
          setLoading(false);
          setRefreshing(false);
        }
      });

    onWeather((w) => {
      if (!alive) return;
      setData(w);
      setCachedAt(Date.now());
      setError(null);
      setLoading(false);
      setRefreshing(false);
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

  const shell = (title: string, sub: ReactNode, body: ReactNode) => (
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

  // Nothing on screen yet and nothing on disk: show the shape of the card
  // rather than a blank rectangle, so the window does not jump when the data
  // lands.
  if (loading && !data) {
    return shell(
      cfg.locationName || "天气",
      <span className="wx-updating">
        <i className="wx-dot" />
        加载中
      </span>,
      <WeatherSkeleton />,
    );
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
  // How many forecast cells fit on one row; anything beyond that is one
  // sideways scroll away. The arrow only appears when there is really
  // something to scroll to, so a 3-day card stays clean.
  const cols = Math.max(
    1,
    Math.min(cfg.weatherForecastCols || FORECAST_COLS_DEFAULT, FORECAST_COLS_MAX),
  );
  const slots = Math.max(1, Math.min(daily.length, cols));
  // `flex: 0 0 <width>` - the `0 0` matters: with the default `flex-shrink: 1`
  // the cells squeezed back into the row and there was nothing left to
  // scroll to, which is why the sideways scroll looked broken.
  const cellFlex = `0 0 ${100 / slots}%`;
  const hasMore = daily.length > cols;
  const adviceLines = wrapAdvice(data.advice?.text ?? "");

  // A payload older than half an hour is still shown - it beats an empty card
  // when the network is down - but its timestamp is dimmed so it cannot be
  // mistaken for a live reading.
  const stale = cachedAt > 0 && Date.now() - cachedAt > WEATHER_STALE_MS;
  const sub = refreshing ? (
    <span className="wx-updating">
      <i className="wx-dot" />
      更新中
    </span>
  ) : error ? (
    "更新失败"
  ) : (
    <span className={stale ? "wx-stamp is-stale" : "wx-stamp"}>
      {obsStamp(now.obsTime)}
    </span>
  );

  return shell(
    data.locationName || "天气",
    sub,
    <>
      <div className="wx-now">
        <div className="wx-now-hero">
          <WeatherIcon code={now.icon} size={53} />
          <div className="wx-now-stack">
            <span className="wx-temp">
              {now.temp}
              <span className="wx-temp-deg" ref={degRef}>
                °
              </span>
            </span>
            {/* The reserve is the measured degree sign, so the word centres on
                the digits while the temperature keeps its exact position. */}
            <span
              className="wx-cond"
              style={degWidth > 0 ? { paddingRight: degWidth } : undefined}
            >
              {now.text}
            </span>
          </div>
        </div>
        <div className="wx-feels">体感 {now.feelsLike}°</div>
      </div>

      {/* Always rendered, even when the plan has no index: the block owns a
          fixed three lines so the window height never follows the sentence. */}
      <div className="wx-advice">
        <div className="wx-advice-lines">
          {adviceLines.map((line) => (
            <span className="wx-advice-line" key={line}>
              {line}
            </span>
          ))}
        </div>
      </div>

      <div className="wx-stats">
        <span>湿度 {now.humidity}%</span>
        <span>
          {now.windDir} {now.windScale}级
        </span>
        <span>能见度 {now.vis}km</span>
      </div>

      {daily.length ? (
        <div className={"wx-daily" + (hasMore ? " has-more" : "")}>
          <div
            className={
              "wx-daily-scroll" + (hasMore ? " has-more" : "")
            }
            onWheel={(e) => {
              const el = e.currentTarget;
              if (el.scrollWidth <= el.clientWidth) return;
              el.scrollLeft += e.deltaY;
            }}
          >
            {daily.map((d, i) => (
              <div
                key={d.fxDate}
                className="wx-day-slot"
                style={{ flex: cellFlex }}
              >
                <DayCell item={d} index={i} />
              </div>
            ))}
          </div>
          {hasMore ? (
            <span className="wx-daily-more" aria-hidden="true">
              ›
            </span>
          ) : null}
        </div>
      ) : null}
    </>,
  );
}
