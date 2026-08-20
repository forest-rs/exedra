# ADR-0003: Constructive Conversion Seam

- Status: Accepted
- Date: 2026-08-19
- Owners: Cambium maintainers

## Context

ADR-0002 (multi-domain geometry architecture) prescribes explicit, lossy,
provenance-carrying conversions between geometry heads, orchestrated by
Cambium. The constructive head (`exedra_constructive`, its ADR-0001) now
exists: immutable recipes evaluated into tessellated meshes with source
maps and fidelity reports. Cambium needs its seam.

Two shapes were considered:

1. **An `EditOperator`** (`convert.constructive.evaluate`) running through
   `OperatorRunner` against an (empty) mesh.
2. **An explicit conversion function** in `cambium::convert`, following the
   analytic seam precedent (`analytic_shell_to_mesh`).

## Decision

The seam is an explicit conversion function:

```rust
cambium::convert::constructive_recipe_to_mesh(&Recipe, &ConstructiveToMeshParams)
    -> Result<ConstructiveToMeshOutput, ConstructiveEvalError>
```

`convert.rs`'s own charter already settles this: conversions are "explicit,
typed conversion seams rather than forcing analytic state through the
mesh-only `EditOperator` trait." A recipe conversion produces new meshes
(potentially several bodies); it does not edit an existing one, so the
compile/apply lifecycle does not fit. Wrapping it in an operator against an
empty mesh would be ceremony without semantics.

Contract points:

- **Fingerprint-bound.** The output carries the recipe's content
  fingerprint (`RecipeFingerprint`, stamped with the evaluation schema
  version), so caches and future plan bindings detect staleness exactly.
  Plan-lifecycle integration (fingerprint-bound `EditPlan`s over recipes,
  preview/apply parity) is `cam-n2nc`'s scope and builds on this seam.
- **Diagnostics mapped, report preserved.** The constructive
  `GeometryReport` crosses the seam intact, and its diagnostics are
  additionally mapped into Cambium `Diagnostic`s (`DiagCode` gains
  `UnsupportedOperation` for not-yet-evaluable operations such as CSG
  before the boolean pipeline). Nothing silently degrades: unsupported
  nodes arrive as errors-with-envelopes, never as approximate geometry.
- **Determinism.** The conversion is a pure function of
  `(recipe, params)`; tests pin signature equality across repeated
  conversions.

## Consequences

- `cambium` depends on `exedra_constructive` (no_std matrix preserved via
  feature forwarding; the constructive crate's libm-always policy is
  unaffected).
- `OperatorDomain::Convert` remains metadata for now; making the
  domain-aware runner path real happens with recipe plan-binding
  (`cam-n2nc`) rather than by forcing this conversion into the operator
  shape.
- `cambium_web_bridge` maps the new `DiagCode` variant for viewers.
