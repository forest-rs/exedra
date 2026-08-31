# setout_generate

`setout_generate` expands resolved exact setout inputs into immutable, labeled
topology fragments. Stable semantic keys survive re-expansion; explicit
omissions remain attached to those keys and unknown targets remain visible as
orphans.

The crate is deliberately consumer-neutral. It does not evaluate setout
relations, create construction elements, or lower exact coordinates to a mesh
or assembly representation. Those translations belong in adapters at the
consumer boundary.
