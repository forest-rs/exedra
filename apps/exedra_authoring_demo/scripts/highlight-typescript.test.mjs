// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { highlightTypeScript } from "./highlight-typescript.mjs";
import { sampleSourcePlugin } from "./sample-source-plugin.mjs";

function renderedText(html) {
  return html
    .replace(
      /<span class="ts-(keyword|string|number|comment|punctuation)">|<\/span>/g,
      "",
    )
    .replace(
      /&(amp|lt|gt|quot|#39|#13);/g,
      (entity) =>
        ({
          "&amp;": "&",
          "&lt;": "<",
          "&gt;": ">",
          "&quot;": '"',
          "&#39;": "'",
          "&#13;": "\r",
        })[entity],
    );
}

test("HTML in comments and literals remains escaped source text", () => {
  const source = `/** <b>doc & \"quotes\"</b> */\r\n\tconst html = '<img src=x onerror=alert(1)>'; // </span> &\n`;
  const html = highlightTypeScript(source);
  assert.equal(renderedText(html), source);
  assert.ok(!html.includes("<img"));
  assert.ok(html.includes("&#13;\n"));
  assert.ok(!html.includes("<b>"));
  assert.ok(html.includes('<span class="ts-comment">/** &lt;b&gt;'));
  assert.ok(html.includes('<span class="ts-keyword">const</span>'));
  assert.ok(html.includes('<span class="ts-string">&#39;&lt;img'));
});

test("nested template expressions and regex literals use compiler token boundaries", () => {
  const source =
    "const text = `outer ${ { value: `inner ${count + 2}` }.value } <tail>`;\nconst matches = /[}<>]/g.test(text);\n";
  const html = highlightTypeScript(source);
  assert.equal(renderedText(html), source);
  assert.ok(html.includes('<span class="ts-string">`outer ${</span>'));
  assert.ok(html.includes('<span class="ts-string">`inner ${</span>'));
  assert.ok(html.includes('<span class="ts-number">2</span>'));
  assert.ok(html.includes('<span class="ts-string">} &lt;tail&gt;`</span>'));
  assert.ok(html.includes('<span class="ts-string">/[}&lt;&gt;]/g</span>'));
});

test("empty input and trailing comment trivia preserve their complete text", () => {
  for (const source of [
    "",
    " \n\t",
    "// <end> &\n",
    "const n = 1n; /* end */\n\n",
  ]) {
    assert.equal(renderedText(highlightTypeScript(source)), source);
  }
});

test("Vite exposes only the demo sample and generates HTML from its actual source", async () => {
  const plugin = sampleSourcePlugin();
  const main = fileURLToPath(new URL("../src/main.ts", import.meta.url));
  const sample = fileURLToPath(new URL("../src/sample.ts", import.meta.url));
  assert.equal(plugin.resolveId("./other.ts?highlighted", main), undefined);
  assert.equal(
    plugin.resolveId("./sample.ts?highlighted", "/elsewhere/main.ts"),
    undefined,
  );
  assert.equal(
    plugin.resolveId("./sample.ts?highlighted", undefined),
    undefined,
  );
  assert.equal(await plugin.load.call({}, "/elsewhere/sample.ts"), undefined);
  const id = plugin.resolveId("./sample.ts?highlighted", main);
  assert.ok(id);
  const watched = [];
  const module = await plugin.load.call(
    { addWatchFile: (path) => watched.push(path) },
    id,
  );
  assert.deepEqual(watched, [sample]);
  const html = JSON.parse(module.slice("export default ".length, -1));
  assert.equal(renderedText(html), await readFile(sample, "utf8"));
});
