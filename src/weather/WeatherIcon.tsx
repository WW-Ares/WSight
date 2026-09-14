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

export function WeatherIcon({ code, size = 46, night = false }: IconProps) {
  const kind = iconKind(code);
  const resolved: Kind = night && kind === "sun" ? "moon" : kind;

  const common = {
    width: size,
    height: size,
    viewBox: "0 0 48 48",
    fill: "none" as const,
    strokeLinecap: "round" as const,
    strokeLinejoin: "round" as const,
  };

  if (resolved === "sun" || resolved === "moon") {
    const isSun = resolved === "sun";
    return (
      <svg {...common}>
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
            d="M30 32a10 10 0 1 1-9.6-13.2A11 11 0 0 0 30 32Z"
            fill={MOON}
          />
        )}
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
        <circle cx="32" cy="16" r="6.4" fill={SUN} opacity="0.9" />
        {cloudShape}
      </svg>
    );
  }

  if (resolved === "overcast") {
    return (
      <svg {...common}>
        <path
          d="M13 36h21a7.4 7.4 0 0 0 .6-14.7A11 11 0 0 0 13.4 24 7 7 0 0 0 13 36Z"
          fill={FOG}
        />
      </svg>
    );
  }

  if (resolved === "fog") {
    return (
      <svg {...common}>
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
      </svg>
    );
  }

  if (resolved === "snow") {
    return (
      <svg {...common}>
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
      </svg>
    );
  }

  // rain
  return (
    <svg {...common}>
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
    </svg>
  );
}
