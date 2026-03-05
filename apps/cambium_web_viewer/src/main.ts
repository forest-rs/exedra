import "./style.css";
import {
  AmbientLight,
  Box3,
  BufferAttribute,
  BufferGeometry,
  CanvasTexture,
  Color,
  DoubleSide,
  DirectionalLight,
  Float32BufferAttribute,
  Group,
  HemisphereLight,
  LineBasicMaterial,
  LineSegments,
  Mesh,
  MeshStandardMaterial,
  PerspectiveCamera,
  RepeatWrapping,
  SRGBColorSpace,
  Scene,
  Vector3,
  WebGLRenderer,
} from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";

type StepStats = {
  faces_processed: number;
  created_vertices: number;
  created_faces: number;
  deleted_faces: number;
  deleted_vertices: number;
  corners_written: number;
  edges_written: number;
};

type StepDiagnostic = {
  level: string;
  code: string;
  message: string;
};

type MeshBuffers = {
  indices: number[];
  positions: number[];
  uvs: number[];
  normals: number[];
  topology_lines: number[];
  region_ids: number[];
};

type StepSnapshot = {
  label: string;
  operator: string | null;
  plan_fingerprint: number | null;
  mesh_signature: number;
  mesh: MeshBuffers;
  stats: StepStats;
  diagnostics: StepDiagnostic[];
};

type ScenarioResponse = {
  scenario: string;
  steps: StepSnapshot[];
};

type ScenarioOptions = {
  include_initial: boolean;
};

type WasmApi = {
  list_scenarios_json: () => string;
  run_scenario_json: (name: string, optionsJson: string) => string;
};

const scenarioSelect = document.getElementById("scenario") as HTMLSelectElement;
const runButton = document.getElementById("run") as HTMLButtonElement;
const stepSlider = document.getElementById("step-slider") as HTMLInputElement;
const stepLabel = document.getElementById("step-label") as HTMLDivElement;
const operatorLabel = document.getElementById("operator") as HTMLDivElement;
const fingerprintLabel = document.getElementById("fingerprint") as HTMLDivElement;
const signatureLabel = document.getElementById("mesh-signature") as HTMLDivElement;
const statsPre = document.getElementById("stats") as HTMLPreElement;
const stepDeltaDiv = document.getElementById("step-delta") as HTMLDivElement;
const diagnosticsPre = document.getElementById("diagnostics") as HTMLPreElement;
const wireframeToggle = document.getElementById("wireframe") as HTMLInputElement;
const regionColorsToggle = document.getElementById("region-colors") as HTMLInputElement;
const topologyLinesToggle = document.getElementById("topology-lines") as HTMLInputElement;
const viewport = document.getElementById("viewport") as HTMLDivElement;

const scene = new Scene();
scene.background = new Color(0xf2f5fb);

const camera = new PerspectiveCamera(50, 1, 0.01, 1000);
camera.position.set(0, 0, 4);

const renderer = new WebGLRenderer({ antialias: true });
viewport.appendChild(renderer.domElement);

const controls = new OrbitControls(camera, renderer.domElement);
controls.enableDamping = true;

renderer.outputColorSpace = SRGBColorSpace;

const ambient = new AmbientLight(0xffffff, 0.18);
const hemisphere = new HemisphereLight(0xf7fbff, 0xc8d0dd, 0.72);
const keyLight = new DirectionalLight(0xfff6eb, 1.75);
keyLight.position.set(1.6, 2.1, 2.4);
const fillLight = new DirectionalLight(0xd9e8ff, 0.75);
fillLight.position.set(-1.4, 0.8, 1.2);
const rimLight = new DirectionalLight(0xffffff, 0.55);
rimLight.position.set(-0.8, -1.3, 1.8);
scene.add(ambient, hemisphere, keyLight, fillLight, rimLight);

