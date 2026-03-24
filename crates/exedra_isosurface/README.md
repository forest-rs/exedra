# exedra_isosurface

Implicit-field seams and extraction-facing data types for the Exedra workspace.

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
- a first dual-contouring extractor over a culled max-depth octree.

The current mesher is intentionally phase-1:

- interval-driven octree culling,
- one dual vertex per active max-depth cell,
- explicit triangle emission from primal-edge patches with deterministic diagonal choice,
- QEF placement with edge-sharpness tagging and Hermite mass-point anchoring,
- authored corner normals from field gradients for smoother render extraction,
- optional face-region tagging from `ProvenanceField<u32>`,
- first-pass seam tagging on shared edges where adjacent face regions differ.

It does not yet attempt manifold DC, variable-depth stitching, or seam tagging.
