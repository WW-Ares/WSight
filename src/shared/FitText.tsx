import {
  useCallback,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";

interface FitTextProps {
  /** The text to draw, and what the tooltip says when it has to be clipped. */
  text: string;
  className?: string;
  /**
   * How far the type may shrink, as a fraction of its CSS size. 0.82 takes a
   * 10 px caption to 8.2 px - still legible, and the caption is a label, not a
   * measurement, so it is not being asked to do two jobs at once.
   *
   * Past this floor the text stops shrinking and `text-overflow` takes over,
   * which is why the floor exists rather than shrinking without limit.
   */
  floor?: number;
  /**
   * Text that is not on screen anywhere - the raw memory part number behind a
   * `金士顿 D4 3200` caption, say. When given, the tooltip shows this always:
   * the usual rule is that a tooltip must be earned by clipping, but this is
   * information the user cannot get any other way, so it is not noise.
   */
  hint?: string;
  children?: ReactNode;
}

/**
 * A caption that shrinks a little rather than truncating.
 *
 * The shortening rules in `monitor/hwName.ts` already walk a ladder of
 * progressively harsher spellings and stop at the first that fits, so on every
 * name in the corpus this component does nothing at all. It exists for the
 * names the ladder has never seen - a new vendor, a spelling nobody predicted -
 * because a public build meets machines its author never will, and the failure
 * mode without a net is a truncated word on the card.
 *
 * Why not just always shrink: the ladder keeps the columns at one size, and
 * uniform type across four gauges is worth more than the extra characters. The
 * shrink is the exception, not the rule.
 *
 * Measurements are in *layout* px (`clientWidth` / `scrollWidth`), never in
 * `getBoundingClientRect()`. The whole panel is drawn at a fixed 300 px design
 * width and then scaled by the stage, so rects come back multiplied by the
 * display's DPI factor while these two are not - comparing one against the
 * other would be off by 1.4x on this machine. Because both sides of the
 * comparison are layout px, the ratio is right regardless of the display.
 */
export function FitText({
  text,
  className,
  floor = 0.82,
  hint,
  children,
}: FitTextProps) {
  const ref = useRef<HTMLSpanElement | null>(null);
  /** The CSS font size, remembered before any inline override lands. */
  const baseRef = useRef(0);
  const [clipped, setClipped] = useState(false);

  const measure = useCallback(() => {
    const el = ref.current;
    if (!el) return;
    // Drop the previous override before measuring. Without this the ratio is
    // computed against the already-shrunk size and every pass shrinks again,
    // which walks a long caption down to the floor one frame at a time.
    el.style.fontSize = "";
    const css = parseFloat(getComputedStyle(el).fontSize);
    if (css > 0) baseRef.current = css;
    const base = baseRef.current;
    const avail = el.clientWidth;
    if (!base || !avail) return;
    // `scrollWidth` is rounded up to a whole pixel, so a hair of overflow is
    // not a clipped word.
    if (el.scrollWidth <= avail + 1) {
      setClipped(false);
      return;
    }
    const need = el.scrollWidth;
    el.style.fontSize = `${Math.max(base * floor, base * (avail / need))}px`;
    // Measure again: only if the floor was not enough does the ellipsis show.
    setClipped(el.scrollWidth > el.clientWidth + 1);
  }, [floor]);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    // The first layout runs before the system font is in place, so the ratio
    // would be computed against the fallback face's metrics.
    const fonts = (document as Document & { fonts?: FontFaceSet }).fonts;
    fonts?.ready.then(measure).catch(() => {});
    return () => observer.disconnect();
  }, [measure]);

  // After every render, not only when the text changes: two different captions
  // can be the same length, so the effect cannot key off the content. Setting
  // the same values twice is a no-op to React, so this cannot loop.
  useLayoutEffect(() => {
    measure();
  });

  const title = hint && hint !== text ? hint : clipped ? text : undefined;

  return (
    <span ref={ref} className={className} title={title}>
      {children ?? text}
    </span>
  );
}
