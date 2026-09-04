import "./inspector.css";
import {
  BufferGeometry,
  Color,
  DirectionalLight,
  DoubleSide,
  Float32BufferAttribute,
  Group,
  HemisphereLight,
  LineBasicMaterial,
  LineSegments,
  Matrix4,
  Mesh,
  MeshStandardMaterial,
  PerspectiveCamera,
  Raycaster,
  SRGBColorSpace,
  Scene,
  Vector2,
  Vector3,
  Box3,
  WebGLRenderer,
} from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";

// --- payload types (exedra-ops-inspect-v1) ---

type MeshBuffers = {
  indices: number[];
  positions: number[];
  uvs: number[];
  normals: number[];
  topology_lines: number[];
  region_ids: number[];
};

type InspectionFeature = {
  kind: string;
  loop_index: number | null;
  seg: number | null;
  band: number | null;
  operand: number | null;
  row: number | null;
  col: number | null;
};

type InspectionFace = {
  feature: InspectionFeature;
  region: number;
};

type InspectionBody = {
  part: string | null;
  node: number;
  mesh: MeshBuffers;
  faces: InspectionFace[];
  tri_face: number[];
};

type InspectionNode = {
  part: string | null;
  id: number;
  kind: string;
  fingerprint: string;
  source: string | null;
  material: string | null;
  issue: string | null;
};

type InspectionInstance = {
  path: string;
  body: number;
  matrix: number[];
};

type InspectionFidelity = {
  part: string | null;
  node: number;
  verdict: string;
  policy: string | null;
  issue: string | null;
};

type InspectionPolicyUse = {
  part: string | null;
  node: number;
  policy: string;
};

type InspectionDiagnostic = {
  part: string | null;
  severity: string;
  code: string;
  message: string;
  node: number | null;
};

type InspectionCounters = {
  bodies: number;
  tessellations: number;
  faces: number;
  vertices: number;
  envelope_only: number;
  unimplemented: number;
  source_map_bytes: number;
};

type InspectionResponse = {
  format: string;
  scenario: string;
  nodes: InspectionNode[];
  bodies: InspectionBody[];
  instances: InspectionInstance[];
  fidelity: InspectionFidelity[];
  policy_curves: InspectionPolicyUse[];
  diagnostics: InspectionDiagnostic[];
  counters: InspectionCounters;
};

type WasmApi = {
  list_inspection_scenarios_json: () => string;
  run_inspection_scenario_json: (name: string) => string;
};

// --- DOM ---

const scenarioSelect = document.getElementById("scenario") as HTMLSelectElement;
const reloadButton = document.getElementById("reload") as HTMLButtonElement;
const wireframeToggle = document.getElementById("wireframe") as HTMLInputElement;
const edgesToggle = document.getElementById("edges") as HTMLInputElement;
const formatLabel = document.getElementById("format") as HTMLDivElement;
const summaryTable = document.getElementById("summary") as HTMLTableElement;
const selectionDiv = document.getElementById("selection") as HTMLDivElement;
const fidelityDiv = document.getElementById("fidelity") as HTMLDivElement;
const diagnosticsDiv = document.getElementById("diagnostics") as HTMLDivElement;
const viewport = document.getElementById("viewport") as HTMLDivElement;

// --- scene ---

const scene = new Scene();
scene.background = new Color(0x0b0e13);

const camera = new PerspectiveCamera(50, 1, 0.01, 100000);
camera.position.set(0, 0, 4);

const renderer = new WebGLRenderer({ antialias: true });
renderer.outputColorSpace = SRGBColorSpace;
viewport.appendChild(renderer.domElement);

const controls = new OrbitControls(camera, renderer.domElement);
controls.enableDamping = true;

scene.add(new HemisphereLight(0x8fb4d8, 0x151a22, 0.9));
const keyLight = new DirectionalLight(0xdfe8f2, 1.1);
keyLight.position.set(1.4, 1.8, 2.2);
scene.add(keyLight);

const surfaceMaterial = new MeshStandardMaterial({
  vertexColors: true,
  roughness: 0.85,
  metalness: 0.05,
  side: DoubleSide,
  wireframe: false,
});

// --- state ---

type PickTarget = {
  mesh: Mesh<BufferGeometry, MeshStandardMaterial>;
  bodyIndex: number;
  instanceIndex: number;
  baseColors: Float32Array;
};

let currentResponse: InspectionResponse | null = null;
let currentRoot: Group | null = null;
let pickTargets: PickTarget[] = [];
let selected: { target: PickTarget; faceOrdinal: number } | null = null;

const ACCENT = new Color(0x4fd1c5);

