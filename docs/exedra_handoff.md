# Exedra + Cambium Handover Spec

## Context

We are building a production-capable geometry kernel and operator stack for the Lightweald ecosystem.

* **Exedra**: the *structural* mesh kernel (topology + attributes + deterministic extraction + debuggable B-rep booleans).
* **Cambium**: the *growth/operator* layer on top of Exedra (subdivision, procedural modeling ops, refinement, etc.).

This document defines goals, non-goals, core data structures, invariants, APIs, and a milestone plan from **v0.1 → v0.5 → v0.9 → v1.0**.

---

## Engineering Constraints

### Forest Engineering Tenets (non-negotiable)

1. **We Build to Endure.** Systems difficult to outgrow/entangle; easy to reason about; easy to measure.
2. **Modularity Is Power.** Narrow responsibilities, minimal dependency surface, replaceable internals.
3. **Incrementalism Everywhere.** Deltas over rewrites, caches over recomputation, budgeted work.
4. **Introspection Is Non-Optional.** Time, memory, work units, bandwidth; diagnostics are architecture.
5. **Explicit Over Implicit.** No hidden state; predictable performance.
6. **Long-Term > Short-Term.** Architectural leverage over demo velocity.
7. **Replaceability Is a Constraint.** Different backends/allocators/platforms tolerated.
8. **Calm Interfaces.** Public APIs are boring, stable, intentional.
9. **No Sacred Subsystems.** Evolve forward; remove complexity when possible.

### Rust / crate policy

* **MSRV**: Rust **1.92** (target stable toolchain **1.93**).
* Prefer **`#![no_std]` + `alloc`** where possible.
* Avoid allocations in hot loops; preallocate and reuse scratch buffers.
* When hash tables are required, use **hashbrown**.
* Crate names use **underscores**.
* Performance/benchmark crates are called **“wind tunnels”**, e.g. `exedra_wind_tunnel`.

---

## Repo / Workspace Layout

We expect multiple crates. Use a Cargo workspace.

Recommended workspace (repo name can be `exedra`):

* `crates/exedra/` — kernel
* `crates/cambium/` — ops layer (depends on `exedra`)
* `crates/exedra_testkit/` — fixtures, generators, golden outputs, debug dumps (can use `std`)
* `crates/exedra_wind_tunnel/` — benchmarks and perf regression harness (uses `std`)

Optional later:

* `crates/exedra_io/` — OBJ/gltf import/export helpers (likely `std`)

Policy: keep `std` dependencies out of `crates/exedra` unless unavoidable.

---

## What Exedra Is

Exedra is a **production** pointerless mesh kernel:

* Canonical mesh is **polygonal half-edge** (n-gons).
* Uses **stable IDs** (index + generation) for safety and caching.
* Has a real **attribute model** including **corner (face-vertex) attributes**.
* Provides deterministic **triangulation** and **render extraction** (splitting vertices where attributes differ).
* Provides a debuggable **B-rep boolean pipeline** (union/intersect/difference) with strong diagnostics.

Exedra is *not* a toy data structure; it is a foundation layer.

---

## What Exedra Is Not (Non-goals)

For v1.0:

* No SDF/implicit modeling.
* No *full* UV unwrapping pipeline (charting + packing) in Exedra.

  * We **do** expect basic UV coordinate generation utilities to exist in the ecosystem (see Cambium notes).
* No full mesh repair suite (basic weld/snap helpers are ok).
* No subdivision algorithms in Exedra (Cambium owns them).
* No promise of CAD-grade exact arithmetic everywhere; instead:

  * explicit tolerance policy
  * deterministic behavior
  * strong diagnostics and failure artifacts

---

## Conceptual Model

### Topology vs shading

Topology (shared vertices) is distinct from shading/parameterization (often discontinuous).

Key concept:

* **CornerId == HalfEdgeId**
* UVs, custom normals, tangents are **corner-domain attributes**.

This enables:

* UV seams without topological splits
* Hard/soft shading without topological splits
* Controlled bevel appearance via custom corner normals

Render extraction may duplicate (“split”) *render vertices* as required; topology remains stable.

---

## Core Data Structures

### IDs

Public IDs are stable handles (index + generation). Internals may use raw indices.

* `VertexId`
* `HalfEdgeId` (also `CornerId`)
* `FaceId`
* Optional later: `EdgeId` (otherwise edge = `{h, twin(h)}`)

### Topology records (conceptual)

**Vertex**

* `out: HalfEdgeId` (one outgoing half-edge, or INVALID if isolated)

**HalfEdge**

* `to: VertexId`
* `face: FaceId` (boundary represented via outside face or boundary loops)
* `next: HalfEdgeId`
* `twin: HalfEdgeId`

**Face**

* `edge: HalfEdgeId`
* `degree: u32` (optional cache; can be derived)

### Boundary model

