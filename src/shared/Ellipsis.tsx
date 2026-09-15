import {
  useCallback,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";

interface EllipsisProps {
  /**
   * The full text. It is also what the tooltip says, which is why a caller
   * never has to keep a separate label around.
   */
  text: string;
  className?: string;
  /**
   * Render something other than the text itself - a coloured figure, an
   * already-formatted pair. The tooltip still reads `text`, which is how a
   * compact `18.9/63.8G` can explain itself in full.
   */
  children?: ReactNode;
}

/**
 * Text that only grows a tooltip when it does not fit.
 *
 * The widget used to carry a `title` on nearly every element - the gauges, the
 * disk tiles, the throughput block - and the result was a tooltip over
 * everything the pointer crossed, most of them repeating what was already
 * legible. A floating card has no room for that kind of noise.
 *
 * So the tooltip is now *earned*: an element measures itself once it is laid
 * out, and only when its content is wider than its box - the case where
 * `text-overflow: ellipsis` is doing its work - does the `title` appear. A
 * value that was cut off can still be read in full; a value that fits says
 * nothing extra.
 *
 * The measurement re-runs whenever the text changes, whenever the element's
 * own box changes (the gauge columns get wider as neighbours are switched
 * off) and once the webfont settles, so the flag never goes stale.
 */
export function Ellipsis({ text, className, children }: EllipsisProps) {
  const ref = useRef<HTMLSpanElement | null>(null);
  const [clipped, setClipped] = useState(false);

  const measure = useCallback(() => {
    const el = ref.current;
    if (!el) return;
    // +1 rather than 0: sub-pixel layout routinely leaves scrollWidth a
    // fraction wider than clientWidth, and a hair of difference is not a
    // truncated word.
    setClipped(el.scrollWidth > el.clientWidth + 1);
  }, []);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    // The first layout happens before the system font is in place; the flag
    // would otherwise be computed against a fallback face's metrics.
    const fonts = (document as Document & { fonts?: FontFaceSet }).fonts;
    fonts?.ready.then(measure).catch(() => {});
    return () => observer.disconnect();
  }, [measure]);

  // Measured after every render, not only when the text changes: the strings
  // here are formatted figures, and two different values can easily be the
  // same length - so the effect cannot key off the content. Setting the same
  // boolean twice is a no-op to React, so this cannot loop.
  useLayoutEffect(() => {
    measure();
  });

  return (
    <span ref={ref} className={className} title={clipped ? text : undefined}>
      {children ?? text}
    </span>
  );
}
