# Cambium Handover Spec

*(Operator / growth layer on top of Exedra)*

## Context

We are building **Cambium**, the operator layer for the Lightweald ecosystem.

* **Exedra** is the calm, stable **mesh kernel** (topology + attributes + deterministic extraction + booleans).
* **Cambium** is the **growth layer** that provides higher-level modeling operations, procedural workflows, and operator stacking—while preserving Exedra’s invariants and determinism.

Cambium must move faster than Exedra, but it must not “leak chaos” into Exedra’s API.

---

## Goals

1. **Operator Stack Foundation**

   * Provide a composable operator framework (procedural + interactive).
   * Operators are deterministic, inspectable, and testable.

2. **High-Level Modeling Operations**

   * Subdivision, bevel-ish workflows, extrude, inset, bridge, boolean composition orchestration, remesh (later).
   * UV coordinate generation utilities (projection-based first).

3. **Incremental Workflows**

   * Interactive ops produce previews without full rebuilds by default.
   * Use Exedra `ChangeSet`/`DirtySet` to drive incremental extraction.

4. **Introspection + Diagnostics**

   * Operator timing, allocations, intermediate artifacts, failure kinds.
   * Debug dumping of intermediate meshes/fields for reproducibility.

5. **Boundary Discipline**

   * Cambium never requires Exedra to know about “tools,” “UI,” “nodes,” or “procedural graphs.”
   * Cambium treats Exedra as a capability provider.

---

## Non-Goals (v1.0)

* Full automatic UV unwrapping (chart segmentation + packing).
* Full mesh repair suite (Cambium may provide light “cleanup helpers,” but not a comprehensive healing pipeline).
* CAD-grade exact arithmetic everywhere (follow Exedra’s numeric policy; use explicit tolerances).

---

## Engineering Constraints

### Inherited Tenets

Cambium follows the same Forest Engineering Tenets as Exedra:

* explicit over implicit
* introspection non-optional
* deterministic outputs
* replaceability

### Rust / crate posture

* `cambium` should be `#![no_std]` + `alloc` **where feasible**, but may use `std` earlier than Exedra if needed.
* Avoid allocations in hot loops; reuse scratch buffers.
* Use `hashbrown` when needed.

---

## Repo / Workspace Layout

* `crates/cambium/` — operator framework + canonical ops
* `crates/cambium_testkit/` — fixtures, golden outputs, operator corpuses (`std`)
* `crates/cambium_wind_tunnel/` — perf scenarios (`std`)

Optional later:

* `crates/cambium_nodes/` — node-graph integration / operator graph runtime adapters
* `crates/cambium_uv/` — UV generators as a separate crate if it grows large

---

## Architectural Overview

Cambium is two things:

1. **Operator Runtime**: how ops are represented, executed, cached, debugged.
2. **Operator Library**: the actual modeling ops built on Exedra edit primitives.

### Key design principle

Cambium is an *orchestrator* over Exedra, not a parallel mesh kernel.

* Cambium:

  * chooses sequences of Exedra edits
  * supplies propagation policies
  * defines “meaning” and “tool semantics”
  * stages previews and commits results

* Exedra:

  * performs topology edits safely + deterministically
  * tracks change sets / dirtiness
  * provides extraction / booleans / validation

---

## Locked Architectural Decisions

The following decisions are locked for the foreseeable future. Changing them requires strong justification.

### 1) Preview vs commit is first-class

* Preview outputs are produced explicitly and may be approximate/budgeted.
* Commit outputs are explicit and must be reproducible.
* Preview is expected to be discardable without mutating the committed base mesh.

### 2) Edit operators are the primary execution path

* Cambium’s core operators should execute via Exedra transactions and return `ChangeSet`.
* Pure operators that return a new `Mesh` are permitted, but are secondary and primarily for offline/batch transforms.

### 3) Operator reports and bounded artifacts are mandatory

* Each operator produces an `OpReport` containing:

  * counters and stats
  * timing buckets
  * optional bounded artifacts
* Artifact ordering must be deterministic; no hash iteration leakage.

---

## Core Concepts

### 1) Operator

An operator is a deterministic transformation:

`Input State + Params + Policies → Output State + Report`

Operators come in two forms:

* **Edit ops**: run via Exedra transactions and return a `ChangeSet` on commit.
* **Pure ops**: take an input mesh and produce a new mesh (offline/batch).

### 2) Operator Session

Interactive tools require staging:

* a *base mesh* (committed)
* a *preview mesh* (ephemeral)
* a *commit* step (explicit)

### 3) Operator Report

Every operator produces:

* timings
* counters (created faces, splits, etc.)
* optional debug artifacts (intermediate meshes, point sets, segment graphs)

### 4) Determinism

Cambium must not introduce nondeterministic ordering:

* no hash iteration order leaks
* stable ID tie-breaking (prefer smallest stable IDs)
* stable traversal rules

---

## Public API Shape (high-level)

This section describes the intended public surface. Implementations may evolve behind these interfaces.

### Operator context

Cambium operators execute inside an `OpContext`, which carries:

* **policy**: higher-level Cambium policies (preview/commit quality, UV policy, etc.)
* **numeric**: Exedra numeric policy (tolerances)
* **scratch**: reusable buffers (hot loops must not allocate)
* **diagnostics**: structured diagnostics sink (bounded, deterministic ordering)
* **clock**: timing recorder (best-effort; counters/stats must be deterministic)

