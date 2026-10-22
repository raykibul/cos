import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

// Vite serves on 0.0.0.0 so `make ui-dev` works from Docker; Tauri's devUrl
// is http://localhost:5173, and `make ui-build` writes dist/ for frontendDist.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  server: { host: "0.0.0.0", port: 5173, strictPort: true },
  preview: { host: "0.0.0.0", port: 4173, strictPort: true },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    // WKWebView on macOS 13 and later.
    target: "safari16",
    chunkSizeWarningLimit: 2048,
  },
  test: {
    environment: "jsdom",
    setupFiles: ["src/test/setup.ts"],
    css: false,
    restoreMocks: true,
  },
});
