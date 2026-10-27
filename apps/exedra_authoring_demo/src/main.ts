// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

import "./style.css";
import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import { createSession, ExedraError, type AuthoringSession, type NativeDocument, type NativePart, type Realization, type FaceSelection, type NativeMeasurement } from "@exedra/authoring";
import { authorPanel, authorCollar, authorRoundedTurn, authorFlangedSleeve, authorTurnedFitting, type PanelParameters, type CollarParameters } from "./sample.js";
import sampleSource from "./sample.ts?raw";

const app = document.querySelector<HTMLDivElement>("#app")!;
app.innerHTML = `
<header><div class="brand">exedra<span>geometry gallery</span></div><div class="pill">Units: mm</div></header>
<main class="layout">
 <aside><label class="label" for="sample">Sample</label><select id="sample" class="quality"><option value="panel">Pocketed plate</option><option value="collar">Collar</option><option value="roundedRing">Rounded ring</option><option value="torus">Solid seal</option><option value="hollowTorus">Hollow seal / cutaway</option><option value="sleeve">Flanged sleeve</option><option value="fitting">Turned fitting</option></select><h1 id="study-title">Inputs</h1><p class="intro" id="study-intro"></p>
  <div id="panel-controls"><div class="control"><label class="label" for="width">Panel width <span class="value" id="width-value"></span></label><input id="width" type="range" min="100" max="160" step="1" value="120"></div>
  <div class="control"><label class="label" for="height">Panel height <span class="value" id="height-value"></span></label><input id="height" type="range" min="70" max="110" step="1" value="80"></div>
  <div class="control"><label class="label" for="thickness">Panel thickness <span class="value" id="thickness-value"></span></label><input id="thickness" type="range" min="8" max="20" step="0.5" value="12"></div>
  <div class="control"><label class="label" for="pocketDepth">Recess depth <span class="value" id="pocketDepth-value"></span></label><input id="pocketDepth" type="range" min="1" max="22" step="0.5" value="5"><div class="hint">Try a depth beyond the panel thickness. The last valid model stays visible.</div></div>
  <div class="control"><label class="label" for="cornerRadius">Corner radius <span class="value" id="cornerRadius-value"></span></label><input id="cornerRadius" type="range" min="2" max="15" step="0.5" value="6"></div>
  <div class="control"><label class="label" for="ventRadius">Vent radius <span class="value" id="ventRadius-value"></span></label><input id="ventRadius" type="range" min="1" max="5" step="0.25" value="3"></div>
  </div><div id="collar-controls" hidden>
   <div class="control"><label class="label" for="radius"><span id="radius-label">Bore radius</span> <span class="value" id="radius-value"></span></label><input id="radius" type="range" min="1" max="80" step="0.5" value="12"></div>
   <div class="control"><label class="label" for="wall"><span id="wall-label">Wall thickness</span> <span class="value" id="wall-value"></span></label><input id="wall" type="range" min="1" max="30" step="0.5" value="7"></div>
   <div class="control"><label class="label" for="collarHeight">Collar height <span class="value" id="collarHeight-value"></span></label><input id="collarHeight" type="range" min="8" max="60" step="1" value="30"></div>
   <div class="control"><label class="label" for="angle">Turn angle <span class="value" id="angle-value"></span></label><input id="angle" type="range" min="30" max="360" step="5" value="270"></div>
   <div class="control" id="rounding-control" hidden><label class="label" for="rounding">Corner radius <span class="value" id="rounding-value"></span></label><input id="rounding" type="range" min="0.5" max="12" step="0.5" value="2"></div>
   <div class="control" id="innerRadius-control" hidden><label class="label" for="innerRadius">Tube bore radius <span class="value" id="innerRadius-value"></span></label><input id="innerRadius" type="range" min="0.5" max="20" step="0.5" value="4"></div>
  </div>
  <h2>Display</h2><label class="label" for="quality">Quality <select id="quality" class="quality"><option value="0.45">Coarse · 0.45 mm</option><option value="0.12" selected>Medium · 0.12 mm</option><option value="0.035">Fine · 0.035 mm</option></select></label>
  <label class="checkline"><input id="wireframe" type="checkbox"> Show mesh triangles</label>

  <div class="divider"></div><button id="lifecycle" disabled>Check rebuild / release</button><p id="lifecycle-result" class="hint">12 rebuilds; live owner counts.</p>
 </aside>
 <section id="viewport" class="viewport" aria-label="Interactive native model">
  <div class="toolbar"><div><button id="fit">Fit model</button><button id="code-toggle" aria-expanded="false">View TypeScript</button></div><span class="pill" id="revision">Loading kernel</span></div>
  <div class="code" id="code" hidden><pre></pre></div>
  <div class="status" id="status" role="status"><strong>Loading the native kernel…</strong><div class="detail">Authoring and geometric validation run in Rust/Wasm.</div></div>
 </section>
 <aside><h2 style="margin-top:0">Measurements · <span id="source-name">panel</span></h2><div class="stats">
  <div class="stat wide"><small>Volume · mm³</small><b id="volume">—</b></div><div class="stat wide"><small>Surface area · mm²</small><b id="area">—</b></div>
  <div class="stat"><small>Native faces</small><b id="native-faces">—</b></div><div class="stat"><small>Shared edges</small><b id="native-edges">—</b></div></div>
  <h2>Display data</h2><div class="metrics-line"><span>Triangles</span><code id="triangles">—</code></div><div class="metrics-line"><span>Render vertices</span><code id="vertices">—</code></div><div class="metrics-line"><span>Reported error</span><code id="deviation">—</code></div><div class="metrics-line"><span>CPU display buffers</span><code id="buffers">—</code></div><div class="metrics-line"><span>Compile / cache hits</span><code id="cache">—</code></div>
  <div class="metrics-line"><span>Build / realize APIs</span><code id="timing">—</code></div><h2>Features</h2><div id="features"></div><p class="hint">Pick a face or feature.</p>
  <h2>Selection</h2><div id="selection" class="selection">No selection.</div><p id="selection-notice" class="notice"></p>
  <h2>Construction</h2><div id="attachment" class="hint">The gold pin is placed from the tagged recess floor.</div>
  <h2>Export</h2><div class="downloads"></div><div id="step-result" class="step-result">STEP: one part per file.</div>
  <h2>Memory / ownership</h2><div id="ownership" class="hint">—</div>
 </aside>
</main>`;
const element = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const viewport = element<HTMLElement>("viewport");
element("code").querySelector("pre")!.textContent = sampleSource;
const controlsNames = ["width", "height", "thickness", "pocketDepth", "cornerRadius", "ventRadius"] as const;
const parameters = (): PanelParameters => Object.fromEntries(controlsNames.map(key => [key, Number(element<HTMLInputElement>(key).value)])) as unknown as PanelParameters;
const collarNames = ["radius","wall","collarHeight","angle","rounding","innerRadius"] as const;
const collarParameters = (): CollarParameters => Object.fromEntries(collarNames.map(key=>[key,Number(element<HTMLInputElement>(key).value)])) as unknown as CollarParameters;
const authorCurrent = (delta=0) => {
  const sample=element<HTMLSelectElement>("sample").value;
  const p={...collarParameters(),radius:collarParameters().radius+delta};
  switch(sample) {
    case "collar": return authorCollar(p);
    case "roundedRing": case "torus": case "hollowTorus": return authorRoundedTurn(p,sample);
    case "sleeve": return authorFlangedSleeve(p);
    case "fitting": return authorTurnedFitting(p);
    default:return authorPanel({...parameters(),width:parameters().width+delta});
  }
};
const tolerance = () => Number(element<HTMLSelectElement>("quality").value);
const format = (n: number, digits = 2) => n.toLocaleString(undefined, { maximumFractionDigits: digits });
const formatMeasure = (value: NativeMeasurement) => value.status === "available" ? format(value.value,3) : value.status === "unavailable" ? "Not available" : "Numeric limit";
let lastBuildMs=0, lastRealizeMs=0;
const featureColors: Record<string, string> = { panel: "#4baca7", vents: "#6f9fdb", recess: "#d2a36d", pin: "#f1c77f", collar:"#8bbbc4" };

