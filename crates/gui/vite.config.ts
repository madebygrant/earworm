import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { homedir } from "node:os";

// The thumbnail cache, where the browser preview reads a recorded session's covers from.
const thumbs = `${process.env.XDG_CACHE_HOME || `${homedir()}/.cache`}/earworm/thumbs`;

// Tauri expects a fixed port and fails rather than drifting to another one.
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: { port: 1420, strictPort: true, watch: { ignored: ["**/src-tauri/**"] }, fs: { allow: ["../..", thumbs] } },
  test: { environment: "node" },
});
