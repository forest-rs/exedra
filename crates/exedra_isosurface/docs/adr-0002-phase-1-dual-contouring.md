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
- for a partial `MaxDepthCompatibility` leaf whose routed component has no
  usable finite QEF, alias every such route in that leaf to one finite
  all-constraints compatibility representative rather than emitting synthetic
  component centers; cyclic validation sees that shared alias, and if neither
  representative is usable or the alias makes the neighborhood invalid,
  return the existing mesh `BuildError`,
- when a zero crossing lies at a primal-edge endpoint and its sampled field
  gradient is undefined, use the oriented primal-edge direction as the narrow
  Hermite-normal fallback; non-finite interior-crossing gradients remain
  invalid refinement evidence,
- use root-relative integer `CornerKey` and `CellKey` coordinates as the
  authoritative cell geometry for interval classification, Hermite/QEF
  analysis, projection bounds, cache ownership, and output ordering; integer
  coordinate zero and `resolution` reproduce the caller's exact root bounds,
  while only interior coordinates use step recomposition,
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
