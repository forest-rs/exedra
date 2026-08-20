# ADR-0001: Fidget Adapter Scope

- Status: Accepted
- Date: 2026-03-24
- Owners: Exedra implicit-surface maintainers

## Context

The implicit branch needs a real backend adapter once the generic field seam is
stable. Fidget is the first such backend, but it should not distort ownership
of the generic field traits or the mesher.

## Decision

`exedra_fidget` owns:

- dependency integration with `fidget`,
- wrapper types that implement `exedra_isosurface::ScalarField` and extension
  traits,
- optional JIT-backed aliases layered on top of the same adapter contract.

It does not own:

- `ScalarField` itself,
- octree traversal or extraction,
- generic analytic/profile field construction,
- a canonical implicit scene graph.

## Consequences

Positive:

- `fidget` remains a replaceable backend rather than becoming the implicit
  architecture,
- the mesher stays generic over field implementations,
- JIT support can remain an adapter feature instead of a hard dependency.

Tradeoffs:

- the adapter may need to mirror some `fidget` naming or evaluator patterns,
- provenance support may lag behind the base evaluation adapter if upstream
  APIs stay narrow.