Default for booleans and robust traversal:

* **Explicit boundary half-edges** with a designated “outside face” and/or boundary loops.
* Prefer sentinel IDs over `Option` in hot fields.

---

## Attribute System

Attributes are typed layers keyed by a domain.

Domains:

* **Vertex**: positions (required), scalar/vector fields
* **Face**: material/region ids, tags
* **Edge**: sharpness / crease / bevel weights (or per-half-edge paired with twin)
* **Corner (HalfEdge)**: UVs, **normals**, tangents (later)

Storage policy:

* Dense `Vec<T>` for required layers (e.g. positions)
* Optional layers stored as sparse/dense-option based on usage
* Must support remapping if arenas compact; avoid compaction by default for stability

Normals policy:

* **Derived by default** from smoothing rules.
* **Overridable** via an optional corner-normal layer.

---

## Invariants and Validation

Validation is mandatory (debug-by-default posture).

### Half-edge invariants

* `twin(twin(h)) == h`
* `next` is in the same face loop
* face loop closes

### Face invariants

* loop is non-empty
* loop visits expected number of half-edges (degree) when cached

### Vertex invariants

* from `v.out`, traversal around the vertex is well-defined (with boundary semantics)

### Attribute invariants

* layers match arena capacities or are explicitly sparse
* render extraction splits vertices where corner attributes differ

Provide:

* `validate_fast()` — cheap checks
* `validate_deep()` — graph walks, manifold checks, “explain invalidity” reporting

---

## Public API Shape (high-level)

This section describes the intended public surface. Implementations may evolve behind these interfaces.

### `no_std` posture

`exedra` is `#![no_std]` with `extern crate alloc;` and uses `alloc::vec::Vec` for arenas and layers.

```rust
#![no_std]
extern crate alloc;
```

### Core types (API sketches)

> Note: these are *shape* sketches to guide implementation and review; exact field names may vary.

```rust
use core::num::NonZeroU32;

/// Stable handle: (slot index, generation).
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct Id {
    pub index: u32,
    pub gen: NonZeroU32,
}

pub type VertexId = Id;
pub type HalfEdgeId = Id;
pub type FaceId = Id;

/// CornerId is the half-edge in its face loop.
pub type CornerId = HalfEdgeId;

#[derive(Copy, Clone, Debug)]
pub enum Domain {
    Vertex,
    Face,
    HalfEdge,
}

#[derive(Copy, Clone, Debug)]
pub enum CsgOp {
    Union,
    Intersect,
    Difference,
}

/// Reserved face representing the outside region.
/// (Implementation may use a sentinel FaceId or a dedicated constant.)
#[derive(Copy, Clone, Debug)]
pub struct OutsideFace;
```

### Mesh topology records

```rust
#[derive(Copy, Clone, Debug)]
pub struct Vertex {
    pub out: HalfEdgeId,
}

#[derive(Copy, Clone, Debug)]
pub struct HalfEdge {
    pub to: VertexId,
    pub face: FaceId,
    pub next: HalfEdgeId,
    pub twin: HalfEdgeId,
}

#[derive(Copy, Clone, Debug)]
pub struct Face {
    pub edge: HalfEdgeId,
    pub degree: u32,
}

pub struct Mesh {
    // arenas (stable handles)
    // vertices: Arena<Vertex>
    // half_edges: Arena<HalfEdge>
    // faces: Arena<Face>
    // attributes: Attributes
}
```

### Attributes

```rust
/// Strongly-typed attribute key.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct AttrKey<T> {
    _phantom: core::marker::PhantomData<T>,
    pub domain: Domain,
    pub name: &'static str,
}

pub struct Attributes {
    // Implementation can store layers in a registry.
    // Required layers are dense; optional layers may be sparse.
}

impl Mesh {
    pub fn attrs(&self) -> &Attributes { /* ... */ }
    pub fn attrs_mut(&mut self) -> &mut Attributes { /* ... */ }
}
```

### Shading policy (corner normals)

```rust
#[derive(Copy, Clone, Debug)]
pub enum NormalsSource {
    Derived,
    CustomOrDerived,
    CustomOnly,
}

#[derive(Copy, Clone, Debug)]
pub struct NormalParams {
    pub auto_sharp_angle_degrees: Option<f32>,
    pub weight_mode: NormalWeightMode,
}

#[derive(Copy, Clone, Debug)]
pub enum NormalWeightMode {
    Angle,
    Area,
    AngleArea,
}
```

### Extraction (full + incremental)

