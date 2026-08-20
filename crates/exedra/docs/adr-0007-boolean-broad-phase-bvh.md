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

## Amendment (2026-08-19)

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

## Amendment (2026-08-19)

Two classification/contract rules changed after the boolean_oracle
cross-validation harness (ei-c48a) found silently wrong outputs:

- **Patch sampling.** A patch whose every vertex lies on the cut curve (a
  through-hole disk, for example) can no longer be sampled from an
  arbitrary face: cut vertices are f32-narrowed constructions, so sliver
  faces hugging the curve genuinely poke a hair past the other solid's
  surface, and a sample taken on such a sliver classified the whole patch
  by narrowing noise (the misclassified disk left the result an open tube
  — watertightness is not implied by `validate_deep`, which permits
  boundaries). The fallback sample is now the centroid of the
  largest-area triangle across the patch, maximizing clearance from the
  cut deterministically. Regression fixtures with independently computed
  exact volumes live in the stitch tests (`oracle_regression_*`).
- **Invariant-violation contract.** `BooleanFailureKind::
  InternalInvariantViolation` diagnostics recorded during a run now fail
  `boolean_mesh` with the typed `BooleanError::InvariantViolation` instead
  of returning geometry the pipeline no longer vouches for.
