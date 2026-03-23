# exedra_isosurface

Implicit-field seams and extraction-facing data types for the Exedra workspace.

Current scope:

- `ScalarField` and its extension traits,
- Hermite intersection and per-cell bridge data,
- analytic reference fields (`SphereField`, `BoxField`, `CylinderField`,
  `TorusField`, `HalfSpaceField`),
- simple CSG combinators and provenance tagging wrappers for tests,
- the stable field-evaluation boundary consumed by future extraction code.

This crate does not yet own a full mesher. The initial slice is the honest
field boundary that later dual-contouring and marching-cubes paths will use.
