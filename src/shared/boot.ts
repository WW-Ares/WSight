import { createRoot } from "react-dom/client";
import type { ReactElement } from "react";
import { api, onConfig } from "./api";
import { applyTheme } from "./theme";
import { BASE_WIDTH } from "./uiScale";
import { DEFAULT_CONFIG, type AppConfig } from "./types";

function mount(render: (cfg: AppConfig) => ReactElement, cfg: AppConfig): void {
  const host = document.getElementById("root");
  if (host) {
    createRoot(host).render(render(cfg));
  }
}

/**
 * Swallow the webview's right-click menu.
 *
 * WebView2 pops up its own menu (刷新 / 另存为 / 检查…) on right-click. On a
 * floating desktop panel that is pure noise, and because the menu is drawn by
 * the platform it follows the *system* theme - a bright white box in the
 * middle of a dark widget.
 *
 * Text fields are exempted: there the menu is the only way to paste an API key,
 * and paste-by-keyboard is not something to take away from the user.
 */
function suppressContextMenu(): void {
  window.addEventListener(
    "contextmenu",
    (event) => {
      const target = event.target as Element | null;
      if (target?.closest?.("input, textarea, [contenteditable='true']")) {
        return;
      }
      event.preventDefault();
    },
    { capture: true },
  );
}

/**
 * Shared bootstrap for every window: load the persisted config, paint the
 * theme *before* the first render (no flash of the wrong colours), keep the
 * theme in sync with later changes and finally mount the React tree.
 *
 * `widget` lets a floating panel start at the zoom level it was left at, so
 * the first frame is not a card that is visibly too small for its window.
 */
export async function boot(
  render: (cfg: AppConfig) => ReactElement,
  options: { widget?: "monitor" | "weather" } = {},
): Promise<void> {
  suppressContextMenu();

  let cfg = DEFAULT_CONFIG;
  try {
    cfg = await api.getConfig();
  } catch {
    // keep the defaults so the window still renders something readable
  }

  if (options.widget) {
    const width =
      options.widget === "monitor" ? cfg.monitorWidth : cfg.weatherWidth;
    if (width > 0) {
      document.documentElement.style.setProperty(
        "--ui-scale",
        String(width / BASE_WIDTH),
      );
    }
  }

  applyTheme(cfg);
  mount(render, cfg);

  try {
    await onConfig((next) => applyTheme(next));
  } catch {
    // event system unavailable - the window just will not restyle live
  }
}
