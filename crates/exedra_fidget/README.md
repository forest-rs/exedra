# exedra_fidget

Thin adapter crate that lets `fidget` shapes implement the
`exedra_isosurface::ScalarField` seam.

Current scope:

- VM-backed evaluation through `fidget`,
- optional JIT-backed aliases behind the `jit` feature,
- no ownership of meshing, octrees, or generic implicit semantics.