function createUvDebugTexture(): CanvasTexture {
  const size = 256;
  const canvas = document.createElement("canvas");
  canvas.width = size;
  canvas.height = size;
  const ctx = canvas.getContext("2d");
  if (!ctx) {
    throw new Error("failed to create canvas context for UV debug texture");
  }

  ctx.fillStyle = "#f4f4f4";
  ctx.fillRect(0, 0, size, size);

  const cells = 8;
  const cellSize = size / cells;
  for (let y = 0; y < cells; y += 1) {
    for (let x = 0; x < cells; x += 1) {
      const dark = (x + y) % 2 === 0;
      ctx.fillStyle = dark ? "#161616" : "#dadada";
      ctx.fillRect(x * cellSize, y * cellSize, cellSize, cellSize);
    }
  }

  ctx.strokeStyle = "#3da0ff";
  ctx.lineWidth = 2;
  ctx.strokeRect(1, 1, size - 2, size - 2);
  ctx.strokeStyle = "#ffffff";
  ctx.lineWidth = 1;
  for (let i = 1; i < cells; i += 1) {
    const p = i * cellSize;
    ctx.beginPath();
    ctx.moveTo(p, 0);
    ctx.lineTo(p, size);
    ctx.stroke();
    ctx.beginPath();
    ctx.moveTo(0, p);
    ctx.lineTo(size, p);
    ctx.stroke();
  }

  const texture = new CanvasTexture(canvas);
  texture.wrapS = RepeatWrapping;
  texture.wrapT = RepeatWrapping;
  texture.repeat.set(1, 1);
  texture.colorSpace = SRGBColorSpace;
  texture.needsUpdate = true;
  return texture;
}

const uvDebugTexture = createUvDebugTexture();

const shadedMaterial = new MeshStandardMaterial({
  color: new Color(0xffffff),
  map: uvDebugTexture,
  roughness: 0.55,
  metalness: 0.05,
  side: DoubleSide,
  wireframe: false,
});
const regionMaterial = new MeshStandardMaterial({
  vertexColors: true,
  roughness: 0.55,
  metalness: 0.05,
  side: DoubleSide,
  wireframe: false,
});

let currentMesh: Mesh<BufferGeometry, MeshStandardMaterial> | null = null;
let currentTopologyLines: LineSegments<BufferGeometry, LineBasicMaterial> | null = null;
let currentModelRoot: Group | null = null;
let currentResponse: ScenarioResponse | null = null;

function clearCurrentModel(): void {
  if (currentModelRoot) {
    scene.remove(currentModelRoot);
    currentModelRoot = null;
  }
  if (currentMesh) {
    currentMesh.geometry.dispose();
    currentMesh = null;
  }
  if (currentTopologyLines) {
    currentTopologyLines.geometry.dispose();
    currentTopologyLines.material.dispose();
    currentTopologyLines = null;
  }
}

function resizeRenderer(): void {
  const { clientWidth, clientHeight } = viewport;
  renderer.setSize(clientWidth, clientHeight, false);
  camera.aspect = clientWidth / Math.max(clientHeight, 1);
  camera.updateProjectionMatrix();
}

function animate(): void {
  requestAnimationFrame(animate);
  controls.update();
  renderer.render(scene, camera);
}

