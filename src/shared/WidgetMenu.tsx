import { useEffect, useRef, useState } from "react";
import { BASE_WIDTH } from "./uiScale";

const MENU_W = 112;
const ROW_H = 23;
const PAD = 5;

type ActionKey = "adjusting" | "onTop" | "settings";

const ITEMS: { key: ActionKey; label: string }[] = [
  { key: "adjusting", label: "调整" },
  { key: "onTop", label: "置顶" },
  { key: "settings", label: "设置…" },
];

function clamp(value: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, value));
}

interface WidgetMenuProps {
  /** click position in window CSS px */
  x: number;
  y: number;
  /** which toggles are currently on, so the menu can tick them */
  adjusting: boolean;
  onTop: boolean;
  onAdjust: () => void;
  onToggleTop: () => void;
  onSettings: () => void;
  onClose: () => void;
}

/**
 * A widget's own right-click menu.
 *
 * Drawn inside the widget rather than by WebView2 or the OS: it matches the
 * theme (the native menu follows the *system* theme, so it turns into a white
 * box on a dark desktop), and it knows how big the window is. A widget window
 * is only as large as its card, so the menu is clamped to stay inside it
 * instead of being clipped at the window edge - `useStage` scales the whole
 * stage, so working in design px keeps the menu proportional at any zoom.
 *
 * Arrow keys and Enter work as well as the mouse, and the menu takes focus on
 * open: a right-click menu that only answers to a mouse is a trap.
 */
export function WidgetMenu({
  x,
  y,
  adjusting,
  onTop,
  onAdjust,
  onToggleTop,
  onSettings,
  onClose,
}: WidgetMenuProps) {
  const [active, setActive] = useState(0);
  const buttons = useRef<(HTMLButtonElement | null)[]>([]);

  useEffect(() => {
    buttons.current[active]?.focus();
  }, [active]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        onClose();
      } else if (event.key === "ArrowDown") {
        event.preventDefault();
        setActive((i) => (i + 1) % ITEMS.length);
      } else if (event.key === "ArrowUp") {
        event.preventDefault();
        setActive((i) => (i - 1 + ITEMS.length) % ITEMS.length);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const scale = Math.max(0.2, window.innerWidth / BASE_WIDTH);
  const stageHeight = window.innerHeight / scale;
  const menuHeight = ROW_H * ITEMS.length + PAD * 2;
  const left = clamp(x / scale, 3, Math.max(3, BASE_WIDTH - MENU_W - 3));
  const top = clamp(y / scale, 3, Math.max(3, stageHeight - menuHeight - 3));

  const ticked: Record<ActionKey, boolean> = {
    adjusting,
    onTop,
    settings: false,
  };
  const actions: Record<ActionKey, () => void> = {
    adjusting: onAdjust,
    onTop: onToggleTop,
    settings: onSettings,
  };

  return (
    <div
      className="wmenu"
      style={{ left, top, width: MENU_W }}
      // clicking inside the menu must not be read as "click outside -> close"
      onMouseDown={(event) => event.stopPropagation()}
      onContextMenu={(event) => event.preventDefault()}
    >
      {ITEMS.map((item, index) => (
        <button
          key={item.key}
          ref={(el) => {
            buttons.current[index] = el;
          }}
          type="button"
          className={`wmenu-item${ticked[item.key] ? " on" : ""}`}
          onMouseEnter={() => setActive(index)}
          onClick={actions[item.key]}
        >
          <i className="wmenu-tick">{ticked[item.key] ? "✓" : ""}</i>
          <span>{item.label}</span>
        </button>
      ))}
    </div>
  );
}
