# Brief: Deterministic triangulation strategy (and what we defer)

> **Implemented by:** `exedra_triangulate`
> (`crates/exedra_triangulate/docs/adr-0001-deterministic-triangulation-scope.md`),
> including the lowest-stable-index ear tie-break prescribed below. This brief
> remains as design rationale.

## Decision
Triangulation in Exedra must be **deterministic** and stable with respect to face-loop ordering. Early versions may use a simple, documented strategy with known limitations, with an explicit path to improved robustness later.

## Why
Triangulation sits on the hot path for extraction and many derived computations. If it is nondeterministic:

- caches become unreliable
- golden tests become flaky
- debugging geometry issues becomes difficult

A simple deterministic strategy is better than a complex “sometimes different” one.

## Strategy (v0.1 posture)
- Treat each polygon face as an ordered loop (deterministic `Face.edge` walk).
- Use a deterministic ear-clipping or fan strategy with explicit tie-breaking.
- When ear choices are ambiguous (equal scores), prefer the lowest stable corner/half-edge id.

Document limitations explicitly (non-simple polygons, highly concave polygons, self-intersections).

## Alternatives considered
- **Robust constrained triangulation / CDT everywhere.** More robust, but heavier and may introduce numeric sensitivity and complex dependencies. Better as a later staged improvement.
- **Randomized or heuristic ear choices.** Can “work better” on some inputs, but breaks determinism and makes outputs unstable.

## Implications
- Exedra must specify triangle output ordering (face order → corner walk → triangle emission order).
- Derived data that depends on triangulation (e.g., geometric normals) must use the same deterministic strategy.
- Wind tunnels should track triangulation performance and failure rates on a corpus.

## Non-goals / deferrals
- Guaranteed triangulation of arbitrary non-simple polygons in v0.1.
- Exact arithmetic triangulation; numeric policy remains explicit and approximate.
