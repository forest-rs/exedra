# Byzantine Basilica Ruin Pipeline

*(A concrete end-to-end example tying Exedra + Cambium together)*

This document is a **worked example** that makes Exedra and Cambium concrete. It describes a complete procedural pipeline that generates a Byzantine-flavored basilica ruin, including:

* the operator sequence (a “Cambium Program”)
* what each operator reads/writes
* what artifacts are emitted
* how `Txn → ChangeSet → DirtySet` drives incremental extraction
* where `understory_dirty` channels apply (Cambium caches)
* how an LLM can generate or vary the program safely

This is not a spec for every operator listed here; it is an **example pipeline** that we can use as a guiding demo and a future wind tunnel scenario.

---

## Design goals for this demo

1. **Visually legible quickly**: walls + arcade openings + apse + dome read as “Byzantine.”
2. **Deterministic**: same inputs → same output topology/attributes/extraction ordering.
3. **Incremental-friendly**: small parameter tweaks should support preview/commit without full rebuild.
4. **Debuggable**: every step produces reports and optional bounded artifacts.
5. **Minimal dependencies**: early versions avoid booleans; later steps can swap to boolean cutters.

---

## Operator palette (used by this pipeline)

### Shape / massing

* `shape.floor_plan.basilica` *(pure operator)*
* `shape.extrude.walls` *(edit operator)*
* `shape.add.drum` *(edit operator)*
* `shape.add.dome` *(edit operator)*

### Openings / articulation

* `detail.openings.arcade` *(edit operator; boolean-free early, boolean-cutter later)*

### Shading and UV

* `shade.sharpness.from_angle` *(edit operator; writes edge sharpness)*
* `uv.box` or `uv.planar` *(edit operator; writes corner UV layer)*

### Ruinization

* `ruin.damage.mask` *(pure operator producing a selection artifact; may optionally tag a face weight layer later)*
* `ruin.damage.delete_faces` *(edit operator)*
* `ruin.damage.chip_edges` *(edit operator; budgetable; optional v0.5+)*
* `ruin.growth.emit_vines` *(pure operator producing polylines/points artifacts; optional)*

---

## Canonical inputs and outputs

### Inputs

* `seed: u64` — top-level seed controlling any randomized decisions.
* `NumericPolicy` — explicit tolerances.
* `PolicySet` — Cambium policies (preview/commit, budgets, artifact limits, validation).

### Outputs

* `exedra::Mesh` — canonical modeling mesh.
* Renderable extraction (`TriMesh`) is produced by Exedra extraction using `ChangeSet.dirty` (incremental mode) or full rebuild.

---

## Region tags and selections

This demo uses **semantic regions** as face-domain tags (Exedra) and canonical face selections (Cambium).

### Face-domain region tag

A minimal region tag is a `u32` or small enum stored in a face-domain attribute layer (built-in or demo-local). Example region IDs:

* `REGION_FOOTPRINT`
* `REGION_WALL_OUTER`
* `REGION_WALL_INNER`
* `REGION_NAVE`
* `REGION_AISLE_L`
* `REGION_AISLE_R`
* `REGION_APSE`
* `REGION_DRUM`
* `REGION_DOME`

### Canonical selection representation (Cambium)

Selections passed between operators are canonical `Vec<FaceId>`:

* sorted by stable id order
* deduplicated

Selections may also be emitted as artifacts (FaceSet).

---

## The Cambium Program (baseline)

This is the baseline “script” the demo runs. Each operator produces an `OpReport`, and commit-mode operators produce an Exedra `ChangeSet`.

1. `shape.floor_plan.basilica`
2. `shape.extrude.walls`
3. `detail.openings.arcade`
4. `shape.add.drum`
5. `shape.add.dome`
6. `shade.sharpness.from_angle`
7. `uv.box` (or `uv.planar`)
8. `ruin.damage.mask`
9. `ruin.damage.delete_faces`
10. `ruin.damage.chip_edges` *(optional)*
11. `ruin.growth.emit_vines` *(optional; artifacts only)*

A typical interactive workflow uses preview/commit around steps 8–10.

---

## Step-by-step: what each operator does

### 1) `shape.floor_plan.basilica` (pure)

**Purpose**: Construct a 2D footprint mesh for a basilica-like plan.

**Recommended v0.1 posture**: output a single outer footprint face plus guide artifacts (fast), rather than fully partitioned interior regions.

**Params (example)**

* `length: f32`
* `width: f32`
* `nave_width: f32`
* `apse_radius: f32`
* `apse_segments: u32`
* `n_bays: u32` *(for arcade placement guides)*

**Reads**: none.

