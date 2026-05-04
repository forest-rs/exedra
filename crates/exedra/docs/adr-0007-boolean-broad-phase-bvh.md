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
