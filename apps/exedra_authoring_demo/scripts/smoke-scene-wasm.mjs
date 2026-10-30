// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import {
  ArrayHandle,
  Layerstack,
  LayerstackError,
  PACKAGE_VERSION,
} from "@forest-rs/layerstack";
import { layerstackRevision } from "./check-layerstack-partner.mjs";

const path = new URL(
  "../../../packages/exedra/dist/scene_wasm/",
  import.meta.url,
);
const wasm = await import(new URL("exedra_scene_web_bridge.js", path));
const initialized = await wasm.default({
  module_or_path: await readFile(
    new URL("exedra_scene_web_bridge_bg.wasm", path),
  ),
});
const layerstack = new Layerstack({
  Stage: wasm.Stage,
  EditBatch: wasm.EditBatch,
  nativeVersion: wasm.nativeVersion,
  memory: initialized.memory,
});
assert.equal(layerstack.version.package, PACKAGE_VERSION);
assert.equal(layerstack.version.crate, PACKAGE_VERSION);
assert.equal(layerstack.version.revision, await layerstackRevision());
assert.equal(layerstack.version.abi, 3);

// Ordinary stages keep provenance optional; the geometry adapter requires it.
const ordinary = layerstack.open({
  identifier: "ordinary.usda",
  bytes: wasm.scene_root(),
});
try {
  assert.throws(
    () =>
      ordinary.withNative(
        "exedra.create",
        (native) => new wasm.SceneKernel(native, 3, crypto.randomUUID()),
      ),
    (error) =>
      error instanceof LayerstackError &&
      error.code === "scene.provenance" &&
      error.operation === "exedra.create",
  );
} finally {
  ordinary.free();
}

const stage = layerstack.open(
  { identifier: "scene.usda", bytes: wasm.scene_root() },
  { provenance: true },
);
const foreign = layerstack.open(
  { identifier: "other.usda", bytes: wasm.scene_root() },
  { provenance: true },
);
const changes = [];
const stop = stage.onChange((change) => changes.push(change));
const kernel = stage.withNative(
  "exedra.create",
  (native) => new wasm.SceneKernel(native, 3, crypto.randomUUID()),
);
const receipts = [];
let heldPoints;
let heldCopy;
const inspect = (name) => stage.attribute(`/World/PanelAsset.${name}`)?.get();
const sync = () =>
  JSON.parse(stage.withNative("exedra.sync", (native) => kernel.sync(native)));