const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
renderer.setPixelRatio(Math.min(devicePixelRatio, 2));
renderer.shadowMap.enabled = true;
renderer.shadowMap.type = THREE.PCFSoftShadowMap;
renderer.outputColorSpace = THREE.SRGBColorSpace;
renderer.setClearColor(0x000000, 0);
viewport.prepend(renderer.domElement);
const scene = new THREE.Scene();
const camera = new THREE.PerspectiveCamera(38, 1, 0.1, 3000);
camera.up.set(0, 0, 1);
const orbit = new OrbitControls(camera, renderer.domElement);
orbit.enableDamping = true; orbit.dampingFactor = 0.08;
scene.add(new THREE.HemisphereLight(0xd4e6f5, 0x364044, 2.4));
const key = new THREE.DirectionalLight(0xfff0d6, 3.6); key.position.set(-80, -130, 200); key.castShadow = true;
key.shadow.mapSize.set(2048, 2048); key.shadow.camera.left = -150; key.shadow.camera.right = 150; key.shadow.camera.top = 150; key.shadow.camera.bottom = -150; key.shadow.normalBias = 0.08;
scene.add(key);
const fill = new THREE.DirectionalLight(0xb3d4fa, 1.5); fill.position.set(140, 70, 120); scene.add(fill);
const ground = new THREE.Mesh(new THREE.PlaneGeometry(500, 400), new THREE.ShadowMaterial({ color: 0x080e15, opacity: 0.5 })); ground.position.z = -0.3; ground.receiveShadow = true; scene.add(ground);
const grid = new THREE.GridHelper(400, 40, 0x60717d, 0x3c4c59); grid.rotation.x = Math.PI / 2; grid.position.z = -0.25; (grid.material as THREE.Material).transparent = true; (grid.material as THREE.Material).opacity = 0.2; scene.add(grid);
const resize = () => { const { width, height } = viewport.getBoundingClientRect(); renderer.setSize(width, height); camera.aspect = width / height; camera.updateProjectionMatrix(); };
new ResizeObserver(resize).observe(viewport); resize();
function frame() {
  // Consume the latest edit before drawing, so the new model is submitted in this frame.
  if (pendingBuild) { pendingBuild = false; rebuild(); }
  orbit.update(); renderer.render(scene, camera); requestAnimationFrame(frame);
}
requestAnimationFrame(frame);

