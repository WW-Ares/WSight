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
  /**
   * No live sample yet. The arc is drawn whole - the gauge is *present*, it
   * just has nothing to report - at low opacity, and the percentage gives way
   * to a dash. Drawing a confident 0% would be the one reading the gauge
   * cannot honestly give before the first measurement.
   */
  pending?: boolean;
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
 *
 * No tooltip: the percentage is inside the gauge and always legible, and a
 * card this small cannot afford a hover box over every element. What the
 * figure means (rated clock, swap, VRAM) lives in the caption line below it -
 * see `secondLine.ts` - and that line grows a tooltip of its own when it is
 * too long to fit.
 */
export function Ring({
  value,
  size = 50,
  stroke = 7,
  color,
  ramp = 900,
  pending = false,
}: RingProps) {
  const radius = (size - stroke) / 2;
  const circumference = 2 * Math.PI * radius;
  const eased = useEased(
    Number.isFinite(value) ? Math.max(0, Math.min(100, value)) : 0,
    ramp,
  );
  // Pending draws the full circle, so it needs no dash at all - and an
  // omitted `stroke-dasharray` is the one form that cannot be mis-read as a
  // partial arc by an odd `C 0` pair.
  const dash = (eased / 100) * circumference;

  return (
    <div className="ring-graphic" style={{ width: size, height: size }}>
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
          strokeOpacity={pending ? "0.35" : undefined}
          strokeLinecap={pending ? undefined : "round"}
          strokeDasharray={pending ? undefined : `${dash} ${circumference - dash}`}
          transform={`rotate(-90 ${size / 2} ${size / 2})`}
        />
      </svg>
      <span
        className="ring-value"
        style={pending ? { color: "var(--fg-dim)", opacity: 0.55 } : undefined}
      >
        {pending ? "--" : eased.toFixed(0)}
      </span>
    </div>
  );
}
