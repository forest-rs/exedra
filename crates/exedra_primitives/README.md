# exedra_primitives

Deterministic mesh primitive generators for exedra.

Provides a small library of primitive shapes (quad, box, cylinder, UV
sphere) that produce Exedra modeling meshes with semantic region tags
and canonical selections. Useful for testing, demos, and wind tunnels.

Primitive constructors also author default edge sharpness for semantically
obvious feature boundaries, such as box outer edges and capped rotational rims,
so extracted shading matches typical modeling expectations without requiring an
angle-based fallback.

This is `#![no_std]` (with `alloc`) — IO and debug dumping live in
`exedra_testkit`.

## Numeric Policy

Rotational primitives use trig only for coordinate generation. Topology,
regions, and canonical selections are derived from integer segment/ring indices,
so small backend math differences cannot change mesh structure.

The default `std` feature uses platform `f32` math. Enabling `libm` selects the
optional `libm` backend even if Cargo feature unification also enables `std`;
`no_std` callers use it with default features disabled. The crate does not
maintain a custom polynomial approximation and does not promise bit-identical
coordinates between std-only and libm builds; output is deterministic for a
fixed backend, target, and parameter set.

Tests enforce a `2e-6` unit-circle sampled-angle absolute error budget relative
to an `f64` reference. Coordinate error scales with primitive radius.

## Design

See the [handoff spec](../../docs/exedra_primitives_handoff.md) for the
full design document. See [`docs/adr-0001-primitive-feature-edge-sharpness.md`](docs/adr-0001-primitive-feature-edge-sharpness.md)
for the default sharp-edge contract and
[`docs/adr-0002-trig-backend-policy.md`](docs/adr-0002-trig-backend-policy.md)
for the trigonometric backend policy.

## License

Licensed under either of Apache License 2.0 or MIT license at your
option. See the workspace root for license files.
