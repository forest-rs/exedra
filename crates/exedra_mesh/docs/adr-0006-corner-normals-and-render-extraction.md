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

## Amendment (M5, exe-e4df): incremental extraction boundary

`ExtractMode::Incremental` originally hid a `debug_assert!(false)`
(panic in debug, silent full rebuild in release). It is now defused:
a bare `to_trimesh` call under `Incremental` performs a full rebuild
counted in `ExtractStats::incremental_fallbacks`, and actual reuse
routes through `Mesh::to_trimesh_cached` with a caller-owned
`TrimeshCache` pinned to `Mesh::revision()` (the source-map pinning
precedent).

**Why reuse is whole-output, not spliced.** Extraction output ordering
is global: render vertices are appended on first encounter during the
face traversal, and the dedup key map spans faces. Patching changed
faces into a prior buffer would reuse the old encounter order, which a
fresh full rebuild of the edited mesh would not reproduce — so any
sub-linear splice is structurally incompatible with the bit-identity
contract (`incremental == full rebuild`, signature-for-signature) that
the whole determinism architecture rests on. The profitable reuse
boundaries are therefore:

1. **Whole-output reuse** when the revision and parameters are
   unchanged (this amendment): extraction is a pure function of mesh
   state, so the cached output *is* the rebuild's output.
2. **Derived-normal patching** (exe-phy0, follow-up): re-derive only
   the corners affected by moved vertices and re-run the (cheaper)
   emission loop — bit-identical because emission order never depends
   on how normals were computed.

## Amendment: extraction-time UV source policy

UVs now have the same explicit source choice normals gained in decision 3.
`ExtractParams::uvs: UvSource` selects what a face corner without an
authored `attr::CORNER_UV` emits:

- `UvSource::CustomOnly` (default): `[0.0, 0.0]`, the historical output.
  Defaults reproduce the previous bytes exactly.
- `UvSource::CustomOrBoxProjected { scale }`: the corner's destination
  vertex position projected on the face's dominant-axis plane, multiplied
  by `scale`. Authored UVs are never overwritten under either variant.

**Per-face dominant axis.** The plane is chosen once per face, not per
corner, from the signed fan-sum face normal, normalized, with
`DEFAULT_BOX_NORMAL_EPSILON` (`1e-6`) as a tie-break between unit-normal
components in the order X, then Y, then Z. Normalizing first makes the
selection depend on orientation only: previously the epsilon was compared
against the area-scaled sum, so a valid sub-millimetre face fell back to
`+Z` and its projection collapsed to a line. Degeneracy is a separate
decision made by the normalization itself (zero area or non-finite
positions), and it falls back to `+Z`; extraction reports each such face in
`ExtractStats::uv_projection_fallback_count`. This is the same rule the
`uv.box` operator applies, and the projection math now lives in
`exedra_mesh` (`dominant_box_plane`, `project_corner_box`,
`project_box_position`) so the operator and extraction cannot drift. For a
mesh with no authored UVs, extracting under `CustomOrBoxProjected { scale }`
and extracting after `uv.box` with that scale and zero offset produce
identical render buffers; where authored UVs exist, extraction keeps them
while the operator overwrites them unless `write_missing_only` is set.
Because projected UVs enter the render-vertex key like authored ones,
adjacent faces on different planes split their shared vertices exactly as
authored seams do.

**Why extraction time.** The compiled output of the structure head is
triangle buffers only, so a consumer cannot run `uv.box` after
`compile_parts`; and writing projected UVs into authored mesh state would
make a viewing preference part of content identity. A policy keeps the
mesh authored-only while letting a renderer request full coverage.

**Consequences in the structure head.** `CompilePolicy::uvs` passes the
policy through part compilation. `policy_fingerprint` advanced its prefix
to `assembly-compile-v2` and appends a UV variant byte plus the scale
bits, so persisted fingerprints from earlier versions never collide with
current ones. `RegionRange::has_uvs` now means "every emitted corner in
the range has a finite UV after the policy is applied": unchanged under
`CustomOnly`; under `CustomOrBoxProjected` a projected corner counts when
its emitted coordinates are finite, which compilation checks on the emitted
buffer because a finite position times a finite scale can overflow, and an
authored non-finite UV still disqualifies its range.

**Out of scope.** Planar and cylindrical extraction policies, texture
transforms in glTF, and any change to how authored UVs are read.
