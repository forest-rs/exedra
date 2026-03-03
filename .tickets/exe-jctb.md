---
id: exe-jctb
title: Ngon/polygon construction API (MeshBuilder + add_face)
status: closed
deps: [exe-cbv1, exe-8hfg, exe-mid7]
links: []
created: 2026-03-03T07:06:36Z
type: Ngon/polygon construction API (MeshBuilder + add_face)
priority: P1
assignee: Bruce Mitchener
---
# Ngon/polygon construction API (MeshBuilder + add_face)

Provide a MeshBuilder (or from_polygons constructor) that accepts arbitrary polygon faces as loops of vertex IDs, not just indexed triangles. Required for real quad faces in exedra_primitives (quad primitive, cylinder caps) and for any operator that creates non-triangular faces.

**Critical: this is ngon construction, not polygon soup ingestion.** The builder must produce real ngon faces in the half-edge mesh — a quad is one 4-sided face with a 4-edge loop, NOT two triangles. A cylinder cap with N segments is one N-gon face, not a triangle fan. The builder must never silently triangulate input polygons.

## Design

### API: builder-local vertex indices

Face loops use **builder-local `u32` indices**, not `VertexId`. Callers don't have Exedra IDs until `build()` completes.

```
MeshBuilder::push_vertex(pos: [f32; 3]) -> u32   // builder-local index
MeshBuilder::add_face(loop: &[u32])               // indices into vertex list
MeshBuilder::build() -> Result<Mesh, BuildError>   // final mesh with Exedra IDs
```

Convenience wrapper: `Mesh::from_polygons(positions: &[[f32; 3]], face_loops: &[&[u32]]) -> Result<Mesh, BuildError>`.

Vertex-to-`VertexId` mapping is implicit by insertion order. Face-to-`FaceId` mapping follows face insertion order.

### Provenance mapping (for primitives and selections)

`build()` returns the mesh plus a provenance mapping so callers can identify which Exedra IDs correspond to their input:

- builder face index `i` → `FaceId`
- builder vertex index `i` → `VertexId`
- face loop edge `(v_i, v_{i+1})` → `HalfEdgeId`

This makes region tagging and selection construction in `exedra_primitives` trivial without geometric re-detection. Can be a separate `BuildResult` struct or optional callback.

### Face loop validation

Each `add_face` call must validate:
- Loop length ≥ 3
- No repeated vertices within a face loop (reject degenerate/bowtie loops)
- No zero-length edges (`v_i == v_{i+1}`)

Winding convention: CCW with outward normals. Wrong winding produces inside-out faces but topology remains consistent — treat as user error, not a builder error in v0.1.

### Edge matching and twin assignment

Each consecutive pair `(v_i, v_{i+1})` in a face loop creates a directed half-edge. Twin matching uses undirected edge keys:

- `key = (min(a, b), max(a, b))` where `a`, `b` are builder vertex indices
- If the same undirected key is encountered once in each direction → twins
- Same undirected key encountered twice in the **same direction** → non-manifold, error
- Same undirected key encountered more than twice → non-manifold, error

Internal matching may use a hash map keyed by `(min, max)`; iteration order of the map must not leak into output. External ordering comes from face/vertex insertion order only.

### Boundary half-edge creation and OUTSIDE loop stitching

After all interior twins are assigned, every unmatched interior half-edge `h` (going `v_i → v_{i+1}`) produces one OUTSIDE boundary half-edge `b`:

- `b.twin = h`, `h.twin = b`
- `b.face = FaceId::OUTSIDE`
- `b.to = from(h)` (i.e., `v_i` — the reverse direction)

OUTSIDE boundary half-edges are linked into closed loops via `next` pointers:

- `next(b)` is the boundary half-edge whose `to` equals `from(b)` and continues the boundary cycle consistently
- If multiple candidates exist at a vertex → non-manifold input, error
- Loop construction order is deterministic: start from smallest boundary half-edge ID not yet linked

### Relationship to `from_indexed_triangles`

v0.1: both paths coexist independently. No requirement that IDs match across the two construction paths — only that each is deterministic within itself and produces topologically/geometrically equivalent results for equivalent input. Later we may unify them, but that is not a v0.1 goal.

## Acceptance Criteria

### Core ngon construction
- A single quad (4 vertices, 1 face loop of length 4) produces a mesh with 1 face of degree 4 — not 2 triangles
- A box (8 vertices, 6 quad face loops) produces a mesh with 6 faces each of degree 4
- A cylinder cap (N vertices, 1 face loop of length N) produces 1 face of degree N
- Mixed-degree input (triangles + quads + ngons in one mesh) works correctly

### Boundary loop smoke tests
- Single quad → exactly 1 boundary loop (4 OUTSIDE half-edges)
- Box (6 quads, closed) → no boundary loops
- Cylinder side without caps → 2 boundary loops (top rim, bottom rim)
- Cylinder with cap ngons → no boundary loops (closed manifold), cap face degree == N

### Validation and errors
- All built meshes pass `validate_fast()`
- Deterministic output for identical input
- Non-manifold input (duplicate directed edge, >2 faces sharing edge) returns structured error
- Degenerate face loops (length < 3, repeated vertices, zero-length edges) return structured error

### Provenance
- Caller can map builder face index → FaceId after build
- Caller can map builder vertex index → VertexId after build


## Notes

**2026-03-03T11:19:40Z**

Implemented MeshBuilder-based polygon/ngon construction with builder-local indices: push_vertex, add_face validation, and build() returning MeshBuildResult provenance (vertex_ids, face_ids, face_edge_ids). Added Mesh::from_polygons convenience wrapper. Added structured InvalidFaceLoop errors (TooShort, RepeatedVertex, ZeroLengthEdge, IndexOutOfBounds), deterministic twin matching, OUTSIDE boundary creation/stitching, and vertex.out initialization. Added acceptance-oriented tests for quad/ngon face degree, mixed-degree input, empty input, non-manifold detection, validation errors, box closed manifold, open-cylinder boundary loops, provenance mapping, and existing constructor invariants. Validation run: cargo fmt --all; cargo clippy --workspace --all-targets --all-features -- -D warnings; cargo test --workspace --all-features; cargo doc --no-deps.
