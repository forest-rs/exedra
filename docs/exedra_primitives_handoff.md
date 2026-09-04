# Handoff: `exedra_primitives` crate

This document is a handoff spec for implementing a new crate **`exedra_primitives`**: a small library of deterministic mesh primitive generators that produce **Exedra modeling meshes** plus **semantic selections** and **region tags**. The goal is to provide “hello world” content that immediately exercises Exedra’s invariants, validation, extraction, and (later) booleans — and that plugs naturally into Exedra Ops mesh workflows (like the Byzantine basilica ruin demo).

This is **not** Exedra core. `exedra_primitives` is tooling/support code designed to accelerate development, testing, demos, and wind tunnels.

---

## Goals

1. **Deterministic primitives**
   - Same params + seed → identical topology/attributes and stable ID ordering for the supported toolchain range.
   - Deterministic vertex/face ordering and winding.

2. **Modeling-mesh first**
   - Output is an Exedra **half-edge modeling mesh**, not just indexed triangles.
   - Geometry is suitable as input to Exedra edits, validation, extraction, and later booleans.

3. **Pipeline-friendly outputs**
   - Each primitive returns:
     - `Mesh`
     - **Region tags** (face-domain `u32` IDs) for semantic grouping
     - **Selections** (canonical FaceSets/EdgeSets) for common parts (caps, side walls, seams, rims).

4. **Validation baseline**
   - Every primitive is expected to pass `exedra_mesh::Mesh::validate_deep()` (where available) with no errors.
   - If a primitive fails validation, treat as a kernel/primitive bug.

5. **Minimal dependencies**
   - `#![no_std]` with `alloc`. `hashbrown` allowed.
   - Deterministic RNG (seeded) for any stochastic variation (rare in v0.1).

---

## Non-goals (v0.1)

- A full scene-graph or instancing system.
- A huge catalog of shapes.
- Mesh repair/healing beyond what Exedra already provides.
- Fancy parameterizations (e.g., full UV unwrapping).
- High-quality boolean-friendly tessellation for every primitive (we’ll improve as booleans mature).

---

## Crate placement and workspace layout

Create a new crate:

- `crates/exedra_primitives/`

Recommended companion crates already exist:

- `crates/exedra_testkit/` — fixtures, golden dumps (may depend on `std`)
- `crates/exedra_wind_tunnel/` — perf scenarios (may depend on `std`)

`exedra_primitives` should be usable from `exedra_testkit` and `exedra_ops_testkit` later.

---

## Rust / feature posture

- `#![no_std]` with `extern crate alloc` from the start.
- No `std` dependency needed — this crate constructs meshes from geometry, nothing more.
- `hashbrown` allowed.
- IO, debug dumping, and file-based golden tests live in `exedra_testkit`, not here.

---

## Public API shape

### Primary return type

```rust
pub struct Primitive {
    pub mesh: exedra_mesh::Mesh,

    /// Face-domain semantic tag layer (demo/tooling level).
    /// This is not required to be a built-in Exedra layer.
    pub face_region: FaceRegionLayer,

    /// Canonical selections for common sub-parts.
    pub selections: Selections,
}
```

### Region tags

Region tags are a face-domain `u32` layer with a small set of conventional IDs per primitive.

```rust
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct RegionId(pub u32);

pub struct FaceRegionLayer {
    /// One entry per live face, indexed by FaceId.
    /// Representation can be dense-with-default (recommended).
    pub default: RegionId,
    pub values: alloc::vec::Vec<RegionId>,
}
```

Notes:
- Keep this layer local to `exedra_primitives` for v0.1.
- Later, if Exedra introduces a standard face “region/material” layer, we can switch.

### Canonical selections

Selections are canonical and deterministic.

```rust
pub struct FaceSet(pub alloc::vec::Vec<exedra_mesh::FaceId>);
pub struct EdgeSet(pub alloc::vec::Vec<exedra_mesh::HalfEdgeId>);

pub struct Selections {
    /// Named face sets (e.g. "cap_top", "walls", "sides").
    pub face_sets: alloc::vec::Vec<(SelectionName, FaceSet)>,

    /// Named edge sets (e.g. "rim_top", "seam").
    pub edge_sets: alloc::vec::Vec<(SelectionName, EdgeSet)>,
}

pub struct SelectionName(pub &'static str);
```

Canonical invariants:
- all sets are **sorted by stable-id order**
- deduplicated
- names are stable `'static` identifiers (dot-separated is fine)

Examples:
- `"faces.all"`, `"faces.side"`, `"faces.cap_top"`, `"faces.cap_bottom"`
- `"edges.rim_top"`, `"edges.rim_bottom"`, `"edges.seam"`

---

## Built-in attribute usage (Exedra boundary)

Primitives must write required Exedra built-ins:

- `exedra_mesh::attr::VERTEX_POSITION` (required, dense)
- **Do not** require UVs by default. Many primitives should leave:
  - `exedra_mesh::attr::CORNER_UV` absent/missing, so Exedra Ops UV operators can be validated.

Optional:
- some primitives may emit a convenient initial UV set (behind a flag), but keep that separate from the default.

---

## Determinism requirements

For every primitive:
- vertex positions are generated in a deterministic order
- face loops use deterministic winding (pick CCW with outward normals; be consistent)
- any internal map/set iteration must not leak ordering
- if randomness is used:
  - it must be explicitly seeded (`seed: u64`)
  - use a deterministic RNG (e.g., `rand_chacha` with fixed algorithm) or a tiny custom xorshift
  - record/return the seed in the primitive result

---

## Primitive set (v0.1)

Implement the following primitives first. Each should return `Primitive`.

### 1) `quad` / `plane`

Purpose:
- smallest mesh for validation, extraction, UV planar tests.

API:

