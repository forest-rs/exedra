# `exedra_fidget`

Adapts [Fidget](https://crates.io/crates/fidget) expression shapes to
`exedra_isosurface::ScalarField`.

```rust
use exedra_fidget::VmField;
use exedra_isosurface::ScalarField;
use fidget::{context::Tree, vm::VmShape};

let x = Tree::x();
let y = Tree::y();
let z = Tree::z();
let sphere = (x.square() + y.square() + z.square()).sqrt() - 1.0;
let field = VmField::new(VmShape::from(sphere)).expect("shape uses only x/y/z");

let mut values = [0.0; 2];
field.eval_points(&[[0.0, 0.0, 0.0], [2.0, 0.0, 0.0]], &mut values);
assert_eq!(values, [-1.0, 1.0]);
```

`VmField` uses Fidget's portable VM backend. Enable the `jit` feature for the
`JitField` alias on supported native targets. Both cache and reuse evaluator
storage for interval queries, point batches, and gradient batches.

## Boundary

The adapter accepts shapes that depend only on the `x`, `y`, and `z` axes;
additional variables are rejected at construction. Surface extraction,
octrees, and the generic implicit-field semantics remain in
`exedra_isosurface` and its supporting crates.

## License

Apache-2.0 OR MIT
