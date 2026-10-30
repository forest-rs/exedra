// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

import assert from "node:assert/strict";
import test from "node:test";
import { mountViews } from "../src/views.ts";
import * as THREE from "three";
import { meshTriangles } from "../src/mesh-overlay.ts";
import { updateInstanceBounds } from "../src/scene-bounds.ts";
const settle = () => new Promise((resolve) => setImmediate(resolve));
function harness(loadScene: Parameters<typeof mountViews>[0]["loadScene"]) {
  const select = Object.assign(new EventTarget(), {
    value: "geometry",
  }) as unknown as HTMLSelectElement;
  const element = () => ({ hidden: false, textContent: "" }) as HTMLElement;
  const geometry = [element(), element(), element()],
    scene = element(),
    sceneControls = element(),
    geometryActivity: boolean[] = [];
  const views = mountViews({
    select,
    geometry,
    scene,
    sceneControls,
    loadScene,
    setGeometryActive: (active) => geometryActivity.push(active),
  });
  return {
    select,
    geometry,
    scene,
    sceneControls,
    views,
    geometryActivity,
    choose(value: string) {
      select.value = value;
      select.dispatchEvent(new Event("change"));
    },
  };
}
test("Scene loads lazily and one native stage survives view switches", async () => {
  let imports = 0,
    mounts = 0,
    disposals = 0;
  const activity: boolean[] = [];
  const h = harness(async () => {
    imports++;
    return {
      async mountScene() {
        mounts++;
        return {
          setActive(active: boolean) {
            activity.push(active);
          },
          dispose() {
            disposals++;
          },
        };
      },
    };
  });
  assert.equal(imports, 0);
  h.choose("scene");
  await settle();
  assert.equal(mounts, 1);
  assert.ok(h.geometry.every((e) => e.hidden));
  assert.equal(h.sceneControls.hidden, false);
  h.choose("geometry");
  assert.equal(activity.at(-1), false);
  assert.equal(disposals, 0);
  assert.ok(h.geometry.every((e) => !e.hidden));
  h.choose("scene");
  assert.equal(activity.at(-1), true);
  assert.equal(imports, 1);
  assert.equal(mounts, 1);
  h.views.dispose();
  h.views.dispose();
  assert.equal(disposals, 1);
});
test("a late Scene mount is retained inactive and repeated clicks cannot create duplicate stages", async () => {
  let complete!: (value: {
    setActive(active: boolean): void;
    dispose(): void;
  }) => void;
  let mounts = 0,
    disposals = 0;
  const activity: boolean[] = [];
  const h = harness(async () => ({
    mountScene() {
      mounts++;
      return new Promise((resolve) => {
        complete = resolve;
      });
    },
  }));
  h.choose("scene");
  await settle();
  h.choose("scene");
  h.choose("geometry");
  complete({
    setActive: (active) => activity.push(active),
    dispose() {
      disposals++;
    },
  });
  await settle();
  assert.equal(mounts, 1);
  assert.deepEqual(activity, [false]);
  assert.equal(disposals, 0);
  h.choose("scene");
  assert.equal(activity.at(-1), true);
  h.views.dispose();
  assert.equal(disposals, 1);
});
test("teardown releases an in-flight late mount exactly once", async () => {
  let complete!: (value: {
    setActive(active: boolean): void;
    dispose(): void;
  }) => void;
  let disposals = 0;
  const h = harness(async () => ({
    mountScene: () =>
      new Promise((resolve) => {
        complete = resolve;
      }),
  }));
  h.choose("scene");
  await settle();
  h.views.dispose();
  complete({
    setActive() {
      throw Error("disposed view activated");
    },
    dispose() {
      disposals++;
    },
  });
  await settle();
  assert.equal(disposals, 1);
});
test("failed lazy imports remain diagnosable and a new selection retries", async () => {
  let attempts = 0;
  const h = harness(() => {
    if (attempts++ === 0) throw Error("unavailable kernel");
    return Promise.resolve({
      async mountScene() {
        return { setActive() {}, dispose() {} };
      },
    });
  });
  h.choose("scene");
  await settle();
  assert.equal(h.scene.textContent, "unavailable kernel");
  h.choose("geometry");
  h.choose("scene");
  await settle();
  assert.equal(attempts, 2);
  h.views.dispose();
});

