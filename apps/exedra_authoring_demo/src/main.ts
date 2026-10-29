// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

import "./style.css";
import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import { createSession, ExedraError, type AuthoringSession, type NativeDocument, type NativePart, type Realization, type FaceSelection, type NativeMeasurement } from "@exedra/authoring";
import { authorPanel, authorCollar, authorRoundedTurn, authorFlangedSleeve, authorSphere, authorDome, authorPointedCone, authorTurnedFitting, authorRail, authorVariableLoft, authorFinishedPlate, authorAngledBore, authorHousing, constructionChart, type PanelParameters, type CollarParameters, type FlangeParameters, type HousingParameters } from "./sample.js";
import sampleSource from "./sample.ts?raw";
import { mountViews } from "./views.js";
import { meshTriangles } from "./mesh-overlay.js";

const app = document.querySelector<HTMLDivElement>("#app")!;
app.innerHTML = `
<main class="layout">
 <aside><div class="brand">exedra</div><div class="view-control"><label class="label" for="view">View</label><select id="view" class="quality"><option value="geometry">Geometry</option><option value="scene">Scene</option></select></div><div id="geometry-controls"><label class="label" for="sample">Sample</label><select id="sample" class="quality"><option value="housing">Machined housing</option><option value="panel" selected>Pocketed plate</option><option value="collar">Collar</option><option value="roundedRing">Rounded ring</option><option value="torus">Solid seal</option><option value="hollowTorus">Hollow seal / cutaway</option><option value="sleeve">Flanged sleeve</option><option value="fitting">Turned fitting</option><option value="sphere">Sphere</option><option value="dome">Spherical cap</option><option value="cone">Pointed cone</option><option value="rail">Swept rail</option><option value="hollowRail">Hollow swept rail</option><option value="loft">Variable-section loft</option><option value="fillet">Outer axial fillets</option><option value="chamfer">Inner axial chamfers</option><option value="bore">Angled bore</option></select><h1 id="study-title">Inputs</h1><p class="intro" id="study-intro"></p>
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
  <div id="rail-controls" hidden>
    <div class="control"><label class="label" for="railRadius">Section radius <span class="value" id="railRadius-value"></span></label><input id="railRadius" type="range" min="2" max="12" step="0.5" value="6"></div>
    <div class="control"><label class="label" for="bendRadius">Bend radius <span class="value" id="bendRadius-value"></span></label><input id="bendRadius" type="range" min="20" max="70" step="1" value="40"></div>
    <div class="control"><label class="label" for="turnAngle">Bend angle <span class="value" id="turnAngle-value"></span></label><input id="turnAngle" type="range" min="30" max="200" step="5" value="135"></div>
  </div>
  <div id="loft-controls" hidden>
    <div class="control"><label class="label" for="loftHeight">Height <span class="value" id="loftHeight-value"></span></label><input id="loftHeight" type="range" min="60" max="180" step="1" value="120"></div>
    <div class="control"><label class="label" for="loftTwist">Section twist <span class="value" id="loftTwist-value"></span></label><input id="loftTwist" type="range" min="-25" max="25" step="1" value="14"></div>
    <div class="control"><label class="label" for="loftOffset">Section offset <span class="value" id="loftOffset-value"></span></label><input id="loftOffset" type="range" min="0" max="25" step="1" value="18"></div>
  </div>
  <div id="finish-controls" hidden>
    <div class="control"><label class="label" for="finishSize">Finish radius / distance <span class="value" id="finishSize-value"></span></label><input id="finishSize" type="range" min="0.5" max="12" step="0.5" value="5"></div>
    <div class="control"><label class="label" for="finishHeight">Height <span class="value" id="finishHeight-value"></span></label><input id="finishHeight" type="range" min="8" max="60" step="1" value="30"></div>
    <p class="hint">Selected profile axial edges. Rim and pocket edges are outside this operation.</p>
  </div>
  <div id="flange-controls" hidden>
    <div class="control"><label class="label" for="flangeBore">Bore radius <span class="value" id="flangeBore-value"></span></label><input id="flangeBore" type="range" min="1" max="40" step="0.5" value="12"></div>
    <div class="control"><label class="label" for="sleeveOuter">Sleeve outer radius <span class="value" id="sleeveOuter-value"></span></label><input id="sleeveOuter" type="range" min="2" max="70" step="0.5" value="18"></div>
    <div class="control"><label class="label" for="flangeOuter">Flange outer radius <span class="value" id="flangeOuter-value"></span></label><input id="flangeOuter" type="range" min="2" max="80" step="0.5" value="22"></div>
    <div class="control"><label class="label" for="shoulderHeight">Shoulder height <span class="value" id="shoulderHeight-value"></span></label><input id="shoulderHeight" type="range" min="0.5" max="30" step="0.25" value="8.75"></div>
    <label class="label" for="shoulderFinish">Shoulder finish</label><select id="shoulderFinish" class="quality"><option value="fillet">Fillet</option><option value="chamfer">Chamfer</option></select>
    <div class="control"><label class="label" for="shoulderSize"><span id="shoulderSize-label">Fillet radius</span> <span class="value" id="shoulderSize-value"></span></label><input id="shoulderSize" type="range" min="0" max="12" step="0.25" value="2"><p class="hint">Zero leaves a sharp shoulder. Larger treatments must leave room on both adjacent edges.</p></div>
    <div class="control"><label class="label" for="sleeveHeight">Total height <span class="value" id="sleeveHeight-value"></span></label><input id="sleeveHeight" type="range" min="1" max="80" step="0.5" value="35"></div>
    <div class="control"><label class="label" for="flangeAngle">Turn angle <span class="value" id="flangeAngle-value"></span></label><input id="flangeAngle" type="range" min="30" max="360" step="5" value="270"></div>
  </div>
  <div id="pole-controls" hidden>
   <div class="control"><label class="label" for="poleRadius"><span id="poleRadius-label">Sphere radius</span> <span class="value" id="poleRadius-value"></span></label><input id="poleRadius" type="range" min="2" max="60" step="0.5" value="25"></div>
   <div class="control" id="poleHeight-control"><label class="label" for="poleHeight">Height <span class="value" id="poleHeight-value"></span></label><input id="poleHeight" type="range" min="0.5" max="100" step="0.5" value="40"></div>
   <p class="hint" id="pole-hint">Full revolutions with native poles. Show triangles to inspect the mesh near the tip.</p>
  </div>
  <div id="bore-controls" hidden>
   <div class="control"><label class="label" for="boreWidth">Plate width <span class="value" id="boreWidth-value"></span></label><input id="boreWidth" type="range" min="60" max="160" step="1" value="100"></div>
   <div class="control"><label class="label" for="boreHeight">Plate height <span class="value" id="boreHeight-value"></span></label><input id="boreHeight" type="range" min="40" max="120" step="1" value="70"></div>
   <div class="control"><label class="label" for="boreThickness">Plate thickness <span class="value" id="boreThickness-value"></span></label><input id="boreThickness" type="range" min="5" max="35" step="0.5" value="20"></div>
   <div class="control"><label class="label" for="boreRadius">Bore radius <span class="value" id="boreRadius-value"></span></label><input id="boreRadius" type="range" min="2" max="25" step="0.5" value="8"></div>
   <div class="control"><label class="label" for="boreAngle">Axis tilt from plate normal <span class="value" id="boreAngle-value"></span></label><input id="boreAngle" type="range" min="0" max="88" step="1" value="60"></div>
   <div class="control"><label class="label" for="boreSeparation">Minimum edge clearance <span class="value" id="boreSeparation-value"></span></label><input id="boreSeparation" type="range" min="0.5" max="30" step="0.5" value="2"></div>
  </div>
  <div id="housing-controls" hidden>
  <p class="hint">A flanged polygon blank, two mount holes, an oblique main passage and a service port. Every dimension is independent. Invalid or intersecting cuts retain the last valid model.</p>
   <div class="control"><label class="label" for="housingWidth">Flange width <span class="value" id="housingWidth-value"></span></label><input id="housingWidth" type="range" min="80" max="160" step="1" value="104"></div>
   <div class="control"><label class="label" for="housingBodyWidth">Body width <span class="value" id="housingBodyWidth-value"></span></label><input id="housingBodyWidth" type="range" min="40" max="100" step="1" value="64"></div>
   <div class="control"><label class="label" for="housingHeight">Total height <span class="value" id="housingHeight-value"></span></label><input id="housingHeight" type="range" min="65" max="130" step="1" value="90"></div>
   <div class="control"><label class="label" for="housingFlangeDepth">Flange depth <span class="value" id="housingFlangeDepth-value"></span></label><input id="housingFlangeDepth" type="range" min="12" max="40" step="1" value="24"></div>
   <div class="control"><label class="label" for="housingThickness">Stock thickness <span class="value" id="housingThickness-value"></span></label><input id="housingThickness" type="range" min="5" max="40" step="0.5" value="18"></div>
   <div class="control"><label class="label" for="housingPassageRadius">Main passage radius <span class="value" id="housingPassageRadius-value"></span></label><input id="housingPassageRadius" type="range" min="2" max="26" step="0.5" value="12"></div>
   <div class="control"><label class="label" for="housingPassageY">Main passage center Y <span class="value" id="housingPassageY-value"></span></label><input id="housingPassageY" type="range" min="-10" max="30" step="1" value="15"></div>
   <div class="control"><label class="label" for="housingPassageAngle">Main passage tilt <span class="value" id="housingPassageAngle-value"></span></label><input id="housingPassageAngle" type="range" min="0" max="70" step="1" value="20"></div>
   <div class="control"><label class="label" for="housingMountSpacing">Mount center spacing <span class="value" id="housingMountSpacing-value"></span></label><input id="housingMountSpacing" type="range" min="50" max="110" step="1" value="76"></div>
   <div class="control"><label class="label" for="housingMountRadius">Mount radius <span class="value" id="housingMountRadius-value"></span></label><input id="housingMountRadius" type="range" min="2" max="14" step="0.5" value="5"></div>
   <div class="control"><label class="label" for="housingPortY">Service port center Y <span class="value" id="housingPortY-value"></span></label><input id="housingPortY" type="range" min="-30" max="20" step="1" value="-10"></div>
   <div class="control"><label class="label" for="housingPortRadius">Service port radius <span class="value" id="housingPortRadius-value"></span></label><input id="housingPortRadius" type="range" min="2" max="14" step="0.5" value="5"></div>
   <div class="control"><label class="label" for="housingClearance">Minimum material clearance <span class="value" id="housingClearance-value"></span></label><input id="housingClearance" type="range" min="0.25" max="8" step="0.25" value="1"></div>
  </div>
  <h2>Display</h2><label class="label" for="quality">Quality <select id="quality" class="quality"><option value="0.45">Coarse · 0.45 mm</option><option value="0.12" selected>Medium · 0.12 mm</option><option value="0.035">Fine · 0.035 mm</option></select></label>
  <label class="checkline"><input id="wireframe" type="checkbox"> Show mesh triangles</label>
  <label class="checkline"><input id="checker" type="checkbox"> Construction checker</label><p id="chart-support" class="hint"></p>
  <div id="chart-controls" hidden>
   <div class="control"><label class="label" for="uvRepeat">UV repeat · mm</label><input id="uvRepeat" type="number" min="1" step="1" value="20"></div>
   <div class="control" id="revolve-chart" hidden><label class="label" for="referenceRadius">Rest radius · mm</label><input id="referenceRadius" type="number" min="0" step="1" value="28"></div>
   <div id="loft-chart" hidden><div class="control"><label class="label" for="restLength">Rest length · mm</label><input id="restLength" type="number" min="0" step="1" value="120"></div>
    <div class="control"><label class="label" for="referenceSection">Reference section · zero-based</label><input id="referenceSection" type="number" min="0" max="3" step="1" value="0"></div>
    <label class="label" for="stations">Station spacing</label><select id="stations" class="quality"><option value="datumDistance">Datum distance</option><option value="sectionIndex">Section index</option></select>
   </div>
  </div>

  <div class="divider"></div><button id="lifecycle" disabled>Check rebuild / release</button><p id="lifecycle-result" class="hint">12 rebuilds; live owner counts.</p>
 </div><div id="scene-controls" hidden></div></aside>
 <section id="viewport" class="viewport" aria-label="Interactive native model">
  <div class="toolbar"><div><button id="fit">Fit model</button><button id="code-toggle" aria-expanded="false">View TypeScript</button></div><span class="revision" id="revision">Loading kernel</span></div>
  <div class="code" id="code" hidden><pre></pre></div>
  <div class="status" id="status" role="status"><strong>Loading the native kernel…</strong><div class="detail">Authoring and geometric validation run in Rust/Wasm.</div></div>
 </section>
 <aside id="geometry-inspector" class="geometry-inspector"><h2 style="margin-top:0">Measurements · <span id="source-name">panel</span></h2><div class="stats">
  <div class="stat wide"><small>Volume · mm³</small><b id="volume">—</b></div><div class="stat wide"><small>Surface area · mm²</small><b id="area">—</b></div>
  <div class="stat"><small>Native faces</small><b id="native-faces">—</b></div><div class="stat"><small>Native edges</small><b id="native-edges">—</b></div></div>
  <h2>Display data</h2><div class="metrics-line"><span>Triangles</span><code id="triangles">—</code></div><div class="metrics-line"><span>Render vertices</span><code id="vertices">—</code></div><div class="metrics-line"><span>Reported error</span><code id="deviation">—</code></div><div class="metrics-line"><span>CPU display buffers</span><code id="buffers">—</code></div><div class="metrics-line"><span>Compile / cache hits</span><code id="cache">—</code></div>
  <div class="metrics-line"><span>Construction UV vertices</span><code id="uv-vertices">—</code></div><div class="metrics-line"><span>Generation attempts</span><code id="attempts">—</code></div><h2>Timing</h2><div class="metrics-line"><span>Build / validation call</span><code id="build-kernel-time">—</code></div><div class="metrics-line"><span>Realize calls</span><code id="realize-kernel-time">—</code></div><div class="metrics-line"><span>STEP export call</span><code id="step-time">—</code></div><div class="metrics-line"><span>Last refused call</span><code id="refused-time">—</code></div><details><summary>SDK / transfer breakdown</summary><div class="metrics-line"><span>SDK prepare / transfer</span><code id="sdk-transfer-time">—</code></div><div class="metrics-line"><span>Workplane query</span><code id="workplane-time">—</code></div><p class="hint">Host-observed Wasm calls, in ms. SDK/transfer are separate; excludes rendering and GPU presentation.</p></details><h2>Features</h2><div id="features"></div><p class="hint">Pick a face or feature.</p>
  <h2>Selection</h2><div id="selection" class="selection">No selection.</div><p id="selection-notice" class="notice"></p>
  <h2>Construction</h2><div id="attachment" class="hint">The gold pin is placed from the tagged recess floor.</div>
  <h2>Export</h2><div class="downloads"></div><div id="step-result" class="step-result">STEP: one part per file.</div>
  <h2>Memory / ownership</h2><div id="ownership" class="hint">—</div>
 </aside>
 <section id="scene-view" class="scene-slot" aria-label="Native stage" hidden></section>
</main>`;
const element = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const viewport = element<HTMLElement>("viewport");
element("code").querySelector("pre")!.textContent = sampleSource;
const controlsNames = ["width", "height", "thickness", "pocketDepth", "cornerRadius", "ventRadius"] as const;
const parameters = (): PanelParameters => Object.fromEntries(controlsNames.map(key => [key, Number(element<HTMLInputElement>(key).value)])) as unknown as PanelParameters;
const collarNames = ["radius","wall","collarHeight","angle","rounding","innerRadius"] as const;
const collarParameters = (): CollarParameters => Object.fromEntries(collarNames.map(key=>[key,Number(element<HTMLInputElement>(key).value)])) as unknown as CollarParameters;
const operationNames=["railRadius","bendRadius","turnAngle","loftHeight","loftTwist","loftOffset","finishSize","finishHeight"] as const;
const value=(key:string)=>Number(element<HTMLInputElement>(key).value);
const boreNames=["boreWidth","boreHeight","boreThickness","boreRadius","boreAngle","boreSeparation"] as const;
const boreParameters=(delta=0)=>({width:value("boreWidth"),height:value("boreHeight"),thickness:value("boreThickness"),radius:value("boreRadius")+delta,angle:value("boreAngle"),separation:value("boreSeparation")});
const housingNames=["housingWidth","housingBodyWidth","housingHeight","housingFlangeDepth","housingThickness","housingPassageRadius","housingPassageY","housingPassageAngle","housingMountSpacing","housingMountRadius","housingPortY","housingPortRadius","housingClearance"] as const;
const housingParameters=(delta=0):HousingParameters=>({width:value("housingWidth")+delta,bodyWidth:value("housingBodyWidth"),height:value("housingHeight"),flangeDepth:value("housingFlangeDepth"),thickness:value("housingThickness"),passageRadius:value("housingPassageRadius"),passageY:value("housingPassageY"),passageAngle:value("housingPassageAngle"),mountSpacing:value("housingMountSpacing"),mountRadius:value("housingMountRadius"),portY:value("housingPortY"),portRadius:value("housingPortRadius"),clearance:value("housingClearance")});
const flangeNames=["flangeBore","sleeveOuter","flangeOuter","shoulderHeight","shoulderSize","sleeveHeight","flangeAngle"] as const;
const flangeParameters=(delta=0):FlangeParameters=>({boreRadius:value("flangeBore")+delta,sleeveOuterRadius:value("sleeveOuter"),flangeOuterRadius:value("flangeOuter"),shoulderHeight:value("shoulderHeight"),height:value("sleeveHeight"),angle:value("flangeAngle"),shoulder:element<HTMLSelectElement>("shoulderFinish").value==="fillet"?{kind:"fillet",radius:value("shoulderSize")}:{kind:"chamfer",distance:value("shoulderSize")}});
const poleNames=["poleRadius","poleHeight"] as const;
const poleParameters=(delta=0)=>({radius:value("poleRadius")+delta,height:value("poleHeight")});
const authorCurrent = (delta=0) => {
  const sample=element<HTMLSelectElement>("sample").value;
  const p={...collarParameters(),radius:collarParameters().radius+delta};
  switch(sample) {
    case "rail":case "hollowRail":return authorRail({radius:value("railRadius")+delta,bendRadius:value("bendRadius"),angle:value("turnAngle"),hollow:sample === "hollowRail"});
    case "loft":return authorVariableLoft({height:value("loftHeight")+delta,twist:value("loftTwist"),offset:value("loftOffset")});
    case "fillet":case "chamfer":return authorFinishedPlate({size:value("finishSize")+delta,height:value("finishHeight"),treatment:sample,inner:sample === "chamfer"});
    case "housing":return authorHousing(housingParameters(delta));
    case "bore":return authorAngledBore(boreParameters(delta));
    case "collar": return authorCollar(p);
    case "roundedRing": case "torus": case "hollowTorus": return authorRoundedTurn(p,sample);
    case "sleeve": return authorFlangedSleeve(flangeParameters(delta));
    case "sphere": return authorSphere(poleParameters(delta));
    case "dome": return authorDome(poleParameters(delta));
    case "cone": return authorPointedCone(poleParameters(delta));
    case "fitting": return authorTurnedFitting(p);
    default:return authorPanel({...parameters(),width:parameters().width+delta});
  }
};
const tolerance = () => Number(element<HTMLSelectElement>("quality").value);
const format = (n: number, digits = 2) => n.toLocaleString(undefined, { maximumFractionDigits: digits });
const formatMeasure = (value: NativeMeasurement) => value.status === "available" ? format(value.value,3) : value.status === "unavailable" ? "Not available" : "Numeric limit";
const featureColors: Record<string, string> = {stock:"#7eabb8","left mount":"#d7ab76","right mount":"#d7ab76","main passage":"#7ec9bb","service port":"#b9a1d6", shell:"#8bbbc4",base:"#c69c76",plate:"#4baca7",bore:"#bf9878",panel: "#4baca7", vents: "#6f9fdb", recess: "#d2a36d", pin: "#f1c77f", collar:"#8bbbc4" };

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
let pendingBuild=false;
let pendingDisplay=false;
let frameRequest:number|null=null;
let renderingFrame=false;
let geometryActive=true;
function invalidate(){
  if(!geometryActive || document.hidden || renderingFrame || frameRequest!==null)return;
  frameRequest=requestAnimationFrame(frame);
}
function setGeometryActive(active:boolean){
  geometryActive=active;orbit.enabled=active;
  if(!active && frameRequest!==null){cancelAnimationFrame(frameRequest);frameRequest=null;}
  if(active)resize();
}
const resize = () => {
  const {width,height}=viewport.getBoundingClientRect();if(width<=0 || height<=0)return;
  renderer.setSize(width,height);camera.aspect=width/height;camera.updateProjectionMatrix();invalidate();
};
new ResizeObserver(resize).observe(viewport);resize();
orbit.addEventListener("change",invalidate);
document.addEventListener("visibilitychange",()=>{if(!document.hidden)invalidate();});
function frame(){
  frameRequest=null;if(!geometryActive || document.hidden)return;
  renderingFrame=true;
  let moving=false;
  try {
    // Latest inputs are consumed before submitting this frame's model.
    if(pendingBuild){pendingBuild=false;rebuild();}
    if(pendingDisplay){pendingDisplay=false;redraw();}
    moving=orbit.update();renderer.render(scene,camera);
  }finally{renderingFrame=false;}
  if(moving)invalidate();
}

