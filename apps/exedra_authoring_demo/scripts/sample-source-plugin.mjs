// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

import { readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { highlightTypeScript } from "./highlight-typescript.mjs";

const samplePath = fileURLToPath(new URL("../src/sample.ts", import.meta.url));
const virtualId = "\0exedra-highlighted-sample";

/** Compile the demo's runnable sample into escaped code-view HTML at build time. */
export function sampleSourcePlugin() {
  return {
    name: "exedra-sample-source",
    enforce: "pre",
    resolveId(id, importer) {
      if (
        id === "./sample.ts?highlighted" &&
        importer &&
        resolve(dirname(importer), "sample.ts") === samplePath
      )
        return virtualId;
    },
    async load(id) {
      if (id !== virtualId) return;
      this.addWatchFile(samplePath);
      const html = highlightTypeScript(await readFile(samplePath, "utf8"));
      return `export default ${JSON.stringify(html)};`;
    },
  };
}
