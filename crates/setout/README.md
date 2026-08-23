# setout

`setout` is the exact, deterministic setting-out kernel for Exedra. It resolves
typed quantities from explicit roots and multi-way relations while retaining the
structural provenance of every result.

In architecture, **setting-out** turns design intent and controlling
measurements into the datums, points, dimensions, and alignments from which the
work is located. Here the same idea applies to virtual construction: a span,
wall head, plate, and rise determine the seats, ridge, pitch, endpoints, and
every dependent element instead of leaving each modeler to retype coordinates.

The crate is `no_std` by default. It owns quantity propagation, conflicts,
decisions, explain, fingerprints, and quantity-granular change reporting. It does
not own building construction, historical-reconstruction policy, mesh geometry,
or generative topology.

The first production consumer is the basilica roof: its wall plates, ridge,
rafters, gables, and roof skin are resolved from one exact section rather than
from repeated floating-point calculations.
