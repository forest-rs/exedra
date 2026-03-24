# ei-shxu Mass-point anchoring for dual contour QEF

## Goals
- Reduce visible cell-center quantization on smooth implicit surfaces.
- Keep the change incremental: better low-rank anchoring, not a whole new mesher.
- Preserve the existing `QefSolver::solve` API for current callers.

## Non-goals
- Mixed-depth adaptive stitching.
- Full manifold DC.
- Rewriting the eigensolver or changing sharpness semantics.

## Steps
1. Add an explicit anchor-aware solve path in `exedra_qef` and cover its semantics in unit tests.
2. Compute the Hermite mass point for each active cell in `exedra_isosurface`.
3. Pass that mass point as the QEF anchor instead of the geometric cell center.
4. Add a regression test that sphere-cell solutions no longer collapse to the center lattice, then revalidate and regenerate the sphere OBJ.

## Risks
- Mass-point anchoring may improve smooth cells while modestly changing sharp-feature placement.
- If quantization persists after this, the remaining problem is deeper than the anchor choice.
