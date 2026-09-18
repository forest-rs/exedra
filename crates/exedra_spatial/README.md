# `exedra_spatial`

Small, deterministic spatial primitives for adaptive geometry algorithms.

Current scope:

- `Aabb` utilities for axis-aligned spatial bounds,
- a flat adaptive octree with deterministic child ordering,
- visitor-driven construction,
- deterministic depth-first and breadth-first traversal,
- leaf-neighbor queries by spatial adjacency,
- incremental leaf refinement without rebuilding the whole tree.

```rust
use core::convert::Infallible;
use exedra_spatial::{Aabb, CellRef, Octree, OctreeVisitor};

struct OneLevel;

impl OctreeVisitor for OneLevel {
    type Payload = [f32; 3];
    type Error = Infallible;

    fn should_subdivide(&mut self, cell: CellRef) -> Result<bool, Self::Error> {
        Ok(cell.depth == 0)
    }

    fn make_leaf_payload(&mut self, cell: CellRef) -> Result<Self::Payload, Self::Error> {
        Ok(cell.bounds.center())
    }
}

let bounds = Aabb::new([0.0; 3], [1.0; 3]).expect("ordered bounds");
let tree = Octree::build(bounds, 1, Some(9), &mut OneLevel).expect("bounded tree");
assert_eq!(tree.leaf_ids().count(), 8);
```

`Octree` is a flat, append-only arena: cell IDs remain stable when a leaf is
refined. It is not yet a broad spatial-query toolkit. In particular,
`leaf_neighbors` is a correctness-first linear scan over stored leaves.
`Aabb::new` rejects non-finite or reversed bounds, and `refine_leaf` reports a
typed error when a requested leaf is missing, already internal, or at the
requested depth limit.

Construction and refinement also stop on visitor failure or a stored-cell
limit, checked before allocation. Failed refinement restores the original tree;
visitor side effects remain, so discard associated visitor state before retrying.
Errors preserve the failing bounds and cell count. Cell limits do not bound
arbitrary visitor or payload allocations.

Migration: visitor callbacks now return `Result` and declare `type Error`.
Use `Infallible` and `Ok(...)` for infallible visitors. `build` and `refine_leaf`
take a cell-limit argument (`None` for unrestricted cell counts) and return
`Result` with an `OctreeError`.

`leaf_ids()` returns an iterator without allocating a worklist. Call `.count()`
for the number of leaves or explicitly collect when a retained list is needed.

This crate is `no_std` with `alloc` and has no dependencies. It is intentionally
geometry-agnostic: fields, Hermite sampling, QEF fitting, and mesh extraction
remain outside its boundary.

## License

Apache-2.0 OR MIT
