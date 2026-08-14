import { defineConfig } from "vite";
import { sveltekit } from "@sveltejs/kit/vite";

const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [sveltekit()],

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri` AND the Rust build dir.
      // `target/` moved to the repo ROOT in the Phase 2b workspace split, so it's
      // no longer under `src-tauri/`. Watching it makes Vite race the linker on
      // `target/debug/deps/climax_lib.dll` and crash with EBUSY (resource busy)
      // whenever the Rust backend rebuilds - taking the whole `tauri dev` down.
      ignored: ["**/src-tauri/**", "**/target/**"],
    },
  },
}));
