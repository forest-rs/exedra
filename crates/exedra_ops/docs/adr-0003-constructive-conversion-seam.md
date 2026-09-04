# ADR-0003: Constructive Conversion Seam

- Status: Accepted
- Date: 2026-08-19
- Owners: Exedra Ops maintainers

## Context

ADR-0002 (multi-domain geometry boundaries) prescribes explicit, lossy,
provenance-carrying conversions between geometry heads. The constructive head
(`exedra_constructive`, its ADR-0001) now exists: immutable recipes evaluated
into tessellated meshes with source maps and fidelity reports. Exedra Ops
provides the typed adapter for this seam.

Two shapes were considered:

1. **An `EditOperator`** (`convert.constructive.evaluate`) running through
   `OperatorRunner` against an (empty) mesh.
2. **An explicit conversion function** in `exedra_ops::convert`, following the
   analytic seam precedent (`analytic_shell_to_mesh`).

## Decision

The seam is an explicit conversion function:

```rust
exedra_ops::convert::constructive_recipe_to_mesh(&Recipe, &ConstructiveToMeshParams)
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
  version), so caches detect staleness exactly. Recipe plan binding has a
  recipe-specific lifecycle separate from the mesh `EditPlan` and
  `OperatorRunner`; it does not make the mesh runner domain-aware.
- **Diagnostics mapped, report preserved.** The constructive
  `GeometryReport` crosses the seam intact, and its diagnostics are
  additionally mapped into Exedra Ops `Diagnostic`s (`DiagCode` gains
  `UnsupportedOperation` for typed evaluation refusals). Nothing silently
  degrades: unsupported nodes arrive as errors-with-envelopes, never as
  approximate geometry.
- **Determinism.** The conversion is a pure function of
  `(recipe, params)`; tests pin signature equality across repeated
  conversions.

## Consequences

- The `constructive` feature enables the `exedra_constructive` dependency and
  conversion surface. Mesh-only builds do not compile the constructive head.
- Cross-domain conversion is represented by this typed adapter rather than an
  `EditOperator`; it has its own input, output, diagnostics, and failure
  contract and does not participate in the mesh `OperatorRunner` lifecycle.
- `exedra_ops_web_bridge` maps the new `DiagCode` variant for viewers.
