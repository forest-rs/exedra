import { defineConfig } from "vite";
import { fileURLToPath } from "node:url";

export default defineConfig({
  server: { fs: { allow: [fileURLToPath(new URL("../../", import.meta.url))] } },
});