/// Muted instrument palette: region 0 is neutral, others spread by
/// golden-angle hue at low saturation so structure reads without glare.
function regionColor(region: number): Color {
  if (region === 0) {
    return new Color().setHSL(0.58, 0.08, 0.42);
  }
  const hue = (region * 0.618033988749895) % 1.0;
  return new Color().setHSL(hue, 0.35, 0.5);
}

function clearModel(): void {
  if (currentRoot) {
    scene.remove(currentRoot);
    currentRoot.traverse((obj) => {
      if (obj instanceof Mesh || obj instanceof LineSegments) {
        obj.geometry.dispose();
      }
    });
    currentRoot = null;
  }
  pickTargets = [];
  selected = null;
}

/// Builds a non-indexed geometry (per-triangle colors need unshared
/// vertices) for one body; triangle order matches the payload's
/// tri_face/region_ids order, so raycast faceIndex indexes them directly.
function bodyGeometry(body: InspectionBody): { geometry: BufferGeometry; colors: Float32Array } {
  const tri = body.mesh.indices.length / 3;
  const positions = new Float32Array(tri * 9);
  const colors = new Float32Array(tri * 9);
  for (let t = 0; t < tri; t += 1) {
    const color = regionColor(body.mesh.region_ids[t] ?? 0);
    for (let v = 0; v < 3; v += 1) {
      const src = body.mesh.indices[t * 3 + v] * 3;
      const dst = (t * 3 + v) * 3;
      positions[dst] = body.mesh.positions[src];
      positions[dst + 1] = body.mesh.positions[src + 1];
      positions[dst + 2] = body.mesh.positions[src + 2];
      colors[dst] = color.r;
      colors[dst + 1] = color.g;
      colors[dst + 2] = color.b;
    }
  }
  const geometry = new BufferGeometry();
  geometry.setAttribute("position", new Float32BufferAttribute(positions, 3));
  geometry.setAttribute("color", new Float32BufferAttribute(colors.slice(), 3));
  geometry.computeVertexNormals();
  geometry.computeBoundingBox();
  geometry.computeBoundingSphere();
  return { geometry, colors };
}

function buildScene(response: InspectionResponse): void {
  clearModel();
  const root = new Group();
  response.instances.forEach((instance, instanceIndex) => {
    const body = response.bodies[instance.body];
    if (!body) {
      return;
    }
    const { geometry, colors } = bodyGeometry(body);
    const mesh = new Mesh(geometry, surfaceMaterial);
    const matrix = new Matrix4().fromArray(instance.matrix);
    mesh.applyMatrix4(matrix);
    root.add(mesh);
    pickTargets.push({ mesh, bodyIndex: instance.body, instanceIndex, baseColors: colors });

    if (body.mesh.topology_lines.length > 0) {
      const lineGeometry = new BufferGeometry();
      lineGeometry.setAttribute(
        "position",
        new Float32BufferAttribute(body.mesh.topology_lines, 3),
      );
      const lines = new LineSegments(
        lineGeometry,
        new LineBasicMaterial({
          color: 0x33465c,
          transparent: true,
          opacity: 0.8,
          depthTest: true,
        }),
      );
      lines.applyMatrix4(matrix);
      lines.visible = edgesToggle.checked;
      lines.userData.isEdges = true;
      root.add(lines);
    }
  });
  currentRoot = root;
  scene.add(root);
  fitCamera(root);
}

function fitCamera(root: Group): void {
  const box = new Box3().setFromObject(root);
  const center = box.getCenter(new Vector3());
  const size = box.getSize(new Vector3());
  const radius = Math.max(size.x, size.y, size.z, 1e-6) * 0.6;
  camera.position.set(center.x + radius * 1.7, center.y + radius * 1.4, center.z + radius * 2.1);
  camera.near = radius / 1000;
  camera.far = radius * 100;
  camera.updateProjectionMatrix();
  controls.target.copy(center);
  controls.update();
}

// --- panels ---

function setSummary(response: InspectionResponse): void {
  const triangles = response.bodies.reduce((n, b) => n + b.tri_face.length, 0);
  const rows: [string, string][] = [
    ["scenario", response.scenario],
    ["bodies", String(response.bodies.length)],
    ["instances", String(response.instances.length)],
    ["nodes", String(response.nodes.length)],
    ["faces", String(response.counters.faces)],
    ["triangles", String(triangles)],
    ["vertices", String(response.counters.vertices)],
    ["tessellations", String(response.counters.tessellations)],
    ["envelope-only", String(response.counters.envelope_only)],
    ["source-map bytes", String(response.counters.source_map_bytes)],
  ];
  summaryTable.replaceChildren(
    ...rows.map(([key, value]) => {
      const tr = document.createElement("tr");
      const k = document.createElement("td");
      k.textContent = key;
      const v = document.createElement("td");
      v.textContent = value;
      tr.append(k, v);
      return tr;
    }),
  );
}

