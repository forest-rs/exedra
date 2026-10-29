import { defineConfig } from "vite";
import { fileURLToPath } from "node:url";
import { sampleSourcePlugin } from "./scripts/sample-source-plugin.mjs";

export default defineConfig({
  plugins: [sampleSourcePlugin()],
  server: {
    fs: { allow: [fileURLToPath(new URL("../../", import.meta.url))] },
  },
  build: {
    rollupOptions: {
      input: {
        main: fileURLToPath(new URL("./index.html", import.meta.url)),
        scene: fileURLToPath(new URL("./scene.html", import.meta.url)),
      },
    },
  },
});
