# setout_generate

`setout_generate` expands resolved exact setout inputs into immutable, labeled
topology fragments. Stable semantic keys survive re-expansion; explicit
omissions remain attached to those keys and unknown targets remain visible as
orphans.

The crate is deliberately consumer-neutral. It does not evaluate setout
relations, create construction elements, or lower exact coordinates to a mesh
or assembly representation. Those translations belong in adapters at the
consumer boundary.

The Basilica ruin supplies two integration proofs. A generated station maps to
one buttress on each elevation in the first; in the second, one west nave-truss
station expands into seven independently addressable fitted timber members and
an omitted station remains explicit. The single east truss stays outside the
generator because a singleton is not a linear distribution.
