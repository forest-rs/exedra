---
id: ec-uoij
status: closed
deps: []
links: []
created: 2026-08-21T13:40:41Z
type: feature
priority: 1
assignee: Bruce Mitchener
---
# Add profile offset for clearance generation

## Problem

The construction layer (`joiner`, structure-lab ADR 0002) derives both sides
of a fit from one nominal interface profile: the inserted side retains it,
the receiving side is its clearance offset. `exedra_constructive` has no
profile offset today (`builders.rs` offers `rect`, `rounded_rect`,
`l_profile`, `circle`, `ring`; `profile.rs` has none). Without it a mortise
must be a second hand-built profile that merely agrees with the tenon by
construction discipline.

## Fence

`exedra_constructive` owns the offset as an ordinary profile operation with
the domain's closure, orientation, provenance, and determinism guarantees.
It does not own fit classes, clearance policy, or any notion of which side of
a joint receives the offset; those are `joiner` concerns.

## Design

- Offset a `Profile2` (outer loop plus holes) by a finite signed distance.
  Outward grows the outer loop and shrinks holes; inward does the reverse.
- All curve math routes through the pinned kurbo; no parallel offset math.
- Corner policy is explicit (round, or miter with a limit), never implicit.
- The result is a `Profile2` with the same closure-by-construction guarantee.
  `SegTag` provenance is carried from source segments where the mapping is
  one-to-one and documented where it is not.
- Degenerate results (a hole collapses; loops self-intersect or touch after
  offset) fail with a typed `ProfileError` rather than yielding an invalid
  profile.
- **Robustness fence.** Line and bulge-arc segments offset *exactly* (a line
  to a parallel line, an arc to a concentric arc with the same center and
  sweep); cubics offset through kurbo's `CubicOffset` and `fit_to_bezpath`
  at the documented tolerance. Self-intersection after an inward offset is
  *detected and rejected*, never healed: no general offset-curve cleanup or
  loop-trimming algorithm is in scope. Joinery profiles are overwhelmingly
  rectilinear and rounded-rectilinear; the exact path covers them.
- Offsets participate in content hashing like any other profile. If offset
  output is not bit-stable across kurbo versions, that is an
  `EVAL_SCHEMA_VERSION` bump.

## Acceptance

- `Profile2` offset by a finite signed distance with an explicit corner
  policy, kurbo only.
- Outer/hole orientation and closure invariants hold on the result.
- Collapsed holes and self-touching results fail typed.
- Segment tags survive where one-to-one; the exceptions are documented.
- Deterministic across targets (golden or bit-pattern test).
- Public API documented; constructive ADR 0001 updated to list offset among
  profile operations.

## Close note

Implemented `Profile2::offset(distance, CornerPolicy)` as a construction-time
profile operation. Positive distance grows material; negative distance shrinks
it; zero preserves the profile. Lines and bulge arcs use exact parallel or
concentric constructions, while cubics use kurbo fitting at a documented
tolerance. Source tags survive one-to-one exact segments; fitted and inserted
segments document their provenance limits.

The operation deliberately adds no IR node or schema-version change. Round and
bounded miter joins are explicit, a miter never silently becomes a bevel, and
unsupported cubic trimming or invalid results fail typed. Regression coverage
includes collapsed holes, self-intersection, undercut material, hole/outer
contact, and two inward-growing holes reaching each other.

Validation passed: `typos`, `cargo fmt --all`, `taplo fmt`, workspace clippy
with warnings denied, all-feature workspace tests, `cargo doc --no-deps`, and
`cargo check -p exedra_constructive --no-default-features`.
