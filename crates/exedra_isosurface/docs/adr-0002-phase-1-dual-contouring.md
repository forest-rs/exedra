# ADR-0002: Phase-1 Dual Contouring in `exedra_isosurface`

- Status: Accepted
- Date: 2026-03-24

## Context

The implicit branch now has the field seam, Hermite bridge, QEF solver, and
MeshBuilder attribute tagging needed for a first real extractor.

The full target design still includes harder work:

- manifold DC,
- richer provenance and seam tagging,
- certified error bounds beyond the current measured refinement indicators.

Waiting for the entire end-state before landing an extractor would delay the
 first point where the workspace can actually turn scalar fields into Exedra
 meshes.

## Decision

`exedra_isosurface` will ship a phase-1 dual-contouring path now with these
constraints:

- build an interval-culled octree via `exedra_spatial`,
- when `max_depth` permits, refine every intersecting candidate to depth 2
  before considering retention, because primal-edge emission needs at least
  three cyclic-distinct incident dual vertices to form a face;
- retain a candidate leaf beyond that private emitter floor only when it has
  complete Hermite evidence for every
  classified crossing edge, one non-checkerboard surface component, an
  unclamped finite QEF solve, and both its QEF RMS and normal-turn curvature
  indicator fit within one quarter of the max-depth finest-cell diagonal;
  uncertain, partial, or topology-unsafe cells refine toward `max_depth`,
- as a strict exception when the preceding decision would refine solely for
  normal-turn curvature, retain a leaf with the private typed
  `RetainRedundantHermitePlanes` decision only when its normalized finite
  Hermite normals form exact oriented groups in stable cube-edge order,
  signed zero is canonicalized, each same-normal group is exactly coplanar in
  cell-local coordinates against its first hit, the actual QEF rank is one
  through three and equals the group count, every group contains at least two
  distinct cube-edge hits, and every normalized plane has exactly zero local
  residual at the QEF point; no tolerance or tunable threshold participates,
- place one vertex per usable classified surface-component QEF in each
  contributing octree leaf using Hermite samples and `exedra_qef`;
  `active_cells` continues to count contributing leaves while `vertices`
  counts emitted component or compatibility representatives,
- classify sign-changing cube edges into deterministic, face-connected surface
  components before solving: ordinary two-crossing faces join their crossings,
  while checkerboard faces use the bilinear asymptotic determinant over a
  canonical in-plane corner cycle; exact determinant ties use the canonical
  `(0, 3)`/`(1, 2)` edge-slot pairing,
- solve one QEF per classified component and retain the original
  all-constraints QEF as compatibility data; unambiguous one-component cells
  reuse their component result only when it consumed every Hermite plane,
- for a `MaxDepthCompatibility` leaf whose routed component has no usable finite
  QEF, alias the route to one compatibility representative rather than emitting
  synthetic component centers. The ordinary path requires a finite
  all-constraints QEF. A bounded exception admits a deterministic cell-center
  representative only when every scalar crossing is represented by a finite
  Hermite position and every component is constraintless; partial crossing
  masks, non-finite corner values/positions, and all non-max-depth decisions
  still return the existing mesh `BuildError`. Cyclic validation sees the
  shared alias, so this fallback cannot silently create a missing or duplicate
  edge,
- when a zero crossing lies at a primal-edge endpoint and its sampled field
  gradient is undefined, use the oriented primal-edge direction as the narrow
  Hermite-normal fallback; non-finite interior-crossing gradients remain
  invalid refinement evidence,
- use root-relative integer `CornerKey` and `CellKey` coordinates as the
  authoritative cell geometry for interval classification, Hermite/QEF
  analysis, projection bounds, cache ownership, and output ordering; integer
  coordinate zero and `resolution` reproduce the caller's exact root bounds,
  while interior coordinates interpolate the exact endpoint values in `f64`
  before one final `f32` rounding; depth validation uses the same endpoint
  extent and requires a step at least as large as the largest coordinate ULP,
- balance all face-adjacent leaves to a depth difference of at most one through
  deterministic, monotone refinement before emission,
- enumerate minimal primal-edge segments from sorted dyadic edge intervals,
  sample only their sorted missing endpoint keys, and emit each interior
  sign-changing segment once in canonical axis/coordinate order,
- route every incident leaf to the component containing the corresponding
  local cube edge; when a coarse leaf covers a fine edge in a face interior,
  require that leaf to have exactly one component,
- cyclically collapse only adjacent duplicate component vertices. Missing
  component mappings or too-short/non-cyclic loops refine a non-max-depth
  culprit; an unresolved max-depth configuration returns the existing mesh
  `BuildError` instead of silently dropping a patch,
- triangulate transition quads with the shorter diagonal when both splits have
  two scale-aware nondegenerate triangles; if only one split is valid, use it,
  and if neither is valid, return the existing typed mesh `BuildError`,
