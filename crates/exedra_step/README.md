# exedra_step

Explicit STEP exchange from retained native boundaries. Display meshes are not
read or reconstructed. The exporter is `no_std` + `alloc`; `std` and `libm`
features select the same numeric backends as the analytic head.

`export_body(&body, placement, &StepOptions)` writes a deterministic ISO 10303-21
text document declaring AP214 `AUTOMOTIVE_DESIGN`. The first supported solid
family is one connected, oriented closed shell with planar, cylindrical, conical
spherical and regular ring-toroidal supporting faces and line/circle/ellipse edges. This
covers exact extrusions, through holes, blind pockets, supported axial finishes,
and supported line/arc revolutions. Certified spherical pole boundaries
have explicit point topology. Multiple boundary shells are supported for exact
rectilinear solids: one connected material component with disjoint closed
cavities. Other multi-shell support families, disconnected solids, touching or
nested void boundaries, open incidence and ambiguous multiple-turn ranges are
typed refusals.

Multiple-shell export independently certifies whole material membership in the
finite exact coordinate-plane arrangement, matches every input face patch to
its outward material transition, and checks geometric boundary links and void
ownership. It does not reconstruct a Boolean or mesh. `shell_validation` bounds
native topology/trim work, arrangement cells and face scans separately. Flat
scratch memory is reported, including the plane-vertex link table (at most eight
vertices per arrangement cell). This certificate is restricted to axis-aligned
plane/line boundaries; it makes no claim about arbitrary curved solids.

The writer emits `BREP_WITH_VOIDS`, one base `CLOSED_SHELL` per boundary, and one
`ORIENTED_CLOSED_SHELL` with orientation `FALSE` per void. Void base face and bound
senses are reversed deliberately; the wrapper restores native material-outward
orientation. `correspondence.reversed_faces` records that distinction. Counts
include boundary shells, not the extra oriented wrappers, and always one solid.

Length units state the physical meaning of one source coordinate: millimetres
or metres. Geometry is not silently rescaled. Occurrence placement is explicit
and baked in f64; no currently displayed realization is required. Metadata names
use STEP quote/reverse-solidus/Unicode encoding. Timestamp metadata is supplied
by the caller; the default epoch placeholder keeps default exports deterministic.

The report separates input support agreement, a conservative allowance for
normalized STEP frame directions and placed f64 arithmetic, and support-parameter
conversion error. Their sum and the independent material certificate's support/arithmetic
allowance must both fit the declared uncertainty. Defining geometry remains analytic; no curve fitting
or tessellation approximation is introduced. Single-shell support checks do not establish
global solid validity, trim nesting or shell containment. The distinct multi-shell
material evidence is included in the report. Neither form represents
independent-reader certification. A successful core export is bounded native
serialization with the stated checks.

Native cone height is axial. Its signed radial slope becomes a STEP semi-angle;
a negative slope reverses the surface Y/Z frame axes and both parameter
directions, preserving the outward normal while emitting a positive angle.
Zero slope emits an exact cylinder. Conversion error is bounded over the native
height domain and the complete incident curve enclosures; insufficient declared uncertainty, unrepresentable angles and
ill-conditioned extreme slopes or nonzero semi-angles below `1e-12` radians are
typed refusals. The latter prevents the observed independent reader's silent
angle clamp, which otherwise changes native volume and area. Cone domains avoid the apex.
Torus parameters and radii map directly. Each periodic support coordinate spans
at most one turn; shared seam edges remain ordinary shared topology, including
a doubly periodic single-face torus. The writer emits 3D defining curves and
incidence; independent reader checks cover its periodic trim reconstruction.

Sphere longitude/latitude parameters and radius map directly. A certified
collapsed boundary maps to its authoritative exported `VERTEX_POINT` through
`StepEdgeEntity::CollapsedVertex`; a regular edge maps to `Curve(EDGE_CURVE)`
or `CurveChain` for several exact elliptic-arc fragments.
Only unused source edges are `None`. Collapsed coedges are omitted from the
serialized edge loop, retaining ordinary meridian seams. A pole-only loop emits
`VERTEX_LOOP` rather than a zero-length curve. Counts describe actual emitted
records; an independent reader may reconstruct natural degenerate edges and
pole vertices absent from those records. No fake circle or line represents a
collapsed boundary.

Native vertex, edge, loop and face IDs map to explicit STEP representations.
Elliptic spans use exact shared arcs on the original `ELLIPSE` support. The
bounded subdivision policy starts with eight fragments per turn and increases
by powers of two, up to 128, with physical axial excursion and ellipse aspect
ratio. The cap and ordinary output budgets bound work; the defining geometry
remains exact when the cap is reached. This
addresses observed independent-reader reconstruction errors on closed rephased
rims and metre-valued files; it is an empirical interoperability guard, not a
universal reader parameter-curve certificate. Very large, highly eccentric
sections can still exceed the observed reader's trim-reconstruction limits. Exact equal-axis ellipses emit
`CIRCLE` and retain one edge. No fitted curve or mesh approximation is introduced.
`StepEdgeEntity::curve_entities` enumerates every fragment in native traversal
order; reversed coedges traverse that same sequence backward. Each fragment is
shared by both incident faces. Counts include the added vertices, regular edges
and oriented occurrences, and ordinary output budgets include all added entities.
Detached unused entries stay `None` in correspondence. Caller feature labels
remain available through that mapping; exchange does not own application scenes
or material policy. Entity/input-work and byte budgets bound failure before a
partial document could escape. Output is streamed into one bounded String with
reusable reference scratch rather than allocating a temporary string per entity.

Run `cargo run --release -p step_exchange --bin step_exchange -- /tmp/step-fixtures` for complete
native fixtures. The separate example's reader-oracle script checks transfer,
raw resulting BRep validity, topology, exact measures, placement and unit scaling
through independently installed Open CASCADE Python bindings. Missing reader
support is reported as unavailable with a nonzero exit, never as a passed check.

The implementation was written independently from Exedra's own data model and
public STEP entity documentation. No OpenGeometry or usd-brep/SMLib implementation
source was read or copied for this work. Relevant format references:

- [STEP Tools advanced_face entity](https://www.steptools.com/stds/stp_aim/html/t_advanced_face.html)
- [STEP Tools vertex_loop entity](https://www.steptools.com/docs/stp_aim/html/t_vertex_loop.html)
- [STEP Tools spherical_surface entity](https://www.steptools.com/docs/stp_aim/html/t_spherical_surface.html)
- [STEP Tools edge_curve entity](https://www.steptools.com/stds/stp_aim/html/t_edge_curve.html)
- [STEP Tools advanced BRep representation](https://www.steptools.com/stds/stp_aim/html/t_advanced_brep_shape_representation.html)
- [STEP Tools conical_surface entity](https://www.steptools.com/docs/stp_aim/html/t_conical_surface.html)
- [STEP Tools toroidal_surface entity](https://www.steptools.com/docs/stp_aim/html/t_toroidal_surface.html)
- [Open CASCADE cone parameterization](https://dev.opencascade.org/doc/refman/html/class_geom___conical_surface.html)
- [Open CASCADE torus parameterization](https://dev.opencascade.org/doc/refman/html/class_geom___toroidal_surface.html)
- [Open CASCADE STEP translator documentation](https://dev.opencascade.org/doc/overview/html/occt_user_guides__step.html)

Licensed under Apache-2.0 OR MIT.
