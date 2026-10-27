# exedra_step

Explicit STEP exchange from retained native boundaries. Display meshes are not
read or reconstructed. The exporter is `no_std` + `alloc`; `std` and `libm`
features select the same numeric backends as the analytic head.

`export_body(&body, placement, &StepOptions)` writes a deterministic ISO 10303-21
text document declaring AP214 `AUTOMOTIVE_DESIGN`. The first supported solid
family is one connected, oriented closed shell with planar/cylindrical supporting
faces and line/circle edges. This covers exact extrusions, through holes, blind
pockets and supported axial finishes. Other surfaces, multiple shells/cavities,
open incidence and ambiguous multiple-turn circle ranges are typed refusals.

Length units state the physical meaning of one source coordinate: millimetres
or metres. Geometry is not silently rescaled. Occurrence placement is explicit
and baked in f64; no currently displayed realization is required. Metadata names
use STEP quote/reverse-solidus/Unicode encoding. Timestamp metadata is supplied
by the caller; the default epoch placeholder keeps default exports deterministic.

The report separates input support agreement from a conservative allowance for
normalized STEP frame directions and placed f64 arithmetic. Their sum must fit
the declared uncertainty. Defining geometry remains analytic; no curve fitting
or tessellation approximation is introduced. These checks do not establish
global solid validity, trim nesting or shell containment, and do not represent
independent-reader certification. A successful core export is bounded native
serialization with the stated checks.

Native vertex, edge, loop and face IDs map to topological STEP entity IDs. Shared
edges are emitted once and oriented edge uses preserve their native traversal.
Detached unused entries stay `None` in correspondence. Caller feature labels
remain available through that mapping; exchange does not own application scenes
or material policy. Entity/input-work and byte budgets bound failure before a
partial document could escape. Output is streamed into one bounded String with
reusable reference scratch rather than allocating a temporary string per entity.

Run `cargo run --release -p step_exchange -- /tmp/step-fixtures` for complete
native fixtures. The separate example's reader-oracle script checks transfer,
raw resulting BRep validity, topology, exact measures, placement and unit scaling
through independently installed Open CASCADE Python bindings. Missing reader
support is reported as unavailable with a nonzero exit, never as a passed check.

The implementation was written independently from Exedra's own data model and
public STEP entity documentation. No OpenGeometry or usd-brep/SMLib implementation
source was read or copied for this work. Relevant format references:

- [STEP Tools advanced_face entity](https://www.steptools.com/stds/stp_aim/html/t_advanced_face.html)
- [STEP Tools edge_curve entity](https://www.steptools.com/stds/stp_aim/html/t_edge_curve.html)
- [STEP Tools advanced BRep representation](https://www.steptools.com/stds/stp_aim/html/t_advanced_brep_shape_representation.html)
- [Open CASCADE STEP translator documentation](https://dev.opencascade.org/doc/overview/html/occt_user_guides__step.html)

Licensed under Apache-2.0 OR MIT.