**Writes**:

* Constructs a new `exedra::Mesh` with:

  * vertex positions (`exedra::attr::VERTEX_POSITION`)
  * one face tagged `REGION_FOOTPRINT`

**Artifacts (optional, bounded)**:

* `Polyline2("guide.centerline")` — basilica axis
* `Polyline2("guide.aisle_bounds")` — aisle boundary lines
* `Polyline2("guide.bays")` — bay division marks along walls (optional)
* `FaceSet("region.footprint")` — just the footprint face

**Report counters**:

* `faces_processed = 1`
* `corners_written = 0`

**Determinism**:

* Outer polygon vertex order is fixed (e.g., clockwise starting at entrance-left corner).
* Apse points are generated in a fixed order along the semicircle.

---

### 2) `shape.extrude.walls` (edit)

**Purpose**: Turn the 2D footprint into a 3D wall shell.

**Params (example)**

* `face_set: FaceSet = region.footprint`
* `height: f32`
* `thickness: f32`
* `cap_style: enum { None, Flat }`

**Reads**:

* footprint face loop
* vertex positions

**Writes** (via `Txn`):

* new faces/half-edges/vertices forming walls
* face region tags for:

  * outer wall faces
  * inner wall faces
  * top rim (optional)

**Expected ChangeSet / DirtySet**:

* `created_faces/half_edges/vertices` populated
* `dirty_faces`: all newly created faces + affected footprint face
* `dirty_vertices`: vertices in the footprint and wall shell
* `dirty_corners`: conservative (all corners of new faces)

**Artifacts (optional)**:

* `FaceSet("region.wall_outer")`
* `FaceSet("region.wall_inner")`
* `EdgeSet("loop.wall_top")` (top rim loop)

**Report counters**:

* `faces_processed = #faces extruded` (deterministic)

---

### 3) `detail.openings.arcade` (edit)

**Purpose**: Add a repeated arcade opening pattern along selected wall faces.

**Params (example)**

* `wall_faces: FaceSet = region.wall_outer` (or a subset)
* `n_openings: u32`
* `opening_width: f32`
* `opening_height: f32`
* `arch_type: enum { Rect, Semi }` *(v0.1 may use Rect; later swap to boolean cutters)*
* `sill_height: f32`
* `spacing: f32`

**Reads**:

* wall face geometry and vertex positions
* bay guide artifact (optional) or computed spacing

**Writes**:

* split faces along opening boundaries
* delete interior panels to create holes (rectangular in v0.1)
* tag opening boundary edges as sharp (or via a later sharpness pass)

**Expected ChangeSet / DirtySet**:

* `dirty_faces`: affected wall faces (split) and adjacent faces
* `dirty_corners`: corners of affected faces

**Artifacts (optional)**:

* `FaceSet("openings.arcade.panels_deleted")`
* `Polyline3("openings.arcade.frames")` (guide outlines)

**Determinism**:

* opening placement order is stable (e.g., from entrance → apse).

---

### 4) `shape.add.drum` (edit)

**Purpose**: Add an octagonal/12-sided drum at a specified location.

**Params (example)**

* `center: [f32; 3]`
* `radius: f32`
* `sides: u32` (8/12/16)
* `height: f32`
* `thickness: f32`

**Reads**:

* current mesh bounds / guides (optional)

**Writes**:

* new faces for drum walls and rim
* tags `REGION_DRUM`

**Artifacts**:

* `EdgeSet("loop.drum_top")` — used by dome step

---

### 5) `shape.add.dome` (edit)

**Purpose**: Add a dome cap over the drum top loop.

**Params (example)**

* `base_loop: EdgeSet = loop.drum_top`
* `profile: enum { Hemisphere, Segment }`
* `segments_u: u32`
* `segments_v: u32`

**Reads**:

* base loop positions

**Writes**:

* new faces for dome surface
* tags `REGION_DOME`

**Determinism**:

* loop traversal uses deterministic order (start half-edge chosen deterministically).

---

### 6) `shade.sharpness.from_angle` (edit)

**Purpose**: Tag sharp edges based on dihedral angle.

**Params (example)**

* `angle_deg: f32`
* `scope: enum { WholeMesh, FaceSet }`

**Reads**:

* adjacency around edges
* face normals (geometric; deterministic)

**Writes**:

* edge sharpness flags/weights (edge-domain attribute)

**DirtySet**:

* may mark vertices/corners dirty for derived normals in extraction.

---

### 7) UV projection (`uv.box` or `uv.planar`) (edit)

**Purpose**: Populate corner UVs for texturing stone/brick/mosaic.

**Reads**:

* vertex positions

