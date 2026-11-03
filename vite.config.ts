import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

const host = process.env.TAURI_DEV_HOST;

// Tauri expects a fixed port; src-tauri and Rust target dirs are not watched.
export default defineConfig(() => ({
  plugins: [react()],
  clearScreen: false,
  build: {
    target: "es2022",
    cssMinify: true,
    reportCompressedSize: false,
  },
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: { ignored: ["**/src-tauri/**", "**/crates/**", "**/target/**"] },
  },
}));