function stepToGeometry(step: StepSnapshot, regionMode: boolean): BufferGeometry {
  const geometry = new BufferGeometry();
  geometry.setIndex(step.mesh.indices);
  geometry.setAttribute("position", new Float32BufferAttribute(step.mesh.positions, 3));
  if (step.mesh.uvs.length > 0) {
    geometry.setAttribute("uv", new Float32BufferAttribute(step.mesh.uvs, 2));
  }

  if (regionMode) {
    const deindexed = geometry.toNonIndexed();
    deindexed.computeVertexNormals();
    const position = deindexed.getAttribute("position");
    const colors = new Float32Array(position.count * 3);
    // Map region IDs to distinct hues via golden-angle spacing.
    const regionColors = new Map<number, Color>();
    const regionHue = (id: number): Color => {
      let c = regionColors.get(id);
      if (!c) {
        const hue = id === 0 ? 0.0 : (id * 0.618033988749895) % 1.0;
        const sat = id === 0 ? 0.0 : 0.58;
        const lum = id === 0 ? 0.7 : 0.56;
        c = new Color().setHSL(hue, sat, lum);
        regionColors.set(id, c);
      }
      return c;
    };
    // region_ids is per-triangle; the deindexed mesh stores triangle vertices
    // sequentially, so each face color can be assigned without clobbering
    // neighboring regions that share topology vertices.
    const regionIds = step.mesh.region_ids ?? [];
    if (regionIds.length > 0) {
      for (let tri = 0; tri < regionIds.length; tri += 1) {
        const c = regionHue(regionIds[tri]);
        for (let v = 0; v < 3; v += 1) {
          const idx = tri * 3 + v;
          colors[idx * 3] = c.r;
          colors[idx * 3 + 1] = c.g;
          colors[idx * 3 + 2] = c.b;
        }
      }
    } else {
      // Fallback: color by X-position when region data is unavailable.
      for (let i = 0; i < position.count; i += 1) {
        const x = position.getX(i);
        const normalized = (Math.sin(x * 1.5) + 1.0) * 0.5;
        const c = new Color().setHSL(0.12 + normalized * 0.55, 0.58, 0.56);
        colors[i * 3] = c.r;
        colors[i * 3 + 1] = c.g;
        colors[i * 3 + 2] = c.b;
      }
    }
    deindexed.setAttribute("color", new BufferAttribute(colors, 3));
    deindexed.computeBoundingBox();
    deindexed.computeBoundingSphere();
    geometry.dispose();
    return deindexed;
  }

  // v0.1 bridge emits zero normals; always compute from geometry.
  geometry.computeVertexNormals();
  geometry.computeBoundingBox();
  geometry.computeBoundingSphere();
  return geometry;
}

function fitCameraToMesh(mesh: Mesh<BufferGeometry, MeshStandardMaterial>): void {
  const box = new Box3().setFromObject(mesh);
  const center = box.getCenter(new Vector3());
  const size = box.getSize(new Vector3());
  const radius = Math.max(size.x, size.y, size.z) * 0.6;
  camera.position.set(center.x + radius * 1.8, center.y + radius * 1.5, center.z + radius * 2.2);
  controls.target.copy(center);
  controls.update();
}

function formatDelta(value: number): string {
  if (value > 0) return `+${value}`;
  if (value < 0) return `${value}`;
  return "0";
}

function updatePanel(step: StepSnapshot, index: number, total: number): void {
  stepLabel.textContent = `Step ${index + 1}/${total}: ${step.label}`;
  operatorLabel.textContent = `Operator: ${step.operator ?? "none"}`;
  fingerprintLabel.textContent = `Fingerprint: ${step.plan_fingerprint ?? "-"}`;
  signatureLabel.textContent = `Mesh signature: ${step.mesh_signature}`;
  statsPre.textContent = JSON.stringify(step.stats, null, 2);
  const df = step.stats.created_faces - step.stats.deleted_faces;
  const dv = step.stats.created_vertices - step.stats.deleted_vertices;
  if (df !== 0 || dv !== 0) {
    stepDeltaDiv.textContent = `Delta: ${formatDelta(df)} faces, ${formatDelta(dv)} vertices`;
  } else {
    stepDeltaDiv.textContent = "";
  }
  diagnosticsPre.textContent =
    step.diagnostics.length === 0
      ? "No diagnostics."
      : step.diagnostics
          .map((diag) => `${diag.level}:${diag.code}: ${diag.message}`)
          .join("\n");
}

function renderStep(stepIndex: number): void {
  if (!currentResponse) {
    return;
  }
  const step = currentResponse.steps[stepIndex];
  if (!step) {
    return;
  }

  clearCurrentModel();

  const geometry = stepToGeometry(step, regionColorsToggle.checked);
  const material = regionColorsToggle.checked ? regionMaterial : shadedMaterial;
  material.wireframe = wireframeToggle.checked;

  currentMesh = new Mesh(geometry, material);
  currentMesh.renderOrder = 1;
  currentModelRoot = new Group();
  currentModelRoot.add(currentMesh);
  if (step.mesh.topology_lines.length > 0 && topologyLinesToggle.checked) {
    const lineGeometry = new BufferGeometry();
    lineGeometry.setAttribute(
      "position",
      new Float32BufferAttribute(step.mesh.topology_lines, 3),
    );
    const lineMaterial = new LineBasicMaterial({
      color: 0x111111,
      transparent: true,
      opacity: 0.85,
      depthTest: false,
      depthWrite: false,
    });
    currentTopologyLines = new LineSegments(lineGeometry, lineMaterial);
    currentTopologyLines.renderOrder = 2;
    currentModelRoot.add(currentTopologyLines);
  }
  scene.add(currentModelRoot);
  fitCameraToMesh(currentMesh);
  updatePanel(step, stepIndex, currentResponse.steps.length);
}

