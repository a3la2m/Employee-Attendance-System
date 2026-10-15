import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// منفذ 1420 مطابق لـ tauri.conf.json (devUrl)
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
});
