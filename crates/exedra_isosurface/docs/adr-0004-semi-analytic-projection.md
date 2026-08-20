# ADR-0004: Opt-in semi-analytic surface projection

- Status: Accepted
- Date: 2026-08-19

## Context

`ScalarField` deliberately describes black-box value, interval, and gradient
evaluation. `ProvenanceField` adds labels, but labels do not contain primitive
geometry. Neither boundary can project a QEF result onto a known primitive or
recover an exact feature curve without guessing concrete field types.

Adding projection hooks to `ScalarField` would burden every field, complicate
trait-object use, and silently change existing extraction. A second concrete
CSG tree would duplicate the generic `Union`, `Intersection`, and `Difference`
types.

## Decision

Add a separate, opt-in `SemiAnalyticField` capability. It exposes:

- deterministic projection of a candidate vertex within one extraction cell;
- a stable `u32` primitive identity;
- optional leaf geometry through `AnalyticPrimitive` for supported pairwise
  feature recovery.

Tagged axis-aligned boxes and finite cylinders implement the capability.
Translation, positive uniform scale, and rigid transforms forward it by
projecting in primitive-local space and mapping the result back to world
space. References and boxed trait values preserve the capability explicitly.

The existing `ScalarField`, `ProvenanceField`, `dual_contour`, and
`dual_contour_with_regions` APIs and behavior remain unchanged.

“Exact” means a deterministic, closed-form construction from the analytic
primitive parameters. It does not mean zero real-number residual after `f32`
rounding: normalized arbitrary axes and circular coordinates are commonly
irrational. Validation therefore uses scale-aware residual bounds and repeated
bit-identical results.

## Invariants

- Candidate enumeration and exact-distance ties have stable descriptor order.
- Primitive projection may return a surface point outside the source cell;
  extraction owns containment and displacement-budget rejection.
- Invalid or unsupported primitive geometry returns `None`; it never fabricates
  a projection.
- `axis` and `-axis` describe identical cylinder projection results.
- A projection carries primitive identity independently of face provenance.
- Supported wrappers preserve identity and perform projection in local space.
- Black-box extraction never observes or pays for the analytic capability.

## Consequences

- Existing callers require no migration. New callers opt into the additive
  capability and, once available, the additive semi-analytic extractor.
- Arbitrary primitive surface projection is available without committing the
  system to arbitrary primitive-pair intersection solving.
- Relative rigid rotations can produce clipped conics. Those configurations
  must remain explicit fallbacks until a robust conic/component solver exists.
- A `u32` identity is intentionally narrower than recipe-level semantic
  provenance. Bridging those domains belongs to the constructive integration
  layer, not this projection kernel.

## Amendment: opt-in extraction and aligned pair curves (`ei-wd1p`)

`dual_contour_semi_analytic` applies the capability after the bounded QEF solve
and before vertex emission. It independently verifies finiteness, cell
containment, and a one-cell-diagonal displacement budget. Rejected candidates
leave the QEF result unchanged.

The cell diagonal is a topology-preservation budget, not an accuracy
tolerance. It permits relocation anywhere within the cell that owns the dual
vertex, while preventing a malformed projector from jumping farther than that
cell's full extent from the QEF representative. Cell containment remains a
separate invariant so satisfying the distance budget alone is insufficient.

Generic `Union`, `Intersection`, and `Difference` implementations preserve the
existing left-winning value tie policy for primitive identity. A supported
leaf pair may supersede ordinary surface projection with one transverse
intersection-curve candidate.

The first exact pair envelope is deliberately narrow:

- a box whose reported analytic axes are the exact identity frame (signed or
  permuted coordinate frames are currently unsupported even when they bound
  the same world-axis-aligned solid);
- an exactly coordinate-axis-aligned finite cylinder;
- non-tangent and non-coincident surface patches;
- exactly one clipped feature component in the active cell.

The solver enumerates box-face / cylinder-side and box-face / cylinder-cap
circle or line pieces in stable descriptor order, clips each piece to both
primitive patches and the active cell, verifies scale-aware residuals, and
snaps only a unique connected component. Circle/rectangle clipping counts
disconnected arcs rather than collapsing them to one nearest point. Tangent
and coincident classification is local to the active cell and both finite
primitive patches. Arbitrary relative rotations imply clipped conics and are
unsupported. Malformed primitives are `Invalid`, while tangent, coincident,
ambiguous, over-budget, and unsupported cases remain distinct counted QEF
fallbacks.

This is additive API. Existing callers require no migration. Callers choosing
the new extractor receive a `SemiAnalyticContourResult`; its mesh and ordinary
stats mirror `DualContourResult`, with additional projection/fallback counters.
