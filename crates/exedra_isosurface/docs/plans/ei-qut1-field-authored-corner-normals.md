# ei-qut1 Field-authored corner normals for dual contouring

## Goals
- Improve implicit mesh shading without changing Exedra's core render policy.
- Keep normal authorship inside `exedra_isosurface` as extraction-time data.
- Preserve determinism and avoid widening the topology contract.

## Non-goals
- Tangent generation.
- Seam tagging.
- Full feature-aware normal splitting beyond what face-local sampling can infer.

## Steps
1. Build the DC mesh as before.
2. Collect one sample point per emitted corner by nudging each corner position toward its face centroid.
3. Batch-evaluate field gradients at those points and write `CORNER_NORMAL_OVERRIDE` through the public edit API.
4. Add regression tests and update docs/ticket notes.

## Risks
- Face-local inset sampling may still produce weak normals near pathological or highly noisy fields.
- Some hard-feature corners may remain visually imperfect until seam/feature-aware shading work exists.
