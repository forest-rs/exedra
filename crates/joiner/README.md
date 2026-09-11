# joiner

The construction layer for the Exedra stack: building elements, how they are
related, and the rules that turn a relation into coordinated geometry.

`exedra_constructive` compiles one part's recipe into meshes and
`exedra_assembly` arranges parts as placed instances. Neither knows what a
rafter or a window is. `joiner` describes those elements and their fits. An
optional adapter checks declared contact areas against already compiled parts.

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

Shape construction stays in `exedra_constructive` and `exedra_mesh`. Joiner
owns none of: site, massing, and plan layout; statics, finite-element analysis,
capacity, or code compliance; rendering and export; or an erased,
document-shaped parameter boundary.

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

`validate` checks schema and coherence, analytic contact anchors and extents
at a documented `1e-9 m` tolerance, and load paths witnessed by those claims,
relations, and supports. Extent overlap alone does not prove a bearing surface:
a round purlin can touch a beam along a line while their boxes overlap broadly.

`ContactPatch::with_footprint` (or `with_footprint_meters` for derived dimensions)
declares a rectangle centered on each anchor along the contact tangents. Analytic
validation checks its dimensions, minimum overlap, and containment in both
extents. `measure_contact_geometry` separately checks that both compiled parts
cover that rectangle with outward-facing surfaces near the contact plane. The
caller supplies the current composed parts and a mesh distance tolerance. The
check insets the rectangle by that tolerance, detects interior holes and partial
support, and counts duplicate triangles only once. It does not discover contacts,
check solid interpenetration, or silently evaluate geometry during `validate`.

Neither validation layer is a static analysis, finite-element model, capacity
check, building-code result, or engineering certification.

## API additions

`lower` and `lower_selected` retain one part per element. Use
`lower_shared(&construction, |element| family_key(element))` to share exact
composed recipes within caller-named families. Different cuts, source references,
slot tables or default slots get separate parts; differing materials use instance
bindings. Element instance paths and provenance metadata are preserved. Shared
part keys identify the first family recipe or an element-specific variant, so
consumers resolve `instance_path(element)` and read its part rather than looking
up `part_key(element)`.

Existing contact callers keep their analytic extent checks. Add a footprint
and opt into `measure_contact_geometry` when the generated bearing surface
matters; a clean `validate` report alone remains an analytic claim.

`Construction::apply_rule(application, relation, &rule, &params)` replaces the
repeated context/instantiate/application sequence when the application should
inherit the relation's evidence. It returns `ApplyRuleError`; explicit
`RuleContext` and `RuleApplication` construction remains supported.
`OrientedBox::{local_point, local_direction, local_placement}` convert world
geometry into an element's orthonormal local frame, including reflected frames.

See `docs/adr-0001-construction-layer-scope.md` for the scope contract, and
`examples/basilica_structure_lab/docs/adr-0002-joiner-construction-layer.md`
for the decision that created this crate.
