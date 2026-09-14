import { useEffect, useState } from "react";
import { CONFIG_EVENT } from "./api";
import { listen } from "@tauri-apps/api/event";
import type { AppConfig } from "./types";

/**
 * Config that stays in sync with the backend. Structural options (which rings
 * to show, forecast length, city name...) need a re-render, while pure colours
 * are handled by CSS variables in `applyTheme`.
 */
export function useLiveConfig(initial: AppConfig): AppConfig {
  const [cfg, setCfg] = useState(initial);

  useEffect(() => {
    let alive = true;
    let unlisten: (() => void) | undefined;

    listen<AppConfig>(CONFIG_EVENT, (e) => {
      if (alive) setCfg(e.payload);
    })
      .then((u) => {
        if (alive) unlisten = u;
        else u();
      })
      .catch(() => {});

    return () => {
      alive = false;
      unlisten?.();
    };
  }, []);

  return cfg;
}
