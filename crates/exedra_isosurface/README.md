# `exedra_isosurface`

Surface meshes extracted from implicit fields. Fields are defined in
`exedra_implicit`; this crate turns their zero sets into meshes.

## Restricted Delaunay meshing

`delaunay_mesh` meshes the zero set of an `exedra_implicit::ScalarField` with
near-isotropic triangles that lie close to the surface, keep its creases and
corners sharp, and close into a manifold around every vertex.

```rust
use exedra_implicit::{Aabb, Cuboid, Cylinder, Difference};
use exedra_isosurface::{DelaunayParams, delaunay_mesh};

let drilled = Difference::new(
    Cuboid::new([0.0; 3], [0.6; 3]).unwrap(),
    Cylinder::new([0.0; 3], [0.0, 0.0, 1.0], 0.3, 1.0).unwrap(),
);
let bounds = Aabb::new([-1.0; 3], [1.0; 3]).unwrap();
let result = delaunay_mesh(&drilled, &DelaunayParams::new(bounds, 0.125)).unwrap();

assert!(result.report.is_complete());
assert_eq!(result.report.corners, 8);
let mesh = result.mesh().expect("a closed manifold");
```

It needs only what every field answers: values, gradients, interval bounds,
and the creases a formula's `min`, `max` and `abs` mark.

1. **Seeds.** A grid of signs over the bounds gives points on the surface
   where cell edges cross it, points on creases, and points on features
   thinner than a cell, found by probing cells the field's bounds cannot
   prove empty. Blocks of cells whose bounds exclude zero are skipped whole.
2. **Creases.** Each crease is traced from its seeds with steps as long as
   the crease stays within the distance tolerance and turns by less than the
   turning limit, and stops at corners, placed where the field and two ties
   are zero. Following Cheng, Dey and Levine's DelPSC, each crease sample
   becomes a weighted vertex protected by a ball that overlaps its
   neighbors, spaced so no ball reaches another crease.
3. **Refinement.** In the regular tetrahedralization of crease samples and
   seeds (`exedra_tetrahedralize`), each tetrahedron takes the field's sign
   at its orthocenter; faces between inside and outside tetrahedra are the
   restricted Delaunay surface. Faces too large, too badly shaped, too far
   from the surface or folded are refined where their dual edge meets the
   surface, and each vertex's faces must close into one disk. A face that
   needs a point inside a protecting ball, or a crease segment the surface
   cuts across, splits those balls, and the pass starts again.

`DelaunayParams::new` derives every length from the cell size. The result
holds `f64` positions, outward-wound triangles and the edges along creases;
`mesh` builds an Exedra mesh with those edges marked sharp, so derived
normals stay hard across them. `DelaunayReport` counts passes, crease
samples, refinements by reason, field queries, and anything left unmet,
including crease segments the surface cuts across instead of following.
Identical inputs give identical meshes.

With `DelaunayParams::regions` set, each triangle is labelled with the region
of the `exedra_implicit::Tagged` part it lies on, and `mesh` writes the labels
as face regions, so a combination's faces can be told apart by part.

Features thinner than a cell are found only where probing reaches them, and
the minimum crease step bounds the thinnest feature whose creases stay sharp.

The default `std` feature uses native math. For `no_std`, disable defaults and
enable `libm`.

## License

Apache-2.0 OR MIT
