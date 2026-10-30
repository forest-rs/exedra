# exedra_tetrahedralize

Deterministic incremental 3D Delaunay tetrahedralization with exact predicates.

This crate owns Delaunay tetrahedralizations of points in a box: build one from
a rectilinear grid, insert points, and read back tetrahedra with explicit
adjacency. Every decision uses the exact-sign `orient3d` and `insphere`
predicates in `exedra_math::predicates`, so the result is a valid Delaunay
tetrahedralization for any finite input, and identical input bits produce
identical tetrahedra on every platform.

It knows nothing about meshes, fields, or tolerances. Consumers decide what the
tetrahedra mean: an isosurface extractor labels vertices inside or outside a
field and keeps the faces between the two; remeshing and volume meshing are
other natural uses. The crate is `no_std` + `alloc` and depends only on
`exedra_math`.

```rust
use exedra_tetrahedralize::Tetrahedralization;

let axis = [0.0, 0.5, 1.0];
let mut tets = Tetrahedralization::grid(&axis, &axis, &axis).expect("valid grid");
assert_eq!(tets.tetrahedron_count(), 8 * 6);

let vertex = tets.insert([0.3, 0.6, 0.2]).expect("inside the box");
assert_eq!(tets.point(vertex), [0.3, 0.6, 0.2]);
assert!(tets.validate().is_ok());
```

## Grids

`Tetrahedralization::grid` takes one strictly increasing coordinate list per
axis and splits every cell into the six Freudenthal tetrahedra around its main
diagonal. Each cell is an exact axis-aligned box, so its eight corners are
exactly cospherical whatever the coordinates round to: the split is exactly
Delaunay with no jitter or symbolic perturbation, and the grid's box is the
hull. Spacing may vary per axis and per cell.

## Insertion

`insert` adds one point by Bowyer–Watson. A visibility walk with a
deterministic randomized face order locates the tetrahedron holding the point,
starting from the nearest grid vertex, so walks stay short whatever order
points arrive in. The cavity of tetrahedra whose circumspheres strictly contain
the point is replaced by tetrahedra joining its boundary to the new vertex.
Exact predicates keep the cavity star-shaped, so no tetrahedron is ever flat
or inverted.

- A point equal to an existing vertex returns that vertex and changes nothing.
- A point on the hull splits the hull face it lies on; the hull stays the box.
- A point outside the hull returns `TetError::OutsideHull`; a non-finite point
  returns `TetError::NonFinite`.

`VertexId`s are stable and dense: grid vertices first, numbered
`(i * ny + j) * nz + k`, then inserted vertices in order. `TetId`s are valid
until the next insertion, which may reuse a removed tetrahedron's index;
`slot_count` bounds them for per-tetrahedron tables.

## Orientation and faces

Every tetrahedron's vertices `[a, b, c, d]` satisfy `orient3d(a, b, c, d) ==
Below`, Shewchuk's positive orientation and the one `insphere` expects.
`neighbors(tet)[i]` is the tetrahedron across the face opposite vertex `i`, or
`None` on the hull, and `face(tet, i)` returns that face counter-clockwise seen
from outside `tet`. Faces between two classes of tetrahedra therefore form a
consistently oriented surface without further checks.

## Diagnostics

`stats` reports insertions, coincident points, walk steps, cavity sizes and
insphere tests, per tetrahedralization and without global counters.
`validate` checks orientation, adjacency, the local Delaunay property of every
face, and every vertex's recorded tetrahedron; it is linear in the size of the
tetrahedralization and meant for tests and debugging.

## License

Apache-2.0 OR MIT
