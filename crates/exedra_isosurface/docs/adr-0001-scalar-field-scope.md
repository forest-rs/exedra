# ADR-0001: Scalar Field Scope

- Status: Accepted
- Date: 2026-03-24
- Owners: Exedra implicit-surface maintainers
- Ticket: `exe-2r7w`

## Context

The implicit-surface branch needs a stable field-evaluation seam before any
mesher or backend adapter can be built honestly.

## Decision

`exedra_isosurface` initially owns:

- the `ScalarField` trait,
- extension traits for specialization and provenance,
- Hermite bridge types between field evaluation and extraction,
- small reference fields used to validate the trait contract,
- lightweight field-construction wrappers such as transforms that stay at the
  evaluation boundary rather than introducing a full implicit scene graph.

It does not yet own:

- the full dual-contouring pipeline,
- fidget integration,
- a canonical implicit scene/domain model.

## Consequences

Positive:

- extraction code can depend on one stable evaluation contract,
- backend adapters stay replaceable,
- the implicit branch can start with a narrow, testable slice,
- common field edits can compose on top of the seam without growing bespoke
  primitive variants.

Tradeoffs:

- some later extraction-facing types will still land here incrementally,
- a future `exedra_implicit` umbrella crate may still be warranted if implicit
  state grows beyond extraction.
