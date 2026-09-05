# ADR-0007: Truthful discretization tolerance

## Status

Accepted (`ec-xkra`, 2026-09-05).

## Context

`DiscretizePolicy::chord_tolerance` promised an absolute chord-to-curve
deviation, but arc and cubic subdivision silently clamped the required count
to `max_segment_edges`. A unit-radius semicircle requesting tolerance `1e-9`
with a four-edge budget therefore returned four chords whose sagitta was about
`0.07612`. The same cancellation-prone circle formula was copied by external
cylinder adapters; sufficiently small tolerance-to-radius ratios could collapse
their calculated count back to a minimum.

Count arithmetic alone is insufficient. Even when the mathematical count is
correct, adding small-radius samples to a large model-space origin can discard
the low coordinate bits. A public f64 discretization cannot claim a tolerance
that its emitted coordinates do not represent.

## Decision

### One typed accuracy boundary

`exedra_constructive::discretize` owns mathematical chord accuracy, finite
per-curve work budgets, and f64 realization checks. It does not own catalog
units, catalog cylinder construction, or universal cardinal-axis alignment.

Successful `discretize_loop` and `discretize_profile` results satisfy the
requested mathematical chord bound in their f64 output. When the required
accuracy, topology, and alignment count exceeds the finite budget, the result is
`DiscretizeError::ToleranceBudgetExceeded { required, maximum }`. Counts above
the public u32 domain return `EdgeCountOverflow`. Finite source geometry that
cannot be computed or emitted reliably in f64 returns `NumericLimit`. These
errors continue through `TessellateError::Discretize`, `EvalError`, and
assembly `CompileError::Evaluate`, retaining the failing node and part.

The f64 realization audit subtracts 16 ulps of headroom at the source-coordinate
scale from the requested tolerance before choosing a subdivision count. Arc
output is checked in center-relative
coordinates at both endpoints and at the closest point of every chord. Cubic
output combines its analytic flatness bound with measured endpoint-emission
error in source-relative coordinates. Non-finite or duplicate interior samples
fail as `NumericLimit`. The tolerance stops at the public f64 discretization
boundary; later f32 mesh quantization remains the tessellator's documented
representation boundary.

### Shared circular count policy

`circular_edge_count` is the public calculation shared by profile arcs,
revolution angular steps, and external primitive adapters. Its
`CircularEdgeConstraints` separates minimum topology, maximum work, and an
optional edge-count multiple. The multiple defaults to one. Full revolutions
explicitly select four so cardinal meridians preserve exact extrema; ordinary
profile arcs do not inherit that caller-specific rule.

The calculation uses
`4 * asin(sqrt(tolerance) / (sqrt(2) * sqrt(radius)))` for the allowed central
angle per chord. This is the cancellation-resistant form of the sagitta
relation. Required counts and alignment round upward. A budget never rounds a
count down.

### Cubics and explicit cylinders

Uniform cubic sampling retains the bound
`3 * max_second_difference / (4 * edge_count^2)`. An evenly parameterized
straight cubic has zero second difference and needs one edge. A curved cubic
that needs more than the budget fails under the same typed contract as an arc.

`PrimitiveSpec::Cylinder::segments` remains an explicitly authored sampling
count. Evaluation enforces its work cap but does not reinterpret it under
`chord_tolerance`. A caller that wants a round cylinder within a tolerance
derives `segments` with `circular_edge_count` first. This keeps recipe intent
stable and lets callers choose their own minimum and axis alignment.

## Consequences

- Tightening tolerance cannot silently reduce circular or cubic subdivision;
  the operation either refines or returns a typed inability.
- Profile arcs, revolutions, and external cylinder adapters use one stable
  circular count calculation without sharing domain-specific construction.
- Exact source endpoints and per-edge source-segment provenance remain intact.
- Invalid bounds are rejected before geometry traversal as decided by
  `ec-dhp7`; derived topology minima above a valid policy budget are reported
  as tolerance-budget failures.
- The calculation and failure behavior change evaluation outcomes for unchanged
  recipes, so the stacked evaluation schema advances from 11 to 12.
