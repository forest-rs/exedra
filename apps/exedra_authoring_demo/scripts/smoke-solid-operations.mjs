// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

// Run after the demo's build:sdk + build:wasm. This exercises the actual
// SDK → wasm-bindgen → authoring graph → analytic kernel → STEP path.
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import {
  AuthoringSession,
  ExedraError,
  extrude,
  profiles,
} from "@exedra/authoring";
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
const session = new AuthoringSession(new AuthoringKernel(), {
  units: "mm",
  separation: 1e-9,
});
const box = (
  width = 10,
  height = 10,
  depth = 10,
  translation = [0, 0, 0],
  feature = "stock",
  rotationXYZ = [0, 0, 0],
) =>
  extrude(profiles.rectangle({ width, height }).tagged(feature), {
    height: depth,
    placement: { translation, rotationXYZ },
  });
const stock = box(),
  other = box(10, 10, 10, [5, 0, 0], "tool");
const results = [];
function verifyDisplayAndStep(part, shells = 1, components = 1) {
  const display = part.realize({ tolerance: 0.12 });
  assert.equal(display.report.withinTolerance, true);
  assert.ok(display.indices.length > 0);
  assert.ok(display.positions.every(Number.isFinite));
  assert.equal(display.nativeFaces.length * 3, display.indices.length);
  for (const id of display.nativeFaces)
    assert.ok(part.faces().some((face) => face.nativeId === id));
  if (components > 1) {
    assert.throws(
      () => part.exportStep({ timestamp: "2026-10-29T00:00:00Z" }),
      (error) =>
        error instanceof ExedraError &&
        error.diagnostic.code === "exchange.step",
      "one STEP solid cannot contain disconnected material",
    );
    return;
  }
  const file = part.exportStep({ timestamp: "2026-10-29T00:00:00Z" });
  assert.ok(
    file.text.includes("MANIFOLD_SOLID_BREP(") ||
      file.text.includes("BREP_WITH_VOIDS("),
  );
  assert.equal(file.report.faces, part.measurements.faces);
  assert.equal(file.report.shells, shells);
  assert.equal(file.report.solids, 1);
  if (shells > 1) {
    const material = file.report.materialValidation;
    assert.equal(material.family, "rectilinear");
    assert.equal(material.voidShells.length, shells - 1);
    assert.equal(material.shellFaces.length, shells);
    assert.equal(material.volume, part.measurements.volume.value);
    assert.ok(
      material.cells > 0 && material.faceTests <= material.faceTestBound,
    );
    assert.ok(material.planeVertices <= 8 * material.cells);
    assert.ok(
      material.arrangementScratchBytes >= material.linkBytes &&
        material.linkBytes > 0,
    );
    assert.ok(
      file.report.maxDeviation >=
        material.maxNativeSupportGap + material.arithmeticAllowance,
    );
    const reversed = material.voidShells
      .flatMap((shell) => material.shellFaces[shell])
      .sort((a, b) => a - b);
    assert.deepEqual(file.report.correspondence.reversedFaces, reversed);
    assert.ok(
      reversed.every((face) => file.report.correspondence.faces[face] !== null),
    );
  } else {
    assert.equal(file.report.materialValidation, null);
    assert.deepEqual(file.report.correspondence.reversedFaces, []);
  }
}
function evidence(part) {
  assert.equal(part.solidOperations.coordinateSpace, "construction");
  assert.ok(Object.isFrozen(part.solidOperations.operations));
  return part.solidOperations.operations.at(-1);
}
try {
  for (const [kind, solid, volume, area, components, cavities] of [
    ["union", stock.union(other), 1500, 800, 1, 0],
    ["intersect", stock.intersect(other), 500, 400, 1, 0],
    ["subtract", stock.subtract(other), 500, 400, 1, 0],
    ["cavity", stock.subtract(box(2, 2, 2, [0, 0, 4], "tool")), 992, 624, 1, 1],
    [
      "disconnected",
      stock.union(box(10, 10, 10, [20, 0, 0], "tool")),
      2000,
      1200,
      2,
      0,
    ],
  ]) {
    const document = session.build({ result: solid });
    try {
      const part = document.parts.result,
        report = evidence(part);
      assert.equal(part.family, "body");
      assert.deepEqual(part.measurements.volume, {
        status: "available",
        value: volume,
        basis: "nativeBoolean",
      });
      assert.deepEqual(part.measurements.surfaceArea, {
        status: "available",
        value: area,
        basis: "nativeBoolean",
      });
      assert.equal(report.volume, volume);
      assert.equal(report.surfaceArea, area);
      assert.ok(
        report.volumeBounds[0] <= volume && volume <= report.volumeBounds[1],
      );
      assert.ok(
        report.surfaceAreaBounds[0] <= area &&
          area <= report.surfaceAreaBounds[1],
      );
      assert.equal(report.materialComponents, components);
      assert.equal(report.cavities, cavities);
      assert.equal(report.boundaryShells, components + cavities);
      assert.ok(
        report.cells > 0 &&
          report.faceTests > 0 &&
          report.faceTests <= report.faceTestBound,
      );
      assert.equal(report.faceSources.length, part.faces().length);
      for (const sources of report.faceSources) assert.ok(sources.length > 0);
      if (kind === "cavity") {
        assert.ok(
          report.faceSources.some((sources) =>
            sources.some(
              (source) => source.operand === "right" && source.reversed,
            ),
          ),
        );
        assert.ok(
          part.faces("tool").length > 0,
          "difference faces retain tool feature tags through typed origins",
        );
      }
      assert.ok(Object.isFrozen(report.faceSources[0][0]));
      verifyDisplayAndStep(
        part,
        report.boundaryShells,
        report.materialComponents,
      );
      results.push({
        kind,
        volume,
        area,
        components,
        cavities,
        cells: report.cells,
        faceTests: report.faceTests,
      });
    } finally {
      document.dispose();
    }
  }
  const rotated = session.build({
    result: box(10, 10, 10, [20, 30, 40], "stock", [0, 0, 0.7]).union(
      box(10, 10, 10, [20, 30, 45], "tool", [0, 0, 0.7]),
    ),
  });
  try {
    assert.equal(rotated.parts.result.measurements.volume.value, 1500);
    const [lower, upper] = rotated.parts.result.measurements.bounds;
    assert.ok(
      lower[2] <= 40 && lower[2] >= 40 - 1e-9,
      "outward lower bound retains the placed source",
    );
    assert.ok(
      upper[2] >= 55 && upper[2] <= 55 + 1e-9,
      "outward upper bound retains the placed source",
    );
  } finally {
    rotated.dispose();
  }
  const clipped = session.build({
    result: stock.intersectHalfSpace({
      point: [0, 0, 5],
      normal: [0, 0, 1],
      feature: "cut",
    }),
  });
  try {
    const part = clipped.parts.result,
      report = evidence(part);
    assert.equal(report.kind, "intersectHalfSpace");
    assert.equal(report.outcome, "clipped");
    assert.equal(part.measurements.volume.status, "unavailable");
    assert.equal(part.measurements.surfaceArea.status, "unavailable");
    assert.equal(part.faces("cut").length, 1);
    assert.equal(part.faces("cut")[0].nativeId, report.cap);
    assert.ok(report.splits.length > 0);
    verifyDisplayAndStep(part);
    results.push({
      kind: "intersectHalfSpace",
      outcome: report.outcome,
      cap: report.cap,
      splits: report.splits.length,
    });
    const before = session.stats().documents;
    for (const [bad, code, operation] of [
      [
        stock.intersect(box(10, 10, 10, [30, 0, 0])),
        "boolean.empty",
        "solid.intersect",
      ],
      [
        stock.intersectHalfSpace({ point: [0, 0, -1], normal: [0, 0, 1] }),
        "halfspace.empty",
        "solid.intersectHalfSpace",
      ],
      [
        stock.union(box(10, 10, 10, [0, 0, 0], "tool", [0, 0, Math.PI / 2])),
        "boolean.frameMismatch",
        "solid.union",
      ],
      [
        stock.intersectHalfSpace({ point: [0, 0, 0], normal: [0, 0, 1] }),
        "halfspace.nearVertex",
        "solid.intersectHalfSpace",
      ],
    ]) {
      assert.throws(
        () => session.build({ result: bad }),
        (error) =>
          error instanceof ExedraError &&
          error.diagnostic.code === code &&
          error.diagnostic.operation === operation &&
          Number.isInteger(error.diagnostic.node),
      );
      assert.equal(session.stats().documents, before);
      assert.equal(part.faces("cut").length, 1);
    }
    verifyDisplayAndStep(part);
  } finally {
    clipped.dispose();
  }
} finally {
  session.dispose();
}
console.log(JSON.stringify({ solidOperations: results }, null, 2));
