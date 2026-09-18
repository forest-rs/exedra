# ADR-0001: Scalar Field Scope

- Status: Accepted
- Date: 2026-03-24
- Owners: Exedra implicit-surface maintainers

## Context

The implicit-surface branch needs a stable field-evaluation seam before any
mesher or backend adapter can be built honestly.

## Decision

`exedra_isosurface` initially owns:

- the `ScalarField` trait,
- the `ScalarField2d` trait and minimal profile bounds for field lifting,
- extension traits for specialization and provenance,
- Hermite bridge types between field evaluation and extraction,
- profile-lifting operators such as extrusion and revolution,
- small reference fields used to validate the trait contract,
- lightweight field-construction wrappers such as transforms that stay at the
  evaluation boundary rather than introducing a full implicit scene graph.

It does not yet own:

- the full dual-contouring pipeline,
- fidget integration,
- a canonical implicit scene/domain model.

## Consequences

### Conservative lifting intervals

Extrusion and revolution compose the source profile's interval over mapped
profile bounds. Extrusion combines the interval with the axial distance range;
revolution maps the spatial box to its enclosing radius-height rectangle.
Unknown source intervals remain unknown. Sampling values and inflating by a
spatial distance is insufficient for generic fields, whose values need not have
a unit Lipschitz bound. A positive rescaling must not make a surface disappear
through incorrect interval culling.

This corrects interval semantics without changing signatures. Callers with
custom profiles must supply conservative bounds or return `None`.

### Architectural consequences

Positive:

- extraction code can depend on one stable evaluation contract,
- profile-based constructors can reuse the same crate boundary as extraction,
- backend adapters stay replaceable,
- the implicit branch can start with a narrow, testable slice,
- common field edits can compose on top of the seam without growing bespoke
  primitive variants.

Tradeoffs:

- some later extraction-facing types will still land here incrementally,
- a future `exedra_implicit` umbrella crate may still be warranted if implicit
  state grows beyond extraction.
