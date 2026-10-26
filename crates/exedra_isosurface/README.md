# `exedra_isosurface`

Implicit fields and dual-contouring surface extraction for Exedra.

```rust
use exedra_isosurface::{
    Aabb, DualContourParams, EdgeSearchParams, QefParams,
    analytic::SphereField, dual_contour,
};

let field = SphereField::new([0.0, 0.0, 0.0], 1.0).expect("valid sphere");
let params = DualContourParams {
    root_bounds: Aabb::new([-1.5; 3], [1.5; 3]).expect("ordered bounds"),
    max_depth: 4,
    thin_feature_depth: 0,
    cell_budget: None,
    limits: Default::default(),
    witness_limit: 16,
    vertex_merge_tolerance: 0.0,
    edge_search: EdgeSearchParams::default(),
    qef: QefParams::default(),
};
let result = dual_contour(&field, &params).expect("sphere extraction");

assert!(result.stats.faces > 0);
assert!(result.mesh.validate_deep().is_empty());
```

Built-in fields have checked `new` constructors that refuse non-finite
coordinates, nonpositive dimensions and unusable directions with a typed
`FieldError` naming the parameter. Struct literals remain available and skip
the checks.

Implement `ScalarField` to supply interval, point, and gradient evaluation;
then call `dual_contour`. `Aabb` and `QefParams` are re-exported because they
are part of the extraction parameter surface. `DualContourResult` returns both
the mesh and work counters.

Current scope:

- `ScalarField` and its extension traits,
- `ScalarField2d` and 2D profile bounds for profile-based construction,
- Hermite intersection and per-cell bridge data,
- analytic reference fields (`SphereField`, `BoxField`, `CylinderField`,
  `TorusField`, `HalfSpaceField`),
- analytic 2D reference profiles (`CircleField2d`, `RectField2d`,
  `HalfPlaneField2d`),
- lifting operators (`Extrude`, `Revolve`) for profile-based 3D fields,
- further exact primitives after Íñigo Quilez's distance functions
  (`BoxFrameField`, `LinkField`, `OctahedronField`, `PyramidField`,
  `HexPrismField`, `CappedConeField`),
- field-construction wrappers (`Translate`, `UniformScale`, `Transform3`,
  `ScaleValue`, `Offset`, `Shell`),
- domain operators (`Elongate`, `Twist`, `Repeat`),
- `FormulaField`, which makes a field from a caller's `Formula`, a distance
  function written once over any `Number`: evaluating it on `Dual` numbers
  gives exact gradients in one pass, and on `Interval`s bounds over a cell
  that tighten as cells shrink,
- `FnField`, which makes a field from a caller's distance closure and
  Lipschitz claim, deriving intervals, and gradients by central differences,
- simple CSG combinators and provenance tagging wrappers for tests,
- opt-in semi-analytic projection for tagged box/cylinder CSG fields,
- a first dual-contouring extractor over an interval-culled adaptive octree.

The current mesher is intentionally phase-1:

- interval-driven octree culling with conservative mixed-depth leaf retention,
- one search per crossed grid edge, shared by every cell with that edge, starting
  from the cached corner samples: false position rounded to a grid of
  `2^precision_bits` steps, so the step found depends only on signs, as with
  bisection, in a few samples instead of `precision_bits`,
- one dual vertex per classified surface component in each active octree leaf,
- explicit triangle emission from primal-edge patches with deterministic diagonal choice,
- QEF placement with edge-sharpness tagging and Hermite mass-point anchoring,
- authored corner normals from field gradients for smoother render extraction,
- optional face-region tagging from any `ProvenanceField` whose provenance
  implements `ProjectRegion`, including nested CSG of `u32` tags,
- first-pass seam tagging on shared edges where adjacent face regions differ,
- balanced, component-aware transitions across coarse/fine boundaries,
- topology-checked joining of coincident transition vertices, with an explicit
  field-coordinate tolerance for near-coincident solves and measured displacement.

Where a cell's component meets a face in two arcs, as at a thin wedge, and
the neighboring cell's does too, each arc gets a vertex of its own on the face,
also where finer neighbors split the face's sides, so the dual edge between the
two cells is not doubled and the mesh stays manifold. A sample exactly on the surface counts as outside: inside means strictly
negative. Vertices keep a thousandth of a cell off its faces, so distinct cells never
yield coincident vertices, even where a crease runs through grid nodes.

