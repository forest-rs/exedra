# ei-3w32 Mixed-depth adaptive dual contouring

## Goals
- Let intersecting octree leaves contribute dual vertices at their actual depth.
- Preserve deterministic extraction while stitching coarse and fine leaves through the existing finest-grid sign traversal.
- Keep the change local to `exedra_isosurface::dual_contour` and honest about remaining non-goals.

## Non-goals
- Full recursive manifold dual contouring.
- New scalar-field semantics or provenance models.
- Render-path optimization.

## Steps
1. Represent active leaves by bounds, depth, and finest-grid coverage instead of only max-depth cell coordinates.
2. Solve one dual vertex per intersecting leaf, regardless of depth, using the leaf's actual bounds and corner samples.
3. Build a finest-grid coverage map from covered max-depth cells to owning active leaf vertices.
4. Reuse the existing primal-edge face traversal, but source surrounding vertices through the coverage map and skip degenerate faces after mixed-depth deduplication.
5. Add regressions proving a surface can produce active leaves across multiple depths and still validate as a mesh.

## Risks
- Reusing coarse vertices over many finest-grid cells may surface new degenerate-face cases around transitions.
- This should make the extractor genuinely adaptive, but it still will not guarantee full manifold output on all hard cases.
