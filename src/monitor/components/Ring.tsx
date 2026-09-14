interface RingProps {
  /** 0..100 */
  value: number;
  size?: number;
  stroke?: number;
  color: string;
  /** native tooltip; defaults to the bare percentage */
  title?: string;
}

/**
 * The circular gauge on its own - caption and the two value lines belong to
 * the column around it, so that all four columns can share one baseline.
 *
 * Plain SVG on purpose - a charting library would cost more memory than the
 * whole widget.
 */
export function Ring({ value, size = 50, stroke = 7, color, title }: RingProps) {
  const radius = (size - stroke) / 2;
  const circumference = 2 * Math.PI * radius;
  const pct = Number.isFinite(value) ? Math.max(0, Math.min(100, value)) : 0;
  const dash = (pct / 100) * circumference;

  return (
    <div className="ring-graphic" style={{ width: size, height: size }} title={title}>
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
          style={{
            transition: "stroke-dasharray 260ms linear",
            filter: `drop-shadow(0 0 3px ${color})`,
          }}
        />
      </svg>
      <span className="ring-value">{pct.toFixed(0)}</span>
    </div>
  );
}
