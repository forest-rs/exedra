# ADR-0006: Corner Normals and Render Extraction

## Status

Accepted

## Context

Exedra already stores UVs and other shading-related data in the corner domain
 (`CornerId == HalfEdgeId`) so render extraction can split render vertices
without changing topology. Normals need the same treatment.

We need to support:

- deterministic geometry-derived shading normals
- authored corner normal overrides for explicit art direction
- render extraction that emits normals and splits render vertices on normal
  discontinuities in addition to UV discontinuities
- `no_std` builds without forcing `std`

## Decision

Exedra adopts the following normal model:

1. **Derived normals are corner-domain data computed on demand.**
   `Mesh::derive_corner_normals(&NormalParams)` computes deterministic normals
   per corner from mesh geometry and sharp-edge boundaries.

2. **Authored normals are explicit sparse corner overrides.**
   The built-in key `attr::CORNER_NORMAL_OVERRIDE` stores optional authored
   corner normals. They are authored data, not derived caches.

3. **Extraction chooses a normal source explicitly.**
   `NormalsSource` controls whether render extraction uses derived normals,
   authored overrides where present, or authored normals only.

4. **Render extraction splits on `(vertex, uv, normal)` discontinuities.**
   `Mesh::to_trimesh` treats normal differences the same way it treats UV
   differences: they create distinct render vertices while keeping topology
   unchanged.

5. **Core float math stays `no_std`-compatible.**
   Exedra gains a small internal math shim and requires either the `std` or
   `libm` feature so normal derivation and extraction remain available on all
   supported targets.

## Consequences

### Positive

- Shading is deterministic for identical mesh state and params.
- Hard edges and explicit authored overrides both fit the existing corner-domain
  attribute model.
- Extraction semantics are explicit instead of hidden behind viewer-side normal
  generation.
- The design leaves room for future tangent generation without changing the
  topology model.

### Negative

- Extraction is more allocation- and compute-heavy than the old placeholder
  normal path.
- Split and face-edit kernels now need to propagate authored corner normal
  overrides where appropriate.
- Core crates need `std` or `libm` enabled; a featureless build is no longer
  valid.

## Notes

- This ADR covers normals only. Tangents remain future work.
- Derived normals stay derived: Exedra does not persist them as authored mesh
  state.