```rust
pub struct OpContext<'a> {
    pub policy: &'a cambium_policy::PolicySet,
    pub numeric: exedra::NumericPolicy,
    pub scratch: &'a mut Scratch,
    pub diagnostics: &'a mut DiagnosticsSink,
    pub clock: &'a mut Clock,
}
```

### Scratch

`Scratch` is caller-provided reusable memory for operators.

Goals:

* Hot loops should not allocate; allocate once in scratch and reuse.
* Scratch is intentionally **untyped** at the top level, but offers typed buffers for common needs.
* Scratch capacity growth is explicit and measurable (wind tunnels).

API shape (starter):

```rust
pub struct Scratch {
    pub u32s: alloc::vec::Vec<u32>,
    pub u64s: alloc::vec::Vec<u64>,
    pub f32s: alloc::vec::Vec<f32>,
    pub vec2: alloc::vec::Vec<[f32; 2]>,
    pub vec3: alloc::vec::Vec<[f32; 3]>,

    /// Generic reusable id lists.
    pub faces: alloc::vec::Vec<exedra::FaceId>,
    pub half_edges: alloc::vec::Vec<exedra::HalfEdgeId>,
    pub corners: alloc::vec::Vec<exedra::CornerId>,
    pub vertices: alloc::vec::Vec<exedra::VertexId>,

    /// Optional: hash maps/sets used by some operators.
    /// Must be cleared and reused, not recreated.
    ///
    /// Note: `hashbrown` works in `no_std` with `alloc` (it requires an allocator),
    /// so this does *not* require `std`.
    pub maps: ScratchMaps,
}

pub struct ScratchMaps {
    pub u32_to_u32: hashbrown::HashMap<u32, u32>,
}

impl Scratch {
    pub fn clear(&mut self) {
        self.u32s.clear();
        self.u64s.clear();
        self.f32s.clear();
        self.vec2.clear();
        self.vec3.clear();
        self.faces.clear();
        self.half_edges.clear();
        self.corners.clear();
        self.vertices.clear();
        self.maps.u32_to_u32.clear();
    }
}
```

Guidance:

* Scratch buffers are reused across operator calls.
* Operators must treat scratch contents as ephemeral and must not retain references into scratch.
* Any operator that needs significant scratch beyond these common buffers should add a dedicated, reusable field to `Scratch` (not allocate ad-hoc).
* Wind tunnels should report scratch peak capacities to track regressions.

### Reports, stats, and timings

Each operator produces an `OpReport`.

* **Stats/Counters** are part of the determinism contract.
* **Timings** are best-effort and not deterministic; they exist for profiling and wind tunnels.

```rust
pub struct OpReport {
    pub name: &'static str,
    pub stats: Stats,
    pub timings: Timings,
    pub artifacts: Artifacts, // optional / bounded
}

/// Deterministic counters.
pub struct Stats {
    pub elements_touched: ElementsTouched,
    pub elements_created: ElementsCreated,
    pub elements_deleted: ElementsDeleted,

    /// Domain-specific counts; must be deterministic.
    /// Example: corners_written, seams_marked, faces_selected.
    pub counters: SmallCounters,
}

/// v0.1 uses a fixed struct for counters (not a map) to keep the surface calm and
/// avoid ordering issues.
///
/// New counters are added as fields (semver-managed) until we have strong reason
/// to introduce a more generic mechanism.
#[derive(Copy, Clone, Debug, Default)]
pub struct SmallCounters {
    pub faces_processed: u64,
    pub corners_written: u64,
    pub corners_skipped_existing: u64,

    /// Reserved for future common counters.
    pub selections_canonicalized: u64,
}


pub struct ElementsTouched {
    pub vertices: u64,
    pub half_edges: u64,
    pub faces: u64,
    pub corners: u64,
}

pub struct ElementsCreated {
    pub vertices: u64,
    pub half_edges: u64,
    pub faces: u64,
}

pub struct ElementsDeleted {
    pub vertices: u64,
    pub half_edges: u64,
    pub faces: u64,
}

/// Timing buckets for profiling and wind tunnels.
/// Timing values are *not* deterministic, but bucket naming and presence is.
pub struct Timings {
    /// Timing buckets stored in a deterministic order.
    ///
    /// v0.1 stores buckets as a small list of unique names; repeated measurements
    /// accumulate into the existing bucket.
    pub buckets: alloc::vec::Vec<TimeBucket>,

    /// Maximum distinct bucket names recorded in a single report.
    /// This prevents accidental unbounded growth.
    pub max_buckets: usize,
}

pub struct TimeBucket {
    pub name: &'static str,
    pub nanos: u64,
}

impl Timings {
    /// Add elapsed nanos to a named bucket. If the bucket does not exist, it is created
    /// (unless `max_buckets` would be exceeded).
    ///
    /// Determinism: if a new bucket must be dropped due to `max_buckets`, it is dropped
    /// deterministically (the attempted new bucket is ignored).
    pub fn add(&mut self, name: &'static str, nanos: u64) {
        unimplemented!()
    }
}

/// Clock records time spent in named buckets.
/// Implementations may use `std::time::Instant` when available.
/// In `no_std`, the clock may record zeros or use a platform hook.
pub struct Clock { /* ... */ }

impl Clock {
    /// Start a timing bucket and return a guard that records elapsed time on drop.
    ///
    /// Bucket names must be stable `'static` identifiers (e.g. `"select"`, `"compute"`).
    pub fn bucket<'a>(&'a mut self, name: &'static str) -> ClockBucket<'a> {
        unimplemented!()
    }
}