A feature thinner than a `max_depth` cell, such as the tip of an acute wedge where
two surfaces meet, can pass between a cell's corners without changing any of their
signs. With `thin_feature_depth` above zero, such a cell is sampled at the corners
its children would have and refined below `max_depth` where those disagree, down to
`max_depth + thin_feature_depth`; `report.thin_feature_cells` counts them. The probe
is a sample, not a proof: a feature missing those corners stays lost and counts in
`report.unresolved_cells`, and cells whose corners already disagree are not probed. A lone grid node inside such a feature still closes a tiny
surface of its own. Descending the field from inside it, along a path the field's
interval bounds show inside, shows whether the rest of the mesh already encloses the
region it samples: if so it is a stray sample and is dropped, counted in
`report.dropped_islands`; otherwise it is kept, however small.

Sampling also stops short of a crease where two surfaces meet at an acute angle,
and the mesh bridges from one surface to the other in a sawtooth. A field that
knows its pieces says, through `ScalarField::eval_crease`, where the nearest two
meet: every `FormulaField` follows its `min`, `max` and `abs` to the tie between
the sides they choose. Vertices within two finest cells of such a crease move onto
it by Newton steps on the field, the tie and the plane across the crease together,
unless that would turn a face around them over; `report.crease_vertices` counts
them. Edges still bridging from one side to the other are then split where they
cross the tie, the new vertex moves onto the crease, and the faces either side are
divided along it, so the crease runs through vertices and every face keeps to one
piece; `report.crease_splits` counts them. Where a division would turn a triangle
against the field at its corners, as where the pieces just touch and the crease
ends, the bridges stay. Dropping one loses at
most a finest cell or two of a tip the grid already truncates; recovering where the
two surfaces of such a tip meet needs those surfaces from the field, not one value.
Existing parameter literals must add `vertex_merge_tolerance`. Use `0.0` for
exact coincidences only. A positive tolerance allows bounded joins on degenerate
transition patches; it does not certify surface accuracy or feature retention.
Remaining degeneracies and topology refusals return errors. Invalid bounds,
unrepresentable grid depths, QEF policies and join tolerances fail before field
evaluation. `DualContourStats` gains join counts and displacement and implements
`PartialEq` rather than `Eq`.

## Field claims

`ScalarField` fixes only the sign convention. `ScalarField::capabilities`
states what a field's values additionally promise: a `Metric` (an exact
distance, a Lipschitz bound, or nothing) and a `Support` for its interior
(empty, a box established from its parameters with outward rounding, a box the
provider asserts, unbounded, or unknown). The default claims nothing, so caller
fields stay truthful without opting in.

Built-in primitives claim exact distances within their bounds. Combinators
keep only what their arithmetic justifies: a union or difference of distances
is a Lipschitz bound, not a distance; an intersection is bounded by either
operand; a smooth blend grows the support by its largest dip only when both
operands are exact distances, and otherwise its support is unknown.
Translations, uniform and rigid transforms carry claims with widened bounds,
and extrusion and revolution lift profile claims.

Domain operators claim what survives them. `Elongate` keeps any Lipschitz
bound, though not exactness. `Twist` stretches space more the farther from its
axis, without bound, so it claims no metric, only support. `Repeat` checks the
two nearest copies on each axis, which finds the nearest copy only when the
original lies within half a spacing of the origin; then an exact distance stays
exact, and otherwise it claims no metric.

Three scalings are kept apart. `UniformScale` resizes the geometry, so
distances stay distances. `ScaleValue` multiplies values by a positive factor:
the surface stays put and only the Lipschitz claim changes. `Offset` subtracts
a distance, moving the surface along the field; it grows the support only from
exact distances, since a merely Lipschitz field may stay below the offset
arbitrarily far out, while shrinking keeps any support.

## Octree and QEF modules

`spatial` holds `Aabb` and a flat adaptive octree with deterministic child
ordering, visitor-driven construction, depth- and breadth-first traversal,
leaf-neighbor queries and incremental leaf refinement. `qef` accumulates
point-normal plane constraints and solves the bounded 3x3 quadratic error
function for cell-local vertex placement: it anchors rank-deficient directions,
classifies smooth, edge and corner features by rank, reports residual error,
and clamps the result to the cell. Neither depends on fields or meshes; dual
contouring is their consumer. See [ADR-0006](docs/adr-0006-qef-solver-scope.md)
and [ADR-0007](docs/adr-0007-flat-octree-scope.md).

