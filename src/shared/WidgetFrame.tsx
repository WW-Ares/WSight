import { useEffect, useRef, useState, type ReactNode, type RefObject } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api } from "./api";
import { WidgetMenu } from "./WidgetMenu";

interface WidgetFrameProps {
  label: "monitor" | "weather";
  /** panel modifier class, e.g. `monitor` / `weather` */
  variant: string;
  stageRef: RefObject<HTMLDivElement>;
  title: ReactNode;
  sub: ReactNode;
  /** the window's 置顶 setting, straight from the live config */
  onTop: boolean;
  children: ReactNode;
}

/**
 * Shell shared by both floating widgets: the scaled stage, the card, the
 * header and the right-click menu.
 *
 * A widget is *locked* by default - no drag region and a non-resizable window,
 * so it cannot be nudged out of place by a stray click. Everything about
 * moving and sizing goes through 调整 in this menu, which is also where 置顶
 * and a shortcut into 设置 live. Keeping it in one place means the monitor and
 * the weather card cannot drift apart in behaviour.
 */
export function WidgetFrame({
  label,
  variant,
  stageRef,
  title,
  sub,
  onTop,
  children,
}: WidgetFrameProps) {
  const [adjusting, setAdjusting] = useState(false);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  /**
   * Serialises arrow-key nudges.
   *
   * Each nudge is a round trip that reads the window's position and writes it
   * back, so a burst of key repeats fired in parallel would all read the same
   * starting point and land on top of each other, and holding a key down would
   * move the widget far less than it should. Chaining them keeps every step
   * counted.
   */
  const nudgeQueue = useRef<Promise<void>>(Promise.resolve());

  // Never leave the widget unlocked and floating if the window goes away
  // (reload, hot-reload during development, a crash in the webview).
  useEffect(
    () => () => {
      void api.setWidgetAdjust(label, false).catch(() => {});
    },
    [label],
  );

  /**
   * Dismiss the menu when the click lands outside this widget.
   *
   * A click on the desktop or on another application never reaches the page,
   * so `onMouseDown` on the card cannot see it - the widget has to learn about
   * it from losing focus. Two independent signals are used because neither is
   * complete on its own: the webview's own `blur` event, and Tauri's
   * window-focus event (a click on the desktop moves activation away from the
   * window without the renderer always noticing).
   *
   * Clicks *inside* the widget are already handled by `onMouseDown` on the
   * card, and clicks on the menu itself are stopped there, so this effect
   * deliberately does not add a document-level listener: a capture-phase one
   * would fire before the menu's own button and close it mid-click.
   */
  useEffect(() => {
    if (!menu) return;

    const close = () => setMenu(null);
    let dropped = false;
    let unlisten: (() => void) | undefined;

    window.addEventListener("blur", close);

    getCurrentWindow()
      .onFocusChanged(({ payload: focused }) => {
        if (!focused) close();
      })
      .then((off) => {
        if (dropped) off();
        else unlisten = off;
      })
      .catch(() => {});

    return () => {
      dropped = true;
      window.removeEventListener("blur", close);
      unlisten?.();
    };
  }, [menu]);

  /**
   * Arrow keys nudge the widget while 调整 is on.
   *
   * Dragging gets a widget to roughly the right place; placing it *exactly*
   * there - level with the card next to it, or one hair off the screen edge -
   * is a mouse-only job that never quite lands. A key press is one pixel,
   * which is the resolution a pointer cannot give. `Shift` covers the case
   * where the widget is simply in the wrong place.
   *
   * Suppressed while the right-click menu is open: that menu owns the arrow
   * keys for moving between its items, and a widget sliding sideways every
   * time the highlight moves would be absurd.
   */
  useEffect(() => {
    if (!adjusting || menu) return;

    const onKey = (event: KeyboardEvent) => {
      const step = event.shiftKey ? 10 : 1;
      let dx = 0;
      let dy = 0;
      switch (event.key) {
        case "ArrowLeft":
          dx = -step;
          break;
        case "ArrowRight":
          dx = step;
          break;
        case "ArrowUp":
          dy = -step;
          break;
        case "ArrowDown":
          dy = step;
          break;
        default:
          return;
      }
      event.preventDefault();
      nudgeQueue.current = nudgeQueue.current
        .then(() => api.nudgeWidget(label, dx, dy))
        .catch(() => {});
    };

    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [adjusting, menu, label]);

  const toggleAdjust = () => {
    const next = !adjusting;
    setAdjusting(next);
    setMenu(null);
    void api.setWidgetAdjust(label, next).catch(() => {});
  };

  const toggleOnTop = () => {
    setMenu(null);
    void api.setWidgetAlwaysOnTop(label, !onTop).catch(() => {});
  };

  // Tauri starts a window drag from the element the mouse went down on, so the
  // attribute has to be on the surface actually clicked - hence both the card
  // and its header.
  const dragRegion = adjusting ? "" : undefined;

  return (
    <div className="stage" ref={stageRef}>
      <div
        className={`panel ${variant}${adjusting ? " adjusting" : ""}`}
        data-tauri-drag-region={dragRegion}
        onContextMenu={(event) => {
          event.preventDefault();
          setMenu({ x: event.clientX, y: event.clientY });
        }}
        onMouseDown={() => setMenu(null)}
      >
        <div className="panel-header" data-tauri-drag-region={dragRegion}>
          <span className="panel-title">{title}</span>
          <span className="panel-sub">
            {adjusting ? (
              <span className="adjust-badge">调整中 · 拖动或方向键</span>
            ) : (
              sub
            )}
          </span>
        </div>
        {children}
      </div>

      {menu ? (
        <WidgetMenu
          x={menu.x}
          y={menu.y}
          adjusting={adjusting}
          onTop={onTop}
          onAdjust={toggleAdjust}
          onToggleTop={toggleOnTop}
          onSettings={() => {
            setMenu(null);
            void api.openSettings();
          }}
          onClose={() => setMenu(null)}
        />
      ) : null}
    </div>
  );
}
