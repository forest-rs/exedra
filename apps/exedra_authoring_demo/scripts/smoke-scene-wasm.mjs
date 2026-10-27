// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { Stage } from "@layerstack/client";

const path = new URL("../../../packages/exedra/dist/scene_wasm/", import.meta.url);
const wasm = await import(new URL("exedra_scene_web_bridge.js", path));
await wasm.default({ module_or_path: await readFile(new URL("exedra_scene_web_bridge_bg.wasm", path)) });
const kernel = new wasm.SceneKernel(3, crypto.randomUUID());
const stage = await Stage.connect({ request: text => kernel.request(text) });
try {
  const sync = JSON.parse(kernel.request(JSON.stringify({ protocol: "exedra.scene", version: 1, stage: stage.stageId, store: stage.storeId, op: { type: "sync" } })));
  assert.equal(sync.ok, true, JSON.stringify(sync.error));
  const layers = (await stage.inspect()).layers;
  assert.deepEqual(layers.map(layer => layer.name).sort(), ["base.usda", "generated.usda", "overrides.usda", "scene.usda"]);
  const root = layers.find(layer => layer.name === "scene.usda");
  const overrides = layers.find(layer => layer.name === "overrides.usda");
  await stage.transaction(overrides, [
    { type: "definePrim", path: "/World/Placements/Additional" },
    { type: "addReference", path: "/World/Placements/Additional", layer: root, primPath: "/World/PanelAsset" },
    { type: "setInstanceable", path: "/World/Placements/Additional", value: true },
  ]);
  for (const layer of layers) {
    const saved = await stage.layerText(layer);
    assert.equal(saved.name, layer.name);
    assert.ok(saved.text.startsWith("#usda 1.0"));
    if (layer.name === "base.usda" || layer.name === "overrides.usda") assert.ok(saved.text.includes("</World/PanelAsset>"));
    console.log(`${saved.name}: ${saved.text.length} readable USDA characters`);
  }
} finally {
  await stage.dispose();
  kernel.free();
}
