# Exedra Mesh API Surface Audit

Status: updated for the mesh-operation extraction.

Compound algorithms now belong to `exedra_mesh_ops`; see its
[boundary and migration decision](../../exedra_mesh_ops/docs/adr-0001-mesh-operation-boundary.md).

## Goal

This document records the intended public API surface for the `exedra_mesh` kernel.
It is an audit artifact, not a new architecture decision: it describes the
surface that already exists and names which parts are stable entry points,
low-level extension points, or diagnostic/debugging aids.

## Public Entry Points

Most callers should use the crate root re-exports:

- `Mesh`, `MeshBuilder`, `MeshBuildResult`, `BuildParams`, and
  `FaceBuildAttrs` for construction and mesh ownership.
- `Remap` for translating source IDs after explicit `Mesh::compact` calls.
- `VertexId`, `HalfEdgeId`, `CornerId`, `FaceId`, and `Id` for stable handles.
- `EditSession`, `ChangeSet`, `ChangeSetBuilder`, `ChangeSink`,
  `DiscardChanges`, `DirtySet`, `DeletePolicy`, and `PropagatePolicy` for
  explicit edit scopes and change tracking.
- `ExtractParams`, `ExtractMode`, `TriMesh`, `ExtractStats`, `NormalParams`,
  `NormalWeightMode`, `NormalsSource`, and `DerivedCornerNormals` for render
  extraction and normal behavior.
- `ExtractAttribute`, `AttributeStream`, `AttributeBuffer`, `AttributeValue`,
  `AttributeKind`, and `StreamValue` for extra render-vertex streams carried
  from attribute layers.
- `UvSource`, `BoxPlane`, `DEFAULT_BOX_NORMAL_EPSILON`, `dominant_box_plane`,
  `project_box_position`, and `project_corner_box` for the extraction UV
  policy and the box projection shared with the `uv.box` operator.
- `NumericPolicy` for explicit numeric tolerances.
- `attr` for built-in attribute keys.
- `attributes` for typed custom attribute storage.
- `op` for public topology and attribute mutation functions.

The crate root is the preferred import path. Public modules remain available so
rustdoc can group related concepts, but the root exports are the compatibility
surface callers should rely on first.

## Low-Level Public Surface

The following types are intentionally public even though most application code
should not construct them directly:

- `Arena<T>` is available as a small stable-handle arena for advanced callers
  and as part of the kernel's documented stable-ID model. `Mesh` does not expose
  its internal arenas.
- `Vertex`, `HalfEdge`, and `Face` are the topology record shapes used by the
  half-edge model. They are exposed for diagnostics, tests, and documentation of
  invariants; mesh mutation still goes through `Mesh`, `EditSession`, and `op`.
- `FaceLoopIter` is the named iterator returned by `Mesh::face_loop`.
- Error enums and selected-patch result structs are public so callers can handle
  validation, construction, and topology-query failures without string parsing.

## Stability Notes

- Stable IDs include an index and generation. Deletion creates tombstones;
  compaction is explicit through `Mesh::compact` and returns a `Remap` rather
  than changing IDs invisibly.
- `FaceId::OUTSIDE` is a sentinel, not a stored face arena entry.
- `Remap` maps only live source IDs with the matching generation. Deleted and
  stale IDs return `None`; `FaceId::OUTSIDE` maps to itself.
- `CornerId` is an alias for `HalfEdgeId`; corner-domain attributes are keyed by
  directed face-loop half-edges.
- Attribute domains are `Vertex`, `Face`, and `HalfEdge`. Edge-wide built-ins
  such as seam and sharpness use the canonical half-edge representative.
- `attr::VERTEX_SHARPNESS` is a sparse vertex-domain authored override for
  subdivision corner classification; absence means derive from incident edge
  sharpness.
- Mutation APIs are eager. `ChangeSink` controls whether changes are recorded,
  not whether the mesh is mutated.
- `PropagatePolicy::split_face_diagonal_edge_attr` is the split-face-specific
  diagonal edge-sharpness policy. Its default `FromEdgePolicy` preserves v0.1
  behavior for existing `edge_attr`-based callers; set an explicit mode for new
  split-face code.
- `ExtractMode::Incremental` is reserved in v0.1 and currently behaves as a full
  rebuild.

## Audit Result

- Public kernel items are documented and covered by the workspace lint set.
- No `pub(crate)` implementation detail is reachable through the public API
  except by intentional root re-export.
- Boolean and rounding imports move to `exedra_mesh_ops`. Topology IDs and
  edit-scope semantics remain owned by the kernel.