function setFidelity(response: InspectionResponse): void {
  if (response.fidelity.length === 0) {
    fidelityDiv.textContent = "—";
    return;
  }
  fidelityDiv.replaceChildren(
    ...response.fidelity.map((entry) => {
      const line = document.createElement("div");
      const scope = entry.part ? `${entry.part}#${entry.node}` : `#${entry.node}`;
      let text = `${scope} ${entry.verdict}`;
      if (entry.policy) {
        text += ` policy=${entry.policy}`;
      }
      if (entry.issue) {
        text += ` issue=${entry.issue}`;
      }
      line.textContent = text;
      if (entry.verdict === "conflicted" || entry.verdict === "envelope_only") {
        line.className = "warning";
      }
      return line;
    }),
  );
}

/// Mirrors the GeometryReport ledger exactly: emission order, severity,
/// code, message, and the concerned node.
function setDiagnostics(response: InspectionResponse): void {
  if (response.diagnostics.length === 0) {
    const empty = document.createElement("div");
    empty.className = "empty";
    empty.textContent = "no diagnostics";
    diagnosticsDiv.replaceChildren(empty);
    return;
  }
  diagnosticsDiv.replaceChildren(
    ...response.diagnostics.map((diag) => {
      const line = document.createElement("div");
      const scope = diag.node === null ? "" : ` [node ${diag.node}]`;
      const part = diag.part ? ` (${diag.part})` : "";
      line.textContent = `${diag.severity}:${diag.code}: ${diag.message}${scope}${part}`;
      if (diag.severity === "warning") {
        line.className = "warning";
      } else if (diag.severity === "error") {
        line.className = "error";
      }
      return line;
    }),
  );
}

function chainRow(key: string, value: string, accent = false): HTMLDivElement {
  const row = document.createElement("div");
  row.className = "row";
  const k = document.createElement("div");
  k.className = "key";
  k.textContent = key;
  const v = document.createElement("div");
  v.className = accent ? "value accent" : "value";
  v.textContent = value;
  row.append(k, v);
  return row;
}

function featureLabel(feature: InspectionFeature): string {
  const parts: string[] = [feature.kind];
  if (feature.loop_index !== null) {
    parts.push(`loop=${feature.loop_index}`);
  }
  if (feature.seg !== null) {
    parts.push(`seg=${feature.seg}`);
  }
  if (feature.band !== null) {
    parts.push(`band=${feature.band}`);
  }
  if (feature.operand !== null) {
    parts.push(`operand=${feature.operand}`);
  }
  if (feature.row !== null) {
    parts.push(`row=${feature.row}`);
  }
  if (feature.col !== null) {
    parts.push(`col=${feature.col}`);
  }
  return parts.join(" ");
}

function setSelection(target: PickTarget, faceOrdinal: number): void {
  const response = currentResponse;
  if (!response) {
    return;
  }
  const body = response.bodies[target.bodyIndex];
  const instance = response.instances[target.instanceIndex];
  const face = body.faces[faceOrdinal];
  const node = response.nodes.find((n) => n.part === body.part && n.id === body.node);
  const fidelity = response.fidelity.find((f) => f.part === body.part && f.node === body.node);

  const chain = document.createElement("div");
  chain.className = "chain";
  chain.append(chainRow("instance", instance.path));
  if (body.part !== null) {
    chain.append(chainRow("part", body.part));
  }
  chain.append(
    chainRow("node", node ? `#${node.id} ${node.kind}` : `#${body.node}`, true),
    chainRow("feature", featureLabel(face.feature), true),
    chainRow("region", String(face.region)),
    chainRow("face", `${faceOrdinal} / ${body.faces.length}`),
  );
  if (node?.source) {
    chain.append(chainRow("source", node.source));
  }
  if (node?.material) {
    chain.append(chainRow("material", node.material));
  }
  if (node?.issue) {
    chain.append(chainRow("issue", node.issue));
  }
  if (fidelity) {
    let verdict = fidelity.verdict;
    if (fidelity.policy) {
      verdict += ` policy=${fidelity.policy}`;
    }
    if (fidelity.issue) {
      verdict += ` issue=${fidelity.issue}`;
    }
    chain.append(chainRow("fidelity", verdict));
  }
  if (node?.fingerprint) {
    chain.append(chainRow("fingerprint", node.fingerprint));
  }
  selectionDiv.replaceChildren(chain);
}

function clearSelectionPanel(): void {
  const hint = document.createElement("div");
  hint.className = "hint";
  hint.textContent = "click a face in the viewport";
  selectionDiv.replaceChildren(hint);
}