```rust
pub struct DirtySet {
    // Conservative is okay early; refine over time.
    pub dirty_faces: alloc::vec::Vec<FaceId>,
    pub dirty_vertices: alloc::vec::Vec<VertexId>,
    pub dirty_corners: alloc::vec::Vec<CornerId>,
}

pub enum ExtractMode {
    FullRebuild,
    Incremental,
}

pub struct ExtractParams {
    pub normals: NormalsSource,
    pub normal_params: NormalParams,
    pub include_uvs: bool,
}

pub struct ExtractStats {
    pub triangles: u64,
    pub render_vertices: u64,
    pub splits: u64,
    // plus timing buckets (hooked to wind tunnels)
}

pub struct RenderCache {
    // Opaque caches: face triangulation, derived normals, segment maps, etc.
}

pub struct ExtractScratch {
    // Reusable staging buffers (no allocation in hot loops).
}

pub struct TriMesh {
    pub indices: alloc::vec::Vec<u32>,
    pub positions: alloc::vec::Vec<[f32; 3]>,
    pub normals: alloc::vec::Vec<[f32; 3]>,
    pub uvs: alloc::vec::Vec<[f32; 2]>,
    // optional: per-tri material ids, etc.
}

impl Mesh {
    pub fn extract(
        &self,
        cache: &mut RenderCache,
        dirty: &DirtySet,
        mode: ExtractMode,
        params: &ExtractParams,
        scratch: &mut ExtractScratch,
    ) -> (TriMesh, ExtractStats) {
        // ...
        unimplemented!()
    }
}
```

### Booleans (staged internally)

```rust
pub struct BooleanParams {
    pub tolerance: f32,
    pub keep_artifacts: bool,
}

pub struct BooleanArtifacts {
    // Intersection segments/polylines, suspect regions, stage stats.
}

pub struct BooleanError {
    pub artifacts: BooleanArtifacts,
    // error classification, e.g. non-manifold, coplanar ambiguity, etc.
}

impl Mesh {
    pub fn boolean(
        a: &Mesh,
        b: &Mesh,
        op: CsgOp,
        params: &BooleanParams,
        scratch: &mut BooleanScratch,
    ) -> Result<Mesh, BooleanError> {
        unimplemented!()
    }
}

pub struct BooleanScratch {
    // BVH scratch, intersection staging, hashbrown maps, etc.
}
```

### Construction helpers

```rust
pub struct BuildParams {
    pub weld_tolerance: Option<f32>,
}

impl Mesh {
    pub fn from_indexed_triangles(
        positions: &[[f32; 3]],
        indices: &[[u32; 3]],
        params: &BuildParams,
    ) -> Mesh {
        unimplemented!()
    }
}
```

---

### Construction

* Build from indexed triangles
* Build from polygon soup
* Optional weld/snap helpers (likely in testkit or behind feature)

### Editing primitives (Exedra-level)

* `split_edge` (core)
* `split_face` (insert diagonal / split by polyline)
* `collapse_edge`, `flip_edge` (added when invariants/attribute propagation are solid)

### Shading controls

* Edge sharpness flags/weights
* `compute_corner_normals(params)` (derived)
* Custom corner normals via attribute layer

### Triangulation and render extraction

* deterministic triangulation of n-gons
* `to_trimesh(params)`:

  * triangulate
  * choose normals (custom-or-derived)
  * split render vertices on discontinuities (pos/uv/normal/...)
  * output `TriMesh` suitable for wgpu

### Booleans (Exedra)

* `boolean(a, b, op, params) -> Result<Mesh, BooleanError>`
* `BooleanError` returns diagnostic artifacts (intersection segments, suspect regions, stats)

---

## Scalability

Scalability is a first-class requirement. Exedra must scale from small procedural meshes to large production scenes without requiring full rebuilds as a normal mode.

### Target envelopes (explicit)

These are planning targets for engineering decisions and wind-tunnel scenarios. They are not promises for every operation on every mesh, but they define the intended scale of the system.

**Interactive envelope (primary)**

* Typical: **50k–500k faces** (authoring / procedural ops / editing)
* Stretch: **~1M faces** (interactive inspection + localized edits)
* Attribute load: UVs + derived normals + sharp edges, multiple layers
* Expectation: localized edits and incremental extraction should be practical; full rebuilds are acceptable as an explicit fallback, not the default.

**Heavy interactive envelope (secondary)**

* Typical: **1M–5M faces** (scene prep, heavy assets, larger environments)
* Expectation: operations must be budgetable/chunked; extraction and derived data should support parallelization; booleans are expected to be selective and may require preprocessing (simplification / region restriction) at higher layers.

**Offline envelope (tertiary)**

* Typical: **5M–20M faces**
* Stretch: **~50M faces** for offline tooling (batch processing)
* Expectation: algorithms emphasize asymptotics and memory efficiency; streaming/debug dumping and stepwise processing matter; some operations may be too expensive without coarse culling and chunking.

### Dimensions of scale

* **Element counts:** vertices/half-edges/faces routinely in the **hundreds of thousands to millions**, with offline workflows targeting **tens of millions**.

* **Attribute volume:** large corner-domain data (UVs/normals) and multiple layers.