interface Entry { part: NativePart; display: Realization; mesh: THREE.Mesh<THREE.BufferGeometry, THREE.MeshStandardMaterial[]>; triangles:ReturnType<typeof meshTriangles> }
interface Drawn { group: THREE.Group; entries: Entry[] }
let session: AuthoringSession;
let current: NativeDocument | null = null;
let drawn: Drawn | null = null;
let selected: FaceSelection | null = null;
let selectedFeature:string|null=null;
let highlight: THREE.Mesh | null = null;
type Draft = {kind:"invalid";error:unknown}|{kind:"valid";document:NativeDocument};
let draft:Draft|null=null;
const authoredError=()=>draft?.kind==="invalid"?draft.error:null;
let displayError: unknown = null;
let hasCamera = false;

let checkerTexture:THREE.DataTexture|null=null;
function checkerMap(){
  if(!checkerTexture){
    checkerTexture=new THREE.DataTexture(new Uint8Array([255,255,255,255,55,55,55,255,55,55,55,255,255,255,255,255]),2,2);
    checkerTexture.colorSpace=THREE.SRGBColorSpace;checkerTexture.wrapS=checkerTexture.wrapT=THREE.RepeatWrapping;checkerTexture.magFilter=THREE.NearestFilter;checkerTexture.needsUpdate=true;
  }
  return checkerTexture;
}
const displayChart=(part:NativePart)=>element<HTMLInputElement>("checker").checked?constructionChart(part,{repeat:value("uvRepeat"),referenceRadius:value("referenceRadius"),restLength:value("restLength"),referenceSection:value("referenceSection"),stations:element<HTMLSelectElement>("stations").value as "sectionIndex"|"datumDistance"}):undefined;
function chartControls(document:NativeDocument){
  const supported=Object.values(document.parts).filter(p=>p.chartKind!==null),kind=supported[0]?.chartKind;
  element<HTMLInputElement>("checker").disabled=!supported.length;
  element("chart-support").textContent=supported.length?"Exact native rest coordinates.":"Construction charts unavailable for this source.";
  element("chart-controls").hidden=!supported.length||!element<HTMLInputElement>("checker").checked;
  element("revolve-chart").hidden=kind!=="revolve";element("loft-chart").hidden=kind!=="loft";
}
function materials(part: NativePart,display:Realization) {
  const names = [...new Set([part.name, ...part.faces().flatMap(f => f.features.length?f.features:[part.name])])];
  const values = names.map(name => new THREE.MeshStandardMaterial({ color: featureColors[name] ?? "#87abb7", metalness: 0.25, roughness: 0.35,map:display.uvs?checkerMap():null }));
  return { names, values };
}
function draw(document: NativeDocument): Drawn {
  const group = new THREE.Group(); const entries: Entry[] = [];
  try {
    for (const part of Object.values(document.parts)) {
      const display = part.realize({ tolerance: tolerance(),chart:displayChart(part) });
      const geometry = new THREE.BufferGeometry();
      geometry.setAttribute("position", new THREE.BufferAttribute(display.positions, 3));
      geometry.setAttribute("normal", new THREE.BufferAttribute(display.normals, 3));
      geometry.setIndex(new THREE.BufferAttribute(display.indices, 1));
      if(display.uvs)geometry.setAttribute("uv",new THREE.BufferAttribute(display.uvs,2));
      const style = materials(part,display);
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
      const triangles=meshTriangles(mesh,element<HTMLInputElement>("wireframe").checked);
      group.add(mesh,triangles.mesh); entries.push({ part, display, mesh, triangles });
    }
    return { group, entries };
  } catch (error) { dispose({ group, entries }); throw error; }
}
function dispose(view: Drawn) { for (const entry of view.entries) { entry.triangles.dispose(); entry.mesh.geometry.dispose(); for (const material of entry.mesh.material) material.dispose(); } }
function clearHighlight() {
  if (!highlight) return;
  scene.remove(highlight); highlight.geometry.dispose();
  (highlight.material as THREE.Material).dispose(); highlight = null;
}
function showSelection(selection: FaceSelection | null, feature?:string) {
  invalidate();
  clearHighlight(); selected = selection;selectedFeature=selection?(feature??selection.feature??null):null;
  if (!selection || !drawn) { element("selection").textContent = "No selection."; return; }
  const entry = drawn.entries.find(e => e.part.id === selection.partId);
  if (!entry) return;
  entry.part.requireSelection(selection);
  element("selection-notice").textContent="";
  const family = selectedFeature ? entry.part.faces(selectedFeature) : [selection];
  const ids = new Set(family.map(f => f.nativeId));
  const indices: number[] = [];
  entry.display.nativeFaces.forEach((face, triangle) => { if (ids.has(face)) indices.push(...entry.display.indices.subarray(triangle * 3, triangle * 3 + 3)); });
  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute("position", new THREE.BufferAttribute(entry.display.positions, 3));
  geometry.setAttribute("normal", new THREE.BufferAttribute(entry.display.normals, 3));
  geometry.setIndex(indices);
  if(entry.display.uvs)geometry.setAttribute("uv",new THREE.BufferAttribute(entry.display.uvs,2));
  highlight = new THREE.Mesh(geometry, new THREE.MeshStandardMaterial({ color: 0xfce4a2,map:entry.display.uvs?checkerMap():null, emissive: 0xa2650c, emissiveIntensity: 0.6, metalness: 0.3, roughness: 0.3, transparent: true, opacity: 0.78, polygonOffset: true, polygonOffsetFactor: -1, polygonOffsetUnits: -1 }));
  scene.add(highlight);
  const box = element("selection"); box.replaceChildren();
  const strong = document.createElement("strong"); strong.textContent = selectedFeature??(selection.features.length?selection.features.join(" / "):`Unlabelled ${selection.role}`); box.append(strong, document.createElement("br"));
  box.append(`${entry.part.name} · ${selection.role} · ${selection.surface}`, document.createElement("br"));
  const detail = document.createElement("code"); detail.textContent = `native face ${selection.nativeId} · ${family.length} feature faces`; box.append(detail);
  const operation=entry.part.machining?.operations.find(operation=>operation.wallFace===selection.nativeId);
  if(operation){
    const evidence=document.createElement("p");evidence.className="hint";evidence.textContent=`Radius ${format(operation.radius,3)} mm. Local axis point [${operation.point.map(value=>format(value,3)).join(", ")}], direction [${operation.direction.map(value=>format(value,3)).join(", ")}]. Certified clearance ${format(operation.minimumClearance,3)} mm. Native rims ${operation.rims.join(" / ")}.`;box.append(evidence);
  }
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
  element("ownership").textContent = `${stats.documents} document · ${stats.nativeParts} parts · ${stats.cacheEntries}/8 cached realizations.${draft?.kind==="valid"&&draft.document!==current?" One model awaiting display.":""}`;
}
function evidence() {
  if (!current || !drawn) return;
  const panel = Object.values(current.parts)[0];
  element("source-name").textContent = `${panel.name} · ${panel.family}`;
  element("volume").textContent = formatMeasure(panel.measurements.volume);
  element("area").textContent = formatMeasure(panel.measurements.surfaceArea);
  for (const [id,value] of [["volume",panel.measurements.volume],["area",panel.measurements.surfaceArea]] as const)
    element(id).title=value.status === "available" ? value.basis === "constructorAndAnalyticMachining" ? "Exact constructor integral plus analytic machining deltas" : "Native source integral" : value.diagnostic.message;
  element("native-faces").textContent = String(panel.measurements.faces);
  element("native-edges").textContent = String(panel.measurements.edges);
  const report = drawn.entries.map(e => e.display.report);
  element("triangles").textContent = format(report.reduce((n, r) => n + r.triangles, 0), 0);
  element("vertices").textContent = format(report.reduce((n, r) => n + r.vertices, 0), 0);
  element("deviation").textContent = `${format(Math.max(...report.map(r => r.maxDeviation)), 5)} mm`;
  const bytes = drawn.entries.reduce((n, e) => n + e.display.positions.byteLength + e.display.normals.byteLength + e.display.indices.byteLength + e.display.nativeFaces.byteLength + (e.display.uvs?.byteLength ?? 0), 0);
  element("buffers").textContent = `${format(bytes / 1024, 1)} KiB`;
  element("cache").textContent = `${report.reduce((n, r) => n + r.nativeRealizations, 0)} / ${report.reduce((n, r) => n + r.cacheHits, 0)}`;
  element("revision").textContent = current.id.split("/").at(-1)!;
  element("uv-vertices").textContent=drawn.entries.some(e=>e.display.uvs)?format(drawn.entries.reduce((sum,e)=>sum+(e.display.uvs?.length??0)/2,0),0):"Off";
  element("attempts").textContent=drawn.entries.map(e=>`${e.part.name}: ${e.display.report.work.attempts}`).join(" / ");
  element("attempts").title="Work that generated each cached output; current-call cache/compile counts are separate.";
  const calls=drawn.entries.map(e=>e.display.timing),build=current.buildTiming;
  element("build-kernel-time").textContent=`${format(build.kernelMs,2)} ms`;
  element("realize-kernel-time").textContent=`${format(calls.reduce((n,t)=>n+t.kernelMs,0),2)} ms`;
  element("sdk-transfer-time").textContent=`${format(build.sdkPrepareMs+calls.reduce((n,t)=>n+t.sdkPrepareMs,0),2)} / ${format(calls.reduce((n,t)=>n+t.transferMs,0),2)} ms`;
  if (current.parts.pin && panel.family === "extrusion") {
    const plane = panel.floor("recess").workplane();
    element("workplane-time").textContent=`${format(plane.timing.kernelMs,2)} ms`;
    element("attachment").textContent = `Native recess floor z = ${format(plane.origin[2])} mm. The pin begins at z = ${format(current.parts.pin.measurements.bounds[0][2])} mm, offset 0.5 mm along its normal.`;
  } else if(panel.machining){
    element("workplane-time").textContent="—";
    const operations=panel.machining.operations;
    element("attachment").textContent=`${operations.length} retained-body cylinder cuts. Minimum certified material clearance ${format(Math.min(...operations.map(operation=>operation.minimumClearance)),3)} mm. Exact analytic volume/area derive from the blank and machining deltas. Select each named passage to inspect its native surface.`;
  } else if(panel.bore){
    const bore=panel.bore;
    element("workplane-time").textContent="—";
    element("attachment").textContent=`Native ellipse rims: ${bore.rims.map(r=>`${format(r.semiMajor,3)} × ${format(r.semiMinor,3)} mm semiaxes`).join(" / ")}. Minimum sidewall clearance ${format(bore.minimumClearance,3)} mm. Axis inputs are in plate construction space.`;
  } else {
    element("workplane-time").textContent="—";
    element("attachment").textContent = `${panel.family}. Bounds enclose native supporting patches and may include space outside trims.`;
  }
  const features = element("features"); features.replaceChildren();
  for (const [partName, part] of Object.entries(current.parts)) {
    for (const feature of [...new Set(part.faces().flatMap(f => f.features))]) {
      const row = document.createElement("button"); row.className = "feature-row"; row.style.width = "100%"; row.style.background = "none"; row.style.borderRadius = "0"; row.style.borderWidth = "0 0 1px";
      row.setAttribute("aria-label", `Select ${feature} feature`);
      const swatch = document.createElement("i"); swatch.className = "swatch"; swatch.style.background = featureColors[feature] ?? "#87abb7";
      const count = document.createElement("span"); count.textContent = `${part.faces(feature).length} faces`;
      row.append(swatch, `${partName} / ${feature}`, count); row.onclick = () => showSelection(part.faces(feature)[0],feature); features.append(row);
    }
  }
  renderDownloads();
  ownership();
}
function status(title: string, detail: string, error = false) {
  const box = element("status"); box.classList.toggle("invalid", error);
  box.querySelector("strong")!.textContent = title; box.querySelector(".detail")!.textContent = detail;
}
function showRefusalTiming(error:unknown) {
  if(error instanceof ExedraError && error.timing) {
    element("refused-time").textContent=`${format(error.timing.kernelMs,2)} ms`;
    element("refused-time").title=`${error.timing.operation} · ${error.diagnostic.code} · SDK total ${format(error.timing.totalMs,2)} ms`;
  }
}
function reportError(error: unknown) {
  const diagnostic = error instanceof ExedraError ? error.diagnostic : { code: "unexpected", operation: "demo", message: String(error) };
  status(current ? (authoredError()?"Showing the last valid model":"Showing the last valid display") : "Model was refused", `${diagnostic.code} · ${diagnostic.operation}: ${diagnostic.message}`, true);
  setExportDisabled(true);if(session)ownership();
}
function accept(document: NativeDocument, view: Drawn, edited: boolean) {
  const old = current, prior = drawn;
  clearHighlight();
  if (prior) { scene.remove(prior.group); dispose(prior); }
  current = document; drawn = view; scene.add(view.group);
  if (edited && selected) { selected = null; element("selection-notice").textContent = "Selection cleared: authored revision changed."; }
  if (old && old !== document) old.dispose();
  evidence(); if (!hasCamera) fit();
  showSelection(selected,selectedFeature??undefined);
  if(edited)draft=null;
  displayError = null;
  if(authoredError()){reportError(authoredError());return;}
  const within = view.entries.every(e => e.display.report.withinTolerance);
  status(within ? "Ready" : "Display tolerance unmet", within ? "Display tolerance met." : "See the reported error and requested tolerance.", !within);
  setExportDisabled(false);
}
function redraw(){
  const target=draft?.kind==="valid"?draft.document:current;if(!target)return;
  chartControls(target);
  try{const view=draw(target);accept(target,view,target!==current);}
  catch(error){displayError=error;showRefusalTiming(error);reportError(authoredError()??error);}
}
function rebuild(){
  if(!session)return;
  if(draft?.kind==="valid")draft.document.dispose();draft=null;
  let candidate:NativeDocument;
  try{candidate=session.build(authorCurrent());draft={kind:"valid",document:candidate};}
  catch(error){draft={kind:"invalid",error};showRefusalTiming(error);reportError(error);return;}
  pendingDisplay=false;redraw();
}
// One pending frame reads the latest controls; dragging never queues obsolete builds.
function scheduleBuild() { pendingBuild = true; invalidate(); }
for (const key of [...controlsNames,...collarNames,...operationNames,...flangeNames,...boreNames,...poleNames,...housingNames]) {
  const input = element<HTMLInputElement>(key);
  const label = () => { element(`${key}-value`).textContent = `${input.value} ${["angle","turnAngle","loftTwist","flangeAngle","boreAngle","housingPassageAngle"].includes(key) ? "°" : "mm"}`; };
  label(); input.addEventListener("input", () => { label(); scheduleBuild(); });
}
element<HTMLSelectElement>("shoulderFinish").onchange=()=>{element("shoulderSize-label").textContent=element<HTMLSelectElement>("shoulderFinish").value==="fillet"?"Fillet radius":"Chamfer distance";scheduleBuild();};
element<HTMLSelectElement>("sample").onchange = () => {
  const sample=element<HTMLSelectElement>("sample").value, panel=sample === "panel", torus=sample === "torus" || sample === "hollowTorus";
  const rail=sample === "rail" || sample === "hollowRail", loft=sample === "loft", finish=sample === "fillet" || sample === "chamfer";
  const pole=sample === "sphere" || sample === "dome" || sample === "cone";
  element("panel-controls").hidden=!panel; element("collar-controls").hidden=panel || rail || loft || finish || pole || sample === "sleeve" || sample === "bore" || sample === "housing";
  element("pole-controls").hidden=!pole;element("poleHeight-control").hidden=sample === "sphere";
  if(pole){
    const height=sample === "dome"?12:40;
    element<HTMLInputElement>("poleRadius").value="25";element("poleRadius-value").textContent="25 mm";
    element<HTMLInputElement>("poleHeight").value=String(height);element("poleHeight-value").textContent=`${height} mm`;
    element("poleRadius-label").textContent=sample === "cone"?"Base radius":"Sphere radius";
    element("pole-hint").textContent=sample === "dome"?"Cap height may reach the sphere diameter. Invalid dimensions retain the last valid model.":"Full revolutions with native poles. Show triangles to inspect the mesh near the tip.";
  }
  element("housing-controls").hidden=sample!=="housing";
  element("flange-controls").hidden=sample!=="sleeve";element("bore-controls").hidden=sample!=="bore";
  element("rail-controls").hidden=!rail;element("loft-controls").hidden=!loft;element("finish-controls").hidden=!finish;
  for(const [key,initial] of [["railRadius",sample === "hollowRail"?8:6],["bendRadius",40],["turnAngle",135],["loftHeight",120],["loftTwist",14],["loftOffset",18],["finishSize",sample === "chamfer"?3:5],["finishHeight",30]] as const){element<HTMLInputElement>(key).value=String(initial);element(`${key}-value`).textContent=`${initial} ${["turnAngle","loftTwist"].includes(key)?"°":"mm"}`;}

  element("rounding-control").hidden=sample !== "roundedRing";element("innerRadius-control").hidden=sample !== "hollowTorus";
  element<HTMLInputElement>("collarHeight").closest<HTMLElement>(".control")!.hidden=torus;
  element("radius-label").textContent=torus ? "Major radius" : "Bore radius";
  element("wall-label").textContent=torus ? "Tube radius" : "Wall thickness";
  const defaults=sample === "roundedRing" ? [22,12,24,360] : torus ? [35,7,30,sample === "hollowTorus" ? 270 : 360] : sample === "sleeve" ? [12,10,35,270] : sample === "fitting" ? [7,15,47,270] : [12,7,30,270];
  for (const [i,key] of ["radius","wall","collarHeight","angle"].entries()) {
    element<HTMLInputElement>(key).value=String(defaults[i]); element(`${key}-value`).textContent=`${defaults[i]} ${["angle","turnAngle","loftTwist","flangeAngle","boreAngle","housingPassageAngle"].includes(key) ? "°" : "mm"}`;
  }
  for(const [key,value] of [["rounding",2],["innerRadius",4]] as const) {element<HTMLInputElement>(key).value=String(value);element(`${key}-value`).textContent=`${value} mm`;}
  element<HTMLInputElement>("referenceRadius").value=String(pole?value("poleRadius"):torus?defaults[0]:defaults[0]+defaults[1]/2);
  element<HTMLInputElement>("restLength").value="120";element<HTMLInputElement>("referenceSection").value="0";
  hasCamera=false; scheduleBuild();
};
function scheduleDisplay(){pendingDisplay=true;invalidate();}
element<HTMLSelectElement>("quality").onchange=scheduleDisplay;
for(const key of ["checker","uvRepeat","referenceRadius","restLength","referenceSection","stations"]){element(key).addEventListener(key==="checker"||key==="stations"?"change":"input",scheduleDisplay);}
element<HTMLInputElement>("wireframe").onchange = () => { for (const e of drawn?.entries ?? []) e.triangles.setVisible(element<HTMLInputElement>("wireframe").checked); invalidate(); };
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
function setExportDisabled(disabled: boolean) { for (const button of document.querySelectorAll<HTMLButtonElement>(".downloads button")) button.disabled=disabled || button.dataset.stepSupported !== "true"; }
function renderDownloads() {
  const box=document.querySelector<HTMLDivElement>(".downloads")!; box.replaceChildren();
  for (const [name,part] of Object.entries(current?.parts ?? {})) {
    const support=part.stepSupport;
    const button=document.createElement("button"); button.id=`step-${name}`;button.textContent=`Download ${name} STEP`;button.onclick=()=>downloadStep(name);
    button.dataset.stepSupported=String(support.status === "supported");button.disabled=support.status !== "supported" || !!authoredError() || !!displayError;box.append(button);
    if(support.status === "unsupported") {
      const reason=document.createElement("small");reason.textContent=`${name}: STEP does not support ${support.diagnostic.geometryFamily ?? "this geometry"}${support.diagnostic.face === undefined ? "" : ` (face ${support.diagnostic.face})`}.`;reason.title=support.diagnostic.message;box.append(reason);
    }
  }
}
function downloadStep(part: string) {
  if (!current || authoredError() || displayError) return;
  try {
    const file = current.parts[part].exportStep({ name: part, timestamp: new Date().toISOString() });
    element("step-time").textContent=`${format(file.timing.kernelMs,2)} ms`;
    element("step-time").title=`Transfer ${format(file.timing.transferMs,2)} ms · SDK total ${format(file.timing.totalMs,2)} ms`;
    const url = URL.createObjectURL(new Blob([file.text], { type: "application/step" }));
    const link = document.createElement("a"); link.href = url; link.download = `exedra-${part}.step`; link.click(); setTimeout(() => URL.revokeObjectURL(url), 1000);
    element("step-result").textContent = `${part}: ${file.report.schema} · ${file.report.faces} native faces · ${file.report.entities} entities · ${file.report.units}. Exported directly from the native source.`;
  } catch (error) {
    element("step-result").textContent = error instanceof Error ? error.message : String(error);
    showRefusalTiming(error);
    if(error instanceof ExedraError && error.timing) {element("step-time").textContent=`${format(error.timing.kernelMs,2)} ms · refused`;element("step-time").title=`${error.diagnostic.code} · SDK total ${format(error.timing.totalMs,2)} ms`;}
  }
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
const views=mountViews({
  select:element<HTMLSelectElement>("view"),
  geometry:[element("geometry-controls"),viewport,element("geometry-inspector")],
  scene:element("scene-view"),sceneControls:element("scene-controls"),
  setGeometryActive,
  loadScene:()=>import("./scene.js"),
});
if(new URL(location.href).searchParams.get("view")==="scene"){
  element<HTMLSelectElement>("view").value="scene";
  element("view").dispatchEvent(new Event("change"));
}
window.addEventListener("beforeunload", () => { views.dispose(); setGeometryActive(false); pendingBuild = false;pendingDisplay=false; clearHighlight(); if (drawn) dispose(drawn); session?.dispose();checkerTexture?.dispose(); renderer.dispose(); orbit.dispose(); });
const requestedSample=new URL(location.href).searchParams.get("sample");
if(requestedSample&&[...element<HTMLSelectElement>("sample").options].some(o=>o.value===requestedSample)){element<HTMLSelectElement>("sample").value=requestedSample;element("sample").dispatchEvent(new Event("change"));}
if(new URL(location.href).searchParams.get("checker")==="1")element<HTMLInputElement>("checker").checked=true;
try {
  session = await createSession({ units: "mm", separation: 1e-6 });
  rebuild(); element<HTMLButtonElement>("lifecycle").disabled = false;
} catch (error) { showRefusalTiming(error);reportError(error); }
