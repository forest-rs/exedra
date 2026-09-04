# `exedra_primitives`

Deterministic mesh primitive generators for Exedra.

The crate provides quads, grids, boxes, cylinders, cones, tori, UV spheres,
and icospheres. Each constructor returns a `Primitive`: an `exedra_mesh::Mesh`
plus semantic face regions and named canonical selections.

```rust
use exedra_primitives::{BoxParams, box_primitive};

let primitive = box_primitive(&BoxParams {
    size: [2.0, 1.0, 0.5],
    ..BoxParams::default()
});
assert!(primitive.mesh.validate_deep().is_empty());

let (_, top) = primitive
    .selections
    .face_sets
    .iter()
    .find(|(name, _)| name.0 == "faces.side_z_pos")
    .expect("boxes publish a top-face selection");
assert_eq!(top.as_slice().len(), 1);
```

Primitive constructors also author default edge sharpness for semantically
obvious feature boundaries, such as box outer edges and capped rotational rims,
so extracted shading matches typical modeling expectations without requiring an
angle-based fallback.

This is `#![no_std]` with `alloc`; IO and debug dumping live elsewhere.

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

See the [handoff spec](https://github.com/forest-rs/exedra/blob/main/docs/exedra_primitives_handoff.md)
for the full design document. See
[`docs/adr-0001-primitive-feature-edge-sharpness.md`](https://github.com/forest-rs/exedra/blob/main/crates/exedra_primitives/docs/adr-0001-primitive-feature-edge-sharpness.md)
for the default sharp-edge contract and
[`docs/adr-0002-trig-backend-policy.md`](https://github.com/forest-rs/exedra/blob/main/crates/exedra_primitives/docs/adr-0002-trig-backend-policy.md)
for the trigonometric backend policy.

## License

Licensed under either of Apache License 2.0 or MIT license at your
option. See the workspace root for license files.