* **Operation locality:** most edits are localized; the system should exploit locality.

* **Extraction cost:** converting to renderable triangle buffers should be incremental when possible.

### Core scalability principles

* **Pointerless, contiguous storage:** arenas backed by `Vec` for cache locality.
* **Stable handles:** (index + generation) enables caches and avoids use-after-free.
* **Incrementalism everywhere:** avoid global recomputation; track dirtiness at face/region granularity.
* **Work budgeting:** long operations support chunking and/or progress reporting.
* **Determinism:** stable ordering and reproducible outputs support caching and debugging.

### Avoiding full rebuilds

Exedra should support incremental workflows:

* Track dirty sets for:

  * faces affected by topology edits
  * vertices whose derived normals need recomputation
  * faces whose triangulation cache is invalid
* Render extraction should support:

  * full rebuild path (simple and reliable)
  * incremental path (update only affected faces/regions) where feasible

### Boolean scalability posture

Booleans are inherently heavier; still:

* Provide early coarse culling (AABB / BVH) to avoid O(n²) triangle intersection.
* Ensure intermediate artifacts can be streamed/dumped without holding everything in RAM.
* Keep tolerance policy explicit and deterministic to reduce “retry storms.”

### Parallelism

* Read-only traversals should be safe to parallelize (topology/attribute arrays are immutable during the read phase).
* Mutation remains single-writer by default; higher-level systems can schedule ops.
* Compute-heavy derived data (normals, triangulation, extraction) should be designed to allow parallel execution later.

### Memory strategy

* Prefer dense arrays for required layers; optional layers can be sparse.
* Avoid per-element heap allocations; use pooled scratch buffers.
* Use `hashbrown` behind isolated components (e.g. intersection graph indices) and reuse allocations when possible.

### Introspection hooks (scalability-critical)

Exedra must expose counters suitable for wind tunnels and production profiling:

* element counts and capacities
* per-layer memory usage estimates
* extraction stats: #triangles, #render-vertex splits, time breakdown
* boolean stats: candidate pairs culled, intersections found, splits performed

---

## Performance and Allocation Strategy

* Hot loops must avoid allocation.
* Provide scratch buffers (caller-supplied or reusable) for:

  * triangulation
  * normal computation
  * boolean intersection staging
* Prefer contiguous memory + indices.
* Use hashbrown only where needed (e.g. intersection graph indexing), and isolate it.

Wind tunnels:

* `exedra_wind_tunnel` contains benches and perf regression scenarios.

---

## Edit Propagation Rules (Appendix)

Edits must preserve invariants and define how attributes move. These rules are defaults; callers (often Cambium) may override via policy objects, but Exedra must have deterministic, documented behavior.

### Domains and terminology

* **Vertex-domain**: attributes keyed by `VertexId` (e.g. position).
* **Face-domain**: attributes keyed by `FaceId` (e.g. material/region).
* **Edge-domain**: attributes keyed by a canonical edge `(h, twin(h))` (e.g. sharpness/crease).
* **Corner-domain**: attributes keyed by `CornerId == HalfEdgeId` (e.g. UV, custom normal).

### General propagation principles

1. **Topology-first:** create/update topology, then propagate attributes.
2. **No hidden work:** edits record dirtiness explicitly (faces/vertices/corners).
3. **Deterministic tie-breaking:** when averaging or choosing “one side,” use a stable rule.
4. **Derived data stays derived:** derived corner normals are recomputed from dirtiness; only custom normals are propagated.

### Policies (hook points)

Edits accept an optional policy struct; if absent, these defaults apply.

```rust
pub struct PropagatePolicy {
    pub position_split: PositionSplit,
    pub uv_split: UvSplit,
    pub normal_override_split: NormalOverrideSplit,
    pub face_attr_split: FaceAttrSplit,
    pub edge_attr_split: EdgeAttrSplit,
}

pub enum PositionSplit { Midpoint, WeightedMidpoint /* later */ }
pub enum UvSplit { Midpoint, CopyFromSide /* deterministic */ }
pub enum NormalOverrideSplit { Clear, CopyFromSide, Average /* unit-length renorm */ }
pub enum FaceAttrSplit { Copy, CopyAndTag /* later */ }
pub enum EdgeAttrSplit { Inherit, Clear, SplitWeights /* later */ }
```

---

## Operation Rules

### 1) `split_edge(e)`

Operation: insert new vertex `v_new` on an existing edge and replace the edge by two edges.

**Topology result (conceptual):**

* old edge `(h, t)` becomes `(h0, t0)` and `(h1, t1)` with a new vertex.

**Vertex-domain**

* `position[v_new] = midpoint(position[v0], position[v1])` (default).
* Other vertex attributes (if present) default to:

  * midpoint for numeric types
  * copy-from-side for non-numeric (policy-controlled)

**Edge-domain**

