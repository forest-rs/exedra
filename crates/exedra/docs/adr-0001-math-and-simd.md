# ADR-0001: Math Libraries and SIMD Strategy

**Status:** Accepted (partially open)
**Date:** 2026-03-03

## Context

Exedra is a `no_std` mesh kernel with a determinism contract: identical inputs +
parameters must produce identical outputs across runs. The choice of math library
and SIMD strategy directly affects both ergonomics and determinism guarantees.

Cambium, the operator layer above Exedra, has different constraints — it owns
high-level modeling operations where ergonomic vector math matters more and
strict cross-platform bit-identical results are less critical.

## Decision

### Cambium

Cambium uses **glam** as its math library. No ambiguity here.

### Exedra public API

Exedra's public API uses **plain arrays** (`[f32; 3]`, `[f32; 2]`, etc.) with
typedefs where helpful. This keeps the API calm, math-library-agnostic, and
avoids coupling callers to a specific math crate.

### Exedra internals

**Open.** Glam may be used internally where convenient, but this is not yet
decided. We will evaluate as implementation proceeds and see how it feels.

Key considerations:

- Glam's platform-specific SIMD can produce bit-different floating-point results
  across architectures. Any determinism-sensitive path must account for this.
- For hot batch kernels (normal computation, triangulation, extraction),
  explicit SIMD via **fearless_simd** is preferred over implicit SIMD from a
  math library.
- If glam is used inside Exedra, it should be behind an internal boundary — not
  leaked into the public API.

### Determinism rule

Any code path covered by the determinism contract must not silently depend on
platform SIMD behavior. Options include:

- Scalar math (always deterministic)
- Explicit SIMD kernels with documented platform behavior
- Glam's `scalar-math` feature (disables platform SIMD, but sacrifices performance)

The right choice may vary per operation. This ADR does not lock a single answer.

### Workspace dependencies

Both `glam` and `fearless_simd` are available as workspace dependencies. Crates
opt in as needed.

## Consequences

- Exedra's public surface remains stable regardless of internal math choices.
- Cambium can use idiomatic vector math freely.
- Determinism-sensitive code must be explicitly identified and tested.
- This ADR should be revisited once Exedra has enough implementation to evaluate
  whether internal glam use is practical and worth the dependency.
