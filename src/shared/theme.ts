import type { AppConfig } from "./types";

/**
 * Paint a config onto the document. Called once on boot and again every time
 * the backend broadcasts `config://changed`, so both widgets react live while
 * the user drags sliders in the settings window.
 *
 * The opacity slider feeds `--panel-opacity`, which is consumed by the panel
 * *background colour* only. It used to be `documentElement.style.opacity`,
 * which faded the text, the gauges and the icons along with the card: at 40%
 * the readouts were barely legible. Now the card gets more transparent while
 * everything drawn on it stays at full contrast.
 */
export function applyTheme(cfg: AppConfig): void {
  const root = document.documentElement;
  root.dataset.theme = cfg.theme;
  root.style.setProperty("--radius", `${cfg.radius}px`);
  root.style.setProperty("--panel-opacity", String(cfg.opacity));
  root.style.setProperty("--accent-cpu", cfg.colorCpu);
  root.style.setProperty("--accent-mem", cfg.colorMem);
  root.style.setProperty("--accent-gpu", cfg.colorGpu);
  root.style.setProperty("--accent-net", cfg.colorNet);
}
