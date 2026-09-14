import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

// This config is loaded as ESM (package.json has "type": "module"), so
// __dirname does not exist - derive it from import.meta.url instead.
const here = dirname(fileURLToPath(import.meta.url));

// Tauri expects a fixed dev port and two html entry points (one per floating window).
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: false,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  build: {
    target: "chrome110",
    minify: "esbuild",
    sourcemap: false,
    rollupOptions: {
      input: {
        monitor: resolve(here, "monitor.html"),
        weather: resolve(here, "weather.html"),
        settings: resolve(here, "settings.html"),
      },
    },
  },
});