- bias low-rank QEF solves toward the Hermite mass point for each active cell
  instead of always anchoring null-space dimensions to the geometric cell
  center,
- author corner-normal overrides from field gradients using a face-local inset
  sample so render extraction can shade smooth regions more honestly,
- tag `EDGE_SHARPNESS` from the QEF rank and optionally tag `FACE_REGION` when
  the field implements `ProvenanceField<Provenance = u32>`; face provenance is
  sampled at the refined zero crossing of the generating primal edge rather
  than at the generally off-surface centroid of its dual vertices,
- derive a first `EDGE_SEAM` pass from post-build face-region discontinuities
  on shared interior edges,
- expose a deterministic `cell_budget` cap on contributing leaves/output.
  Preserve legacy selection by taking contributors in octree leaf-storage
  order, then sort the selected set by integer key for deterministic emission.
  The cap does not bound analysis, balancing, or sparse sampling, and only a
  leaf actually excluded by a binding cap authorizes omission of its patches.

## Consequences

### Coincident transition vertices and migration

`DualContourParams::vertex_merge_tolerance` is a finite nonnegative distance in
field coordinates. Set it to `0.0` to join only exactly coincident vertices;
choose a positive value explicitly when sampled gradients or QEF roundoff leave
near-coincident representatives. Only edges of otherwise degenerate transition
patches initiate joining. The mesh kernel checks topology before each collapse,
and surviving vertices keep their original positions. Extraction reports the
number of joins and the largest displacement from an original representative.
No domain shift, field perturbation, or triangle deletion substitutes for a
successful checked join. A remaining degenerate patch is an error.

The tolerance bounds total displacement from each original representative,
including chains of joins; it is not a surface-error or feature-preservation
guarantee. Existing parameter literals must add `vertex_merge_tolerance: 0.0`
to preserve exact-only behavior. Stats gain join counts and displacement, and
errors distinguish failed topology surgery from mesh construction.

Interior grid coordinates now avoid rounding the offset product in `f32`, which
could alias adjacent keys even when their final coordinates were representable.
Non-dyadic roots can therefore produce different last-bit coordinates from the
previous two-step `f32` computation. Root endpoints remain exact; callers that
persist exact vertex sequences must deliberately rebaseline those coordinates.

### Bounded analysis, completion and failure evidence

Extraction owns resource policy and evidence; the reusable spatial tree owns
fallible traversal and checks its stored-cell cap before allocation. See
[spatial ADR-0001](../../exedra_spatial/docs/adr-0001-flat-octree-scope.md).
Initial subdivision, balancing and transition completion all use the same hard
cell limit. Tree refinement restores the original leaf on failure; extraction
stops and discards its entire private run rather than reusing visitor state.

`ExtractionLimits` applies before controlled allocations, field callback batches,
topology passes and checked joins. Octree and cache limits bound analysis;
transition limits bound each worklist, with conservative reservations where
appropriate. Segment enumeration sweeps sorted intervals to emit unique covered
subsegments, avoiding quadratic duplicate accumulation. Vertex and face limits
apply to intermediate meshes too; per-cell storage has a fixed cube-topology
bound, and mesh scratch storage is bounded by the vertex/face caps. No limits
claim exact allocator bytes, wall time or control over arbitrary caller code.

A checked field adapter meters interval queries, point/gradient rows, provenance
and projection requests. Invalid scalar values and NaN/reversed intervals stop
extraction with query locations; infinite outward intervals and undefined gradient
directions remain legal. Refused batches do not appear as completed work. The
first field failure takes precedence over consequential solver errors.

Hard exhaustion returns a typed error, accumulated work and geometry progress,
and spatial evidence where the failed operation has a location. Transition
witnesses preserve root-relative edge identity, routed cell/component positions
and sampled source attribution where available. Joining witnesses describe
intermediate geometry. IDs are extraction-scoped, not durable source features.
The error contains a boxed context so the ordinary `Result` remains small.

The separate output `cell_budget` retains its deterministic truncation policy.
`ExtractionReport` distinguishes completion from truncation, with exact eligible,
omitted, boundary-crossing, unresolved-finest-cell and compatibility counts.
Witness detail is caller-capped independently of exact counts. Complete means
that the finite run omitted no eligible interior patch; it does not upgrade
sampling evidence to topology, feature-preservation or geometric-error proof.

Migration: add `limits` and `witness_limit` to parameter literals. `None` limits
preserve unrestricted behavior. Results gain `work` and `report`. Match the
former error variants through `DualContourError::kind` and inspect the boxed
`context`; errors are no longer `Copy` or `Eq` because they preserve geometry.

### Extraction scope

- The workspace now has a real field-to-mesh path for spheres, boxes,
  cylinders, simple CSG references, and multi-scale adaptive leaves.
