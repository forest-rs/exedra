# exedra_isosurface

Implicit-field seams and extraction-facing data types for the Exedra workspace.

Current scope:

- `ScalarField` and its extension traits,
- Hermite intersection and per-cell bridge data,
- analytic reference fields (`SphereField`, `BoxField`, `CylinderField`,
  `TorusField`, `HalfSpaceField`),
- simple CSG combinators and provenance tagging wrappers for tests,
- a first dual-contouring extractor over a culled max-depth octree.

The current mesher is intentionally phase-1:

- interval-driven octree culling,
- one dual vertex per active max-depth cell,
- QEF placement with edge-sharpness tagging,
- optional face-region tagging from `ProvenanceField<u32>`.

It does not yet attempt manifold DC, variable-depth stitching, or seam tagging.
