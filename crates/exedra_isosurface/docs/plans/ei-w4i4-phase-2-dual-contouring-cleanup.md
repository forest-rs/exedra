# ei-w4i4 Phase-2 dual contouring cleanup

## Goals
- Fix one concrete phase-1 extraction defect visible in current OBJ exports.
- Make output more robust for visualization without claiming full manifold or mixed-depth DC.
- Keep the change local to `exedra_isosurface` if possible.

## Non-goals
- Full adaptive mixed-depth stitching.
- Full seam tagging/provenance recovery.
- Benchmarking against Fidget.

## Steps
1. Reproduce and characterize the visible artifact from current phase-1 DC output.
2. Change face emission so phase-1 DC no longer relies on later fixed-diagonal triangulation of arbitrary quads.
3. Add regression tests around the chosen emission strategy and update ADR/ticket notes.
4. Re-run targeted validation and regenerate a representative OBJ if useful.

## Risks
- Splitting quads changes face counts and edge metadata semantics.
- A purely local triangulation heuristic may improve artifacts without solving all non-manifold cases.
