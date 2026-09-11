# joiner_timber

Concrete timber-fitting knowledge for the generic `joiner` construction
layer. The current slice provides a housed heel, a keyed through-tenon from
king post to tie, full-section housed bearings for strut feet, strut heads,
and principal-rafter heads, purlins trenched into principal rafters, and common
rafters seated over purlins. Each fit derives its mating geometry from one
nominal interface; the typed fit allowance enlarges only receiving geometry.
The keyed slot keeps its two load-bearing faces line-to-line and uses that
allowance only across the key.

Physical rule parameters are exact, strictly positive `Length` values backed
by joto iotas. A rule lowers them to meters together at its floating-point
recipe-building boundary, so invalid sizes are excluded by the API while
checks on genuinely derived geometry remain explicit.

This crate owns timber joint selection and recipe generation. It explicitly
does not own the construction graph, constructive/Boolean algorithms,
assembly diagnostics, rendering, statics, or connection capacity. The crate
boundary was decided in `joiner` ADR 0001 and `bsl-6ihj`. These rules check
setout, fit, and minimum geometric relish; they do not size a connection for
loads or certify a truss. The secondary-roof rules intentionally remain two
named forms rather than one generic crossing notch: they have different
receivers, load directions, and material-preservation checks.

`RoundPurlinSeatRule` adds a shallow flat seat underneath a circular purlin
crossing a rectangular support. Roles are `round-purlin` and `purlin-support`;
the nominal purlin is a full circular extrusion along local X, inscribed in
its square Y/Z extent. The relation node fixes the support top and finished
seat depth. Setout places both timbers; the rule cuts only the purlin and
leaves the bearing plane line-to-line. Its rectangular contact uses the circle
chord at that depth, with explicit remaining-depth, end-relish, and minimum
bearing checks. This is a section contract, not an inference from a box:
verify custom composed recipes with `joiner::measure_contact_geometry`.

Existing rules retain their roles and receivers. New callers can use
`Construction::apply_rule` to instantiate and register a fit, and
`OrientedBox::local_point` / `local_placement` for target-local anchors and
tools. `FitClass::allowance_meters` now exposes the per-side profile allowance
without callers repeating the enum match; it never enlarges bearing depth.
