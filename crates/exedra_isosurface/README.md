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
- opt-in semi-analytic projection for tagged box/cylinder CSG fields,
- a first dual-contouring extractor over an interval-culled adaptive octree.

The current mesher is intentionally phase-1:

- interval-driven octree culling with conservative mixed-depth leaf retention,
- one dual vertex per active octree leaf,
- explicit triangle emission from primal-edge patches with deterministic diagonal choice,
- QEF placement with edge-sharpness tagging and Hermite mass-point anchoring,
- authored corner normals from field gradients for smoother render extraction,
- optional face-region tagging from `ProvenanceField<u32>`,
- first-pass seam tagging on shared edges where adjacent face regions differ,
- conservative triangle deduplication and edge-incidence limiting across coarse/fine transitions.

`dual_contour_semi_analytic` additionally projects eligible cell vertices onto
the dominating tagged primitive and snaps transverse feature cells for
identity-frame box / coordinate-axis cylinder pairs. Unsupported rotations,
tangencies, coincident patches, ambiguous cells, and out-of-cell projections
retain the bounded QEF result and increment typed counters.

It does not yet attempt full manifold DC, topology-optimal variable-depth
stitching, general clipped-conic feature solving, or richer seam recovery
beyond region boundaries.
