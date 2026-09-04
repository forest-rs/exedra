# Brief: OUTSIDE face boundary model (explicit boundary half-edges)

## Decision
Represent boundaries using **explicit boundary half-edges** and a reserved **OUTSIDE face**. Every topological edge always has two half-edges (`twin` is never `Option`). Boundary loops are half-edge cycles attached to `OUTSIDE`.

## Why
This keeps traversal and validation simple and predictable:

- `twin` is always valid → fewer branches in hot loops
- boundary is a real part of topology → consistent traversal around vertices/faces
- robust foundation for staged boolean operations (splitting/stitching along loops is natural)
- aligns with “explicit over implicit” (no invisible “missing twin” cases)

## Alternatives considered
- **Optional twins**: simpler conceptually, but more branches and special cases everywhere.
- **Separate boundary structure**: can be efficient, but increases complexity and risk of divergence from topology invariants.

## Implications
- Operational details must be explicit (sentinel vs arena entry for `OUTSIDE`, boundary loop orientation/winding conventions).
- Validation can make strong claims (every half-edge has a twin; boundary is `face == OUTSIDE`).
- Higher layers can reason about open meshes without pervasive “is boundary?” branching.

## v0.1 Resolution
For v0.1, `OUTSIDE` is represented as a reserved sentinel `FaceId` and is not a
real face arena entry (see ADR-0003).

## Non-goals / deferrals
- This doesn’t solve non-manifold complexity; it provides a clean, consistent base.
