interface IconProps {
  /** QWeather icon code, e.g. "100", "305", "400" */
  code: string;
  size?: number;
  /** night variant hint (150/151/152...) */
  night?: boolean;
}

type Kind = "sun" | "moon" | "cloud" | "overcast" | "rain" | "snow" | "fog";

/**
 * Map a QWeather icon code to a coarse family so we can draw it with a
 * handful of inline SVG shapes and stay fully offline.
 */
export function iconKind(code: string): Kind {
  const n = Number.parseInt(code, 10);
  if (!Number.isFinite(n)) return "cloud";
  if (n === 100 || n === 150) return n === 150 ? "moon" : "sun";
  if (n >= 101 && n <= 103) return "cloud";
  if (n >= 151 && n <= 153) return "moon";
  if (n === 104) return "overcast";
  if (n >= 300 && n <= 399) return "rain";
  if (n >= 400 && n <= 499) return "snow";
  if (n >= 500 && n <= 515) return "fog";
  if (n === 900) return "sun";
  if (n === 901) return "snow";
  return "cloud";
}

const SUN = "#ffcf5c";
const MOON = "#dfe7ff";
const CLOUD = "#c9d6e4";
const RAIN = "#6fb6ff";
const SNOW = "#dff1ff";
const FOG = "#a9b8c8";

/**
 * Glyph fit, per weather family: `[centreX, centreY, scale]` inside the
 * 48-unit viewBox.
 *
 * Every glyph is drawn wherever its path happens to land, so the raw shapes
 * sat at different heights and sizes - the moon's box was centred 10 units
 * below and 6 left of the viewBox centre, and even ran off the bottom edge.
 * Stacking `icon` next to the temperature then looked tilted, and the gap
 * between them was mostly empty viewBox padding.
 *
 * `fit()` moves each glyph's own bounding box centre onto (24, 24) and scales
 * it so its longest side is `GLYPH_SIZE` units: same optical weight, same
 * axis, no per-glyph CSS nudges. The numbers come from
 * `.verify/icon_bbox.mjs`, which samples the real paths.
 */
const GLYPH_SIZE = 36;
const GLYPH_FIT: Record<Kind, [number, number, number]> = {
  sun: [24.0, 24.0, GLYPH_SIZE / 37.6],
  moon: [22.74, 24.09, GLYPH_SIZE / 35.82],
  cloud: [26.4, 21.8, GLYPH_SIZE / 29.11],
  overcast: [25.35, 25.12, GLYPH_SIZE / 31.1],
  fog: [24.85, 23.22, GLYPH_SIZE / 37.96],
  snow: [26.4, 30.21, GLYPH_SIZE / 31.37],
  rain: [26.4, 30.11, GLYPH_SIZE / 31.17],
};

function fit(kind: Kind): string {
  const [cx, cy, scale] = GLYPH_FIT[kind];
  const tx = 24 - scale * cx;
  const ty = 24 - scale * cy;
  return `translate(${tx.toFixed(3)} ${ty.toFixed(3)}) scale(${scale.toFixed(4)})`;
}

export function WeatherIcon({ code, size = 46, night = false }: IconProps) {
  const kind = iconKind(code);
  const resolved: Kind = night && kind === "sun" ? "moon" : kind;

  const common = {
    width: size,
    height: size,
    // The glyph frame, not the drawing grid: every glyph is centred on
    // (24, 24) and scaled to at most `GLYPH_SIZE` units, so this 36-unit box
    // is their common bounding square. Rendering it edge to edge is what
    // removes the padding that used to sit between the icon and the
    // temperature - the gap you see is now exactly the flex gap.
    viewBox: "6 6 36 36",
    fill: "none" as const,
    strokeLinecap: "round" as const,
    strokeLinejoin: "round" as const,
  };

  if (resolved === "sun" || resolved === "moon") {
    const isSun = resolved === "sun";
    return (
      <svg {...common}>
        <g transform={fit(resolved)}>
          {isSun ? (
            <>
              <circle cx="24" cy="24" r="9" fill={SUN} />
              {Array.from({ length: 8 }).map((_, i) => {
                const a = (i * Math.PI) / 4;
                const x1 = 24 + Math.cos(a) * 13;
                const y1 = 24 + Math.sin(a) * 13;
                const x2 = 24 + Math.cos(a) * 17.5;
                const y2 = 24 + Math.sin(a) * 17.5;
                return (
                  <line
                    key={i}
                    x1={x1}
                    y1={y1}
                    x2={x2}
                    y2={y2}
                    stroke={SUN}
                    strokeWidth="2.6"
                  />
                );
              })}
            </>
          ) : (
            <path
              // Crescent built from two circles (r18 at the centre, r16.5
              // pulled up-right; inner arc sweeps back through the left side)
              // so its bounding box is centred and inside the frame - the
              // hand-tuned path it replaced hung 10 units low and was clipped
              // by the bottom edge.
              d="M39.49 33.17A18 18 0 1 1 21.49 6.18A16.5 16.5 0 0 0 39.49 33.17Z"
              fill={MOON}
            />
          )}
        </g>
      </svg>
    );
  }

  const cloudShape = (
    <path
      d="M15 34h19a7 7 0 0 0 .6-13.97A10.5 10.5 0 0 0 15.2 22.4 6.8 6.8 0 0 0 15 34Z"
      fill={CLOUD}
    />
  );

  if (resolved === "cloud") {
    return (
      <svg {...common}>
        <g transform={fit("cloud")}>
          <circle cx="32" cy="16" r="6.4" fill={SUN} opacity="0.9" />
          {cloudShape}
        </g>
      </svg>
    );
  }

  if (resolved === "overcast") {
    return (
      <svg {...common}>
        <g transform={fit("overcast")}>
          <path
            d="M13 36h21a7.4 7.4 0 0 0 .6-14.7A11 11 0 0 0 13.4 24 7 7 0 0 0 13 36Z"
            fill={FOG}
          />
        </g>
      </svg>
    );
  }

  if (resolved === "fog") {
    return (
      <svg {...common}>
        <g transform={fit("fog")}>
          <path
            d="M13 26h21a7.4 7.4 0 0 0 .6-14.7A11 11 0 0 0 13.4 14 7 7 0 0 0 13 26Z"
            fill={CLOUD}
          />
          {[31, 36, 41].map((y, i) => (
            <line
              key={y}
              x1={10 + i * 3}
              y1={y}
              x2={38 - i * 2}
              y2={y}
              stroke={FOG}
              strokeWidth="2.4"
            />
          ))}
        </g>
      </svg>
    );
  }

  if (resolved === "snow") {
    return (
      <svg {...common}>
        <g transform={fit("snow")}>
          {cloudShape}
          {[37, 42].map((y, row) =>
            [17, 24, 31].map((x, i) => (
              <circle
                key={`${y}-${x}`}
                cx={x + (row % 2 === 0 ? 0 : 3.5)}
                cy={y + (i % 2 === 0 ? 0 : 2)}
                r="1.9"
                fill={SNOW}
              />
            )),
          )}
        </g>
      </svg>
    );
  }

  // rain
  return (
      <svg {...common}>
        <g transform={fit("rain")}>
          {cloudShape}
          {[17, 24, 31].map((x, i) => (
            <line
              key={x}
              x1={x}
              y1={37 + (i % 2 === 0 ? 0 : 1.5)}
              x2={x - 2.2}
              y2={43 + (i % 2 === 0 ? 0 : 1.5)}
              stroke={RAIN}
              strokeWidth="2.4"
            />
          ))}
        </g>
      </svg>
  );
}
