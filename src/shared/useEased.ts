import { useEffect, useRef, useState } from "react";

/**
 * Eases `target` towards the latest value with requestAnimationFrame.
 *
 * The sampler pushes one snapshot per interval (1 s by default). Rendering
 * that raw made every gauge jump once a second and sit still in between -
 * the "stutter" - while the value itself was fine. Easing here keeps the
 * data rate untouched and turns the same two numbers into a continuous
 * ~60 fps motion between samples.
 *
 * Linear on purpose: an ease-out reads as "arrive early, then freeze",
 * which is the exact stutter we are removing. The ramp is slightly shorter
 * than the sampling interval, so the next target usually arrives just as
 * the previous ramp finishes.
 */
export function useEased(target: number, rampMs = 900): number {
  const [shown, setShown] = useState(target);
  const shownRef = useRef(target);
  const rafRef = useRef(0);

  useEffect(() => {
    const from = shownRef.current;
    const t0 = performance.now();
    const tick = (now: number) => {
      const k = Math.min(1, (now - t0) / rampMs);
      const v = from + (target - from) * k;
      shownRef.current = v;
      setShown(v);
      if (k < 1) rafRef.current = requestAnimationFrame(tick);
    };
    rafRef.current = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(rafRef.current);
  }, [target, rampMs]);

  return shown;
}
