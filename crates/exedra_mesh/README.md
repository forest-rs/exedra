# exedra_mesh

Structural half-edge mesh kernel.

Exedra Mesh is the production-capable, `#![no_std]` polygonal mesh core in this
workspace. It owns topology, stable IDs, typed attributes, validation, edit
sessions, explicit compaction, and deterministic render extraction. Higher-level
modeling workflows live in [`exedra_ops`](https://crates.io/crates/exedra_ops); analytic, implicit, and
primitive generation live in sibling crates.

## Guarantees and Non-goals

Exedra Mesh guarantees deterministic traversal and output for fixed mesh state:
vertices/faces iterate in stable arena slot order, fan triangulation is stable,
and `Mesh::to_trimesh` appends render vertices at first encounter. Stable IDs
carry index + generation so stale handles are rejected after deletion/reuse.

Exedra Mesh does not own scene graphs, materials, units, UI workflows, or exact
CAD surfaces. It also does not compact IDs implicitly; call `Mesh::compact`
when a tombstone-free copy and `Remap` are needed.

## Edge finishing

`round_sharp_edges` selects authored sharp edges. `round_edges` accepts an
explicit transient edge set and returns `RoundResult`: work counters and the
input faces responsible for each replacement, strip, or corner patch. Twins
and duplicate targets are canonicalized; failures leave the input mesh
byte-identical. Stable construction targets belong in the caller, not in mesh IDs.

Both operations author radial fillet normals and retain hard chamfer/end
boundaries. Extract with `NormalsSource::CustomOrDerived`. Unchanged faces keep
all attributes; rewritten faces preserve their regions and valid authored
normals at surviving corners. New faces use the requested region or the first
source face's region. Source faces are listed in ascending input-ID order.

Surviving corners keep their exact UVs. New corners interpolate the source
face's robust triangulation; bands and patches project onto the first source
face's chart, matching material ownership. Textures stretch toward perpendicular
tangencies and can fold on surfaces turning beyond them.
Different charts meet at explicit edge seams. Incomplete or non-finite source
charts supply no new UVs. Untextured inputs remain untextured; callers can use
the face provenance to apply a different mapping after finishing.

Explicit segments must be in `1..=256` and control both strip bands and corner
radial layers. Otherwise chord tolerance controls the arc and triangle surfaces,
including corner interiors. A tolerance requiring more than 256 bands or layers
is an error. Finer spherical patches cost more triangles; use an explicit count
or a coarser tolerance when that tradeoff suits the caller.

Radius and clearance decisions use the stored mesh coordinates. Faces that
collapse at final f32 precision, rewritten faces that reverse orientation, and
edge trims that cross are refused. Convex trihedral corners
and gently turning chains are supported. Adjacent edges sharing a planar
flank and equal dihedral angles meet at an exact miter, preserving the setback
or radius on both edges. Set `max_tangent_turn` to `FRAC_PI_2` for square rims;
the default remains 0.7 radians. Fillet miters keep a crease between cylinders,
and automatic band counts account for their elliptical seam curves.

Straight concave chains between planar flanks and perpendicular planar end caps
also support fillets and chamfers.
These fill an internal corner with material; fillet normals point toward the
cylinder center in the void. Collinear chain subdivisions and triangulated end
caps are supported. The existing cap faces retain their ownership and UVs; added
cap triangles extend the first incident cap face's chart and material. Concave
turns, closed rings, and junctions with other selected chains are refused.
Separate convex and concave chains can finish together in one atomic pass.

Concave clearance uses the swept triangle between the original corner and its
two tangencies. It conservatively refuses obstructions even just beyond the
fillet arc. End tangencies must fit before the next existing cap-boundary vertex;
finishing does not dissolve collinear Boolean subdivisions to gain clearance.

Migration for concave finishing: call signatures and policy encoding are
unchanged. Previously refused straight concave selections now add material;
`ConcaveEdge` identifies the remaining unsupported turns and junctions.

Migration: existing `round_sharp_edges` calls retain their return type. Use
`round_edges` when selection or provenance is needed, and choose
`CustomOrDerived` to consume the new normal overrides. Callers that relied on
silently clamped band counts must choose a supported count or tolerance.
Exhaustive `RoundError` matches must handle the new `InvalidEdge` variant.

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
- `boolean`, `BooleanBvh`, `BooleanScratch`: staged boolean broad-phase
  candidate discovery.
- `EditSession`, `ChangeSet`, `DirtySet`, `PropagatePolicy`: edit hosting and
  change reporting.
- `ExtractParams`, `TriMesh`, `ExtractStats`: render extraction.

## Design

- [API surface](https://github.com/forest-rs/exedra/blob/main/crates/exedra_mesh/docs/api-surface.md) — the audited mesh-kernel boundary.
- [Design briefs](https://github.com/forest-rs/exedra/tree/main/crates/exedra_mesh/docs/briefs) — focused decisions on specific topics
  (boundary model, determinism, attribute storage, etc.).
- [ADRs](https://github.com/forest-rs/exedra/tree/main/crates/exedra_mesh/docs) — architectural decision records.

## License

Licensed under either of Apache License 2.0 or MIT license at your
option. See the workspace root for license files.
