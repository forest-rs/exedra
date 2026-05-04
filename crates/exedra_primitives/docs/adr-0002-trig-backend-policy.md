# ADR 0002: Trigonometric Backend Policy

## Status

Accepted.

## Context

`exedra_primitives` is `#![no_std]` with `alloc` and generates deterministic
mesh primitives for downstream modeling workflows. Rotational primitives need
`sin`, `cos`, and `sqrt` for coordinates, but the crate should avoid dependency
creep and should not let floating-point approximation details affect topology
or semantic selections.

## Decision

Use backend-provided `f32` math instead of a custom polynomial approximation.

- With the default `std` feature, use platform `f32` math.
- With `--no-default-features --features libm`, use the optional `libm` backend.
- Require one of `std` or `libm`; there is no hidden fallback approximation.
- Derive topology, face regions, and selections from integer segment/ring
  indices rather than trigonometric results.
- Guarantee deterministic output for a fixed backend, target, and parameter set.
  Bit-identical coordinates across `std` and `libm` backends are not part of the
  contract.

Tests enforce a `2e-6` absolute error budget against an `f64` reference for
unit-circle sampled angles used by cylinder, cone, torus, and UV sphere
generation. Coordinate error scales linearly with primitive radius.

## Consequences

The default build remains dependency-light and uses only `std` math. `no_std`
users opt into one small math dependency through the existing `libm` feature.
Primitive topology and selections remain stable even if backend coordinate bits
differ slightly.