* New edges inherit edge attributes from the original edge (default).

  * `sharpness`, `crease`, `bevel_weight` copy to both children.

**Corner-domain (UV)**

* For each affected face corner on the split edge:

  * new corner UV defaults to midpoint of the two endpoint corner UVs for that face.
  * If UVs are missing on that face/corner, leave missing.

**Corner-domain (custom normal override)**

* Default: **clear** custom normals for the newly created corners (forces derived normals unless explicitly set).
* Optional policy: copy-from-side or average-and-renormalize.

**Face-domain**

* Face attributes unchanged.

**Dirtiness**

* Mark adjacent faces dirty for triangulation.
* Mark vertex star around endpoints and `v_new` dirty for derived normals.

---

### 2) `split_face(face, path)` / `insert_diagonal(face, a, b)`

Operation: split one face into two faces.

**Face-domain**

* Default: both resulting faces copy the original face attributes (material/region).
* Optional policy: copy-and-tag (e.g. set `region_child_index`).

**Corner-domain (UV)**

* Existing corners keep their UVs.
* New diagonal corners:

  * If face has UVs, default UVs are copied from the corresponding vertex’s existing corner in the original face (deterministic selection).
  * If no UVs, remain missing.

**Corner-domain (custom normal override)**

* Existing corners keep.
* New diagonal corners default to cleared overrides.

**Edge-domain**

* The new diagonal edge defaults to `sharp=false` (smooth) unless policy says inherit/force sharp.

**Dirtiness**

* Both new faces dirty.
* Incident vertices dirty for derived normals.

---

### 3) `collapse_edge(e)` (v0.5+)

Operation: remove an edge and merge its endpoints.

This is high-risk; correctness > speed.

**Vertex-domain (position)**

* Default: keep `v_keep` position and remove `v_drop` (deterministic: smaller id wins).
* Optional: midpoint.

**Corner-domain (UV)**

* If UVs conflict at the merged vertex across a face corner:

  * prefer the surviving corner’s UV (deterministic),
  * OR clear UV on conflict (policy) and mark for downstream regeneration.

**Corner-domain (custom normal override)**

* Default: clear overrides on affected corners to avoid invalid shading.

**Edge-domain**

* Recompute/merge edge attributes for edges incident to the merged vertex:

  * if either incident edge is sharp → sharp (default)
  * crease weights: max (default)

**Face-domain**

* Faces that become degenerate are removed; their attributes are dropped.

**Dirtiness**

* Neighborhood faces dirty.
* Vertex star dirty.

---

### 4) `flip_edge(e)` (v0.5+)

Operation: change diagonal in a quad region (typically two triangles).

**Corner-domain (UV)**

* Existing corners keep; newly created corners on the flipped diagonal:

  * default: derive UV by interpolation from neighboring corners if possible; otherwise clear.

**Corner-domain (custom normal override)**

* Default: clear overrides on affected corners.

**Edge-domain**

* Sharpness defaults to `false` for the new diagonal unless policy forces it.

**Dirtiness**

* The two affected faces dirty.
* Surrounding vertices dirty.

---

## Notes on corner normal overrides

* Overrides should be treated as authored data and are not implicitly “reinterpreted”.
* When topology changes create new corners, default is to **clear** overrides unless an explicit policy says otherwise.
* This prevents surprising shading artifacts and aligns with “Explicit Over Implicit.”

---

## Milestone Plan

### v0.1 — Canonical kernel + deterministic extraction (no booleans)

**Goal:** Exedra exists, is testable, deterministic, and supports real attribute domains.

Deliverables:

* Poly half-edge mesh with stable IDs.
* Boundary model selected + implemented.
* Attribute layers:

  * vertex positions (required)
  * corner UVs (optional)
  * edge sharpness flag (or equivalent)
* Deterministic triangulation (simple strategy acceptable; limitations documented).
* `to_trimesh()` that:

  * triangulates
  * splits render vertices on UV seams
  * outputs positions + UVs + indices
* Validation:

  * `validate_fast()` and a partial `validate_deep()`

Non-goals:

* No boolean/CSG.
* No advanced edits unless correct.

Exit criteria:

* Import a mesh, mark sharp edges and UV seams, export stable TriMesh.

---

### v0.5 — Real shading model + serious edit propagation

**Goal:** corner normals become first-class, and edits preserve attributes.

Deliverables:

* Derived corner normals (angle/area-weighted, respects sharp edges).
* Optional custom corner normal layer with override policy.
* Edit primitives with defined attribute propagation:

  * `split_edge`
  * `split_face` (basic)
  * `collapse_edge`/`flip_edge` only if correctness is proven
* `to_trimesh()` now outputs:

  * positions, indices
  * normals (custom-or-derived)
  * UVs
  * splits render vertices on (pos, uv, normal)
* Golden tests for shading cases and deterministic outputs.