**Writes**:

* corner UV layer (`exedra::attr::CORNER_UV`)

**DirtySet**:

* `dirty_faces` and `dirty_corners` for UV-updated faces/corners.

---

### 8) `ruin.damage.mask` (pure)

**Purpose**: Produce a deterministic face selection representing “damage potential.”

**Params (example)**

* `seed: u64`
* `rate: f32` (0..1)
* `bias_by_height: bool`
* `noise_scale: f32`

**Reads**:

* face geometry (positions)
* region tags (prefer walls/dome)

**Writes**: none.

**Artifacts**:

* `FaceSet("ruin.damage.mask")` (canonical sorted ids)
* optional `FieldF32("ruin.damage.weight")` (later; out of v0.1)

**Determinism**:

* selection is stable given `(seed, mesh, params)`.

---

### 9) `ruin.damage.delete_faces` (edit)

**Purpose**: Delete faces from a selection to create holes/collapses.

**Params (example)**

* `face_set: FaceSet = ruin.damage.mask`
* `mode: enum { Holes, Chunks }`
* `min_component_size: u32`

**Reads**:

* selection face list

**Writes**:

* deletes faces and any now-degenerate topology (conservative; correctness over minimality)

**DirtySet**:

* affected neighborhood faces/vertices/corners

---

### 10) `ruin.damage.chip_edges` (edit; optional)

**Purpose**: Add small “chipping” along exposed edges.

**Params (example)**

* `seed: u64`
* `iterations: u32`
* `chip_size: f32`
* `edge_scope: enum { BoundaryEdges, FaceSetBoundary }`

**Budgeting**:

* in preview, obey `max_faces/max_corners` deterministically.

---

### 11) `ruin.growth.emit_vines` (pure; optional)

**Purpose**: Emit artifact polylines/points that can be used later for instancing vines/moss.

**Params (example)**

* `seed: u64`
* `count: u32`
* `prefer_edges: bool`

**Artifacts**:

* `Polyline3("growth.vines")`
* `FieldF32("growth.density")` (optional later)

---

## Incremental extraction: how it fits

In an interactive session:

* Commit-mode steps return a `ChangeSet`.
* The renderer/extractor calls Exedra extraction with:

  * `ExtractMode::Incremental`
  * `dirty = change_set.dirty`

This allows:

* re-triangulating only affected faces
* recomputing derived normals only where needed
* re-splitting render vertices only where corner data changed

Preview-mode steps run against a cloned mesh; the same ChangeSet/DirtySet logic applies to the preview mesh.

---

## `understory_dirty`: where it applies

Cambium uses `understory_dirty` for **operator-runtime caches** and workflow state.

For this demo, typical mappings:

* After UV ops: mark `DirtyChan::UvDerived`
* After topology edits (extrude/openings/dome): mark `DirtyChan::Adjacency`
* After selection changes: mark `DirtyChan::Selection`

This is intentionally coarse and conservative.

---

## LLM-based content generation: safe integration

LLMs should generate **programs and parameters**, not raw mesh data.

### Cambium Program JSON

Example format:

```json
{
  "seed": 1234,
  "ops": [
    {"op": "shape.floor_plan.basilica", "params": {"length": 34.0, "width": 18.0, "nave_width": 9.0, "apse_radius": 6.0, "apse_segments": 24, "n_bays": 6}},
    {"op": "shape.extrude.walls", "params": {"height": 10.0, "thickness": 0.6}},
    {"op": "detail.openings.arcade", "params": {"n_openings": 8, "opening_width": 2.2, "opening_height": 4.5, "arch_type": "Rect", "sill_height": 1.2, "spacing": 1.0}},
    {"op": "shape.add.drum", "params": {"radius": 4.0, "sides": 12, "height": 3.0, "thickness": 0.5}},
    {"op": "shape.add.dome", "params": {"profile": "Segment", "segments_u": 24, "segments_v": 12}},
    {"op": "shade.sharpness.from_angle", "params": {"angle_deg": 40.0}},
    {"op": "uv.box", "params": {"scale": 1.0}},
    {"op": "ruin.damage.mask", "params": {"rate": 0.18, "bias_by_height": true, "noise_scale": 3.0}},
    {"op": "ruin.damage.delete_faces", "params": {"mode": "Chunks", "min_component_size": 12}}
  ]
}
```

### Validation rules (must be enforced)

* Only allow known operator names.
* Validate parameter ranges.
* Require explicit seeds.
* Canonicalize selections deterministically.
* Reject programs that exceed configured limits (op count, artifact limits).

### Style prompts → program compilation

A useful pattern:

1. LLM produces a *style intent* ("collapsed dome, heavy ivy, many arcades")
2. A deterministic compiler maps intent → parameter ranges and operator variants
3. The resulting validated program is executed by Cambium

---

## Future swaps and upgrades (without rewriting the pipeline)

* Replace `detail.openings.arcade` rect cutouts with boolean cutters once Exedra booleans mature.
* Replace dome generation with subdivision-based smooth dome in v0.5.
* Introduce Fields as inputs to ruin mask and material variation.
* Introduce instancing outputs (columns, rubble, ivy) as artifacts that a higher scene layer consumes.

---

## Expected outputs (debug dumps and inspection)

This appendix defines *what we expect to be able to inspect or dump* at each stage of the pipeline. The intent is to make debugging, golden tests, and LLM-driven generation reproducible and concrete.

### Conventions

* **Mesh snapshots** are always Exedra modeling meshes (not TriMesh).
* **Selections** are canonical sorted/dedup `Vec<FaceId>` (or the equivalent artifact form).
* **Artifacts** are bounded and deterministically ordered.
* **Timings** are never included in goldens; only counters and deterministic stats are.

### Minimal dump set (v0.1)

For any pipeline run, the minimum useful outputs are:

1. **Program input**

   * the Cambium Program JSON (or equivalent structured config)
   * seed(s)
   * `NumericPolicy` and `PolicySet` used

2. **Per-step reports**

   * `OpReport.name`
   * `OpReport.stats` (counters and element counts)
   * diagnostic list (bounded)

3. **Final mesh snapshot**

   * modeling mesh topology + attributes sufficient to reproduce extraction

4. **Final extraction snapshot**

   * `TriMesh` buffers: indices, positions, uvs (and later normals)
   * `ExtractStats` counters (not timing)

### Recommended per-step artifacts

The following artifacts are recommended outputs to make the pipeline inspectable.

#### Step 1: `shape.floor_plan.basilica`

* `Mesh("step01.plan")` — footprint mesh
* `Polyline2("guide.centerline")`
* `Polyline2("guide.aisle_bounds")`
* `Polyline2("guide.bays")` (if used)
* `FaceSet("region.footprint")`

#### Step 2: `shape.extrude.walls`

* `Mesh("step02.walls")` (optional; or rely on final mesh snapshot)
* `FaceSet("region.wall_outer")`
* `FaceSet("region.wall_inner")`
* `EdgeSet("loop.wall_top")` (if produced)

#### Step 3: `detail.openings.arcade`

* `FaceSet("openings.arcade.affected")`
* `FaceSet("openings.arcade.deleted_panels")` (when applicable)
* `Polyline3("openings.arcade.frames")` (optional; frame outlines)

#### Step 4: `shape.add.drum`

* `EdgeSet("loop.drum_top")`
* `FaceSet("region.drum")` (optional)

#### Step 5: `shape.add.dome`

* `FaceSet("region.dome")` (optional)

#### Step 6: `shade.sharpness.from_angle`

* `EdgeSet("sharp_edges")` (optional; useful for debugging)

#### Step 7: `uv.box` / `uv.planar`

* `FaceSet("uv.affected_faces")` (optional)
* `Polyline2("uv.bounds")` (optional; one per scope)

#### Step 8: `ruin.damage.mask`

* `FaceSet("ruin.damage.mask")` (required)

#### Step 9: `ruin.damage.delete_faces`

* `FaceSet("ruin.damage.deleted_faces")` (optional; if tracked)
* `FaceSet("ruin.damage.boundary_faces")` (optional; neighborhood)

#### Step 10: `ruin.damage.chip_edges` (optional)

* `EdgeSet("ruin.chip_edges.scope")` (optional)
* `FaceSet("ruin.chip_edges.affected")` (optional)

#### Step 11: `ruin.growth.emit_vines` (optional)

* `Polyline3("growth.vines")` (optional)
* `FieldF32("growth.density")` (optional; later)

### Golden test posture

For deterministic goldens, prefer:

* per-step **Stats counters**
* canonical selections (FaceSets) as sorted lists
* final TriMesh buffers (indices/positions/uvs)

Avoid storing entire intermediate meshes as goldens early unless necessary; meshes are valuable for debugging but are larger and can change with benign topology layout changes.

---

## Notes

This pipeline is intentionally designed so that:

* Exedra stays calm (topology + attributes + deterministic extraction).
* Cambium owns meaning (basilica, dome, ruinization).
* The LLM is “boxed in” to generating **validated programs**, not arbitrary geometry.

Use this document as:

* a first demo target
* a sanity check for API design
* a future wind tunnel scenario definition