/// RAII timing guard. On drop, adds elapsed time to the matching bucket.
pub struct ClockBucket<'a> {
    // holds start timestamp and a mutable reference to the clock
    _priv: core::marker::PhantomData<&'a mut ()>,
}
```

Clock semantics (locked):

* Buckets are **additive**: multiple `bucket("compute")` scopes sum their time.
* Nested buckets are allowed. When nested, time is attributed to **both** parent and child scopes.

  * This keeps the API simple and avoids hidden accounting complexity.
* `Clock` does not allocate in hot paths.
* In `no_std` builds, `Clock::bucket()` may return a no-op guard that records `0`.

Timing guidance:

* Operators should record a small set of stable buckets.
* v0.1 timing buckets are **unique by name** and **accumulate**.
* `Timings.max_buckets` should be small (e.g. 32) to prevent accidental growth.
* Recommended buckets for most ops:

  * `select`
  * `compute`
  * `edit`
  * `attrs`
  * `validate` (optional)

### Artifacts and diagnostics

Cambium differentiates between:

* **Diagnostics**: structured messages (errors/warnings/notes) with stable ordering.
* **Artifacts**: optional bounded payloads (meshes/sets/polylines/fields) used for debugging.

Both are mandatory infrastructure. Operators may emit zero artifacts.

#### DiagnosticsSink

Diagnostics are strings only as *rendered messages*; programmatic identity uses enums/codes.

```rust
pub struct DiagnosticsSink {
    // bounded storage; deterministic ordering
}

pub enum DiagLevel { Note, Warn, Error }

pub struct Diagnostic {
    pub level: DiagLevel,
    pub code: DiagCode,
    pub message: alloc::string::String,
    pub spans: alloc::vec::Vec<DiagSpan>,
}

pub enum DiagCode {
    PreconditionFailed,
    NonManifoldInput,
    MissingRequiredAttribute,
    NumericToleranceIssue,
    InternalInvariantViolation,
    Cancelled,
    BudgetExceeded,
    // ...extend as needed
}

pub enum DiagSpan {
    Vertex(exedra::VertexId),
    HalfEdge(exedra::HalfEdgeId),
    Face(exedra::FaceId),
    Corner(exedra::CornerId),
    // Later: selection set ids, polyline ids, etc.
}
```

Deterministic ordering rule for diagnostics:

* `DiagnosticsSink` must present diagnostics in a deterministic order.
* If diagnostics are gathered via hash maps/sets, the sink must sort by `(level, code, span ids, insertion order)` in a stable manner.

### Diagnostics deduplication (v0.1)

v0.1 does **not** attempt automatic deduplication of diagnostics.

Rationale:

* Dedup rules are easy to get wrong and can hide repeated but important context.
* Boundedness already prevents unbounded spam.

If dedup is introduced later, it must be:

* explicit and deterministic
* keyed by `(level, code, spans, message)` or another documented stable key
* applied only after bounds checks (or with a deterministic policy for which instances survive)

#### Artifacts

Artifacts are optional but when present must be:

* **bounded** by policy (count and bytes)
* **deterministically ordered**
* **serializable** via `cambium_testkit` (std)

```rust
pub struct Artifacts {
    pub items: alloc::vec::Vec<Artifact>,
}