function showScenarioError(selected: string, error: unknown): void {
  currentResponse = null;
  clearCurrentModel();
  stepSlider.min = "0";
  stepSlider.max = "0";
  stepSlider.value = "0";
  stepLabel.textContent = `Scenario failed: ${selected}`;
  operatorLabel.textContent = "Operator: -";
  fingerprintLabel.textContent = "Fingerprint: -";
  signatureLabel.textContent = "Mesh signature: -";
  statsPre.textContent = "-";
  stepDeltaDiv.textContent = "";
  diagnosticsPre.textContent = `Scenario failed:\n${String(error)}`;
}

async function loadWasmApi(): Promise<WasmApi> {
  // Generated by wasm-pack in ../src/wasm_pkg from cambium_web_bridge.
  const module = await import("./wasm_pkg/cambium_web_bridge");
  await module.default();
  return module as WasmApi;
}

function parseScenarios(json: string): string[] {
  return JSON.parse(json) as string[];
}

function parseScenarioResponse(json: string): ScenarioResponse {
  return JSON.parse(json) as ScenarioResponse;
}

async function bootstrap(): Promise<void> {
  const wasm = await loadWasmApi();
  const scenarios = parseScenarios(wasm.list_scenarios_json());
  for (const name of scenarios) {
    const option = document.createElement("option");
    option.value = name;
    option.textContent = name;
    scenarioSelect.appendChild(option);
  }

  const runScenario = (): void => {
    const selected = scenarioSelect.value;
    const options: ScenarioOptions = { include_initial: true };
    try {
      const response = parseScenarioResponse(
        wasm.run_scenario_json(selected, JSON.stringify(options)),
      );
      if (response.scenario !== selected) {
        throw new Error(
          `scenario mismatch: requested ${selected}, received ${response.scenario}`,
        );
      }
      currentResponse = response;
      stepSlider.min = "0";
      stepSlider.max = String(Math.max(response.steps.length - 1, 0));
      stepSlider.value = "0";
      renderStep(0);
    } catch (error) {
      showScenarioError(selected, error);
    }
  };

  runButton.addEventListener("click", runScenario);
  scenarioSelect.addEventListener("change", runScenario);
  stepSlider.addEventListener("input", () => {
    renderStep(Number.parseInt(stepSlider.value, 10) || 0);
  });
  wireframeToggle.addEventListener("change", () => {
    renderStep(Number.parseInt(stepSlider.value, 10) || 0);
  });
  regionColorsToggle.addEventListener("change", () => {
    renderStep(Number.parseInt(stepSlider.value, 10) || 0);
  });
  topologyLinesToggle.addEventListener("change", () => {
    renderStep(Number.parseInt(stepSlider.value, 10) || 0);
  });

  window.addEventListener("keydown", (event) => {
    if (!currentResponse) {
      return;
    }
    const max = currentResponse.steps.length - 1;
    let current = Number.parseInt(stepSlider.value, 10) || 0;
    if (event.key === "ArrowLeft" || event.key === "[") {
      current = Math.max(current - 1, 0);
    } else if (event.key === "ArrowRight" || event.key === "]") {
      current = Math.min(current + 1, max);
    } else {
      return;
    }
    stepSlider.value = String(current);
    renderStep(current);
  });
  window.addEventListener("resize", resizeRenderer);
  resizeRenderer();
  runScenario();
  animate();
}

bootstrap().catch((error) => {
  diagnosticsPre.textContent = `Failed to initialize viewer:\n${String(error)}`;
});
