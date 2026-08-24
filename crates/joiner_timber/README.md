# joiner_timber

Concrete timber-fitting knowledge for the generic `joiner` construction
layer. The current slice provides a housed heel, a keyed through-tenon from
king post to tie, and full-section housed bearings for strut feet, strut
heads, and principal-rafter heads. Each fit derives its mating geometry from
one nominal interface; the typed fit allowance enlarges only receiving
geometry. The keyed slot keeps its two load-bearing faces line-to-line and
uses that allowance only across the key.

Physical rule parameters are exact, strictly positive `Length` values backed
by joto iotas. A rule lowers them to meters together at its floating-point
recipe-building boundary, so invalid sizes are excluded by the API while
checks on genuinely derived geometry remain explicit.

This crate owns timber joint selection and recipe generation. It explicitly
does not own the construction graph, constructive/Boolean algorithms,
assembly diagnostics, rendering, statics, or connection capacity. The crate
boundary was decided in `joiner` ADR 0001 and `bsl-6ihj`. These rules check
setout, fit, and minimum geometric relish; they do not size a connection for
loads or certify a truss.