Exit criteria:

* Can achieve smooth vs flat looks (including bevel appearance control) via normals without topological hacks.

---

### v0.9 — Boolean pipeline exists + debuggable

**Goal:** B-rep boolean pipeline works for a meaningful subset with strong diagnostics.

Deliverables:

* Boolean stages:

  1. tri/tri intersection
  2. intersection graph
  3. split faces/edges along intersections
  4. classify patches
  5. stitch shells
* Supported inputs: closed, oriented triangle meshes (imported into PolyMesh internally).
* Strong diagnostics on failure:

  * intersection segments/polylines
  * suspect triangles/edges lists
  * manifold violation reports
  * tolerance decision summary
* Debug dumping of intermediates (via `exedra_testkit` / `std`).

Exit criteria:

* On representative mesh corpus, booleans usually succeed; failures are explainable via artifacts.

---

### v1.0 — Production-capable kernel

**Goal:** stable APIs, deterministic behavior, robust-enough booleans, real attribute model.

Deliverables:

* Semver-stable API surface.
* Booleans (union/intersect/difference) for:

  * multiple components
  * common degeneracies handled or explicitly rejected with diagnostics
* Tolerance policy object is explicit and documented.
* Validation tooling is strong:

  * manifold checks
  * boundary checks
  * “explain invalidity” reporting
* Perf posture:

  * no allocations in hot loops
  * wind tunnel benchmarks + regression tracking
  * fuzz targets for topology edits and boolean pipeline
* Clear documentation:

  * what Exedra guarantees
  * what it does not attempt
  * how attributes and seams work
  * how render extraction splits vertices

Exit criteria:

* Lightweald can depend on Exedra without “we’ll replace this later.”

---

## Locked Architectural Decisions

The following decisions are locked for the foreseeable future. Changing them requires strong justification.

### 1) Boundary Model: Explicit + Outside Face

* Every topological edge has exactly two half-edges.
* `HalfEdge.twin` is always valid (no `Option`).
* A reserved `FaceId::OUTSIDE` represents the outside/boundary region.
* Boundary loops are represented as half-edge cycles attached to `OUTSIDE`.

Rationale:

* Eliminates branchy `Option` logic in hot loops.
* Simplifies traversal and validation.
* Supports boolean splitting/stitching naturally.
* Aligns with “Explicit Over Implicit.”

### 2) Boolean Architecture: Staged Pipeline with Artifacts

Boolean operations are implemented as an explicit staged pipeline:

1. Broad phase (AABB / BVH culling)
2. Narrow phase (triangle–triangle intersections)
3. Intersection graph construction (segments / polylines)
4. Mesh splitting along intersection graph
5. Patch classification (inside / outside / coplanar)
6. Stitch and cleanup

* Each stage exposes timing and statistics hooks.
* Failures return structured artifacts (not just errors).
* Scratch buffers are reused across stages to avoid allocations.

Public API may provide a simple `boolean(...)` entry point, but internally the staged model is preserved.

### 3) Attribute Storage Strategy: Hybrid Dense + Sparse

* Required layers (e.g. vertex positions) are dense `Vec<T>`.
* Frequently-present layers (e.g. corner UVs) may be dense optional.
* Rare override layers (e.g. custom corner normals) may be sparse.
* Layers are typed and keyed by explicit domains.
* Attribute propagation rules are defined for each edit primitive.

Rationale:

* Balances memory efficiency with cache locality.
* Avoids per-element heap allocations.
* Keeps attribute model extensible without contaminating hot paths.

### Edge representation

* Edge is initially represented canonically as `(h, twin(h))`.
* No separate `EdgeId` in v0.1.
* Introduce `EdgeId` only if ergonomics or performance require it.

---

## Wind Tunnel Scenarios

Wind tunnels are formal performance and scalability validation cases. They live in `exedra_wind_tunnel`.

Each scenario defines:

* mesh size
* attribute load
* operation
* target metrics (time, allocations, memory, splits)

### WT-1: Triangulation Stress

* 500k-face mesh with UVs and sharp edges.
* Measure:

  * triangulation time
  * render vertex split count
  * allocations (should be zero in hot loop)

### WT-2: Normal Generation

* 1M-face mesh.
* Compute derived corner normals.
* Measure:

  * time per million corners
  * parallel scaling potential (future)

### WT-3: Incremental Edit

* 500k-face mesh.
* Perform localized `split_edge` on 100 edges.
* Measure:

  * dirty face count
  * incremental extraction cost vs full rebuild

### WT-4: Boolean Medium

* Two 200k-face meshes with moderate overlap.
* Measure:

  * broad-phase candidate reduction ratio
  * intersection count
  * split count
  * total time per stage

### WT-5: Boolean Heavy (Offline)

* 2M-face meshes (offline envelope).
* Measure:

  * peak memory
  * stage time breakdown
  * diagnostics size

