# ADR-0008: Kernel-Owned Face Triangle Enumeration

- Status: Accepted
- Date: 2026-08-19
- Owners: Exedra maintainers

## Context

Two subsystems triangulate faces today, independently: render extraction
fan-triangulates every face (silently producing overlapping or inverted
triangles for concave ngons), and the boolean broad phase re-derives the
same fan inline to index its BVH — coupling `BooleanTriangleRef.fan_index`
to a strategy no API names. The constructive geometry head emits concave
faces on purpose, and the boolean pipeline's split stage will need robust
retriangulation. One authoritative enumeration must exist, with the
strategy an explicit, recorded parameter.

## Decision

`Mesh::face_triangles(face, strategy)` is the single kernel-owned triangle
enumeration; render extraction and the boolean broad phase both consume it.
A triangle index is meaningful only together with the strategy that
produced it, and any API that stores triangle indices must record the
strategy alongside them.

`FaceTriangulation` (`#[non_exhaustive]`, default `Fan`):

- **`Fan`** — byte-identical to the historical
  `Mesh::triangulate_face_fan`: triangles `(c0, c1, c2), (c0, c2, c3), …`
  from the face loop's first corner. Remains the default so existing
  extraction output, goldens, and wind-tunnel signatures are unchanged.
- **`Robust`** — plane-projected triangulation via `exedra_triangulate`:
  1. Face-loop positions promote exactly from f32 to f64.
  2. The Newell normal is computed; the projection drops the dominant axis
     (largest absolute component, ties to the lowest axis index — a
     deterministic, trig-free rule).
  3. Projection axes are taken in cyclic order `(k+1, k+2) mod 3`; a
     negative dominant component swaps them, so the triangulator always
     receives a counter-clockwise polygon while output triangles wind
     consistently with the face loop in either case.
  4. Output indices map back to face-loop corners; every output vertex is
     an input corner (the triangulator never inserts points), so corner
     attributes and provenance survive unchanged.

**Fallback contract.** Enumeration never fails: when the projected polygon
is not simple (self-intersecting or degenerate under exact predicates),
`Robust` falls back to the fan for that face, deterministically.
`Mesh::face_triangles_counted` exposes the fallback bit;
`ExtractStats::robust_fallback_count` aggregates it during extraction, so
callers can observe rather than guess.

`ExtractParams::face_triangulation` selects the strategy for
`Mesh::to_trimesh` (default `Fan`).

## Consequences

- `exedra` gains a dependency on `exedra_triangulate` (a zero-dependency
  `no_std` leaf below it in the graph, designed for this).
- The boolean broad phase migrates to this enumeration and records its
  strategy (`exe-o1su`); the future narrow phase and split stages must be
  invoked with the same strategy the broad phase recorded.
- Derived data that depends on triangulation (geometric normals, area
  sums) sees different — better — triangles under `Robust`; goldens that
  bake triangle order must state their strategy.
