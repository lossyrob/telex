import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// In a browser the UI talks to `station-headless` through this proxy; inside
// Tauri it calls the engine in-process and the proxy is unused.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1430,
    strictPort: true,
    proxy: { "/api": { target: `http://127.0.0.1:${process.env.STATION_PORT_HTTP ?? 4317}`, changeOrigin: false } },
  },
  test: { environment: "node", include: ["src/**/*.test.ts"] },
});
