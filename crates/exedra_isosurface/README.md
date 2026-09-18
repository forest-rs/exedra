# `exedra_isosurface`

Implicit fields and dual-contouring surface extraction for Exedra.

```rust
use exedra_isosurface::{
    Aabb, DualContourParams, EdgeSearchParams, QefParams,
    analytic::SphereField, dual_contour,
};

let field = SphereField {
    center: [0.0, 0.0, 0.0],
    radius: 1.0,
};
let params = DualContourParams {
    root_bounds: Aabb::new([-1.5; 3], [1.5; 3]).expect("ordered bounds"),
    max_depth: 4,
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
- field-construction wrappers (`Translate`, `UniformScale`, `Transform3`),
- simple CSG combinators and provenance tagging wrappers for tests,
- opt-in semi-analytic projection for tagged box/cylinder CSG fields,
- a first dual-contouring extractor over an interval-culled adaptive octree.

The current mesher is intentionally phase-1:

- interval-driven octree culling with conservative mixed-depth leaf retention,
- one dual vertex per classified surface component in each active octree leaf,
- explicit triangle emission from primal-edge patches with deterministic diagonal choice,
- QEF placement with edge-sharpness tagging and Hermite mass-point anchoring,
- authored corner normals from field gradients for smoother render extraction,
- optional face-region tagging from `ProvenanceField<u32>`,
- first-pass seam tagging on shared edges where adjacent face regions differ,
- balanced, component-aware transitions across coarse/fine boundaries,
- topology-checked joining of coincident transition vertices, with an explicit
  field-coordinate tolerance for near-coincident solves and measured displacement.

Existing parameter literals must add `vertex_merge_tolerance`. Use `0.0` for
exact coincidences only. A positive tolerance allows bounded joins on degenerate
transition patches; it does not certify surface accuracy or feature retention.
Remaining degeneracies and topology refusals return errors. Invalid bounds,
unrepresentable grid depths, QEF policies and join tolerances fail before field
evaluation. `DualContourStats` gains join counts and displacement and implements
`PartialEq` rather than `Eq`.

## Work limits and evidence

`cell_budget` deliberately limits contributing output cells. It still performs
analysis, and may return an empty or open mesh. Inspect `result.report.completion`:
`TruncatedByCellBudget` gives exact omitted-cell and omitted-patch counts, omitted
bounds, and bounded cell witnesses. `Complete` means no eligible interior patches
were omitted by that budget. It does **not** certify a closed surface, feature
retention, geometric accuracy, or enclosure within the caller's root domain.
The report also counts root-boundary crossings, unresolved finest cells and
finest cells using compatibility evidence. `witness_limit` bounds only the detail
list; counts remain exact, including `unreported_witnesses`.

Use `limits: ExtractionLimits { ... }` to stop analysis and generation. Independent
caps cover octree cells (including later refinements), cached corners, transition
worklists, mesh vertices/faces, field evaluations, topology passes and attempted
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
