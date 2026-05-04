# exedra

Structural half-edge mesh kernel.

Exedra is the production-capable, `#![no_std]` polygonal mesh core in this
workspace. It owns topology, stable IDs, typed attributes, validation, edit
sessions, explicit compaction, and deterministic render extraction. Higher-level
modeling workflows live in [cambium](../cambium/); analytic, implicit, and
primitive generation live in sibling crates.

## Guarantees and Non-goals

Exedra guarantees deterministic traversal and output for fixed mesh state:
vertices/faces iterate in stable arena slot order, fan triangulation is stable,
and `Mesh::to_trimesh` appends render vertices at first encounter. Stable IDs
carry index + generation so stale handles are rejected after deletion/reuse.

Exedra does not own scene graphs, materials, units, UI workflows, or exact CAD
surfaces. It also does not compact IDs implicitly; call `Mesh::compact` when a
tombstone-free copy and `Remap` are needed.

## Core Concepts

- **Half-edge topology**: every edge has two directed half-edges. Boundary
  twins are explicit records whose face is `FaceId::OUTSIDE`.
- **Corners**: `CornerId == HalfEdgeId`, so UVs and normal overrides can be
  authored per face corner without splitting topology vertices.
- **Attributes**: vertex, face, half-edge/corner domains with dense required
  layers and sparse authored overlays.
- **Seams and sharpness**: edge-wide tags are stored on the canonical
  half-edge representative for each undirected edge.
- **Render extraction**: `Mesh::to_trimesh` triangulates polygonal faces with a
  stable fan and splits a shared topology vertex into multiple render vertices
  when corner UVs or corner normals differ.
- **Boolean broad phase**: `BooleanBvh` reports deterministic AABB-overlap
  candidate pairs over fan-triangulated mesh faces.
- **Edit sessions**: public mutation goes through `op::*` functions applied to
  an eager `EditSession`; optional `ChangeSet`/`DirtySet` output supports
  incremental consumers.
- **Numeric policy**: `NumericPolicy` centralizes tolerances for geometry
  operations that need near-equality decisions.

## Example

```rust
use exedra::{BuildParams, ChangeSetBuilder, ExtractParams, Mesh, op};

fn main() -> Result<(), exedra::BuildError> {
    let mut mesh = Mesh::from_indexed_triangles(
        &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        &[[0, 1, 2]],
        &BuildParams::default(),
    )?;
    let face = mesh.faces().next().expect("triangle face");
    let corner = mesh.face_loop(face).next().expect("triangle corner");

    let mut edit = mesh.edit_with(ChangeSetBuilder::new());
    op::set_corner_uv(&mut edit, corner, [0.0, 0.0]).expect("corner is live");
    let changes = edit.finish();
    assert!(changes.dirty.has_dirty_corners());

    let (triangles, stats) = mesh.to_trimesh(&ExtractParams::default());
    assert_eq!(triangles.indices.len(), 3);
    assert_eq!(stats.triangle_count, 1);
    Ok(())
}
```

## Key APIs

- `Mesh`, `MeshBuilder`, `BuildParams`: construction and ownership.
- `VertexId`, `HalfEdgeId`, `CornerId`, `FaceId`: stable handles.
- `attr` and `attributes`: built-in and custom typed attribute layers.
- `op`: public kernel mutation catalog.
- `boolean`, `BooleanBvh`, `BooleanScratch`: staged boolean broad-phase
  candidate discovery.
- `EditSession`, `ChangeSet`, `DirtySet`, `PropagatePolicy`: edit hosting and
  change reporting.
- `ExtractParams`, `TriMesh`, `ExtractStats`: render extraction.

## Design

- [Handoff spec](../../docs/exedra_handoff.md) — comprehensive design
  document covering topology, attributes, invariants, APIs, and
  milestone plan.
- [Design briefs](docs/briefs/) — focused decisions on specific topics
  (boundary model, determinism, attribute storage, etc.).
- [ADRs](docs/) — architectural decision records.

## License

Licensed under either of Apache License 2.0 or MIT license at your
option. See the workspace root for license files.