test("triangle overlays retain one surface geometry and preserve native mesh picking", () => {
  const geometry = new THREE.SphereGeometry(2, 16, 8),
    material = new THREE.MeshStandardMaterial(),
    surface = new THREE.Mesh(geometry, [material]);
  geometry.addGroup(0, geometry.index!.count, 0);
  const overlay = meshTriangles(surface, false),
    group = new THREE.Group();
  group.add(surface, overlay.mesh);
  const position = geometry.getAttribute("position").array,
    index = geometry.index;
  let geometryReleases = 0,
    surfaceReleases = 0,
    overlayReleases = 0;
  geometry.addEventListener("dispose", () => geometryReleases++);
  material.addEventListener("dispose", () => surfaceReleases++);
  overlay.mesh.material.addEventListener("dispose", () => overlayReleases++);
  for (let i = 0; i < 20; i++) overlay.setVisible(i % 2 === 0);
  overlay.setVisible(true);
  group.updateMatrixWorld(true);
  assert.strictEqual(overlay.mesh.geometry, geometry);
  assert.strictEqual(geometry.getAttribute("position").array, position);
  assert.strictEqual(geometry.index, index);
  assert.equal(material.wireframe, false);
  assert.equal(material.polygonOffset, true);
  assert.equal(overlay.mesh.material.depthTest, true);
  assert.equal(overlay.mesh.material.depthWrite, false);
  const ray = new THREE.Raycaster(
    new THREE.Vector3(0, 0, 5),
    new THREE.Vector3(0, 0, -1),
  );
  const hits = ray.intersectObjects([surface]);
  assert.ok(hits.length > 0);
  assert.ok(
    hits.every(
      (hit) => hit.object === surface && Number.isInteger(hit.faceIndex),
    ),
  );
  overlay.dispose();
  assert.equal(overlayReleases, 1);
  assert.equal(geometryReleases, 0);
  assert.equal(surfaceReleases, 0);
  geometry.dispose();
  material.dispose();
  assert.equal(geometryReleases, 1);
  assert.equal(surfaceReleases, 1);
});

test("a larger prototype updates instance raycast bounds with zero transform transfer", () => {
  const original = new THREE.BoxGeometry(2, 2, 2);
  const expanded = new THREE.BoxGeometry(20, 2, 2);
  const material = new THREE.MeshBasicMaterial();
  const mesh = new THREE.InstancedMesh(original, material, 1);
  mesh.setMatrixAt(0, new THREE.Matrix4().makeTranslation(5, 0, 0));
  mesh.updateMatrixWorld(true);
  mesh.computeBoundingSphere();
  mesh.computeBoundingBox();
  const oldRadius = mesh.boundingSphere!.radius;
  const matrices = mesh.instanceMatrix.array.slice();
  const ray = new THREE.Raycaster(
    new THREE.Vector3(13, 0, 5),
    new THREE.Vector3(0, 0, -1),
  );
  assert.equal(ray.intersectObject(mesh).length, 0);

  mesh.geometry = expanded;
  assert.equal(
    ray.intersectObject(mesh).length,
    0,
    "the inherited cached sphere rejects a valid hit outside the old extent",
  );
  updateInstanceBounds(mesh, true, false);
  assert.deepEqual(mesh.instanceMatrix.array, matrices);
  assert.ok(mesh.boundingSphere!.radius > oldRadius);
  assert.equal(mesh.boundingBox, null);
  const hits = ray.intersectObject(mesh);
  assert.ok(hits.length > 0);
  assert.ok(hits.every((hit) => hit.instanceId === 0));
  mesh.computeBoundingBox();
  assert.ok(mesh.boundingBox!.max.x >= 15);
  mesh.dispose();
  original.dispose();
  expanded.dispose();
  material.dispose();
});
