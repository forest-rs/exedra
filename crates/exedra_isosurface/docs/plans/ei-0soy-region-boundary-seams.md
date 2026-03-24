# ei-0soy Region-boundary seams for dual contouring

## Goals
- Add a truthful first `EDGE_SEAM` pass for provenance-tagged implicit output.
- Reuse existing face-region tagging instead of inventing a richer provenance trace now.
- Keep the implementation local to `exedra_isosurface` post-processing.

## Non-goals
- Full branch-trace seam attribution.
- Mixed-depth seam stitching.
- UV or tangent generation.

## Steps
1. Build the mesh and existing face-region data.
2. Walk shared interior edges after build and compare adjacent `FACE_REGION` values.
3. Mark `EDGE_SEAM` where the two face regions differ.
4. Add a tagged CSG regression test and update docs/ticket notes.

## Risks
- Face-center provenance can still mislabel ambiguous regions in complex blends.
- This tags semantic region breaks, not every geometrically sharp edge.
