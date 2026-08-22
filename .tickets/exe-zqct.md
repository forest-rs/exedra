---
id: exe-zqct
status: open
deps: []
links: [ea-8tpb]
created: 2026-08-22T16:06:40Z
type: bug
priority: 1
assignee: Bruce Mitchener
tags: [boolean, robustness]
---
# Boolean face splitter defers on ordinary multi-cutter configurations

## Problem

A `Difference` of a plain box by two or more box cutters fails whenever the
cutters touch, interpenetrate, sit close together, or simply number four or
more. The boolean pipeline does not produce wrong geometry: the face splitter
*defers* the configuration, classification then refuses every patch that
borders a deferred face, and `exedra_constructive::evaluate` falls back to
envelope-only. The caller gets `Ok` with **zero bodies**, an
`eval.csg.unsupported` error diagnostic, and one `eval.csg.pipeline` warning
per deferral.

This is the shape of every construction joint: a timber with a seat, a notch,
and a bolt hole is one body minus three cutters that are close together or
share a face. `joiner` lowers each participant's edits into exactly one n-ary
`Difference`, so stage 2 of `bsl-6ihj` (truss heel and king-post foot) will
hit this on its first fixture.

Found while building `joiner` stage 1; recorded in the `bsl-6ihj` ticket note
under "Exedra boolean limitations found". Verified again on `main` at
`99fb11a` (2026-08-22) with the table below.

## Repro

All cases: base `box [6.0, 0.60, 4.0] at [0, 0, 0]`; cutters are boxes that
pass right through the base in y (`y = -0.10 .. 0.70`). "n-ary" is one
`Csg { Difference, [base, c1, c2, ...] }` node; "chained" is
`((base - c1) - c2) - ...`. Evaluated with
`evaluate(&recipe, &EvalPolicy::default())`; every row returned `Ok`.

| Cutters | Form | Bodies | Deferral reason(s) reported |
|---|---|---|---|
| 2 or 3, well separated (1.2 m apart in x) | n-ary | 1 | — |
| 4, well separated | n-ary | **0** | drilled face ring failed hole triangulation |
| 3 or 4, well separated | chained | 1 | — |
| 2, flush (share the plane `z = 1.80`) | n-ary | **0** | coplanar contact region is not cleanly separated in this patch |
| 2, flush | chained | **0** | zero-area triangle ×4; junction (local degree ≥ 3); coplanar ambiguity |
| 2, interpenetrating | n-ary | **0** | intersection chain ends inside the face (dangling cut) ×3 |
| 2, interpenetrating | chained | **0** | zero-area triangle ×2; dangling cut ×2 |
| 3, close but disjoint (10 mm gaps, overlapping in x) | n-ary | **0** | face mixes open cut chains with interior closed loops ×2 |
| 3, close but disjoint | chained | **0** | junction (local degree ≥ 3) ×4 |
| 2, clear of each other (control) | n-ary | 1 | — |

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

## Where it fails

The n-ary rule in `exedra_constructive/src/evaluate.rs` folds cutters
`2..n` with `Union`, then subtracts once. Each deferral comes from
`exedra::boolean`:

- `boolean/split.rs:333` — face contains an intersection junction (local degree ≥ 3)
- `boolean/split.rs:391` — face mixes open cut chains with interior closed loops
- `boolean/split.rs:413` — intersection chain ends inside the face (dangling cut)
- `boolean/split.rs:910` — drilled face ring failed hole triangulation
- `boolean/classify.rs:354` — coplanar contact region not cleanly separated
- `boolean/classify.rs:324` — patch borders a face whose split was deferred (cascade)

Two observations worth keeping:

- Neither form is reliably better. n-ary survives close-but-disjoint cutters
  where chaining does not (junction ≥ 3), but chaining survives operand count
  where n-ary does not (the unioned cutter stack drills a ring the hole
  triangulation cannot close). `joiner` canonicalises to n-ary because that is
  the semantically correct shape, not because it is safe.
- "Dangling cut" on two interpenetrating boxes is suspicious rather than
  merely unsupported: the intersection curve of two boxes is closed, so a
  chain ending inside a face means a segment was lost upstream of the
  splitter.

## Fence

`exedra::boolean` owns the fix: the splitter and classifier must handle
these configurations, or report a typed, per-configuration limitation that
callers can act on. `exedra_constructive` keeps the n-ary fold rule and the
envelope fallback; it does not grow workarounds (reordering cutters, nudging
coplanar faces, retrying chained). Assembly-level visibility of the zero-body
outcome is `ea-8tpb`, not this ticket.

## Acceptance

- Every row in the repro table evaluates to exactly 1 body with no
  `eval.csg.pipeline` warnings, under both forms.
- The `bsl-6ihj` stage-1 fixtures (truss heel, window) still evaluate to the
  same bodies; the `joiner` and `basilica_structure_lab` tests stay green.
- Each splitter deferral that remains has a regression test that pins the
  configuration it refuses and a diagnostic naming it; none are reachable from
  axis-aligned box-on-box booleans.
- The interpenetrating-cutters "dangling cut" is either fixed or explained in
  the close note as a genuine limitation with the missing segment identified.
- The repro table lands as a deterministic `exedra_constructive` test that
  asserts 1 body per row.

## Related

- `ea-8tpb` — assembly compilation drops zero-body instances silently (the
  consumer-side half of this failure).
- `bsl-6ihj` — joiner construction layer; stage 2 depends on this.