## Work limits and evidence

`cell_budget` deliberately limits contributing output cells. It still performs
analysis, and may return an empty or open mesh. Inspect `result.report.completion`:
`TruncatedByCellBudget` gives exact omitted-cell and omitted-patch counts, omitted
bounds, and bounded cell witnesses. `Complete` means no eligible interior patches
were omitted by that budget. It does **not** certify a closed surface, feature
retention, geometric accuracy, or enclosure within the caller's root domain.
For enclosure, `report.enclosure` judges the field's support claim against the
root bounds: `Established` or `Asserted` when the claimed interior fits,
`Exceeded` when it reaches outside, as a component wholly beyond the root does
without crossing any root edge, and `Unbounded` or `Unknown` otherwise.
The report also counts root-boundary crossings, unresolved finest cells and
finest cells using compatibility evidence. A field that bounds its gradient over a
box, through `ScalarField::eval_gradient_interval` as every `FormulaField` does,
lets extraction prove a cell whose corners agree holds no surface even where its
value bounds allow one: along an axis whose derivative keeps one sign, the field's
extreme lies on one face, and following such axes down to a corner settles it.
Such cells are neither refined nor reported unresolved; `gradient_excluded_cells`
counts them. `witness_limit` bounds only the detail
list; counts remain exact, including `unreported_witnesses`.

Faces that oppose the field at every corner are unfolded where possible: first
by flipping an edge when both new triangles then agree with the gradient, as
across a crease split along the wrong diagonal, then by moving a vertex to the
middle of its neighbors and back onto the surface, as for fins at corners where
several cells solved onto one feature; features round slightly there. The
report counts `unfolded_flips` and `relaxed_vertices`, and also
`repaired_slivers`, near-collinear triangles that faced backwards where cells
of different sizes meet and were flipped into two well-shaped ones, and
`inverted_faces` whose winding still opposes the field gradient at every corner,
and, for region extraction, `region_attribution`: how many patches took their
region from a sole source, a hard combinator's winner, or the larger
contributor to a smooth blend, where the surface belongs to both.

Use `limits: ExtractionLimits { ... }` to stop analysis and generation. Independent
caps cover octree cells (including later refinements), cached corners, cached edge
crossings, transition worklists, mesh vertices/faces, field evaluations, topology passes and attempted
vertex joins. A hard limit returns an error and no mesh. `None` leaves a resource
unrestricted; use finite caps for all relevant resources to bound a run. Storage
counts are logical entries, including reserved worklist bounds, not allocator
bytes or process RSS. Evaluation work counts calls/rows through the supplied
field's public methods, not arbitrary work performed inside those methods.

Both success and failure preserve accumulated work. Failure additionally carries
the stage and spatial evidence: cell bounds, a generating transition with routed
component positions, a queried point/domain, or an intermediate mesh patch.
A sampled source ID is attribution at the generating crossing; it is not a
complete CSG contributor trace. Non-finite scalar values and invalid interval
responses are typed failures. Undefined gradient directions remain permissible.

Migration: parameter literals must add `limits: Default::default()` and
`witness_limit` (for example, `16`). Match `error.kind: DualContourErrorKind`
instead of variants on `DualContourError`; inspect `error.context` for stage,
work, geometry progress and witnesses. Errors are now `Clone + PartialEq`, not
`Copy + Eq`. Results gain `work` and `report`; existing `stats` describe geometry.
Default hard limits preserve the previous unrestricted resource policy.

`dual_contour_semi_analytic` additionally projects eligible cell vertices onto
the dominating tagged primitive and snaps transverse feature cells for
identity-frame box / coordinate-axis cylinder pairs. Unsupported rotations,
tangencies, coincident patches, ambiguous cells, and out-of-cell projections
retain the bounded QEF result and increment typed counters.

It does not yet attempt full manifold DC, topology-optimal variable-depth
stitching, general clipped-conic feature solving, or richer seam recovery
beyond region boundaries.

The default `std` feature uses native math. For `no_std`, disable defaults and
enable `libm`.

## License

Apache-2.0 OR MIT
