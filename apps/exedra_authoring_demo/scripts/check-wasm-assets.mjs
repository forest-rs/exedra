// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFile, readdir } from "node:fs/promises";

const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");
const generated = new URL("../../../packages/exedra/dist/", import.meta.url);
const expected = await Promise.all(
  [
    "wasm_pkg/exedra_authoring_web_bridge_bg.wasm",
    "scene_wasm/exedra_scene_web_bridge_bg.wasm",
  ].map(async (name) => digest(await readFile(new URL(name, generated)))),
);
const assets = new URL("../dist/assets/", import.meta.url);
const names = (await readdir(assets)).filter((name) => name.endsWith(".wasm"));
assert.equal(
  names.length,
  expected.length,
  "ship only the gallery and combined scene Wasm",
);
const actual = await Promise.all(
  names.map(async (name) => {
    const bytes = await readFile(new URL(name, assets));
    console.log(`${name}: ${bytes.byteLength.toLocaleString("en-US")} bytes`);
    return digest(bytes);
  }),
);
assert.deepEqual(
  actual.sort(),
  expected.sort(),
  "production Wasm matches the built modules exactly",
);
console.log("No additional standalone Layerstack binary is shipped.");