pub enum Artifact {
    Mesh { name: &'static str, mesh: exedra::Mesh },
    FaceSet { name: &'static str, faces: alloc::vec::Vec<exedra::FaceId> },
    EdgeSet { name: &'static str, half_edges: alloc::vec::Vec<exedra::HalfEdgeId> },
    CornerSet { name: &'static str, corners: alloc::vec::Vec<exedra::CornerId> },
    Polyline3 { name: &'static str, points: alloc::vec::Vec<[f32; 3]> },
    Polyline2 { name: &'static str, points: alloc::vec::Vec<[f32; 2]> },
    FieldF32 { name: &'static str, domain: exedra::Domain, values: alloc::vec::Vec<f32> },
}
```

Boundedness rule:

* `PolicySet` defines max artifact items and max total bytes.
* On overflow, artifacts are truncated deterministically (keep earliest by stable insertion order).

### Error taxonomy

Cambium errors are structured and classifiable. Strings are not an API.

Principles:

* Operators return `OpError` with a stable `OpErrorKind`.
* Operators may still emit diagnostics/artifacts to explain context.
* Preview flows may return `BudgetExceeded` or `Cancelled` without being considered “bugs.”

```rust
pub struct OpError {
    pub kind: OpErrorKind,
    pub diagnostics: alloc::vec::Vec<Diagnostic>,
    pub artifacts: Artifacts,
}

pub enum OpErrorKind {
    /// Operator precondition failed (e.g. empty selection, missing required layer).
    PreconditionFailed,

    /// Input mesh is invalid for this operator.
    InvalidMesh,

    /// Required attribute layer missing or wrong domain.
    MissingAttribute,

    /// Numeric issues (tolerance, degeneracy, NaN/Inf) prevented a robust result.
    NumericFailure,

    /// Operator exceeded a preview budget; caller may retry with higher budget or commit mode.
    BudgetExceeded,

    /// Operator cancelled (user action or orchestration decision).
    Cancelled,

    /// Internal bug or invariant violation.
    InternalInvariantViolation,
}
```

Notes:

* `OpError` is distinct from Exedra boolean/topology errors; Cambium should *wrap* those errors by mapping them into `OpErrorKind` and attaching the original artifacts/diagnostics.
* `OpError.diagnostics` is included for convenience; operators may also write into `ctx.diagnostics`.

### Operator identity and naming (locked)

Operator identity must be stable and machine-friendly.

* `name()` returns a stable identifier using a dot-separated namespace, e.g.:

  * `"uv.planar"`, `"uv.box"`, `"select.mark_seam"`
* Names are part of the determinism/debug contract (appear in reports and wind tunnel output).
* Names are not localized and are not user-facing prose.

### Edit operator trait

Cambium’s primary operator execution path is edit-based: operators apply edits via an Exedra transaction, and Cambium commits the transaction to obtain a `ChangeSet`.

```rust
pub trait EditOperator {
    type Params;

    fn name(&self) -> &'static str;

    fn apply(
        &self,
        txn: &mut exedra::Txn<'_>,
        params: &Self::Params,
        ctx: &mut OpContext<'_>,
    ) -> Result<OpReport, OpError>;
}
```

Notes:

* The `Txn` is committed by Cambium orchestration code, which yields a `ChangeSet`.
* Returning `OpReport` from `apply()` ensures reports exist even if commit later fails.

### Operator orchestration

Cambium provides helpers that standardize preview vs commit execution and the returned value shape.

```rust
pub struct OpResult {
    pub change_set: exedra::ChangeSet,
    pub report: OpReport,
}

pub struct OperatorRunner {
    // persistent caches and reusable scratch
}

impl OperatorRunner {
    pub fn run_commit<O: EditOperator>(
        &mut self,
        mesh: &mut exedra::Mesh,
        op: &O,
        params: &O::Params,
        ctx: &mut OpContext<'_>,
    ) -> Result<OpResult, OpError> {
        unimplemented!()
    }

    pub fn run_preview<O: EditOperator>(
        &mut self,
        mesh: &exedra::Mesh,
        op: &O,
        params: &O::Params,
        ctx: &mut OpContext<'_>,
    ) -> Result<(exedra::Mesh, OpReport), OpError> {
        unimplemented!()
    }
}
```

Preview behavior is intentionally specified at the runner level:

* The runner clones the mesh for preview in v0.1 (requires `exedra::Mesh: Clone`).
* Later designs may introduce copy-on-write or undo logs, but the **external preview contract** remains explicit.

Preview contract stability (locked):

* `run_preview` always returns an owned preview mesh and an `OpReport`.
* Any future optimization must preserve deterministic results and must not mutate the committed base mesh.

### Operator registry (v0.1 posture)

Cambium does not require a global operator registry in v0.1.

* Operators may be constructed and invoked directly (static dispatch is fine).
* A registry becomes useful when we add node-graph integration or a palette of tools.

If a registry is introduced later, it must:

* be deterministic in iteration/order (no hash iteration leakage)
* treat operator `name()` as the stable key
* avoid allocating per invocation (store ops once, reuse)

---

## Policy System

Cambium owns higher-level policy. Exedra owns *defaults*.

Cambium defines:

* seam conventions (what counts as a seam for an op)
* UV generation strategy selection
* boolean orchestration policy
* edit propagation overrides for specific tools
* quality levels (fast preview vs high quality commit)
* bounded diagnostics and artifact limits
* optional validation-on-preview/commit

### PolicySet

A single object passed everywhere.

```rust
pub struct PolicySet {
    pub quality: QualityPolicy,
    pub uv: UvPolicy,
    pub boolean: BooleanPolicy,

    /// Default attribute propagation overrides passed into Exedra edit primitives.
    pub propagate: exedra::PropagatePolicy,

    /// Limits for diagnostics and artifacts.
    pub limits: LimitsPolicy,

    /// Validation knobs.
    pub validate: ValidatePolicy,
}

pub struct QualityPolicy {
    /// A hint to operators for preview vs commit behavior.
    pub mode: QualityMode,

    /// Optional work budget. Interpretation is operator-specific but must be documented.
    pub budget: Option<WorkBudget>,
}

pub enum QualityMode { Preview, Commit }

pub struct WorkBudget {
    /// Maximum faces/corners to touch in preview mode (deterministic limit).
    pub max_faces: Option<u64>,
    pub max_corners: Option<u64>,

    /// Maximum time budget (best-effort). This is advisory; for deterministic truncation
    /// operators should rely on `max_faces`/`max_corners`.
    pub max_millis: Option<u64>,
}

pub struct LimitsPolicy {
    pub max_diagnostics: usize,
    pub max_artifact_items: usize,
    pub max_artifact_bytes: usize,
}

pub struct ValidatePolicy {
    pub validate_on_preview: bool,
    pub validate_on_commit: bool,

    /// On validation failure, Cambium returns an error. For commit runs, the mesh has
    /// already been mutated (explicit over implicit); callers may choose to retry from
    /// a known-good snapshot if they need rollback behavior.
    pub fail_on_error: bool,
}

pub struct UvPolicy {
    pub default_scale: f32,
    pub default_offset: [f32; 2],
    pub allow_overwrite_existing: bool,
}

pub struct BooleanPolicy {
    pub preview_params: exedra::BooleanParams,
    pub commit_params: exedra::BooleanParams,
}
```

Determinism rules:

* Policies are explicit inputs; identical policy values must produce identical operator results.
* Budget-based early exits must be deterministic for a given budget and input selection ordering.

Implementation notes:

* Policy objects should be plain structs/enums with `Copy` where practical.
* If policies grow large, split into sub-crates (`cambium_policy`) and keep `cambium` depending on them.

---

## Attribute access contract (v0.1)

Cambium operators read and write Exedra attributes through **built-in attribute keys** provided by Exedra.

* Exedra exposes built-in keys for common authored/derived layers (at minimum):

  * `exedra::attr::VERTEX_POSITION`
  * `exedra::attr::CORNER_UV`
  * (later) `exedra::attr::CORNER_NORMAL_OVERRIDE`

Rules:

* Cambium must not invent identity for these layers (no ad-hoc string keys).
* Operators may create additional layers via an explicit Exedra API later, but v0.1 only relies on built-ins.

---

## Incremental & Preview Model

### Dirtiness and incremental workflows

Cambium relies on Exedra’s `ChangeSet`/`DirtySet` rather than inferring dirtiness. Cambium may additionally maintain its own fine-grained dirty tracking for derived operator-local caches.

#### `understory_dirty` integration

Cambium uses `understory_dirty` as the primary mechanism for **fine-grained, multi-channel** dirtiness tracking.

* Multiple dirty channels are supported efficiently (bitset-like behavior), which makes it practical to track:

  * selection/region cache invalidation
  * adjacency/one-ring cache invalidation
  * UV cache invalidation
  * operator-local derived fields

Memory guidance:

* Channels must be used intentionally; every new channel is a permanent memory commitment.
* Prefer channel reuse (shared semantics) over creating bespoke channels per operator.
* For very large meshes, prefer marking at face granularity (or region granularity) rather than per-corner unless the operator truly needs corner granularity.

Exedra remains the source of truth for topology/attribute dirtiness (triangulation, derived normals, extraction). Cambium’s `understory_dirty` use is for **operator-runtime caches** and UI/workflow state.

### Preview contract

Operators should support:

* **fast preview**: budgeted work, approximate where allowed, bounded output
* **final commit**: full quality, reproducible

Mechanically, a preview operator does:

1. take base mesh
2. apply edits into a temp mesh (clone first; optimize later)
3. commit the temp transaction to obtain a `ChangeSet`
4. request incremental extraction using that `ChangeSet.dirty`

### Caching

Cambium may maintain:

* derived field caches (e.g., adjacency, selection sets)
* operator-local caches keyed by:

  * mesh identity / mesh version (from Exedra)
  * params hash
  * policy hash

Caching must be explicit and invalidated via:

* Exedra `ChangeSet` (for topology/attribute-related caches)
* `understory_dirty` channels (for Cambium runtime caches)

---

## Canonical Operator Library

### v0.1 “minimum useful Cambium”

Operators that prove the stack:

1. **UV generators**

   * planar projection
   * box projection
   * cylinder projection
   * seam-driven unwrap helper (consumes seam tags / sharp edges)

2. **Selection + tagging ops**

   * mark edge as sharp
   * mark seam edges
   * assign face material/region

3. **Debug / inspect ops**

   * validate mesh and produce a report
   * dump operator artifacts (testkit)

#### v0.1 vertical slice: UV Planar Projection

The v0.1 implementation must include a complete end-to-end workflow that exercises:

* an `EditOperator` (planar UV)
* `OperatorRunner::run_preview` and `run_commit`
* `ChangeSet` generation and deterministic ordering
* incremental render extraction driven by `ChangeSet.dirty`
* golden determinism tests

##### Operator: `uv_planar`

Purpose:

* Populate/overwrite the **corner UV** layer for a selected face set (or entire mesh) using deterministic planar projection.

Params (shape):

```rust
pub struct UvPlanarParams {
    pub scope: UvScope,
    pub plane: UvPlane,
    pub scale: f32,
    pub offset: [f32; 2],
    pub write_missing_only: bool,
}

pub enum UvScope {
    WholeMesh,
    FaceSet, // provided as deterministic FaceId list
}

pub enum UvPlane {
    WorldXY,
    WorldXZ,
    WorldYZ,
    /// Use per-face plane from face normal; for determinism this must specify tie-breaking
    /// and the normal source (derived vs geometric).
    PerFaceFromGeometry,
}
```

Policy hooks:

* `UvPolicy` controls default scale/offset and whether UV writes are allowed to overwrite existing UVs.
* `QualityPolicy` may reduce scope in preview (e.g., only faces in view or only selected faces), but must remain deterministic for a given input selection.

Determinism rules:

* If `scope=WholeMesh`, iterate faces in deterministic arena order.
* If `scope=FaceSet`, the face list must be deterministically ordered (sorted by stable id order) and must not contain duplicates.

  * **Canonical FaceSet representation (v0.1):** `alloc::vec::Vec<FaceId>` that is sorted in increasing stable-id order and deduplicated.
* Corner walk for each face is deterministic: start at `Face.edge`, follow `next`.
* Any computed floats must avoid nondeterministic sources; projection must use stable math operations.

##### Deterministic tie-break for `UvPlane::PerFaceFromGeometry`

When `plane=PerFaceFromGeometry`, the operator selects a projection plane per face using a deterministic rule derived from the **geometric face normal**.

Normal computation (geometric):

* Walk corners in the deterministic face loop order.
* Use a stable triangulation method for the normal accumulator (e.g., a fan around the first vertex in loop order).
* Accumulate cross products in loop order.

Plane selection:

* Let `n = (nx, ny, nz)` be the accumulated normal.
* Compute `ax=|nx|`, `ay=|ny|`, `az=|nz|`.
* Choose the dominant axis by maximum of `(ax, ay, az)`.
* **Tie-break:** if values are equal within `ctx.numeric.normal_epsilon`, prefer **X**, then **Y**, then **Z**.
* Projection plane is chosen orthogonal to the dominant axis:

  * dominant X → project to **YZ**
  * dominant Y → project to **XZ**
  * dominant Z → project to **XY**

Degenerate faces:

* If `max(ax,ay,az) < ctx.numeric.normal_epsilon`, treat the face as degenerate and fall back deterministically to `WorldXY`.

This rule is intentionally simple and stable; higher-quality best-fit planes may be added later behind an explicit policy knob.

Dirtiness expectations:

* UV writes invalidate extraction outputs that include UVs.
* Exedra `ChangeSet.dirty` should include:

  * `dirty_faces` for all faces whose corners were updated
  * `dirty_corners` for updated corners

Operator report:

* `Stats.counters` must include:

  * `faces_processed`
  * `corners_written`
  * `corners_skipped_existing` (if `write_missing_only`)

Timing buckets:

* `select` (determine face list)
* `compute` (projection math)
* `attrs` (writing UVs)
* `validate` (optional, behind policy)

Artifacts (bounded, optional; controlled by policy):

* `FaceSet` of affected faces
* `Polyline2` preview of projected UV bounds (optional)

Golden tests:

* A small deterministic corpus of meshes:

  * triangle + quad + ngon face
  * UV seam cases (existing UVs on some corners)
  * non-planar face (to ensure per-face handling is stable)

For each corpus input:

* Run `uv_planar` with fixed params and numeric policy.
* Verify corner UV layer exactly matches golden output.
* Run Exedra extraction with UV enabled and verify TriMesh buffers match golden (ordering + values).

### v0.5 “modeling credibility”

1. Subdivision (Catmull-Clark) as an operator using Exedra topology + attributes
2. Bevel-like workflows (may be multi-stage + policy-driven)
3. Better region ops (loop selection, flood fill by face tag)
4. UV seam workflows that integrate with subdivision

---

## UV Guidance (Cambium-owned)

Cambium provides pragmatic UV utilities:

* planar: choose plane (world axis, face normal, best-fit)
* box: 6 projections with deterministic choice
* cylinder: axis + seam direction
* seam unwrap helper:

  * take seam edges (or sharp edges as fallback)
  * cut corner UV continuity along seams
  * relax optionally later (not required v1.0)

Output is written into Exedra’s **corner UV** attribute layer.

---

## Boolean Orchestration (Cambium + Exedra)

Exedra provides: `Mesh::boolean(a, b, op, params)`.

Cambium provides:

* preprocessing for workflows:

  * region restriction (operate only where AABBs overlap)
  * staging: “preview boolean” (fast) vs “commit boolean” (robust)
* postprocessing:

  * tagging result patches (inside/outside, source id)
  * cleanup steps (remove tiny components, etc., policy-driven)

Cambium must keep boolean failure artifacts visible and debuggable.

---

## Diagnostics, Artifacts, and Debug Dumping

Every operator can emit:

* meshes (intermediate or final)
* polylines / point clouds (intersection segments, guides)
* selection sets
* scalar fields (weights, distances)

Artifacts must be:

* **bounded** (size limits)
* **deterministically ordered**
* **streamable** (for huge meshes, dump to files via testkit / `std`)

---

## Wind Tunnel Scenarios (Cambium)

Cambium wind tunnels should target:

* interactive preview performance
* incremental extraction effectiveness
* operator pipeline composition overhead

Examples:

* **CWT-1 UV Planar on 500k faces**
* **CWT-2 UV Box on 500k faces**
* **CWT-3 Subdivision level 1 on 200k faces**
* **CWT-4 Boolean preview vs commit**

---

## Determinism Contract (Cambium)

Given identical:

* input meshes
* operator params
* policy set
* numeric policy

Cambium produces identical:

* mesh topology
* attributes (including UVs)
* operator artifact ordering (when applicable)
* reports (counters and stats; timing is not deterministic)

---

## Milestone Plan

### v0.1 — Operator runtime + UV planar generator + tagging + reports

Exit criteria:

* Can run operator pipeline on a mesh and get:

  * deterministic planar UV generation into corner layer
  * edge sharpness & seam tagging
  * validation report output
* Preview vs commit is implemented via `OperatorRunner`.
* Debug artifacts can be dumped and reloaded (via testkit).
* Golden determinism tests exist for:

  * UV output stability
  * TriMesh extraction ordering stability (via Exedra)

### v0.5 — Subdivision + real modeling operators

Exit criteria:

* Catmull-Clark operator produces stable outputs and preserves UV seams via corner domain.
* Extrude/inset exists for a basic region selection model.
* Wind tunnels cover preview vs commit.

### v0.9 — Boolean orchestration + workflow polish

Exit criteria:

* Cambium provides staged boolean workflows (preview/commit).
* Failure artifacts are surfaced cleanly.

### v1.0 — Production operator layer

Exit criteria:

* Stable operator API, stable policy model, and a documented operator library.
* Clear separation between Cambium operators and Exedra kernel.

---

## v0.1 Implementation Plan

This section is an implementation-driving checklist for shipping Cambium v0.1. It is intentionally concrete (files, modules, vertical slice).

### Scope

Ship a minimal Cambium runtime that can:

* run an `EditOperator` in **commit** mode on `&mut exedra::Mesh` and return `(ChangeSet, OpReport)`
* run the same operator in **preview** mode by cloning the mesh and returning `(preview_mesh, OpReport)`
* record **Stats** (deterministic) and **Timings** (best-effort)
* emit **Diagnostics** and bounded **Artifacts**
* track Cambium-local cache dirtiness with **`understory_dirty` channels**

And one canonical operator: `uv_planar`.

### Module and file skeleton (`crates/cambium/`)

Create these modules first:

* `lib.rs`
* `context.rs` — `OpContext`, `Scratch`, `Clock`
* `report.rs` — `OpReport`, `Stats`, `Timings`, bucket helpers
* `diag.rs` — `DiagnosticsSink`, `Diagnostic`, ordering + bounds
* `artifact.rs` — `Artifacts`, `Artifact`, bounding rules
* `error.rs` — `OpError`, `OpErrorKind`, wrap/map helpers
* `policy.rs` — `PolicySet` and sub-policies
* `dirty.rs` — `understory_dirty` integration and channel definitions
* `runner.rs` — `OperatorRunner`, `run_preview`, `run_commit`
* `ops/mod.rs` — operator module
* `ops/uv_planar.rs` — v0.1 operator

Recommended `std`-only crates:

* `crates/cambium_testkit/` — fixtures, golden snapshot formats, debug dumps
* `crates/cambium_wind_tunnel/` — perf scenarios

### `understory_dirty` channels (v0.1)

Define a small fixed channel enum in `dirty.rs`. Start minimal; expand only with justification.

Recommended initial channels:

* `Selection` — selection/regions caches invalid
* `Adjacency` — adjacency/one-ring caches invalid
* `UvDerived` — Cambium UV-related derived caches invalid (not the authored UV layer)
* `OperatorCache` — generic operator-local caches

Rules:

* Channels are defined centrally (no ad-hoc per-operator channel creation).
* Channel additions must document memory impact.

### OperatorRunner behavior

#### Commit path (`run_commit`)

1. Clear scratch (`ctx.scratch.clear()`) at the start of every run (locked for v0.1).
2. Create a transaction: `let mut txn = mesh.begin();`
3. Call `op.apply(&mut txn, params, ctx)`.
4. Commit transaction to obtain `ChangeSet`.
5. Optional validation if `policy.validate.validate_on_commit`.
6. Return `OpResult { change_set, report }`.

#### Preview path (`run_preview`)

1. Clone the input mesh into a temporary preview mesh (`let mut preview = mesh.clone()`), v0.1.
2. Run the same commit path against `preview`.
3. Return `(preview, report)`.

Notes:

* Preview must still use `Txn` + `commit` on the preview mesh so operator code stays identical and `DirtySet` semantics are exercised.

### Timing buckets

* Counters/Stats are deterministic.
* Timings are best-effort. In `no_std` builds, timings may be zero or use a platform hook.

Runner-level buckets (recommended):

* `op.apply`
* `txn.commit`
* `validate` (optional)

Operator-level buckets (recommended):

* `select`
* `compute`
* `edit`
* `attrs`

Rules:

* Operators must not create unbounded numbers of timing buckets.

### Budget determinism rules

`WorkBudget` exists primarily to make preview runs predictable.

* Deterministic truncation must be implemented using `max_faces` / `max_corners`.

  * Operators must check these limits at deterministic checkpoints (per-face boundary, or per-N corners in a fixed stride).
* `max_millis` is advisory and may cause nondeterministic early exit; it should not be used as the sole budget mechanism when determinism matters.
* When a budget is exceeded, operators should return `OpErrorKind::BudgetExceeded` and may emit partial artifacts/diagnostics (bounded and deterministically ordered).

### Bounded diagnostics and artifacts

Enforce boundedness at the sink level:

* `DiagnosticsSink` enforces `LimitsPolicy.max_diagnostics`.

  * Overflow handling is deterministic and **severity-aware**:

    * retain all `Error` diagnostics first, then `Warn`, then `Note`
    * within each level, retain earliest insertion order
    * truncate at `max_diagnostics` after applying this ordering
  * Severity-aware retention is part of the determinism contract: for identical diagnostic emission,
    the retained set is identical.
* `Artifacts` enforces `LimitsPolicy.max_artifact_items` and `LimitsPolicy.max_artifact_bytes`.

  * Overflow handling is deterministic (keep earliest by stable insertion order).

Artifact byte accounting (v0.1):

* For `Vec<T>`-backed artifacts, estimate bytes as `len * size_of::<T>()`.
* For `FieldF32`, estimate as `values.len() * size_of::<f32>()`.
* For `Mesh` artifacts, v0.1 applies **item-count** limits only (byte estimate = 0) until
  Exedra exposes a stable `estimate_bytes()` API.

### v0.1 vertical slice checklist: `uv_planar`

Implement `ops/uv_planar.rs` according to the **v0.1 vertical slice specification** defined earlier in this document under:

* **Canonical Operator Library → v0.1 vertical slice: UV Planar Projection → Operator: `uv_planar`**

Checklist summary:

* deterministic face iteration:

  * `WholeMesh`: arena order
  * `FaceSet`: sorted stable id order
* deterministic corner iteration: start at `Face.edge`, follow `next`
* deterministic projection:

  * world planes are trivial
  * per-face plane selection uses a documented tie-break rule
* writes into Exedra corner-UV layer
* produces:

  * `faces_processed`, `corners_written`, `corners_skipped_existing`
  * timing buckets: `select`, `compute`, `attrs`
  * optional bounded artifacts (policy-controlled)

### Golden tests and corpus (`cambium_testkit`)

Build an initial small corpus of meshes constructed programmatically (v0.1) and snapshot:

* corner UV layer output in deterministic traversal order
* extracted TriMesh buffers (indices, positions, uvs) with UV enabled

Golden snapshot rules:

* ordering must be deterministic and documented
* do not include timings in goldens

---

## Canonical selection representation (v0.1)

Selection sets and regions will evolve, but v0.1 locks a single canonical representation used in operator params and artifacts.

* A **face selection** is represented as a `Vec<FaceId>` that is:

  * sorted in increasing stable-id order
  * deduplicated
* Operators that accept a face selection must either:

  * require callers to pass canonical selections, or
  * canonicalize internally (sort + dedup) before use.

Rationale:

* Deterministic ordering is guaranteed.
* Memory overhead is predictable.
* Works well for small/medium selections and is a good baseline even for large meshes.

Future work may introduce compressed sets or bitsets, but the deterministic ordering rule remains.

---

## Feature flags and `std` posture (v0.1)

Cambium should compile in `no_std + alloc`, but v0.1 defaults to `std` for ease of testing and profiling.

Recommended feature layout:

* `default = ["std"]`
* `std`:

  * enables real wall-clock timing in `Clock` using `std::time::Instant`
  * enables file IO in `cambium_testkit`

Rules:

* `hashbrown` is permitted in both `std` and `no_std + alloc` builds.
* Timing is best-effort; when `std` is disabled, `Clock` may record zeros.

---

## Mapping Exedra ChangeSets into Cambium dirtiness

Cambium uses `understory_dirty` for operator-runtime caches and UI/workflow state. Exedra remains the source of truth for mesh-derived invalidation.

Rules (v0.1):

* After `run_commit`, Cambium may translate `change_set.dirty` into **coarse** Cambium channels as needed:

  * if any faces/corners are dirty and the operator touches UVs → mark `DirtyChan::UvDerived`
  * if topology edits occurred (created/deleted ids) → mark `DirtyChan::Adjacency`
* This mapping must be deterministic and conservative.
* Do not introduce per-element Cambium dirty marks unless a cache demonstrably benefits.

---

## Geometry Nodes-inspired direction (non-binding)

Cambium is not required to match Blender Geometry Nodes, but it is useful as a north-star for *what kinds of workflows users expect* from a procedural/operator layer.

Cambium should be compatible with a future node-graph runtime by keeping these concepts in mind:

* **Operator graphs**: nodes with typed inputs/outputs; deterministic evaluation order.
* **Selections as data**: face/edge/vertex selections can be produced/consumed by operators (v0.1 uses canonical `Vec<FaceId>`; later may add compressed forms).
* **Fields**: lazy/virtual per-element values (e.g., per-face weight, per-corner UV source) that can be sampled during an operator.
* **Instances**: higher-level scene instancing belongs above Exedra; Cambium may produce instancing descriptors as artifacts/outputs.
* **Preview vs commit**: maps naturally to Geometry Nodes “viewport preview” vs “final evaluation.”

### Field placeholder (non-binding)

A **Field** is a deterministic value source that can be sampled per element without requiring an eagerly materialized attribute layer.

Conceptual shape:

* `Field<T>::Constant(T)` — uniform value
* `Field<T>::Attr(exedra::AttrKey<T>)` — read from an Exedra attribute layer
* `Field<T>::EvalFn(...)` — computed deterministically from mesh + element id

Fields are **not** required for v0.1. This exists to keep operator parameter design compatible with later procedural workflows.

A practical roadmap mapping:

* v0.1: runner + planar UV + canonical selections + deterministic reports
* v0.5: add subdivision + region ops + seam workflows (enables many GN-style stacks)
* v0.9+: consider a `cambium_nodes` adapter crate that:

  * wraps `EditOperator` as nodes
  * provides deterministic graph scheduling
  * supports caching keyed by (mesh version, node id, params hash)

This section is intentionally non-binding; it is guidance for later design, not a v0.1 requirement.

---

## Open Questions (explicitly tracked)

* Do we keep an operator DAG runtime in v0.1, or a simple runner + composition helpers first?
* How do we represent “regions” beyond face lists (e.g. edge loops, vertex groups) deterministically and efficiently?
* Do we need a Cambium-level “mesh version” counter, or do we rely only on Exedra `ChangeSet` contents?
* If we add a node-graph adapter (`cambium_nodes`), what is the minimal set of socket/field types needed to be useful without ballooning the API surface?
