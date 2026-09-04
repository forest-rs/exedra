# Brief: Derived vs authored normals (corner override policy)

## Decision
Normals are **derived by default** from geometry and smoothing rules, but can be **overridden** by an optional corner-domain normal override layer. When topology edits create new corners, the default behavior is to **clear** overrides unless explicitly requested by policy.

## Why
Normals serve two masters:

- derived normals give consistent shading from geometry and are cheap to recompute incrementally
- authored normals are intentional “art data” (bevel look control, baked shading) and should not be implicitly reinterpreted

Clearing overrides on newly created corners is the least surprising default: it prevents accidental propagation of unrelated shading data.

## Alternatives considered
- **Always copy overrides**: often produces surprising artifacts after edits (stale normals in new topology).
- **Always average overrides**: can look plausible but is semantically ambiguous and hard to reason about.

## Implications
- Exedra needs a clear normal source policy (`Derived`, `CustomOrDerived`, `CustomOnly`).
- Edit propagation rules must define override handling (clear/copy/average) deterministically.
- Extraction splits render vertices on normal discontinuities.

## Non-goals / deferrals
- Tangent frame generation is deferred; it follows the same corner-domain + override/derived posture.