interface Entry { part: NativePart; display: Realization; mesh: THREE.Mesh<THREE.BufferGeometry, THREE.MeshStandardMaterial[]> }
interface Drawn { group: THREE.Group; entries: Entry[] }
let session: AuthoringSession;
let current: NativeDocument | null = null;
let drawn: Drawn | null = null;
let selected: FaceSelection | null = null;
let highlight: THREE.Mesh | null = null;
let draftError: unknown = null;
let displayError: unknown = null;
let hasCamera = false;

function materials(part: NativePart) {
  const names = [...new Set([part.name, ...part.faces().map(f => f.feature ?? part.name)])];
  const values = names.map(name => new THREE.MeshStandardMaterial({ color: featureColors[name] ?? "#87abb7", metalness: 0.25, roughness: 0.35, wireframe: element<HTMLInputElement>("wireframe").checked }));
  return { names, values };
}
function draw(document: NativeDocument): Drawn {
  const group = new THREE.Group(); const entries: Entry[] = []; lastRealizeMs=0;
  try {
    for (const part of Object.values(document.parts)) {
      const started = performance.now();
      const display = part.realize({ tolerance: tolerance() });
      lastRealizeMs += performance.now()-started;
      const geometry = new THREE.BufferGeometry();
      geometry.setAttribute("position", new THREE.BufferAttribute(display.positions, 3));
      geometry.setAttribute("normal", new THREE.BufferAttribute(display.normals, 3));
      geometry.setIndex(new THREE.BufferAttribute(display.indices, 1));
      const style = materials(part);
      const faces = new Map(part.faces().map(face => [face.nativeId, face]));
      let start = 0, previous = -1;
      for (let triangle = 0; triangle <= display.nativeFaces.length; triangle++) {
        const feature = triangle === display.nativeFaces.length ? null : (faces.get(display.nativeFaces[triangle])?.feature ?? part.name);
        const material = feature === null ? -1 : style.names.indexOf(feature);
        if (material !== previous) {
          if (triangle > 0) geometry.addGroup(start * 3, (triangle - start) * 3, previous);
          start = triangle; previous = material;
        }
      }
      geometry.computeBoundingBox(); geometry.computeBoundingSphere();
      const mesh = new THREE.Mesh(geometry, style.values); mesh.castShadow = true; mesh.receiveShadow = true;
      group.add(mesh); entries.push({ part, display, mesh });
    }
    return { group, entries };
  } catch (error) { dispose({ group, entries }); throw error; }
}
function dispose(view: Drawn) { for (const entry of view.entries) { entry.mesh.geometry.dispose(); for (const material of entry.mesh.material) material.dispose(); } }
function clearHighlight() {
  if (!highlight) return;
  scene.remove(highlight); highlight.geometry.dispose();
  (highlight.material as THREE.Material).dispose(); highlight = null;
}
function showSelection(selection: FaceSelection | null) {
  clearHighlight(); selected = selection;
  if (!selection || !drawn) { element("selection").textContent = "No selection."; return; }
  const entry = drawn.entries.find(e => e.part.id === selection.partId);
  if (!entry) return;
  entry.part.requireSelection(selection);
  const family = selection.feature ? entry.part.faces(selection.feature) : [selection];
  const ids = new Set(family.map(f => f.nativeId));
  const indices: number[] = [];
  entry.display.nativeFaces.forEach((face, triangle) => { if (ids.has(face)) indices.push(...entry.display.indices.subarray(triangle * 3, triangle * 3 + 3)); });
  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute("position", new THREE.BufferAttribute(entry.display.positions, 3));
  geometry.setAttribute("normal", new THREE.BufferAttribute(entry.display.normals, 3));
  geometry.setIndex(indices);
  highlight = new THREE.Mesh(geometry, new THREE.MeshStandardMaterial({ color: 0xfce4a2, emissive: 0xa2650c, emissiveIntensity: 0.6, metalness: 0.3, roughness: 0.3, transparent: true, opacity: 0.78, polygonOffset: true, polygonOffsetFactor: -1, polygonOffsetUnits: -1 }));
  scene.add(highlight);
  const box = element("selection"); box.replaceChildren();
  const strong = document.createElement("strong"); strong.textContent = selection.feature ?? "Unlabelled cap"; box.append(strong, document.createElement("br"));
  box.append(`${entry.part.name} · ${selection.role} · ${selection.surface}`, document.createElement("br"));
  const detail = document.createElement("code"); detail.textContent = `native face ${selection.nativeId} · ${family.length} feature faces`; box.append(detail);
}
function fit() {
  if (!drawn) return;
  const bounds = new THREE.Box3().setFromObject(drawn.group), center = bounds.getCenter(new THREE.Vector3());
  const size = bounds.getSize(new THREE.Vector3());
  const scale = Math.max(size.x, size.y * camera.aspect, size.z * 1.2, 20);
  camera.position.copy(center).add(new THREE.Vector3(scale * 0.94, -scale * 1.32, scale * 1.24));
  orbit.target.copy(center); orbit.update(); hasCamera = true;
}
function ownership() {
  const stats = session.stats();
  element("ownership").textContent = `${stats.documents} document · ${stats.nativeParts} parts · ${stats.cacheEntries}/8 cached realizations.`;
}
function evidence() {
  if (!current || !drawn) return;
  const panel = Object.values(current.parts)[0];
  element("source-name").textContent = `${panel.name} · ${panel.family}`;
  element("volume").textContent = formatMeasure(panel.measurements.volume);
  element("area").textContent = formatMeasure(panel.measurements.surfaceArea);
  for (const [id,value] of [["volume",panel.measurements.volume],["area",panel.measurements.surfaceArea]] as const)
    element(id).title=value.status === "available" ? "Native source integral" : value.diagnostic.message;
  element("native-faces").textContent = String(panel.measurements.faces);
  element("native-edges").textContent = String(panel.measurements.edges);
  const report = drawn.entries.map(e => e.display.report);
  element("triangles").textContent = format(report.reduce((n, r) => n + r.triangles, 0), 0);
  element("vertices").textContent = format(report.reduce((n, r) => n + r.vertices, 0), 0);
  element("deviation").textContent = `${format(Math.max(...report.map(r => r.maxDeviation)), 5)} mm`;
  const bytes = drawn.entries.reduce((n, e) => n + e.display.positions.byteLength + e.display.normals.byteLength + e.display.indices.byteLength + e.display.nativeFaces.byteLength, 0);
  element("buffers").textContent = `${format(bytes / 1024, 1)} KiB`;
  element("cache").textContent = `${report.reduce((n, r) => n + r.nativeRealizations, 0)} / ${report.reduce((n, r) => n + r.cacheHits, 0)}`;
  element("revision").textContent = current.id.split("/").at(-1)!;
  element("timing").textContent = `${format(lastBuildMs,2)} / ${format(lastRealizeMs,2)} ms`;
  if (current.parts.pin && panel.family === "extrusion") {
    const plane = panel.floor("recess").workplane();
    element("attachment").textContent = `Native recess floor z = ${format(plane.origin[2])} mm. The pin begins at z = ${format(current.parts.pin.measurements.bounds[0][2])} mm, offset 0.5 mm along its normal.`;
  } else {
    element("attachment").textContent = `${panel.family}. Bounds enclose native supporting patches and may include space outside trims.`;
  }
  const features = element("features"); features.replaceChildren();
  for (const [partName, part] of Object.entries(current.parts)) {
    for (const feature of [...new Set(part.faces().flatMap(f => f.feature ? [f.feature] : []))]) {
      const row = document.createElement("button"); row.className = "feature-row"; row.style.width = "100%"; row.style.background = "none"; row.style.borderRadius = "0"; row.style.borderWidth = "0 0 1px";
      row.setAttribute("aria-label", `Select ${feature} feature`);
      const swatch = document.createElement("i"); swatch.className = "swatch"; swatch.style.background = featureColors[feature] ?? "#87abb7";
      const count = document.createElement("span"); count.textContent = `${part.faces(feature).length} faces`;
      row.append(swatch, `${partName} / ${feature}`, count); row.onclick = () => showSelection(part.faces(feature)[0]); features.append(row);
    }
  }
  renderDownloads();
  ownership();
}
function status(title: string, detail: string, error = false) {
  const box = element("status"); box.classList.toggle("invalid", error);
  box.querySelector("strong")!.textContent = title; box.querySelector(".detail")!.textContent = detail;
}
function reportError(error: unknown) {
  const diagnostic = error instanceof ExedraError ? error.diagnostic : { code: "unexpected", operation: "demo", message: String(error) };
  status(current ? "Showing the last valid model" : "Model was refused", `${diagnostic.code} · ${diagnostic.operation}: ${diagnostic.message}`, true);
  setExportDisabled(true);
}
function accept(document: NativeDocument, view: Drawn, edited: boolean) {
  const old = current, prior = drawn;
  clearHighlight();
  if (prior) { scene.remove(prior.group); dispose(prior); }
  current = document; drawn = view; scene.add(view.group);
  if (edited && selected) { selected = null; element("selection-notice").textContent = "Selection cleared: authored revision changed."; }
  if (old && old !== document) old.dispose();
  evidence(); if (!hasCamera) fit();
  showSelection(selected);
  if (edited) draftError = null;
  displayError = null;
  if (draftError) { reportError(draftError); return; }
  const within = view.entries.every(e => e.display.report.withinTolerance);
  status(within ? "Ready" : "Display tolerance unmet", within ? "Display tolerance met." : "See the reported error and requested tolerance.", !within);
  setExportDisabled(false);
}
function rebuild() {
  if (!session) return;
  let candidate: NativeDocument | undefined;
  try { const start=performance.now(); candidate = session.build(authorCurrent()); lastBuildMs=performance.now()-start; const view = draw(candidate); accept(candidate, view, true); }
  catch (error) { candidate?.dispose(); draftError = error; reportError(error); }
}
// One pending frame reads the latest controls; dragging never queues obsolete builds.
let pendingBuild = false;
function scheduleBuild() { pendingBuild = true; }
for (const key of [...controlsNames,...collarNames]) {
  const input = element<HTMLInputElement>(key);
  const label = () => { element(`${key}-value`).textContent = `${input.value} ${key === "angle" ? "°" : "mm"}`; };
  label(); input.addEventListener("input", () => { label(); scheduleBuild(); });
}
element<HTMLSelectElement>("sample").onchange = () => {
  const sample=element<HTMLSelectElement>("sample").value, panel=sample === "panel", torus=sample === "torus" || sample === "hollowTorus";
  element("panel-controls").hidden=!panel; element("collar-controls").hidden=panel;
  element("rounding-control").hidden=sample !== "roundedRing";element("innerRadius-control").hidden=sample !== "hollowTorus";
  element<HTMLInputElement>("collarHeight").closest<HTMLElement>(".control")!.hidden=torus;
  element("radius-label").textContent=torus ? "Major radius" : "Bore radius";
  element("wall-label").textContent=torus ? "Tube radius" : "Wall thickness";
  const defaults=sample === "roundedRing" ? [22,12,24,360] : torus ? [35,7,30,sample === "hollowTorus" ? 270 : 360] : sample === "sleeve" ? [12,10,35,270] : sample === "fitting" ? [7,15,47,270] : [12,7,30,270];
  for (const [i,key] of ["radius","wall","collarHeight","angle"].entries()) {
    element<HTMLInputElement>(key).value=String(defaults[i]); element(`${key}-value`).textContent=`${defaults[i]} ${key === "angle" ? "°" : "mm"}`;
  }
  for(const [key,value] of [["rounding",2],["innerRadius",4]] as const) {element<HTMLInputElement>(key).value=String(value);element(`${key}-value`).textContent=`${value} mm`;}
  hasCamera=false; scheduleBuild();
};
element<HTMLSelectElement>("quality").onchange = () => {
  if (!current) return;
  try { const view = draw(current); accept(current, view, false); } catch (error) { displayError = error; reportError(draftError ?? displayError); }
};
element<HTMLInputElement>("wireframe").onchange = () => { for (const e of drawn?.entries ?? []) for (const m of e.mesh.material) m.wireframe = element<HTMLInputElement>("wireframe").checked; };
element("fit").onclick = fit;
element("code-toggle").onclick = () => { const code = element("code"); code.hidden = !code.hidden; element("code-toggle").setAttribute("aria-expanded", String(!code.hidden)); };
const raycaster = new THREE.Raycaster(); let down: [number, number] | null = null;
renderer.domElement.onpointerdown = event => { if (event.button === 0) down = [event.clientX, event.clientY]; };
renderer.domElement.addEventListener("pointerup", event => {
  if (!down || !drawn || Math.hypot(event.clientX - down[0], event.clientY - down[1]) > 5) return;
  down = null;
  const box = renderer.domElement.getBoundingClientRect();
  raycaster.setFromCamera(new THREE.Vector2((event.clientX - box.left) / box.width * 2 - 1, -(event.clientY - box.top) / box.height * 2 + 1), camera);
  const hit = raycaster.intersectObjects(drawn.entries.map(e => e.mesh), false)[0];
  if (!hit || hit.faceIndex == null) { showSelection(null); return; }
  const entry = drawn.entries.find(e => e.mesh === hit.object)!;
  showSelection(entry.display.pickTriangle(hit.faceIndex)); element("selection-notice").textContent = "";
});
function setExportDisabled(disabled: boolean) { for (const button of document.querySelectorAll<HTMLButtonElement>(".downloads button")) button.disabled=disabled; }
function renderDownloads() {
  const box=document.querySelector<HTMLDivElement>(".downloads")!; box.replaceChildren();
  for (const part of Object.keys(current?.parts ?? {})) {
    const button=document.createElement("button"); button.id=`step-${part}`;button.textContent=`Download ${part} STEP`;button.onclick=()=>downloadStep(part);box.append(button);
  }
}
function downloadStep(part: string) {
  if (!current || draftError || displayError) return;
  try {
    const file = current.parts[part].exportStep({ name: part, timestamp: new Date().toISOString() });
    const url = URL.createObjectURL(new Blob([file.text], { type: "application/step" }));
    const link = document.createElement("a"); link.href = url; link.download = `exedra-${part}.step`; link.click(); setTimeout(() => URL.revokeObjectURL(url), 1000);
    element("step-result").textContent = `${part}: ${file.report.schema} · ${file.report.faces} native faces · ${file.report.entities} entities · ${file.report.units}. Exported directly from the native source.`;
  } catch (error) { element("step-result").textContent = error instanceof Error ? error.message : String(error); }
}
element<HTMLButtonElement>("lifecycle").onclick = () => {
  const baseline = session.stats();
  try {
    for (let i = 0; i < 12; i++) {
      const trial = session.build(authorCurrent((i+1)*0.01));
      try { Object.values(trial.parts)[0].realize({ tolerance: tolerance() }); } finally { trial.dispose(); }
    }
    const after = session.stats();
    if (after.documents !== baseline.documents || after.nativeParts !== baseline.nativeParts || after.cacheEntries > 8) throw new Error("Native ownership did not return to its expected bound");
    element("lifecycle-result").textContent = `12 revisions released. ${after.documents} document, ${after.nativeParts} named parts; cache bounded at ${after.cacheEntries}/8. Native ownership returned to baseline.`;
    ownership();
  } catch (error) { element("lifecycle-result").textContent = String(error); }
};
window.addEventListener("beforeunload", () => { pendingBuild = false; clearHighlight(); if (drawn) dispose(drawn); session?.dispose(); renderer.dispose(); orbit.dispose(); });
try {
  session = await createSession({ units: "mm", separation: 1e-6 });
  rebuild(); element<HTMLButtonElement>("lifecycle").disabled = false;
} catch (error) { reportError(error); }
