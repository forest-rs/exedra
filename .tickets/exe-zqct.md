---
id: exe-zqct
status: closed
deps: []
links: [ea-8tpb]
created: 2026-08-22T16:06:40Z
type: bug
priority: 1
assignee: Bruce Mitchener
tags: [boolean, robustness]
---
# Support ordinary multi-cutter box differences

## Problem

Before this fix, a `Difference` of a plain box by two or more box cutters could
defer when the cutters touched, interpenetrated, sat close together, or simply
numbered four or more. The face splitter deferred the configuration,
classification refused patches bordering a deferred face, and
`exedra_constructive::evaluate` fell back to envelope-only. The caller received
`Ok` with zero bodies plus `eval.csg.unsupported` and `eval.csg.pipeline`
diagnostics.

This is the shape of an ordinary construction joint: a timber with a seat, a
notch, and a bolt hole is one body minus several cutters that may be close or
share a face. `joiner` lowers those edits into one n-ary `Difference`, so this
behavior is part of the boolean contract rather than a joiner-specific case.

## Repro

The regression matrix uses a `box [6.0, 0.60, 4.0] at [0, 0, 0]`; cutters
pass through the base in y. It covers two through six well-separated cutters,
flush cutters in both orders, interpenetrating cutters in both orders, and
three cutters separated by a 10 mm gap in both orders. Each arrangement is
evaluated both as one n-ary difference and as a chain of binary differences.
Every row must produce exactly one body, the independently computed analytic
volume, and no `eval.csg.pipeline` warning.

Smallest failing case (one node, two cutters, flush):

```text
base  = box [6.00, 0.60, 4.00] at [0.00,  0.00, 0.00]
cut A = box [1.20, 0.80, 1.40] at [2.40, -0.10, 1.80]
cut B = box [1.60, 0.80, 0.20] at [2.20, -0.10, 1.60]
Csg { op: Difference, operands: [base, A, B] }  ->  Ok, bodies = 0
```

Moving cut B to `[0.30, 0.80, 0.30] at [0.60, -0.10, 1.00]` (same count,
same form, clear of A) gives 1 body.

Boxes are built as `builders::rect(sx, sy)` extruded `sz` along +Z at
`Placement3::translate(x, y, z)`.

## Root causes

The failures were one pipeline-wide robustness gap rather than one bad branch:

- interval clipping could lose mesh-vertex provenance, and independently
  evaluated intersections of the same stored carriers could drift apart before
  narrowing;
- graph welding needed both representation and topology: equal `f32` positions
  are candidates, but only compatible mesh carriers establish identity (global
  coordinate welding silently merges unrelated seams);
- valid polygon faces could contribute zero-area fan triangles to the broad
  phase when a seam introduced collinear vertices;
- face splitting assumed simple chains and loops, so T-junctions and already
  materialized boundary vertices could corrupt or defer a split, while valid
  sub-loops could still be reinserted in an order that transiently violated the
  half-edge boundary invariant;
- coplanar membership did not consistently stop at per-face contact boundaries
  or propagate across bookkeeping faces with zero area; and
- hole triangulation accepted area-equivalent results whose bridge corridor did
  not reproduce the polygon's exact simplified boundary.

The durable invariants and the reasons for the non-obvious algorithms live in
the relevant module documentation, public rustdoc, and local code comments.
No crate ADR is warranted: this repairs the existing boolean contract without
changing crate ownership or introducing a new public abstraction.

## Fence

`exedra::boolean` owns the fix: the splitter and classifier must handle
these configurations, or report a typed, per-configuration limitation that
callers can act on. `exedra_constructive` keeps the n-ary fold rule and the
envelope fallback; it does not grow workarounds (reordering cutters, nudging
coplanar faces, retrying chained). Assembly-level visibility of the zero-body
outcome is `ea-8tpb`, not this ticket.

## Acceptance

- Every case in the regression matrix evaluates to exactly 1 body with no
  `eval.csg.pipeline` warnings and the analytic volume, under both forms.
- The `bsl-6ihj` stage-1 fixtures (truss heel, window) still evaluate to the
  same bodies; the `joiner` and `basilica_structure_lab` tests stay green.
- Scale-extreme axis-aligned stress cases under both forms either return the
  exact analytic volume or refuse with `eval.csg.unsupported`; they never panic,
  emit a partial body set, or return clean incorrect geometry.
- The interpenetrating-cutters "dangling cut" is fixed and pinned by a direct
  boolean regression; the flush chained result is exact in both cutter orders.
- The regression matrix lands as a deterministic `exedra_constructive` test that
  asserts body count and volume per row.
- Seeded mesh/isosurface/closed-form cross-validation reports no membership
  disagreements outside its documented numerical bands.

## Related

- `ea-8tpb` — assembly compilation drops zero-body instances silently (the
  consumer-side half of this failure).
- `bsl-6ihj` — joiner construction layer; stage 2 depends on this.

## Notes

**2026-08-23T18:24:15Z**

Implemented the ordinary multi-cutter fix across intersection materialization,
broad-phase fan handling, face splitting and classification, and
polygon-with-holes triangulation. Graph endpoints now recover exact stored
vertex and carrier-intersection positions, while same-position welding requires
compatible topology on both meshes. The splitter handles planar T-junctions and
orders face reconstruction without transiently corrupting the OUTSIDE cycle;
any unresolved split independently withholds the result even when downstream
classification cannot rediscover it. Coplanar membership respects per-face
contact boundaries and zero-area bookkeeping faces. Hole triangulation requires
exact simplified-boundary incidence, preventing an area-equivalent result from
hiding a zero-width bridge corridor; this is correctness hardening, not a mesh
quality policy, so the pending CDT work can replace the triangulator while
preserving the invariant.

The only public-semantic change is stricter failure behavior:
`BooleanError::SuspectPatches { count }` can now be returned for a split-stage
deferral even when classification reports no suspect patch. Its shape is
unchanged; callers already handling the variant need no migration. No crate ADR
was added because this restores the existing boolean contract without changing
ownership or adding a public abstraction; the durable numeric and topology
invariants are documented beside the enforcing code.

Validation covered 4,900 seeded generated cases across convex, curved,
non-convex, chained, adversarial, scale, and empty-result classes, comparing the
mesh result and an isosurface field against a closed-form referee. There were
zero mesh or field membership disagreements; unsupported cases were counted as
typed skips. The full validation set also passed: `cargo test --workspace
--all-features`; `cargo clippy --workspace --all-targets --all-features -- -D
warnings`; `cargo doc --workspace --all-features --no-deps`; `cargo check -p
exedra --no-default-features --features libm`; `cargo fmt --all -- --check`;
`taplo fmt --check`; `typos`; and `git diff --check`.
