// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

// Exercise actual release Wasm, retained evidence, independent controls and
// recovery. No mock transport stands in for geometry or exchange here.
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { AuthoringSession, ExedraError } from "@exedra/authoring";
import {
  authorValveBlock,
  authorFusedHousing,
  authorHousing,
} from "../src/sample.ts";
import {
  AuthoringKernel,
  default as initialize,
} from "../../../packages/exedra/dist/wasm_pkg/exedra_authoring_web_bridge.js";

await initialize({
  module_or_path: await readFile(
    new URL(
      "../../../packages/exedra/dist/wasm_pkg/exedra_authoring_web_bridge_bg.wasm",
      import.meta.url,
    ),
  ),
});
const defaults = {
  width: 112,
  depth: 72,
  height: 42,
  topAngle: 12,
  portRadius: 8,
  counterboreRadius: 14,
  counterboreDepth: 10,
  blindDepth: 14,
  chamberRadius: 7,
  chamberSpacing: 10,
  chamberHeight: 14,
};
const session = new AuthoringSession(new AuthoringKernel(), { units: "mm" });
const document = session.build(authorValveBlock(defaults));
const block = document.parts.block;
const observations = [];
try {
  const operations = block.solidOperations.operations;
  const passages = operations.find(
    (operation) => operation.kind === "subtractParallelPassages",
  );
  const bores = operations.filter(
    (operation) => operation.kind === "subtractBore",
  );
  assert.equal(operations.length, 6);
  assert.equal(passages.openingComponents, 1);
  assert.equal(passages.intersectionVertices, 2);
  assert.equal(passages.pairTests, 1);
  assert.equal(passages.walls.length, passages.wallTools.length);
  assert.deepEqual(new Set(passages.wallTools), new Set([0, 1]));
  assert.equal(bores.length, 5);
  assert.equal(bores[0].shoulders.length, 1);
  assert.equal(bores[0].floor, null);
  assert.equal(bores.filter((bore) => bore.floor !== null).length, 4);
  assert.ok(Object.isFrozen(passages.wallTools));
  assert.ok(Object.isFrozen(bores[0].steps[0]));
  assert.equal(block.faces("joined chamber").length, passages.walls.length);
  assert.equal(block.faces("stepped port").length, 3);
  assert.equal(block.faces("blind fasteners").length, 8);
  assert.equal(block.measurements.volume.status, "available");
  const boundary = block.validateBoundary();
  assert.equal(boundary.orientedManifold, true);
  assert.equal(boundary.shells.length, 1);
  assert.equal(boundary.globalSolidValidity, "notChecked");
  assert.ok(
    boundary.faceTrims.some((trim) => trim.status === "unsupported"),
    "mixed-arc trims must remain explicit unsupported checks",
  );
  assert.throws(
    () => block.validateBoundary({ maxCoedges: 1 }),
    (error) =>
      error instanceof ExedraError &&
      error.diagnostic.code === "boundary.workLimit",
  );

  for (const tolerance of [0.35, 0.12, 0.035]) {
    const display = block.realize({ tolerance });
    assert.equal(display.report.withinTolerance, true);
    assert.ok(display.positions.every(Number.isFinite));
    for (const bore of bores) {
      if (bore.floor !== null)
        assert.ok(display.nativeFaces.includes(bore.floor));
      for (const shoulder of bore.shoulders)
        assert.ok(display.nativeFaces.includes(shoulder));
    }
    observations.push({
      tolerance,
      triangles: display.report.triangles,
      buildKernelMs: document.buildTiming.kernelMs,
      realizeKernelMs: display.timing.kernelMs,
    });
  }
  const file = block.exportStep({
    timestamp: "2026-10-29T00:00:00Z",
    tolerance: 1e-6,
  });
  assert.ok(file.text.includes("MANIFOLD_SOLID_BREP("));
  assert.equal(file.report.faces, block.faces().length);
  for (const [key, value] of [
    ["width", 118],
    ["depth", 78],
    ["height", 46],
    ["topAngle", 16],
    ["portRadius", 9],
    ["counterboreRadius", 15],
    ["counterboreDepth", 12],
    ["blindDepth", 16],
    ["chamberRadius", 7.5],
    ["chamberSpacing", 11],
    ["chamberHeight", 15],
  ]) {
    const edited = session.build(
      authorValveBlock({ ...defaults, [key]: value }),
    );
    try {
      assert.notEqual(edited.parts.block.fingerprint, block.fingerprint, key);
      assert.equal(
        edited.parts.block.realize({ tolerance: 0.12 }).report.withinTolerance,
        true,
        key,
      );
    } finally {
      edited.dispose();
    }
  }
  const stableFingerprint = block.fingerprint;
  for (const [change, code] of [
    [{ blindDepth: 50 }, "bore.depthClearance"],
    [{ counterboreRadius: 6 }, "bore.invalidStep"],
    [{ chamberSpacing: 14 }, "passages.contact"],
  ]) {
    assert.throws(
      () => session.build(authorValveBlock({ ...defaults, ...change })),
      (error) => {
        assert.ok(error instanceof ExedraError);
        assert.equal(error.diagnostic.code, code);
        assert.equal(error.timing.outcome, "refused");
        if (code.startsWith("bore."))
          assert.ok(Number.isInteger(error.diagnostic.step));
        if (code === "passages.contact") {
          assert.equal(error.diagnostic.leftTool, 0);
          assert.equal(error.diagnostic.rightTool, 1);
        }
        return true;
      },
    );
    assert.equal(session.stats().documents, 1);
    assert.equal(block.fingerprint, stableFingerprint);
    assert.equal(
      block.realize({ tolerance: 0.12 }).report.withinTolerance,
      true,
    );
  }
  // Fractional edits must keep lug/body planes identical in the authoring
  // input; strict native Booleans must not need near-coordinate snapping.
  for (const width of [104.01, 104.15, 105.37]) {
    const parameters = {
      width,
      bodyWidth: 64,
      height: 90,
      flangeDepth: 24,
      thickness: 18,
      passageRadius: 12,
      passageY: 15,
      passageAngle: 20,
      mountSpacing: 76,
      mountRadius: 5,
      portY: -10,
      portRadius: 5,
      clearance: 1,
    };
    const edited = session.build({
      fused: authorFusedHousing(parameters).housing,
      outlined: authorHousing(parameters).housing,
    });
    try {
      assert.ok(
        Math.abs(
          edited.parts.fused.measurements.volume.value -
            edited.parts.outlined.measurements.volume.value,
        ) < 1e-8,
      );
      assert.equal(
        edited.parts.fused.realize({ tolerance: 0.12 }).report.withinTolerance,
        true,
      );
    } finally {
      edited.dispose();
    }
  }
  console.log(
    JSON.stringify(
      {
        sample: "valveBlock",
        independentControls: 11,
        joinedOpeningComponents: passages.openingComponents,
        nativeOperations: operations.length,
        shoulders: 1,
        blindFloors: 4,
        recoveryCases: 3,
        stepBytes: file.text.length,
        observations,
      },
      null,
      2,
    ),
  );
} finally {
  document.dispose();
  session.dispose();
}