Wind tunnel results are treated as regression baselines. Any architectural change must be measurable against them.

---

## Determinism Contract

Determinism is a feature.

Given identical:

* input meshes (topology + attributes)
* parameters (numeric policy, extraction settings, boolean params)
* Rust toolchain/MSRV within the supported range

Exedra should produce identical outputs across runs:

* identical topology results for edit primitives
* identical triangle ordering for triangulation/extraction
* identical render-vertex splitting and buffer ordering

Rules:

* Stable iteration order over arenas (slot order).
* Deterministic tie-breaking (prefer smaller stable IDs).
* Never leak hash iteration order into externally-visible ordering.

  * If hashbrown is used internally, any externally-visible lists derived from it must be sorted deterministically.

See ‘Kernel Boundary Contract’ for ordering rules on ChangeSets and extraction outputs.

---

## Numeric Policy

No hidden epsilons. All geometric comparisons and snapping/welding decisions flow through a policy object.

```rust
#[derive(Copy, Clone, Debug)]
pub struct NumericPolicy {
    pub epsilon: f32,
    pub merge_tolerance: f32,
    pub coplanar_tolerance: f32,
    pub normal_epsilon: f32,
}
```

Guidance:

* Defaults must be documented and tested in wind tunnels.
* Policies are passed explicitly into operations that depend on them (booleans, welding, intersection).

---

## Failure and Diagnostics Taxonomy

Errors must be classifiable. Strings are not an API.

### Boolean failure kinds

```rust
#[derive(Copy, Clone, Debug)]
pub enum BooleanFailureKind {
    NonManifoldInput,
    SelfIntersectionDetected,
    CoplanarAmbiguity,
    ToleranceExceeded,
    NumericalInstability,
    InternalInvariantViolation,
}

pub struct BooleanError {
    pub kind: BooleanFailureKind,
    pub artifacts: BooleanArtifacts,
}
```

Similar taxonomies should exist for:

* invalid topology operations (precondition failures)
* extraction failures (e.g. invalid attribute layer state)

Diagnostics principles:

* Failure returns structured artifacts sufficient to reproduce and inspect.
* Artifacts are bounded/streamable for large meshes.

---

## Supported Mesh Classes

Exedra supports a range of mesh “classes,” but operations may restrict inputs.

* Editing/topology operations: may operate on open meshes, boundary loops, and intermediate states.
* Boolean operations (v0.9+): inputs are expected to be:

  * closed
  * oriented
  * manifold (2-manifold)

Non-manifold or self-intersecting inputs must be rejected with a clear failure kind and artifacts.

---

## Compaction Policy

Compaction is supported as an **explicit operation**.
Compaction is never implicit; it is caller-controlled.

Rationale:

* Stable handles + tombstones are convenient during editing.
* Long-running sessions and offline pipelines may require reclaiming memory.

Rules:

* The resulting mesh has a new set of stable IDs; old IDs are not valid for the new mesh.
* A `Remap` is returned to translate IDs from old → new for all live elements.
* Elements that were deleted/tombstoned have no mapping (or map to an explicit invalid sentinel).

API sketch:

```rust
pub struct Remap {
    // old -> new mappings per domain
}

impl Mesh {
    pub fn compact(&self) -> (Mesh, Remap) {
        unimplemented!()
    }
}
```

Determinism rule for compaction: the new arenas must be laid out deterministically (stable traversal/slot order of the old mesh, excluding tombstones), so that compaction produces reproducible results.

---

## Unsafe Policy

`unsafe` is permitted only when it is:

* localized
* documented with the invariants it assumes
* covered by tests/validation

Rules:

* No unsafe in public API.
* Prefer safe code; unsafe is an optimization tool, not a design tool.
* Unsafe must not weaken topology invariants.

---

## Kernel Boundary Contract: Transactions, ChangeSets, Dirtiness, and Ordering

This section defines the **contract surface** between Exedra (kernel) and higher layers (notably Cambium). It exists to prevent “folk knowledge” dependencies and to keep incremental workflows deterministic and debuggable.

### Transactions are the unit of mutation

All topology/attribute mutations happen inside an explicit transaction.

* Transactions are **single-writer**.
* A transaction records **what changed** and produces a `ChangeSet` on commit.
* Higher layers must not infer dirtiness by inspection; they consume the `ChangeSet` output.

API sketch (shape only):