const packet = () => {
  const owned = stage.withNative("exedra.pack", (native) =>
    kernel.pack(native),
  );
  try {
    const info = JSON.parse(owned.metadata());
    const positions = owned.take_positions();
    const normals = owned.take_normals();
    const matrices = owned.take_matrices();
    assert.equal(positions.byteLength + normals.byteLength, info.geometryBytes);
    assert.equal(matrices.byteLength, info.transformBytes);
    return { info, positions, normals, matrices };
  } finally {
    owned.free();
  }
};
try {
  assert.equal(stage.status().complete, true);
  const initial = sync();
  const first = packet();
  assert.equal(initial.stats.geometryEvaluations, 1);
  assert.equal(first.info.instanceCount, 3);
  assert.ok(first.info.geometryBytes > 0);
  const picked = JSON.parse(
    stage.withNative("exedra.pick", (native) =>
      kernel.pick(native, first.info.instances[0], 0, first.info.sourceScope),
    ),
  );
  assert.equal(picked.instance, first.info.instances[0]);
  assert.equal(picked.sourceScope, first.info.sourceScope);
  assert.ok(Number.isInteger(picked.nativeFace));
  assert.ok(Array.isArray(picked.tags));
  assert.ok(
    changes.some((change) =>
      [...change.created, ...change.resynced, ...change.changedInfoOnly].some(
        (p) => p.includes("PanelAsset"),
      ),
    ),
    "official onChange observes native initialization and publication",
  );
  const layers = stage.layers();
  assert.deepEqual(layers.map((layer) => layer.identifier).sort(), [
    "base.usda",
    "generated.usda",
    "overrides.usda",
    "scene.usda",
  ]);
  const overrides = layers.find(
    (layer) => layer.identifier === "overrides.usda",
  );
  assert.ok(overrides);
  const actualOutput = stage
    .children(first.info.source)
    .find((p) => stage.prim(p)?.typeName === "Mesh");
  assert.ok(
    actualOutput,
    "native publication is visible through the official Stage",
  );
  const points = stage.attribute(`${actualOutput}.points`)?.get();
  assert.equal(points?.kind, "array");
  assert.ok(points.array instanceof ArrayHandle);
  heldPoints = points.array;
  {
    // A borrowed view expires at the next module call. Retain only copy() data.
    const view = heldPoints.view();
    assert.ok(view instanceof Float32Array);
    assert.equal(
      view.buffer,
      initialized.memory.buffer,
      "the official binding views the combined module's memory",
    );
  }
  heldCopy = heldPoints.copy();
  assert.ok(heldCopy.length > 0);

  const noop = sync();
  const unchanged = packet();
  assert.equal(noop.stats.geometryEvaluations, 0);
  assert.equal(unchanged.info.geometryBytes, 0);
  assert.equal(unchanged.info.transformBytes, 0);

  const instance = first.info.instances[0];
  const move = stage.apply(
    [
      {
        type: "setValue",
        path: `${instance}.xformOp:translate`,
        value: { type: "double3", value: [50, 25, 10] },
      },
    ],
    overrides,
  );
  receipts.push(move);
  assert.deepEqual(
    stage.attribute(`${instance}.xformOp:translate`).get().value,
    [50, 25, 10],
  );
  assert.equal(sync().stats.geometryEvaluations, 0);
  const moved = packet();
  assert.equal(moved.info.geometryBytes, 0);
  assert.equal(moved.info.transformBytes, 16 * Float64Array.BYTES_PER_ELEMENT);
  assert.deepEqual(moved.info.instances, [instance]);
  move.undo().release();
  assert.equal(sync().stats.geometryEvaluations, 0);
  assert.equal(packet().info.geometryBytes, 0);

  const material = stage.apply(
    [
      { type: "definePrim", path: "/World/Blue", typeName: "Material" },
      {
        type: "setTargets",
        path: "/World/PanelAsset.material:binding",
        targets: ["/World/Blue"],
      },
    ],
    overrides,
  );
  receipts.push(material);
  assert.equal(sync().stats.geometryEvaluations, 0);
  assert.equal(packet().info.geometryBytes, 0);
  assert.deepEqual(
    stage.relationship("/World/PanelAsset.material:binding").targets(),
    ["/World/Blue"],
  );

  const width = stage.apply(
    [
      {
        type: "setValue",
        path: "/World/PanelAsset.primvars:example:width",
        value: { type: "double", value: 132 },
      },
    ],
    overrides,
  );
  receipts.push(width);
  assert.equal(inspect("primvars:example:width").value, 132);
  assert.equal(sync().stats.geometryEvaluations, 1);
  const edited = packet();
  assert.ok(edited.info.geometryBytes > 0);
  assert.notEqual(edited.info.sourceScope, first.info.sourceScope);
  assert.throws(
    () =>
      stage.withNative("exedra.pick", (native) =>
        kernel.pick(native, instance, 0, first.info.sourceScope),
      ),
    (error) =>
      error instanceof LayerstackError &&
      error.code === "scene.stale_pick" &&
      error.operation === "exedra.pick",
  );
  assert.throws(
    () => foreign.withNative("exedra.sync", (native) => kernel.sync(native)),
    (error) =>
      error instanceof LayerstackError &&
      error.code === "scene.stage_affinity" &&
      error.operation === "exedra.sync",
  );

  const invalid = stage.apply(
    [
      {
        type: "setValue",
        path: "/World/PanelAsset.primvars:example:depth",
        value: { type: "double", value: 14 },
      },
    ],
    overrides,
  );
  receipts.push(invalid);
  assert.throws(
    sync,
    (error) =>
      error instanceof LayerstackError &&
      error.code === "scene.geometry" &&
      error.operation === "exedra.sync",
  );
  assert.equal(inspect("primvars:example:depth").value, 14);
  invalid.undo().release();
  assert.equal(inspect("primvars:example:depth").value, 5);
  sync();
  const recovered = packet();

  stage.withNative("exedra.quality", () => kernel.set_display_tolerance(0.04));
  assert.equal(
    sync().stats.sourceEvaluations,
    0,
    "quality changes preserve native source geometry",
  );
  const refined = packet();
  assert.ok(refined.info.geometryBytes > 0);
  assert.equal(refined.info.sourceScope, recovered.info.sourceScope);
  assert.equal(
    JSON.parse(
      stage.withNative("exedra.pick", (native) =>
        kernel.pick(native, instance, 0, refined.info.sourceScope),
      ),
    ).sourceScope,
    recovered.info.sourceScope,
    "quality changes preserve native picking scope",
  );
  assert.equal(stage.muteLayer(overrides), true);
  assert.equal(stage.layer(overrides.id), overrides);
  assert.deepEqual(stage.mutedLayers(), [overrides]);
  assert.equal(changes.at(-1).mutedLayers[0], overrides);
  assert.equal(stage.muteLayer(overrides), false);
  assert.equal(inspect("primvars:example:width").value, 120);
  sync();
  packet();
  assert.equal(stage.unmuteLayer(overrides), true);
  assert.deepEqual(stage.mutedLayers(), []);
  assert.equal(changes.at(-1).unmutedLayers[0], overrides);
  assert.equal(inspect("primvars:example:width").value, 132);
  sync();
  packet();

  const saved = stage.save();
  assert.deepEqual(saved.failures, []);
  assert.ok(
    saved.written.some((item) => item.identifier === "generated.usda"),
    "save observes native publication",
  );
  assert.ok(
    saved.written.some((item) => item.identifier === "overrides.usda"),
    "save observes typed authoring edits",
  );
  const sources = new Map();
  for (const layer of layers) {
    const bytes = stage.exportLayer(layer, "usda");
    const text = new TextDecoder().decode(bytes);
    assert.ok(text.startsWith("#usda 1.0"));
    sources.set(layer.identifier, bytes);
    console.log(`${layer.identifier}: ${bytes.byteLength} readable USDA bytes`);
  }
  const reopened = layerstack.open(
    { identifier: "scene.usda", bytes: sources.get("scene.usda") },
    { provenance: true },
  );
  try {
    await reopened.resolve({
      fetch: async (identifier) => sources.get(identifier) ?? null,
    });
    assert.equal(reopened.status().complete, true);
    assert.deepEqual(reopened.status().compositionErrors, []);
    assert.equal(
      reopened.attribute("/World/PanelAsset.primvars:example:width").get()
        .value,
      132,
    );
    const reopenedPoints = reopened.attribute(`${actualOutput}.points`).get();
    assert.equal(reopenedPoints.kind, "array");
    try {
      assert.ok(reopenedPoints.array instanceof ArrayHandle);
      assert.ok(reopenedPoints.array.length > 0);
    } finally {
      reopenedPoints.array.release();
    }
  } finally {
    reopened.free();
  }
  assert.ok(layerstack.transfer().bytesOut > 0);
  assert.ok(layerstack.transfer().bytesViewed > 0);
  console.log(
    JSON.stringify({
      officialBinding: layerstack.version,
      layers: layers.length,
      initialTriangles: first.info.triangles,
      refinedTriangles: refined.info.triangles,
      changeBatches: changes.length,
      transfer: layerstack.transfer(),
    }),
  );
} finally {
  stop();
  for (const receipt of receipts) receipt.release();
  foreign.free();
  stage.free();
  kernel.free();
}
assert.equal(stage.disposed, true);
assert.throws(
  () => stage.status(),
  (error) =>
    error instanceof LayerstackError && error.code === "stage.disposed",
);
assert.deepEqual(
  heldPoints.copy(),
  heldCopy,
  "native array snapshots outlive scene and compiler owners",
);
heldPoints.release();
assert.throws(
  () => heldPoints.view(),
  (error) =>
    error instanceof LayerstackError && error.code === "array.released",
);
