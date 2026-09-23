# exedra_mesh

Structural half-edge mesh kernel.

Exedra Mesh is the production-capable, `#![no_std]` polygonal mesh core in this
workspace. It owns topology, stable IDs, typed attributes, validation, edit
sessions, explicit compaction, and deterministic render extraction. Higher-level
modeling algorithms live in [`exedra_mesh_ops`](../exedra_mesh_ops/README.md); analytic, implicit, and
primitive generation live in sibling crates.

## Guarantees and Non-goals

Exedra Mesh guarantees deterministic traversal and output for fixed mesh state:
vertices/faces iterate in stable arena slot order, fan triangulation is stable,
and `Mesh::to_trimesh` appends render vertices at first encounter. Stable IDs
carry index + generation so stale handles are rejected after deletion/reuse.

Exedra Mesh does not own scene graphs, materials, units, UI workflows, or exact
CAD surfaces. It also does not compact IDs implicitly; call `Mesh::compact`
when a tombstone-free copy and `Remap` are needed.

For checks on the actual render buffers, call `TriMesh::validate_geometry()`.
It rejects incomplete or out-of-bounds triangles, nonfinite positions and exactly
zero-area triangles, with the offending buffer index in `TriMeshGeometryError`.
Exact predicates preserve tiny valid triangles and detect collapse at stored
`f32` precision. The check is additive and opt-in; extraction behavior is
unchanged. It does not certify a closed solid, winding, intersections or render
attributes, and empty geometry passes.

Compound Boolean and edge-finishing algorithms live in
[`exedra_mesh_ops`](../exedra_mesh_ops/README.md). See its
[boundary and migration note](../exedra_mesh_ops/docs/adr-0001-mesh-operation-boundary.md).

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
  when corner UVs or corner normals differ. `ExtractParams::normals`
  (`NormalsSource`) and `ExtractParams::uvs` (`UvSource`) choose what corners
  without authored data emit. `UvSource::CustomOrBoxProjected { scale }`
  box-projects each unauthored corner on its face's dominant-axis plane with
  the same math as the `uv.box` operator; authored UVs are never overwritten,
  and the default `CustomOnly` keeps the historical zero fallback.
- **Edit sessions**: public mutation goes through `op::*` functions applied to
  an eager `EditSession`; optional `ChangeSet`/`DirtySet` output supports
  incremental consumers.
- **Numeric policy**: `NumericPolicy` centralizes tolerances for geometry
  operations that need near-equality decisions.

## Example

```rust
use exedra_mesh::{BuildParams, ChangeSetBuilder, ExtractParams, Mesh, op};

fn main() -> Result<(), exedra_mesh::BuildError> {
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
- `op`: public kernel mutation surface.
- `EditSession`, `ChangeSet`, `DirtySet`, `PropagatePolicy`: edit hosting and
  change reporting.
- `ExtractParams`, `TriMesh`, `ExtractStats`: render extraction.
- `ExtractAttribute`, `AttributeStream`, `AttributeBuffer`: extra render-vertex
  streams (second UV set, colors, caller-defined data) carried from attribute
  layers, and `TangentUv` for MikkTSpace tangents.
- `NormalsSource`, `UvSource`, `dominant_box_plane`, `project_corner_box`:
  extraction policies and the shared box projection behind `UvSource`.

## Design

- [API surface](https://github.com/forest-rs/exedra/blob/main/crates/exedra_mesh/docs/api-surface.md) — the audited mesh-kernel boundary.
- [Design briefs](https://github.com/forest-rs/exedra/tree/main/crates/exedra_mesh/docs/briefs) — focused decisions on specific topics
  (boundary model, determinism, attribute storage, etc.).
- [ADRs](https://github.com/forest-rs/exedra/tree/main/crates/exedra_mesh/docs) — architectural decision records.

## License

Licensed under either of Apache License 2.0 or MIT license at your
option. See the workspace root for license files.