// --- picking ---

const raycaster = new Raycaster();
const pointer = new Vector2();
let pointerDown: { x: number; y: number } | null = null;

/// Repaints one target's colors: base regions everywhere, the selected
/// face's triangles in the accent color.
function repaint(target: PickTarget, faceOrdinal: number | null): void {
  const attribute = target.mesh.geometry.getAttribute("color") as Float32BufferAttribute;
  const array = attribute.array as Float32Array;
  array.set(target.baseColors);
  if (faceOrdinal !== null && currentResponse) {
    const body = currentResponse.bodies[target.bodyIndex];
    for (let t = 0; t < body.tri_face.length; t += 1) {
      if (body.tri_face[t] !== faceOrdinal) {
        continue;
      }
      for (let v = 0; v < 3; v += 1) {
        const dst = (t * 3 + v) * 3;
        array[dst] = ACCENT.r;
        array[dst + 1] = ACCENT.g;
        array[dst + 2] = ACCENT.b;
      }
    }
  }
  attribute.needsUpdate = true;
}

function pick(event: MouseEvent): void {
  if (!currentResponse) {
    return;
  }
  // Ignore clicks that were camera drags.
  if (
    pointerDown &&
    Math.hypot(event.clientX - pointerDown.x, event.clientY - pointerDown.y) > 4
  ) {
    return;
  }
  const rect = renderer.domElement.getBoundingClientRect();
  pointer.x = ((event.clientX - rect.left) / rect.width) * 2 - 1;
  pointer.y = -((event.clientY - rect.top) / rect.height) * 2 + 1;
  raycaster.setFromCamera(pointer, camera);
  const hits = raycaster.intersectObjects(
    pickTargets.map((t) => t.mesh),
    false,
  );
  const hit = hits[0];
  if (!hit || hit.faceIndex === undefined || hit.faceIndex === null) {
    return;
  }
  const target = pickTargets.find((t) => t.mesh === hit.object);
  if (!target) {
    return;
  }
  const body = currentResponse.bodies[target.bodyIndex];
  const faceOrdinal = body.tri_face[hit.faceIndex];
  if (faceOrdinal === undefined) {
    return;
  }
  if (selected) {
    repaint(selected.target, null);
  }
  selected = { target, faceOrdinal };
  repaint(target, faceOrdinal);
  setSelection(target, faceOrdinal);
}

// --- bootstrap ---

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

async function loadWasmApi(): Promise<WasmApi> {
  const module = await import("./wasm_pkg/exedra_ops_web_bridge");
  await module.default();
  return module as unknown as WasmApi;
}

async function bootstrap(): Promise<void> {
  const wasm = await loadWasmApi();
  const scenarios = JSON.parse(wasm.list_inspection_scenarios_json()) as string[];
  for (const name of scenarios) {
    const option = document.createElement("option");
    option.value = name;
    option.textContent = name;
    scenarioSelect.appendChild(option);
  }

  const runScenario = (): void => {
    const selectedName = scenarioSelect.value;
    try {
      const response = JSON.parse(
        wasm.run_inspection_scenario_json(selectedName),
      ) as InspectionResponse;
      currentResponse = response;
      formatLabel.textContent = response.format;
      buildScene(response);
      setSummary(response);
      setFidelity(response);
      setDiagnostics(response);
      clearSelectionPanel();
    } catch (error) {
      currentResponse = null;
      clearModel();
      formatLabel.textContent = "—";
      const failure = document.createElement("div");
      failure.className = "error";
      failure.textContent = `scenario failed: ${String(error)}`;
      diagnosticsDiv.replaceChildren(failure);
    }
  };

  scenarioSelect.addEventListener("change", runScenario);
  reloadButton.addEventListener("click", runScenario);
  wireframeToggle.addEventListener("change", () => {
    surfaceMaterial.wireframe = wireframeToggle.checked;
  });
  edgesToggle.addEventListener("change", () => {
    currentRoot?.traverse((obj) => {
      if (obj.userData.isEdges) {
        obj.visible = edgesToggle.checked;
      }
    });
  });
  renderer.domElement.addEventListener("pointerdown", (event) => {
    pointerDown = { x: event.clientX, y: event.clientY };
  });
  renderer.domElement.addEventListener("click", pick);
  window.addEventListener("resize", resizeRenderer);
  resizeRenderer();
  runScenario();
  animate();
}

bootstrap().catch((error) => {
  const failure = document.createElement("div");
  failure.className = "error";
  failure.textContent = `failed to initialize inspector:\n${String(error)}`;
  diagnosticsDiv.replaceChildren(failure);
});
