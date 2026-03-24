# ef-1dz9 Fidget gradient seeding

## Goals
- Fix `exedra_fidget` so gradient evaluation follows Fidget's documented grad-slice calling convention.
- Restore meaningful Hermite normals and QEF placement for Fidget-backed extraction.
- Add focused regression coverage that would have caught the original bug.

## Non-goals
- Benchmarking.
- Provenance recovery.
- Mixed-depth meshing improvements.

## Steps
1. Replace zero-derivative `Grad::from(value)` inputs with basis-seeded `Grad::new(...)` inputs.
2. Add a direct gradient regression on simple `x`/sphere-style shapes.
3. Add a Fidget-backed sphere extraction regression that checks vertices are no longer all on the cell-center lattice.
4. Revalidate and regenerate the sphere OBJ.

## Risks
- Some existing test expectations may shift because Hermite normals become materially different.
- If the sphere still collapses after this fix, the remaining problem lies elsewhere in the mesher path.