```rust
pub struct QuadParams {
    pub size: [f32; 2],
    pub centered: bool,
}

pub fn quad(params: &QuadParams) -> Primitive;
```

Selections:
- `"faces.all"` (the single face)
- `"edges.boundary"` (its boundary loop)

Regions:
- `REGION_FACE = 1`

Notes:
- Use a single ngon face (quad polygon), not two triangles, to exercise ngon triangulation.

---

### 2) `box`

Purpose:
- basic hard-surface primitive; exercises sharp edges, region tags, UV box.

API:

```rust
pub struct BoxParams {
    pub size: [f32; 3],
    pub centered: bool,
    pub segments: [u32; 3], // v0.1 may require [1,1,1]
}

pub fn box_primitive(params: &BoxParams) -> Primitive;
```

Selections (recommended):
- `"faces.all"`
- `"faces.side_x_pos"`, `"faces.side_x_neg"`, `"faces.side_y_pos"`, `"faces.side_y_neg"`, `"faces.side_z_pos"`, `"faces.side_z_neg"`
- `"edges.rim_*"` if segmenting creates interior rims (optional in v0.1)

Regions:
- one region per side face group (6 regions)

Determinism:
- fixed vertex numbering and face emission order:
  - order faces by axis and sign consistently (e.g., +X, -X, +Y, -Y, +Z, -Z)

---

### 3) `cylinder` (optionally capped)

Purpose:
- exercises seam behavior and later cylinder UV projection; good for columns/drums.

API:

```rust
pub struct CylinderParams {
    pub radius: f32,
    pub height: f32,
    pub segments: u32,     // around
    pub capped: bool,
    pub centered: bool,
}

pub fn cylinder(params: &CylinderParams) -> Primitive;
```

Selections:
- `"faces.side"`
- `"faces.cap_top"` (if capped)
- `"faces.cap_bottom"` (if capped)
- `"edges.seam"` (the 0-angle seam edge chain; deterministic)
- `"edges.rim_top"`, `"edges.rim_bottom"` (if capped)

Regions:
- side, cap_top, cap_bottom

Determinism:
- ring vertices generated for angles in increasing order
- seam always at angle 0 (first segment)

---

### 4) `uv_sphere` (preferred over icosphere for early UV testing)

Purpose:
- has poles and a seam; great for UV and normal edge cases.

API:

```rust
pub struct UvSphereParams {
    pub radius: f32,
    pub lat_segments: u32, // excluding poles
    pub lon_segments: u32,
    pub centered: bool,
}

pub fn uv_sphere(params: &UvSphereParams) -> Primitive;
```

Selections:
- `"faces.all"`
- `"edges.seam"` (lon seam chain)
- `"faces.pole_top"`, `"faces.pole_bottom"` (optional)

Regions:
- all faces region; optionally separate “pole cap” regions

Notes:
- This primitive will surface triangulation and derived normal corner cases early.

---

## Construction strategy

### Recommended approach
Build primitives using the Exedra ingestion path, then convert to polygonal half-edge as needed:

- Start from an indexed triangle mesh generator (deterministic):
  - positions: `Vec<[f32;3]>`
  - indices: `Vec<[u32;3]>`
- Use `exedra_mesh::Mesh::from_indexed_triangles(...)` to ingest.
- Optionally, as Exedra gains polygon-soup/ngon construction APIs, migrate `quad` to build as an ngon face directly.

Rationale:
- quickest path to “real mesh exists” for v0.1
- exercises ingestion and stable ID/generation logic immediately

### Region/selections mapping
After ingestion:
- classify faces into regions deterministically by geometric tests (axis-aligned box), or by construction provenance (preferred).
- collect FaceSets/EdgeSets, then **sort & dedup**.

---

## Validation expectations

Every primitive should:
- pass `validate_fast()` in all builds
- pass `validate_deep()` in test builds

If `validate_deep()` isn’t implemented yet, provide a placeholder:
- run `validate_fast()` and document that deep validation is pending

---

## Golden tests (v0.1)

Add tests in `exedra_primitives` (or `exedra_testkit`) that:

1. Build each primitive with fixed params
2. Validate it
3. Extract a `TriMesh` with UV disabled and check:
   - stable indices/positions ordering (golden)
4. Run Exedra Ops `uv_planar` later against `quad` and `box` and validate UV results (future)

Golden snapshot guidance:
- do **not** include timings
- include counts and deterministic buffers
- keep goldens small and representative

---

## Wind tunnel hooks

Provide a few “large” parameter presets (not necessarily goldens) used by wind tunnels:
- `cylinder(segments=512, capped=true)`
- `uv_sphere(lat=256, lon=512)`
- `box(segments=[64,64,64])` (later)

Expose as helper constructors so wind tunnels can call them.

---

## Implementation checklist (agent-friendly)

1. Create crate `crates/exedra_primitives/` with `Cargo.toml`.
2. Define `Primitive`, `Selections`, `FaceSet`, `EdgeSet`, and canonicalization helpers.
3. Implement `quad` (ngon if possible; otherwise triangulated but deterministic).
4. Implement `box_primitive`.
5. Implement `cylinder` (capped + uncapped).
6. Implement `uv_sphere`.
7. Add deterministic sorting/dedup for all selections.
8. Add validation tests calling Exedra validation.
9. Add minimal extraction tests (TriMesh) if Exedra extraction exists; otherwise stub until available.
10. Document region IDs and selection names in module docs.

---

## Acceptance criteria

- All primitives compile and build.
- All selections are canonical (sorted/dedup).
- Primitives pass `validate_fast()` and (where available) `validate_deep()`.
- Primitive outputs are stable across runs.
- The returned semantic selections are sufficient to drive early Exedra Ops mesh operators (UV projection, sharpness tagging, region selection).
