// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import {
  Stage,
  type Layer,
  type UndoReceipt,
  type Edit,
} from "@layerstack/client";
import "./scene.css";

interface Work {
  stats: Record<string, number>;
  source: {
    volume: number;
    surfaceArea: number;
    scope: string;
    fingerprint: string;
  };
  accuracy: {
    tolerance: number;
    maxDeviation: number;
    withinTolerance: boolean;
  };
}
interface PacketInfo {
  geometryChanged: boolean;
  resetInstances: boolean;
  instances: string[];
  source: string;
  sourceScope: string;
  triangles: number;
  nativePicking: boolean;
  geometryBytes: number;
  transformBytes: number;
  instanceCount: number;
  transformQueries: number;
}
interface Controller {
  setActive(active: boolean): void;
  dispose(): void;
}
const number = (value: number, digits = 2) =>
  value.toLocaleString(undefined, { maximumFractionDigits: digits });

/** App-only view. Generic native Stage ownership belongs to the injected client. */
export async function mountScene({
  container,
  controls,
}: {
  container: HTMLElement;
  controls?: HTMLElement;
}): Promise<Controller> {
  const wasm =
    await import("../../../packages/exedra/dist/scene_wasm/exedra_scene_web_bridge.js");
  await wasm.default();
  container.innerHTML = `<section class="native-scene">
    <aside class="scene-controls"><h2>Native stage</h2>
      <label>Instances <select data-field="count"><option>1</option><option>16</option><option selected>100</option><option>1000</option></select></label>
      <label>Panel width · mm <input data-field="width" type="range" min="100" max="180" value="120"><output data-value="width">120</output></label>
      <label>Recess depth · mm <input data-field="depth" type="range" min="1" max="14" step="0.5" value="5"><output data-value="depth">5</output></label>
      <label>Source-local tolerance · mm <select data-field="quality"><option value="0.5">0.5</option><option value="0.2" selected>0.2</option><option value="0.04">0.04</option></select></label>
      <label class="scene-check"><input data-field="overrides" type="checkbox" checked> Enable override layer</label>
      <div class="scene-buttons"><button data-action="undo">Undo edit</button><button data-action="noop">No-op sync</button></div>
      <h2>Instance placement</h2>
      <label>Instance <input data-field="instance" type="number" min="0" value="0" step="1"></label>
      <div class="scene-coordinates">${["x", "y", "z"].map((axis) => `<label>${axis.toUpperCase()} · mm<input data-field="${axis}" type="number" step="5" value="0"></label>`).join("")}</div>
      <button data-action="move">Apply translation</button>
      <p class="scene-note">Parameters, overrides, references and placements are native stage data. GPU occurrences use f32 matrices, separately from source-local geometry accuracy.</p>
      <h2>Layers</h2><select data-field="layer"></select>
      <div class="scene-buttons"><button data-action="inspect">Inspect USD</button><button data-action="download">Download layer</button></div>
      <p class="scene-note">Keep scene.usda, base.usda, overrides.usda and generated.usda together.</p>
    </aside>
    <div class="scene-viewport"><div class="scene-toolbar"><button data-action="fit">Fit instances</button><label><input data-field="wireframe" type="checkbox"> Triangles</label><span data-value="scene-label"></span></div><div class="scene-message" data-value="message" role="status">Loading native stage…</div><pre class="scene-layer" data-value="layer-text" hidden></pre></div>
    <aside class="scene-evidence"><h2>Last operation</h2><div data-value="operation">—</div><dl data-value="timings"></dl><h2>Native work</h2><dl data-value="work"></dl><h2>Transfer</h2><dl data-value="transfer"></dl><h2>Source</h2><dl data-value="source"></dl><h2>Selection</h2><pre data-value="selection">Pick an instance surface.</pre></aside>
  </section>`;
  const root = container.querySelector<HTMLElement>(".native-scene")!;
  if (controls) {
    const panel = root.querySelector<HTMLElement>(".scene-controls")!;
    panel.classList.add("scene-controls-external");
    controls.replaceChildren(panel);
    root.classList.add("scene-external-controls");
  }
  const query = <T extends Element>(selector: string) =>
    root.querySelector<T>(selector) ??
    controls?.querySelector<T>(selector) ??
    null;
  const field = <T extends HTMLInputElement | HTMLSelectElement>(
    name: string,
  ) => query<T>(`[data-field="${name}"]`)!;
  const value = (name: string) => query<HTMLElement>(`[data-value="${name}"]`)!;
  const viewport = root.querySelector<HTMLElement>(".scene-viewport")!;
  const renderer = new THREE.WebGLRenderer({ antialias: true });
  renderer.setPixelRatio(Math.min(devicePixelRatio, 2));
  renderer.setClearColor(0x17232d);
  viewport.prepend(renderer.domElement);
  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera(40, 1, 0.1, 100000);
  camera.up.set(0, 0, 1);
  const orbit = new OrbitControls(camera, renderer.domElement);
  orbit.enableDamping = true;
  scene.add(new THREE.HemisphereLight(0xe4f4ff, 0x3c444e, 2.7));
  const light = new THREE.DirectionalLight(0xffffff, 3.0);
  light.position.set(-300, -400, 900);
  scene.add(light);
  const material = new THREE.MeshStandardMaterial({
    color: 0x64b3ad,
    metalness: 0.18,
    roughness: 0.42,
  });
  let geometry = new THREE.BufferGeometry();
  let mesh: THREE.InstancedMesh | null = null;
  let paths: string[] = [],
    pathIndices = new Map<string, number>();
  let info: PacketInfo | null = null;
  let kernel: InstanceType<typeof wasm.SceneKernel>;
  let stage: Stage;
  let layers: Layer[] = [];
  let overrides: Layer;
  let inspectedLayer: Layer | null = null;
  let receipts: UndoReceipt[] = [];
  let disposed = false,
    active = true,
    frame = 0,
    busy = false,
    sourcePending = false,
    first = true;
  const released = new WeakSet<object>();
  async function releaseNative(
    native: InstanceType<typeof wasm.SceneKernel>,
    client?: Stage,
  ) {
    if (!native || released.has(native)) return;
    released.add(native);
    try {
      await client?.dispose();
    } finally {
      native.free();
    }
  }
  let lastStageCall = 0,
    selected = -1;
  const listeners: (() => void)[] = [];
  const listen = (
    element: EventTarget,
    event: string,
    callback: EventListener,
  ) => {
    element.addEventListener(event, callback);
    listeners.push(() => element.removeEventListener(event, callback));
  };
  function invalidate() {
    if (!disposed && active && !frame) frame = requestAnimationFrame(draw);
  }
  async function draw() {
    frame = 0;
    if (disposed || !active) return;
    if (sourcePending && !busy) await updateSource();
    if (disposed || !active) return;
    const changed = orbit.update();
    renderer.render(scene, camera);
    if (changed) invalidate();
  }
  orbit.addEventListener("change", invalidate);
  const resize = () => {
    const { width, height } = viewport.getBoundingClientRect();
    if (width <= 0 || height <= 0) return;
    renderer.setSize(width, height);
    camera.aspect = width / height;
    camera.updateProjectionMatrix();
    invalidate();
  };
  const observer = new ResizeObserver(resize);
  observer.observe(viewport);
  const rows = (target: string, values: [string, string | number][]) => {
    const dl = value(target);
    dl.replaceChildren();
    for (const [label, contents] of values) {
      const dt = document.createElement("dt"),
        dd = document.createElement("dd");
      dt.textContent = label;
      dd.textContent = String(contents);
      dl.append(dt, dd);
    }
  };
  function fit() {
    if (!mesh) return;
    mesh.computeBoundingBox();
    const bounds = mesh.boundingBox;
    if (!bounds || bounds.isEmpty()) return;
    const center = bounds.getCenter(new THREE.Vector3()),
      size = bounds.getSize(new THREE.Vector3());
    const radius = Math.max(size.length() / 2, 80);
    orbit.target.copy(center);
    camera.position
      .copy(center)
      .add(
        new THREE.Vector3(0.65, -0.9, 1.05)
          .normalize()
          .multiplyScalar(radius * 3.1),
      );
    camera.far = Math.max(10000, radius * 20);
    camera.updateProjectionMatrix();
    orbit.update();
    invalidate();
  }
  function extension<T>(op: Record<string, unknown>): T {
    const request = {
      protocol: "exedra.scene",
      version: 1,
      stage: stage.stageId,
      store: stage.storeId,
      op,
    };
    const response = JSON.parse(kernel.request(JSON.stringify(request)));
    if (
      response.protocol !== "exedra.scene" ||
      response.stage !== stage.stageId ||
      response.store !== stage.storeId
    )
      throw new Error("Foreign scene extension response");
    if (!response.ok)
      throw new Error(response.error?.message ?? "Scene operation refused");
    return response.result as T;
  }
  async function synchronize(label: string, editElapsed = 0) {
    const start = performance.now();
    const work = extension<Work>({ type: "sync" });
    const syncElapsed = performance.now() - start;
    const packStart = performance.now(),
      packet = kernel.pack();
    const packElapsed = performance.now() - packStart;
    try {
      const previous = info;
      info = JSON.parse(packet.metadata()) as PacketInfo;
      if (previous && previous.sourceScope !== info.sourceScope)
        value("selection").textContent =
          "Source changed; pick a current instance surface.";
      if (!info.nativePicking)
        value("selection").textContent =
          "Composed geometry has no verified native correspondence.";
      else if (previous && !previous.nativePicking)
        value("selection").textContent = "Pick an instance surface.";
      const transferStart = performance.now();
      const positions = packet.take_positions(),
        normals = packet.take_normals(),
        matrices = packet.take_matrices();
      const transferElapsed = performance.now() - transferStart;
      if (
        positions.byteLength + normals.byteLength !== info.geometryBytes ||
        matrices.byteLength !== info.transformBytes
      )
        throw new Error("Scene transfer byte mismatch");
      if (info.geometryChanged) {
        const previous = geometry;
        geometry = new THREE.BufferGeometry();
        geometry.setAttribute(
          "position",
          new THREE.BufferAttribute(positions, 3),
        );
        geometry.setAttribute("normal", new THREE.BufferAttribute(normals, 3));
        geometry.computeBoundingSphere();
        if (mesh) mesh.geometry = geometry;
        previous.dispose();
      }
      if (info.resetInstances) {
        if (mesh) {
          scene.remove(mesh);
          mesh.dispose();
        }
        paths = [...info.instances];
        pathIndices = new Map(paths.map((path, index) => [path, index]));
        mesh = new THREE.InstancedMesh(geometry, material, paths.length);
        mesh.instanceMatrix.setUsage(THREE.DynamicDrawUsage);
        scene.add(mesh);
        selected = -1;
        field<HTMLInputElement>("instance").max = String(
          Math.max(0, paths.length - 1),
        );
      }
      const matrix = new THREE.Matrix4();
      if (mesh) {
        for (let index = 0; index < info.instances.length; index++) {
          const slot = pathIndices.get(info.instances[index]);
          if (slot === undefined)
            throw new Error("Unknown native instance patch");
          // USD row-vector rows represent the transpose of Three's column matrix;
          // their flat order is the column-major array Three consumes.
          matrix.fromArray(matrices, index * 16);
          mesh.setMatrixAt(slot, matrix);
          mesh.instanceMatrix.addUpdateRange(slot * 16, 16);
        }
        if (matrices.length) {
          mesh.instanceMatrix.needsUpdate = true;
          mesh.computeBoundingSphere();
        }
      }
      rows("work", [
        ["Discovered prims", work.stats.discoveredPrims],
        ["Source lowerings", work.stats.sourceEvaluations],
        ["Geometry builds", work.stats.geometryEvaluations],
        ["Polygon encodings", work.stats.polygonEncodings],
        ["Polygon validations", work.stats.polygonValidations],
        ["Mesh writes", work.stats.meshesWritten],
        [
          "Input reads / reused",
          `${work.stats.inputResolutions} / ${work.stats.inputReused}`,
        ],
      ]);
      rows("timings", [
        ["Stage calls · Wasm", `${number(lastStageCall, 3)} ms`],
        ["Edit · client elapsed", `${number(editElapsed, 3)} ms`],
        ["Sync · Wasm call", `${number(syncElapsed, 3)} ms`],
        ["Pack · Wasm call", `${number(packElapsed, 3)} ms`],
        ["Buffer transfer", `${number(transferElapsed, 3)} ms`],
      ]);
      rows("transfer", [
        ["Geometry numeric data", `${number(info.geometryBytes)} B`],
        ["Matrix numeric data", `${number(info.transformBytes)} B`],
        ["Transform queries", info.transformQueries],
        ["Instance patches", info.instances.length],
        ["Triangles / prototype", info.triangles],
      ]);
      rows("source", [
        ["Volume", `${number(work.source.volume)} mm³`],
        ["Area", `${number(work.source.surfaceArea)} mm²`],
        [
          "Source-local deviation",
          `${number(work.accuracy.maxDeviation, 6)} mm`,
        ],
        ["Tolerance met", String(work.accuracy.withinTolerance)],
      ]);
      value("operation").textContent = label;
      value("scene-label").textContent =
        `${number(info.instanceCount)} native instances`;
      value("message").textContent = info.nativePicking
        ? ""
        : "Composed geometry overrides the producer output; retained source accuracy and native picking do not describe it.";
      root.dataset.geometryBytes = String(info.geometryBytes);
      root.dataset.transformBytes = String(info.transformBytes);
      root.dataset.geometryEvaluations = String(work.stats.geometryEvaluations);
      root.dataset.sourceScope = work.source.scope;
      if (first) {
        first = false;
        fit();
      }
      invalidate();
    } finally {
      packet.free();
    }
  }
  async function initialize() {
    const start = performance.now();
    const created = new wasm.SceneKernel(
      Number(field<HTMLSelectElement>("count").value),
      crypto.randomUUID(),
    );
    kernel = created;
    lastStageCall = performance.now() - start;
    const connected = await Stage.connect({
      request(raw) {
        const start = performance.now();
        const response = created.request(raw);
        lastStageCall += performance.now() - start;
        return response;
      },
    });
    stage = connected;
    inspectedLayer = null;
    value("layer-text").textContent = "";
    value("layer-text").hidden = true;
    if (disposed) {
      await releaseNative(created, connected);
      throw new Error("Scene view was disposed while loading");
    }
    layers = (await stage.inspect()).layers;
    overrides = layers.find((layer) => layer.name === "overrides.usda")!;
    field<HTMLSelectElement>("layer").replaceChildren(
      ...layers.map((layer, index) => new Option(layer.name, String(index))),
    );
    field<HTMLInputElement>("overrides").checked = true;
    receipts = [];
    info = null;
    first = true;
    field<HTMLSelectElement>("quality").value = "0.2";
    await synchronize("Initial composition");
    await refreshInputs();
    root.dataset.ready = "true";
  }
  async function retain(receipt: UndoReceipt) {
    receipts.push(receipt);
    if (receipts.length > 16) await stage.releaseReceipt(receipts.shift()!);
  }
  async function refreshInputs() {
    const source = await stage.query("/World/PanelAsset", [
      "primvars:example:width",
      "primvars:example:depth",
    ]);
    for (const [fieldName, attribute] of [
      ["width", "primvars:example:width"],
      ["depth", "primvars:example:depth"],
    ]) {
      const raw = source.attributes[attribute]?.value;
      if (raw?.type === "double") {
        field<HTMLInputElement>(fieldName).value = String(raw.value);
        value(fieldName).textContent = String(raw.value);
      }
    }
    await queryPlacement();
  }
  async function queryPlacement() {
    const path = paths[Number(field<HTMLInputElement>("instance").value)];
    if (!path) return;
    const part = await stage.query(path, ["xformOp:translate"]);
    const raw = part.attributes["xformOp:translate"]?.value;
    if (raw?.type === "double3")
      ["x", "y", "z"].forEach((axis, index) => {
        field<HTMLInputElement>(axis).value = String(raw.value[index]);
      });
  }
  async function action(label: string, edit?: () => Promise<void>) {
    if (disposed || busy) return;
    busy = true;
    lastStageCall = 0;
    try {
      const start = performance.now();
      await edit?.();
      const elapsed = performance.now() - start;
      if (!disposed) await synchronize(label, edit ? elapsed : 0);
    } catch (error) {
      if (!disposed)
        value("message").textContent =
          `${error instanceof Error ? error.message : String(error)}. Last rendered scene remains visible.`;
    } finally {
      busy = false;
      if (sourcePending && !disposed) invalidate();
    }
  }
  const attribute = (
    path: string,
    name: string,
    raw:
      | { type: "double"; value: number }
      | { type: "double3"; value: [number, number, number] },
  ): Edit => ({ type: "setAttribute", path, name, value: raw });
  async function updateSource() {
    if (busy || disposed) return;
    sourcePending = false;
    await action("Source parameters", async () => {
      const edit = await stage.transaction(overrides, [
        attribute("/World/PanelAsset", "primvars:example:width", {
          type: "double",
          value: Number(field<HTMLInputElement>("width").value),
        }),
        attribute("/World/PanelAsset", "primvars:example:depth", {
          type: "double",
          value: Number(field<HTMLInputElement>("depth").value),
        }),
      ]);
      await retain(edit.receipt);
    });
  }
  for (const name of ["width", "depth"])
    listen(field(name), "input", () => {
      value(name).textContent = field(name).value;
      sourcePending = true;
      invalidate();
    });
  listen(
    field("quality"),
    "change",
    () =>
      void action("Display quality", async () => {
        extension({
          type: "displayPolicy",
          tolerance: Number(field("quality").value),
        });
      }),
  );
  listen(
    field<HTMLInputElement>("overrides"),
    "change",
    () =>
      void action("Layer mute", async () => {
        await stage.mute(
          overrides,
          !field<HTMLInputElement>("overrides").checked,
        );
        await refreshInputs();
      }),
  );
  listen(field("instance"), "change", () => {
    if (!busy) void queryPlacement();
  });
  listen(field<HTMLInputElement>("wireframe"), "change", () => {
    material.wireframe = field<HTMLInputElement>("wireframe").checked;
    invalidate();
  });
  listen(field("count"), "change", () => {
    if (busy || disposed) return;
    busy = true;
    root.dataset.ready = "false";
    void releaseNative(kernel, stage)
      .then(async () => {
        if (!disposed) await initialize();
      })
      .catch((error) => {
        if (!disposed) value("message").textContent = String(error);
      })
      .finally(() => {
        busy = false;
      });
  });
  const buttons = [
    ...root.querySelectorAll<HTMLButtonElement>("[data-action]"),
    ...(controls?.querySelectorAll<HTMLButtonElement>("[data-action]") ?? []),
  ];
  for (const button of buttons)
    listen(button, "click", () => {
      switch (button.dataset.action) {
        case "fit":
          fit();
          break;
        case "noop":
          void action("No-op sync");
          break;
        case "undo":
          void action("Undo", async () => {
            const receipt = receipts.at(-1);
            if (!receipt) return;
            const inverse = await stage.undo(receipt);
            receipts.pop();
            await stage.releaseReceipt(inverse.receipt);
            await refreshInputs();
          });
          break;
        case "move":
          void action("Instance translation", async () => {
            const path = paths[Number(field("instance").value)];
            if (!path) throw new Error("Select a current native instance");
            const raw = ["x", "y", "z"].map((axis) =>
              Number(field(axis).value),
            ) as [number, number, number];
            const edit = await stage.transaction(overrides, [
              attribute(path, "xformOp:translate", {
                type: "double3",
                value: raw,
              }),
            ]);
            await retain(edit.receipt);
          });
          break;
        case "inspect":
        case "download":
          void action("Layer inspection", async () => {
            const layer = layers[Number(field("layer").value)];
            const saved = await stage.layerText(layer);
            if (button.dataset.action === "inspect") {
              const panel = value("layer-text"),
                close = inspectedLayer === layer && !panel.hidden;
              panel.textContent = saved.text;
              panel.hidden = close;
              inspectedLayer = layer;
            } else {
              const url = URL.createObjectURL(
                new Blob([saved.text], { type: "text/plain" }),
              );
              const link = document.createElement("a");
              link.href = url;
              link.download = saved.name;
              link.click();
              setTimeout(() => URL.revokeObjectURL(url), 1000);
            }
          });
          break;
      }
    });
  const raycaster = new THREE.Raycaster(),
    pointer = new THREE.Vector2();
  listen(renderer.domElement, "pointerup", (event) => {
    if (!mesh || !info || busy) return;
    const mouse = event as PointerEvent,
      bounds = renderer.domElement.getBoundingClientRect();
    pointer.set(
      ((mouse.clientX - bounds.left) / bounds.width) * 2 - 1,
      (-(mouse.clientY - bounds.top) / bounds.height) * 2 + 1,
    );
    raycaster.setFromCamera(pointer, camera);
    const hit = raycaster.intersectObject(mesh)[0];
    if (
      hit?.instanceId === undefined ||
      hit.faceIndex === undefined ||
      hit.faceIndex === null
    )
      return;
    selected = hit.instanceId;
    field<HTMLInputElement>("instance").value = String(selected);
    try {
      const picked = extension({
        type: "pick",
        instance: paths[selected],
        triangle: hit.faceIndex,
        sourceScope: info.sourceScope,
      });
      value("selection").textContent = JSON.stringify(picked, null, 2);
      void queryPlacement();
    } catch (error) {
      value("selection").textContent = String(error);
    }
  });
  const dispose = () => {
    if (disposed) return;
    disposed = true;
    if (frame) cancelAnimationFrame(frame);
    observer.disconnect();
    orbit.dispose();
    listeners.forEach((remove) => remove());
    mesh?.dispose();
    geometry.dispose();
    material.dispose();
    renderer.dispose();
    if (kernel) void releaseNative(kernel, stage).catch(() => {});
  };
  const setActive = (enabled: boolean) => {
    active = enabled;
    orbit.enabled = enabled;
    if (!enabled && frame) {
      cancelAnimationFrame(frame);
      frame = 0;
    }
    if (enabled) resize();
  };
  try {
    resize();
    await initialize();
    return { setActive, dispose };
  } catch (error) {
    dispose();
    throw error;
  }
}
