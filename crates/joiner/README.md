# joiner

The construction layer for the Exedra stack: building elements, how they are
related, and the rules that turn a relation into coordinated geometry.

`exedra_constructive` compiles one part's recipe into meshes and
`exedra_assembly` arranges parts as placed instances. Neither knows what a
rafter or a window is. `joiner` does, and it knows nothing about meshes.

```text
elements + relations           the construction: the source of truth
    -> rule.assess / rule.instantiate
    -> RuleOutput              part edits, generated parts, contacts, transfers
    -> Construction::apply     merged into the same tables authored facts live in
    -> validate                schema/coherence, contact, load path
    -> lower                   one Assembly instance per geometry-bearing element
```

## Three relations, one output

Host/fill, member/member, and element/units are first-class siblings. None is
expressed in terms of another: a window is not a degenerate joint, and a bond
is not a stack of two-member fits.

Every rule, of every kind, returns the same four things — part edits,
generated parts, contact patches, load-path edges — so validation and lowering
consume rule output without knowing which rule produced it, or whether a rule
produced it at all.

Authored linear and angular limits use `exedra_measurements` values. Analytic
extents, measured contact gaps and overlaps, and numerical tolerances remain
floating-point geometry. When a rule derives an overlap threshold from those
extents, the `with_minimum_overlap_meters` call makes that boundary visible.

A member/member relation is also the load-path witness for a joint transfer:
the relation *is* the joint. There is no separate joint record to keep
consistent with it.

## Mechanism, not knowledge

This crate contains no knowledge of any particular joint, bond, or profile.
Construction knowledge lives in separate rule-library crates (`joiner_timber`,
`joiner_masonry`, …) so a consumer that needs four timber joints does not
inherit a dependency on thirty, nor on stone.

It owns none of: geometry math (`exedra_constructive`, `exedra_mesh`); site,
massing, and plan layout; statics, finite-element analysis, capacity, or code
compliance; rendering and export; or an erased, document-shaped parameter
boundary.

## Identity, evidence, invalidation

- **Keys are identity.** Element keys are frontend-supplied, stable across
  re-evaluations, and the seed of the `InstancePath` each element lowers to.
  `ElementId` is a handle, never identity, and rule output never uses one — a
  rule names parts it is about to generate, which have no handle yet.
- **Evidence travels with everything.** Elements, relations, contacts, part
  edits, and rule applications each cite a named source at a declared class
  (`Observed`, `DocumentedReconstruction`, `RegionalAnalogy`,
  `ModernEngineeringInference`). Validation checks that the source exists and
  that the classes agree.
- **The element is the dirty-tracking unit**, through the `invalidation`
  crate, on three channels: geometry, contact, load path. Moving one window
  marks one wall and nothing else.

## What validation claims

Schema and coherence, contact geometry at a documented `1e-9 m` tolerance, and
load paths witnessed by contacts, relations, and supports. It is **not** a
static analysis, finite-element model, capacity check, building-code result,
or engineering certification.

See `docs/adr-0001-construction-layer-scope.md` for the scope contract, and
`examples/basilica_structure_lab/docs/adr-0002-joiner-construction-layer.md`
for the decision that created this crate.