- The implementation stays structurally honest about what is still missing:
  no manifold guarantees, no topology-optimal variable-depth stitching, only a
  conservative leaf-retention heuristic, and only face-local gradient sampling
  for authored shading normals. The current seam pass marks region boundaries
  only; it is not a full branch-trace recovery.
- Sparse transition emission now scales sampling with visited leaf corners and
  minimal edge-segment endpoints rather than allocating a full finest lattice
  or finest-cell coverage array. Hash maps are lookup structures only; sorted
  integer keys determine batch sampling and output order.
- Within-cell component identity is explicit, shared-face ambiguity is
  resolved reproducibly from shared samples, and transition emission consumes
  the component mapping. This removes cracks caused solely by 2:1 depth
  transitions, but it does not claim a complete manifold-DC construction for
  every ambiguous max-depth configuration.
- Surface-local provenance makes CSG operand attribution less sensitive to
  dual-vertex placement. It remains sampled provenance, not exact analytic
  feature-curve recovery or exact primitive projection.

## Migration note: error-driven leaf retention

Replacing the crossing-count heuristic changes deterministic mesh output:
flat, well-fitted regions may now use much coarser leaves, while enclosed,
partial-evidence, curved, clamped, or topology-unsafe regions refine for an
explicit reason. No call-site migration is required—`DualContourParams` and the
three public extraction entry points are unchanged, and `max_depth` remains the
quality knob. Consumers that persist exact vertex/face sequences or golden
signatures must deliberately rebaseline them.

`BoxField::eval_interval` now supplies the exact mathematical range of its
axis-aligned box SDF over a finite query AABB, expanded outward by one
representable `f32` at each endpoint. Invalid box parameters or query bounds
return `None`, preserving the scalar-field contract's conservative fallback.
This replaces the earlier corner/center sampling widened by the full cell
diagonal; it does not change the retention rule or any public call shape.
Nevertheless, fields containing `BoxField` can produce different deterministic
octrees and meshes because interval-excluded cells no longer create a broad
balance wave. Exact interval behavior composes through the existing transform
and CSG wrappers; cylinder and torus intervals remain conservatively sampled.

For the depth-7 H1 box-union witness, initial error-driven leaf depths are
`[0, 0, 38, 139, 399, 909, 1850, 5360]`; 2:1 balancing changes them to
`[0, 0, 0, 228, 1633, 4187, 6730, 5360]`, and transition completion adds no
leaves. Balance creates contributing leaves by depth
`[0, 0, 0, 4, 118, 568, 1324, 0]`, while completion creates none. These stage
pins distinguish scalar interval pruning from later topology preparation.

Sparse component-aware transition emission is a second deterministic output
change under the same public entry points: mixed-depth patches no longer pass
through a finest-cell coverage raster, multi-component leaves may emit more
than one vertex, and defensive duplicate/incidence suppression is gone. No
call-site migration is required. `DualContourStats::active_cells` remains the
contributing-leaf count; `vertices` is the emitted component/compatibility
representative count and may now differ. `cell_budget` remains an output/leaf
cap, not a bound on analysis or balancing work. Selection remains octree
leaf-storage order for compatibility; the selected subset and all emitted
geometry are then ordered by integer keys.

The fixed error target is `0.25 * finest_cell_diagonal`. QEF RMS is measured in
world units by re-evaluating the normalized Hermite planes at the solved point;
this avoids cancellation in the QEF solver's expanded world-coordinate
quadratic under translation. The curvature indicator is half the current cell
diagonal times the sine of half the largest sampled normal turn. These are
deterministic, scale-aware error indicators, not a certified Hausdorff proof;
the independent quality oracle remains responsible for validating the final
measured bound.

`RetainRedundantHermitePlanes` is a narrower deterministic-output change under
the same public entry points. It does not relax partial-Hermite, non-finite,
topology, clamp, or QEF-residual precedence; it only replaces a curvature-only
refinement after the exact witness above succeeds. Cell-local offsets and
point-to-plane residuals avoid translation-sensitive expanded quadratics.
Fields without useful interval bounds may still produce a more refined mesh
because conservative subdivision creates a different 2:1 balance context;
closed topology and deterministic output remain required, but exact sequences
need not match an interval-capable equivalent field.

The committed depth-7 H1 oracle changes from `5,570` adaptive vertices and
`11,136` triangles to `939` vertices and `1,874` triangles against the same
`30,122`/`60,240` forced-uniform witness: `32.078807242x` and
`32.145144077x`. Its fixed sampled-deviation cap remains
`5.477462339e-2`; measured maxima are `9.964946452e-4` mesh-to-analytic and
`8.143149493e-4` analytic-to-mesh. The harder box-cylinder control remains
closed and deterministic at `1,864` vertices and `3,728` triangles, with
`1,700` ordinary surface projections, `164` feature snaps, and the complete
`1,864`-leaf projection-counter partition. Consumers persisting exact adaptive
mesh signatures must rebaseline; no call-site migration is required.
