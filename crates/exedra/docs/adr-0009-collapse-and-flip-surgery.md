# ADR-0009: In-place collapse/flip surgery, link condition, and merge semantics

## Status

Accepted (exe-h2rh).

## Context

`collapse_edge` and `flip_edge` complete the kernel's topology-surgery
ops next to `split_edge`/`split_face`. They are the shared
prerequisite for boolean seam cleanup, mesh edge rounding, and
subdivision dart handling, so they must be head-agnostic kernel ops
driven through edit sessions. The ticket's bar was explicit: only
implement if correctness can be proven.

## Decision 1: in-place surgery, not delete-and-rebuild

Both ops mutate the half-edge structure directly instead of deleting the
neighborhood and re-adding substituted faces through
`delete_faces`/`add_face`.

A delete-and-rebuild prototype failed on legal inputs for a structural
reason: the intermediate states are not manifold even when the final
state is. Deleting a vertex's whole star can split a boundary wing
vertex's fan into two arcs (`delete_faces` rejects this as
`BoundaryContinuationAmbiguous`), and re-adding faces mid-fan can pinch
outside loops (the incremental stitcher panics on pinched boundary
vertices). In-place surgery constructs the final state directly, so no
transient state ever exists, every precondition failure happens before
any mutation (failed ops leave the mesh byte-identical), and surviving
faces, corners, and edges keep their identities — which is also what
makes attribute propagation exact.

`flip_edge` re-aims the edge's two half-edges as the new diagonal and
relinks six `next` pointers; nothing is created or deleted.
`collapse_edge` re-aims every half-edge into the removed vertex,
shrinks polygon faces along the edge by one loop vertex, removes
degenerate triangles while fusing their side-edge twins, splices
outside loops across the dying half-edges, and removes the dead
entities.

## Decision 2: collapse legality (the link condition, operationally)

`collapse_edge` accepts an edge exactly when all of these hold, checked
read-only before any mutation:

- **Liveness**: the half-edge and its twin are live
  (`HalfEdgeNotLive`).
- **Boundary pinch**: if both endpoints are boundary vertices, the edge
  itself must be a boundary edge (`BoundaryPinch`). Collapsing an
  interior edge between two boundary vertices would merge two boundary
  arcs into a non-manifold vertex.
- **Face pinch**: no face incident to the removed vertex may end up
  visiting the merged vertex twice — faces containing both endpoints
  non-adjacently are rejected (`FaceWouldPinch`).
- **Link condition, as edge multiplicity**: for every undirected edge
  of the merged neighborhood, the number of incident interior faces
  after the collapse (merged loops plus untouched faces) must be at
  most two (`LinkConditionViolated`). This is the classical
  `link(a) ∩ link(b) = link(ab)` vertex condition generalized to
  polygon faces, checked on final multiplicities rather than on
  simplicial links.
- **Degenerate shell**: no merged face may duplicate the vertex set of
  another face at the survivor (`DegenerateShell`). This is the edge
  part of the simplicial link condition; it rejects collapsing a
  tetrahedron edge into a two-face pillow.

Deterministic survivor: the smaller vertex id wins, and it keeps its
authored position. Degenerate faces (triangles on the edge) are
removed; larger faces on the edge shrink by one vertex. Whole patches
may legally vanish (a lone triangle collapses to two isolated
vertices); vertices never lose their records (KeepIsolated semantics).

`flip_edge` requires a live interior edge between two triangles, with
distinct opposite vertices and no pre-existing edge between them
(`BoundaryEdge`, `NonTriangleFace`, `DegenerateOpposite`,
`DiagonalExists`). Topological orientation is preserved by
construction; geometric fold-over is deliberately not checked — the
kernel makes no float-epsilon decisions, and callers with exact
predicates (the boolean pipeline, constructive evaluation) own that
policy.

## Decision 3: attribute merge semantics (public behavior)

Collapse:

- **Corner UVs**: surviving corners keep their UVs in place. In a
  shrinking face where the corner at the survivor is the one that dies,
  its UV (present or absent) transfers onto the corner that now points
  there — the survivor's authored value always wins over the removed
  endpoint's.
- **Corner normal overrides**: cleared on every corner of every
  surviving face that was incident to the removed vertex and on fused
  side edges; faces not incident to it keep theirs.
- **Edge seam/sharpness**: keyed by canonical half-edge id, they
  survive in place for surviving edges. Where a dropped triangle's two
  side edges fuse into one, seams OR together and sharpness takes the
  maximum; the merged value re-keys to the fused edge's canonical id.
- **Vertex sharpness**: merges by maximum onto the survivor.
- **`FACE_REGION`**: stays with every surviving face record.

Flip:

- **Face regions**: each source triangle's face record is reused by the
  rebuilt triangle that keeps its `next(half_edge)` perimeter edge, so
  `FACE_REGION` follows the face record deterministically.
- **Edge attributes**: perimeter half-edge pairs are untouched, so
  their authored attributes survive in place. The old diagonal's
  entries are cleared; the new diagonal (same canonical pair) starts
  smooth.
- **Corner UVs**: perimeter corners keep their own UVs. The two
  re-aimed diagonal corners derive theirs from the sole source corner
  at their new destination vertex (cleared when missing).
- **Corner normal overrides**: cleared on all six corners.

Neither op takes a `PropagatePolicy` parameter: like
`dissolve_edges`/`dissolve_vertices`, they use fixed deterministic
defaults (the ones above, from the edit-propagation brief). Policy
hooks can be added when a consumer needs an alternative; adding a
parameter later is a deliberate breaking change rather than a
speculative knob today.

## Consequences

- Failed preconditions are typed and leave the mesh byte-identical —
  there are no internal rebuild-failure escape hatches.
- The ops compose with `split_edge`/`split_face` under
  `validate_deep` (covered by the seeded op torture suite, which also
  asserts Euler-characteristic preservation on closed surfaces and
  bit-identical reruns).
- Two latent kernel defects surfaced and were fixed alongside:
  `find_outgoing_half_edge` returned a foreign half-edge for vertices
  with no outgoing entries (corrupting `vertex.out` on isolating
  deletes), and `validate_deep` now verifies that a vertex's stored
  outgoing half-edge actually originates at that vertex.
