import { useEased } from "../../shared/useEased";

interface RingProps {
  /** 0..100 */
  value: number;
  size?: number;
  stroke?: number;
  color: string;
  /** how long one sample's motion is spread over, in ms; ~90% of the
   * sampling interval keeps the needle moving almost continuously */
  ramp?: number;
  /** native tooltip; defaults to the bare percentage */
  title?: string;
}

/**
 * The circular gauge on its own - caption and the two value lines belong to
 * the column around it, so that all four columns can share one baseline.
 *
 * Plain SVG on purpose - a charting library would cost more memory than the
 * whole widget.
 *
 * Motion is driven by `useEased` (requestAnimationFrame) rather than a CSS
 * transition: `stroke-dasharray` transitions repaint the arc every frame and
 * cannot run on the compositor, so a 260 ms transition after each 1 s sample
 * read as a twitch, not an animation. With the ramp the arc and the big
 * number glide between samples at display refresh rate.
 *
 * The old `drop-shadow` filter is gone for the same reason: filters re-run
 * on every repaint of the stroked circle, and a gauge repaints every frame
 * now. A faint static halo ring gives back the glow at a fraction of the
 * cost - it never changes, so it rasterises once.
 */
export function Ring({
  value,
  size = 50,
  stroke = 7,
  color,
  ramp = 900,
  title,
}: RingProps) {
  const radius = (size - stroke) / 2;
  const circumference = 2 * Math.PI * radius;
  const eased = useEased(Number.isFinite(value) ? Math.max(0, Math.min(100, value)) : 0, ramp);
  const dash = (eased / 100) * circumference;

  return (
    <div className="ring-graphic" style={{ width: size, height: size }} title={title}>
      <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`}>
        <circle
          cx={size / 2}
          cy={size / 2}
          r={radius}
          fill="none"
          stroke={color}
          strokeOpacity="0.16"
          strokeWidth={stroke + 3}
        />
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
      <span className="ring-value">{eased.toFixed(0)}</span>
    </div>
  );
}
