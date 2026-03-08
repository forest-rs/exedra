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

## Design

See the [handoff spec](../../docs/exedra_primitives_handoff.md) for the
full design document. See [`docs/adr-0001-primitive-feature-edge-sharpness.md`](docs/adr-0001-primitive-feature-edge-sharpness.md)
for the default sharp-edge contract.

## License

Licensed under either of Apache License 2.0 or MIT license at your
option. See the workspace root for license files.
