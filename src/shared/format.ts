/** Byte / rate / time formatting helpers shared by both widgets. */

export function formatBytes(bytes: number, digits = 1): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  let value = bytes;
  let idx = 0;
  while (value >= 1024 && idx < units.length - 1) {
    value /= 1024;
    idx += 1;
  }
  return `${value.toFixed(idx === 0 ? 0 : digits)} ${units[idx]}`;
}

export function formatRate(bytesPerSec: number): string {
  if (!Number.isFinite(bytesPerSec) || bytesPerSec <= 0) return "0 B/s";
  return `${formatBytes(bytesPerSec, 1)}/s`;
}

/**
 * Short form used by the 4-column monitor layout, where a whole column is
 * only ~70px wide: `475.9K/s` instead of `475.9 KB/s`.
 */
export function formatRateCompact(bytesPerSec: number, digits = -1): string {
  if (!Number.isFinite(bytesPerSec) || bytesPerSec <= 0) return "0/s";
  const units = ["B", "K", "M", "G"];
  let value = bytesPerSec;
  let idx = 0;
  while (value >= 1024 && idx < units.length - 1) {
    value /= 1024;
    idx += 1;
  }
  // `digits < 0` keeps the adaptive default (whole bytes and 3-digit values
  // get no decimal place). Callers in a very narrow column pass 0 outright.
  const places = digits >= 0 ? digits : idx === 0 ? 0 : value >= 100 ? 0 : 1;
  return `${value.toFixed(places)}${units[idx]}/s`;
}

export function formatGb(bytes: number, digits = 1): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0G";
  return `${(bytes / 1024 / 1024 / 1024).toFixed(digits)}G`;
}

/**
 * `18.9/63.8G` - used over total, sharing one unit.
 *
 * A monitor column is only ~85 CSS px wide and `18.9G / 63.8G` overruns it,
 * so the two figures hug the slash and the `G` is written once.
 */
export function formatGbPair(used: number, total: number, digits = 1): string {
  const gb = (n: number) =>
    !Number.isFinite(n) || n <= 0 ? "0" : (n / 1024 / 1024 / 1024).toFixed(digits);
  return `${gb(used)}/${gb(total)}G`;
}

/**
 * Byte total with the unit reduced to one letter: `13.7G`, `1.3T`.
 * Used by the cumulative network lines, where `累计` already says what it is
 * and the space before `GB` is width the column cannot spare.
 */
export function formatBytesCompact(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0";
  const units = ["B", "K", "M", "G", "T"];
  let value = bytes;
  let idx = 0;
  while (value >= 1024 && idx < units.length - 1) {
    value /= 1024;
    idx += 1;
  }
  const digits = idx === 0 ? 0 : value >= 100 ? 0 : 1;
  return `${value.toFixed(digits)}${units[idx]}`;
}

/**
 * Cumulative-traffic figure rounded to whole units: `14G`.
 *
 * The network column shows down and up totals on one ~75px line
 * (`↓14G ↑1G`), so a decimal point is exactly what breaks the budget. The
 * tooltip carries the exact figures.
 */
export function formatBytesTotal(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0";
  const units = ["B", "K", "M", "G", "T"];
  let value = bytes;
  let idx = 0;
  while (value >= 1024 && idx < units.length - 1) {
    value /= 1024;
    idx += 1;
  }
  return `${value.toFixed(0)}${units[idx]}`;
}

export function formatMhz(mhz: number): string {
  if (!Number.isFinite(mhz) || mhz <= 0) return "--";
  return `${(mhz / 1000).toFixed(2)}GHz`;
}

/**
 * `4.31 / 3.70GHz` - the clock the CPU is really running at, against its rated
 * clock, so a boost is visible at a glance.
 *
 * Both figures share one unit suffix to save the width of a second `GHz`, and
 * the pair collapses to the rated clock alone when the platform does not
 * report a live one - otherwise the line would read `0.00 / 3.70GHz`.
 */
export function formatClock(liveMhz: number, ratedMhz: number): string {
  if (!Number.isFinite(ratedMhz) || ratedMhz <= 0) return formatMhz(liveMhz);
  const rated = (ratedMhz / 1000).toFixed(2);
  if (!Number.isFinite(liveMhz) || liveMhz <= 0) return `${rated}GHz`;
  return `${(liveMhz / 1000).toFixed(2)}/${rated}GHz`;
}

/**
 * How long the machine has been up, in Chinese.
 *
 * The weather card had the opposite problem - it showed a raw observation
 * timestamp that lags real time by 5-20 minutes - so both headers were moved
 * onto one convention: the header says something about *this* moment in
 * plain language, and never prints a wall-clock reading that can be compared
 * against the taskbar and found wanting.
 *
 * Minutes are dropped once days appear (`2 天 3 小时`, not `2 天 3 小时 42
 * 分钟`): the header is 300 design px wide and the extra figures buy nothing.
 */
export function formatUptime(totalSeconds: number): string {
  if (!Number.isFinite(totalSeconds) || totalSeconds < 0) return "--";
  const days = Math.floor(totalSeconds / 86400);
  const hours = Math.floor((totalSeconds % 86400) / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  if (days > 0) return `${days} 天 ${hours} 小时`;
  if (hours > 0) return `${hours} 小时 ${minutes} 分钟`;
  if (minutes > 0) return `${minutes} 分钟`;
  return "刚刚启动";
}

/**
 * Age of a cache entry as `X 分钟前更新`.
 *
 * Deliberately *not* a wall-clock stamp. The payload on screen was written
 * `Date.now() - cachedAtMs` ago, and saying so in relative terms is the only
 * phrasing that cannot drift out of step with the system clock.
 *
 * `cachedAtMs` of 0 means "never written" and yields an empty string so the
 * caller can leave the slot blank rather than claim "刚刚更新" about a payload
 * that does not exist yet.
 */
export function formatRelativeAge(ageMs: number): string {
  if (!Number.isFinite(ageMs) || ageMs < 0) return "";
  const minutes = Math.floor(ageMs / 60000);
  if (minutes <= 0) return "刚刚更新";
  if (minutes < 60) return `${minutes} 分钟前更新`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} 小时前更新`;
  return `${Math.floor(hours / 24)} 天前更新`;
}

/** C2D-style rate colour: distinct hues so up/down are readable at a glance. */
export function rateColor(bytesPerSec: number, base = "var(--accent-net)"): string {
  if (bytesPerSec > 8 * 1024 * 1024) return "var(--danger)";
  if (bytesPerSec > 1024 * 1024) return "var(--warn)";
  return base;
}