```rust
pub struct Txn<'a> { /* borrows Mesh mutably */ }

/// Summarizes what changed during a transaction.
/// All externally-visible ID lists are deterministically ordered.
pub struct ChangeSet {
    pub dirty: DirtySet,

    pub created_vertices: alloc::vec::Vec<VertexId>,
    pub created_half_edges: alloc::vec::Vec<HalfEdgeId>,
    pub created_faces: alloc::vec::Vec<FaceId>,

    pub deleted_vertices: alloc::vec::Vec<VertexId>,
    pub deleted_half_edges: alloc::vec::Vec<HalfEdgeId>,
    pub deleted_faces: alloc::vec::Vec<FaceId>,
}

impl Mesh {
    pub fn begin(&mut self) -> Txn<'_> {
        unimplemented!()
    }
}

impl<'a> Txn<'a> {
    pub fn commit(self) -> ChangeSet {
        unimplemented!()
    }
}
```

**Deterministic ordering rule:** any `created_*` and `deleted_*` lists are returned in a stable, deterministic order (typically arena slot order or increasing stable ID order). No hash iteration order may leak into these lists.

### Dirtiness semantics

`DirtySet` is a **conservative** summary of what derived data and caches may be invalid.

* If in doubt, mark more dirty rather than less.
* Dirtiness is **about invalidation**, not necessarily “must recompute everything.”
* Dirtiness is recorded at commit and consumed by incremental systems.

Recommended semantics:

* `dirty_faces`: faces whose triangulation cache and/or per-face extraction data is invalid.
* `dirty_vertices`: vertices whose derived data depends on their one-ring (e.g., smoothing groups, adjacency-derived fields).
* `dirty_corners`: corners whose corner-domain derived data is invalid (e.g., derived normals, tangent frame, seam classification).

Notes:

* Exedra may initially mark dirtiness at coarse granularity (e.g., all adjacent faces) and refine later.
* Edits must update dirtiness deterministically (stable traversal order; deterministic tie-breaking).

### Render extraction ordering and render-vertex identity

Extraction (`extract()` / `to_trimesh()`) must be deterministic and must define externally-visible ordering rules.

**Render-vertex identity** is defined as a stable key derived from:

* the topology source (`VertexId` for position),
* plus the enabled corner-domain attributes at the relevant corner (`CornerId`), e.g.:

  * UV (if enabled/present),
  * chosen normal (custom override or derived, per params),
  * tangents (later),
  * other enabled outputs.

Render extraction duplicates (“splits”) render vertices when these attributes differ across corners, while keeping topology stable.

**Output ordering rule (recommended):**

* Iterate faces in deterministic arena order (excluding `OUTSIDE`).
* For each face, walk its corner loop starting from `Face.edge` and following `next`.
* Triangulate deterministically and emit triangles in a deterministic fan/ear order (as defined by triangulation strategy).
* Emit per-triangle indices and per-vertex attribute streams in the order induced by this traversal.

This guarantees stable output buffers across runs given identical inputs + params.

### Cache invalidation contract

`RenderCache` and other derived caches must be invalidated only through explicit inputs:

* `ExtractMode::FullRebuild`: ignore dirtiness and rebuild all caches/outputs.
* `ExtractMode::Incremental`: consume `DirtySet` (typically from the latest `ChangeSet`) and update only affected regions.

Callers (e.g. Cambium) are expected to:

* keep caches and scratch buffers across frames,
* pass the last `ChangeSet.dirty` into incremental extraction,
* fall back to `FullRebuild` explicitly when needed.

### Boundary model operational clarifications (required)

The boundary model is locked as “Explicit + Outside Face.” The following operational details are part of the contract and must be explicitly documented/implemented:

1. **`OUTSIDE` representation**

   * Either `FaceId::OUTSIDE` is a real arena entry with a valid `Face` record, **or** it is a sentinel constant treated specially.
   * One must be chosen and stated; code and validation must align with that choice.

2. **Boundary loop orientation and traversal**

   * Boundary half-edges form cycles attached to `OUTSIDE`.
   * Traversal/winding conventions for boundary cycles must be deterministic and documented (so higher layers can reason about “inside/outside” consistently).

### Internal hash usage rule (no leakage)

Exedra may use `hashbrown` internally, but:

* any externally-visible ordering derived from hash-based structures must be sorted into deterministic order before being returned,
* and determinism must not depend on hash seed or iteration order.

This applies to:

* lists of intersection artifacts,
* sets of suspect triangles/edges,
* any “collected” IDs returned to callers.

---

## Notes on Cambium

Cambium depends on Exedra and owns high-level modeling:

* subdivision
* procedural ops
* refinement/remeshing (later)
* operator stacks / node-graph friendliness

UV generation guidance:

* Cambium is the right home for **UV coordinate generation** (because it’s an operator/modeling concern), while Exedra remains the storage + invariants layer.
* Start with pragmatic, deterministic generators:

  * planar / box projection
  * cylinder projection
  * “unwrap along seams” helpers (consume user-provided seams)
* Explicitly *not* required for v1.0: full automatic unwrapping with chart segmentation + atlas packing (that can come later, potentially as a dedicated crate).

Cambium may move faster; Exedra stays calm and stable.

Cambium design and v0.1 plan: see docs/cambium_handover.md.
