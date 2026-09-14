import { useEffect, useRef } from "react";
import { api } from "./api";
import { MAX_WIDGET_HEIGHT, MAX_WIDGET_WIDTH } from "./types";

/**
 * Design width of every widget stage, in CSS px.
 *
 * The DOM is *always* laid out at exactly this width and then scaled with a
 * CSS transform, so both windows share one grid: text wraps identically at
 * every zoom level and the two widgets line up with each other.
 */
export const BASE_WIDTH = 300;

const MIN_SCALE = 0.62;
const MAX_SCALE = 2.4;

function clamp(value: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, value));
}

/**
 * Scale-to-fit driver shared by both floating widgets.
 *
 * How it works
 * ------------
 * The webview's own width is the single input: `scale = innerWidth / BASE_WIDTH`.
 * The DOM is then scaled by exactly that factor, so the card fills its window
 * edge to edge at any size and on any DPI - there is no arithmetic that can
 * leave a transparent sliver or clip content.
 *
 * The height is derived from the measured content and pushed to the native
 * window. Crucially the width is NEVER written back from here, so the height
 * fit cannot perturb the input the scale is computed from. An earlier version
 * also derived the zoom from the height and wrote the width back; every
 * logical-to-physical rounding step then fed the next one, and the widget
 * drifted a few percent larger on every launch (300 -> 307 -> 319 ...).
 *
 * Dragging any edge works: a width or corner drag changes `innerWidth`, which
 * changes the zoom, and the height follows. A pure bottom-edge drag snaps back
 * because the height is owned by the content.
 *
 * Why a transform and not `zoom`
 * ------------------------------
 * A CSS transform never affects layout metrics but *is* reflected by
 * `getBoundingClientRect()`. That makes `rect / scale` an unambiguous read of
 * the unscaled content height at any zoom level. `zoom` would blur the line
 * between layout and visual pixels and make the measurement DPI-dependent.
 *
 * `deps` lets a window re-fit when its content height changes for a reason
 * other than a resize (e.g. the user picks a different number of forecast
 * days).
 */
export function useStage(
  label: "monitor" | "weather",
  deps: unknown[] = [],
) {
  const stageRef = useRef<HTMLDivElement | null>(null);
  const scaleRef = useRef(1);
  const sentHeight = useRef(0);
  const saveTimer = useRef<number | null>(null);

  useEffect(() => {
    const root = document.documentElement;
    const stage = stageRef.current;
    if (!stage) return;

    let frame = 0;

    const sync = () => {
      frame = 0;
      const scaleBefore = scaleRef.current || 1;
      const baseHeight = stage.getBoundingClientRect().height / scaleBefore;
      if (!(baseHeight > 1)) return;

      // Bounds the OS will actually grant. Asking for more would leave the
      // panel sticking out of a window that refused to grow.
      const ceiling = Math.max(
        MIN_SCALE,
        Math.min(
          MAX_SCALE,
          MAX_WIDGET_WIDTH / BASE_WIDTH,
          MAX_WIDGET_HEIGHT / baseHeight,
        ),
      );

      const scale = clamp(window.innerWidth / BASE_WIDTH, MIN_SCALE, ceiling);

      if (Math.abs(scale - scaleBefore) > 0.0005) {
        scaleRef.current = scale;
        root.style.setProperty("--ui-scale", String(scale));
      }

      const wantHeight = Math.round(baseHeight * scale);
      if (Math.abs(wantHeight - sentHeight.current) >= 2) {
        sentHeight.current = wantHeight;
        void api.fitWidgetHeight(label, wantHeight).catch(() => {});
      }

      // Remember a size the user dragged to. Sub-pixel noise and the platform's
      // logical<->physical rounding stay well under the backend's threshold, so
      // this can never ratchet the widget bigger on every launch.
      if (saveTimer.current !== null) window.clearTimeout(saveTimer.current);
      saveTimer.current = window.setTimeout(() => {
        saveTimer.current = null;
        void api
          .rememberWidgetWidth(label, Math.round(window.innerWidth))
          .catch(() => {});
      }, 900);
    };

    const schedule = () => {
      if (frame) return;
      frame = requestAnimationFrame(sync);
    };

    scaleRef.current = 1;
    root.style.setProperty("--ui-scale", "1");
    root.style.setProperty("--stage-w", `${BASE_WIDTH}px`);
    sync();

    window.addEventListener("resize", schedule);

    // Content can grow on its own (forecast days, a longer advice line, a
    // late webfont). A ResizeObserver on the stage catches all of those; it
    // never fires for our own transform because transforms are paint-only.
    const observer = new ResizeObserver(schedule);
    observer.observe(stage);

    const fonts = (document as Document & { fonts?: FontFaceSet }).fonts;
    fonts?.ready.then(schedule).catch(() => {});

    return () => {
      if (frame) cancelAnimationFrame(frame);
      if (saveTimer.current !== null) window.clearTimeout(saveTimer.current);
      window.removeEventListener("resize", schedule);
      observer.disconnect();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [label, ...deps]);

  return stageRef;
}
