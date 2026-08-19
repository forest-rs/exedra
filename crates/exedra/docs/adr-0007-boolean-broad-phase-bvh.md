# ADR 0007: Boolean Broad-Phase BVH

## Status

Accepted.

## Context

The staged boolean pipeline needs a deterministic broad phase before
narrow-phase triangle intersection. The broad phase should reduce the naive
`N * M` triangle-pair set, report useful counters, and keep temporary storage
caller-owned so repeated boolean operations can reuse capacity.

## Decision

Add an owned `BooleanBvh` over Exedra's deterministic fan-triangulated faces.
Each leaf primitive is identified by `BooleanTriangleRef { face, fan_index }`.
Queries between two hierarchies write sorted `BooleanCandidatePair` values into
caller-owned output and return `BooleanBroadPhaseStats`.

Use `BooleanScratch` for reusable face-loop and traversal-stack storage. The
query clears scratch and output at entry but retains their capacities. BVH
values are also rebuildable in place so callers can retain hierarchy storage
across repeated operations.

The broad phase owns only AABB overlap candidate discovery. It does not perform
triangle-triangle intersection, coplanarity handling, classification, splitting,
or stitching.

## Consequences

Candidate ordering is deterministic and independent of traversal stack order
because query output is sorted by triangle reference. Broad-phase stats expose
the total pair count, post-cull candidate count, and reduction ratio. Future
boolean stages can consume triangle references directly while preserving access
to source face IDs for diagnostics.

## Amendment (2026-08-19, ticket `exe-o1su`)

The original design coupled triangle references to fan triangulation by
construction: `fan_index` was defined against an inline re-derivation of the
fan, so any change of triangulation strategy would have silently invalidated
stored references. With ADR-0008's kernel-owned enumeration in place, the
contract is now strategy-explicit:

- `BooleanTriangleRef { face, triangle_index }` — the index into
  `Mesh::face_triangles(face, strategy)`; meaningless without the strategy.
- `BooleanBvh::build`/`rebuild` take an explicit `FaceTriangulation`; the
  hierarchy records it (`BooleanBvh::strategy()`), triangle collection routes
  through `face_triangles` instead of re-deriving fans inline, and
  `query_overlaps` debug-asserts both hierarchies used the same strategy.
- `BooleanBroadPhaseStats.strategy` records the enumeration so downstream
  narrow-phase stages (exe-h16i onward) can assert they enumerate with the
  same one.

Broad-phase output under `Fan` is unchanged. The `BooleanScratch` face-corner
buffer is currently unused by collection (the enumeration allocates per
face); reclaiming that allocation belongs to `exe-fui5`.
