import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";

export default defineConfig({
  server: {
    port: 5173,
    strictPort: true,
  },
  build: {
    rollupOptions: {
      input: {
        main: fileURLToPath(new URL("./index.html", import.meta.url)),
        inspector: fileURLToPath(new URL("./inspector.html", import.meta.url)),
      },
    },
  },
});
