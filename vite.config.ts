import { defineConfig } from "vite";
import { resolve } from "node:path";

// Oracle ships two entry points that share the same design system and API layer:
//   index.html  — the full application window
//   panel.html  — the floating tray panel
export default defineConfig({
  root: "src",
  publicDir: resolve(__dirname, "public"),
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    outDir: resolve(__dirname, "dist"),
    emptyOutDir: true,
    target: "chrome120",
    minify: "esbuild",
    sourcemap: false,
    rollupOptions: {
      input: {
        main: resolve(__dirname, "src/index.html"),
        panel: resolve(__dirname, "src/panel.html"),
      },
    },
  },
});
