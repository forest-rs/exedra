# ADR-0001: Plain-array math and benchmark-gated SIMD

**Status:** Accepted; amended 2026-09-04
**Date:** 2026-03-03

## Context

Exedra is a `no_std` mesh kernel with a determinism contract: identical inputs +
parameters must produce identical outputs across runs. The choice of math library
and SIMD strategy directly affects both ergonomics and determinism guarantees.

The workspace now shares deterministic vector arithmetic through `exedra_math`.
Its helpers operate on plain arrays, so the kernel and sibling heads can share
the small operations they genuinely have in common without imposing a vector
type on callers or on native-domain values.

## Decision

### Public API

Exedra's public API uses plain arrays (`[f32; 3]`, `[f32; 2]`, etc.) with
typedefs where helpful. This keeps the API calm, math-library-agnostic, and
avoids coupling callers to a specific math crate.

### Shared arithmetic

`exedra_math` owns the small, deterministic, plain-array helpers shared by
Exedra and sibling crates. It is the workspace boundary for common vector
arithmetic, not a replacement for a domain's native representation or a broad
linear-algebra framework.

### SIMD

SIMD remains benchmark-gated. Scalar, plain-array implementations are the
default until a measured hot path demonstrates that a SIMD implementation is
worth its complexity and platform-validation cost. A future such path may use
`fearless_simd` only when it has earned that dependency, a contained boundary,
and determinism coverage.

### Determinism rule

Any code path covered by the determinism contract must not silently depend on
platform SIMD behavior. Options include:

- Scalar math (always deterministic)
- Explicit SIMD kernels with documented platform behavior and determinism tests

The right choice may vary per measured operation. This ADR does not permit an
unmeasured SIMD dependency or a public vector-type coupling.

### Workspace dependencies

Neither `glam` nor `fearless_simd` is a workspace dependency. Crates use
`exedra_math` for shared arithmetic today.

## Consequences

- Exedra's public surface remains stable regardless of internal math choices.
- Shared vector arithmetic remains small, explicit, and available to each
  native head without changing its value type.
- Determinism-sensitive code must be explicitly identified and tested.
- SIMD work begins with a benchmark and a contained design, not a dependency
  choice in advance.
