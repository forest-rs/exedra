# ADR-0005: Stretch semantics and composition

## Status

Accepted (`ec-14f5`, 2026-09-02).

## Context

`NodeKind::Stretch` is an intent-preserving modeling operation, not an
affine scale. Furniture frontends need to resize selected bands while
leaving ends, hardware zones, radii, and imported detail rigid. The original
consumer format defines a single plane but leaves its orientation,
contraction behavior, and composition ambiguous. Those omissions cannot be
part of Exedra's public recipe contract.

## Decision

### Oriented, normalized plane

The plane equation is `dot(normal, point) = distance`. Evaluation first
normalizes `normal` and divides `distance` by the same magnitude. The
negative half-space, where `dot(unit_normal, point) - unit_distance < 0`,
is rigid. The positive half-space moves by `length * unit_normal`.

For positive `length`, geometry crossing the plane is cut once. The positive
piece translates and the cut section sweeps between its old and new
positions, inserting a prismatic band. Geometry entirely in the negative
half-space is unchanged; geometry entirely in the positive half-space is a
rigid translation.

For negative `length`, let `amount = -length`. Evaluation cuts at both the
declared plane and its positive offset by `amount`, removes input material in
that slab, and translates geometry beyond the offset by `length *
unit_normal`. The two section boundaries must be topologically compatible.
If they cannot be stitched without folding, inventing, or repairing
geometry, evaluation refuses the stretch with an `eval.stretch.*`
diagnostic and returns the child's envelope-only fallback.

### Node-local, sequential composition

A stretch plane is expressed in the coordinate space of the stretch node's
input: after evaluating the child and its descendants, before applying this
node's deformation. An ancestor transform carries the plane and stretched
result together. Consequently, an outer stretch observes the already
deformed output of an inner stretch. Nested parallel stretches therefore
compose sequentially in recipe order, which lets frontends build multi-zone
stretching without a second coordinate convention.

Nested `Stretch` nodes remain sequential. Simultaneous vertex displacement
has a separate node and does not reinterpret this contract.

### Two evaluation paths, one result contract

Evaluation prefers algebraic rewrites when the child retains constructive
structure: axis-aligned boxes change one extent and origin, extrusion-axis
stretches change height, and profile-plane extrusion stretches operate on
`Profile2` segments. These paths preserve curves and tags away from the cut
and remain independent of tessellation resolution. Exact structure flows
through rigid transforms and nested exact stretches, so a parallel multi-zone
recipe does not fall to meshes merely because it has more than one zone.

Other closed bodies, including `MeshImport`, use a deterministic plane
section path. It partitions intersected faces, translates the positive
piece, emits band walls from closed section loops, and stitches without a
tolerance-based repair pass. Open crossing shells, non-manifold sections,
ambiguous tangent or coplanar contact, a face contributing disconnected
section segments, and incompatible contraction sections are typed refusals
with `EnvelopeOnly` fidelity. The same contact refusal applies to an exact
extent boundary; constructive and imported forms of the same shape do not
silently choose different ownership. Material or feature changes inside a removed
slab are not topology and do not prevent contraction; each surviving face
keeps its own provenance at the stitched seam.

### Provenance, materials, creases, and UVs

A mesh-backed band wall extends the source face intersected by its section
edge. It therefore retains that face's `FACE_REGION` and source feature. Its
two section rims additionally map to a stretch-seam vertex feature so callers
can find the operation boundary without losing wall ownership. An exact
profile rewrite carries an internal source-wall map alongside its rewritten
segments. Every split piece and inserted connector retains the original
`FACE_REGION` and `Feature::Wall`; later walls do not change identity merely
because earlier segments split. `SegTag` remains independent frontend
provenance and is preserved, but is not reinterpreted as a material-region
namespace. An exact rewrite has no retained mesh seam—the inserted interval
is absorbed into its analytic primitive/profile and the `Stretch` recipe node
remains the durable operation provenance. Mesh seam edges are marked sharp
only when the incident surface normals exceed
`EvalPolicy::sharp_sin_threshold`; tangent-continuous extensions remain
smooth. Authored edge seams/sharpness, vertex sharpness, and corner-normal
overrides survive on rigid source detail; new cut normals interpolate only
when both source corners provide overrides.

Version 1 extends a face's complete affine planar corner-UV map along a
displacement tangent to that face. The same UV delta shifts the moved face,
so both band rims remain continuous. Missing, partial, underdetermined, or
non-tangent UV data remains absent on the new band; texture-space repair and
higher-level UV operations belong to Exedra Edit. Evaluation counts those faces
and reports `eval.stretch.uv_unmapped` rather than inventing coordinates.

### Vertex displacement

`NodeKind::StretchVertices { child, steps }` selects a different geometric
operation explicitly. Each `VertexStretchStep` contains a plane and signed
length along its normalized local normal. All steps classify the same input
positions; positive-side displacements accumulate in supplied order in f64
and narrow once to stored f32 positions. Selection uses the exact sign of the
unnormalized authored plane equation, with a filtered predicate and exact dyadic
fallback. Normalization supplies the displacement direction only. On-plane
vertices stay stationary. Negative lengths move selected vertices backward
without removing a slab. Nested displacement nodes still compose sequentially.

Constructive evaluation deforms in node-local coordinates before applying
ancestor placement, so equivalent Transform and Instance placement cannot
change selection through world-coordinate rounding. The direct mesh API can
transport selection planes by inverse transpose and displacement by the forward
linear map. Its signs are exact for the f64 transported coefficients and supplied
mesh coordinates; it cannot recover coordinates lost to earlier rounding.

This is an operation on a particular mesh. Adding vertices or changing
triangulation before deformation can change the result even when the original
surface is identical. It does not define a representation-independent resize.

The mesh operation lives in `exedra_mesh_ops::stretch::stretch_vertices`.
Constructive evaluation supplies the evaluated child meshes and preserves
face materials and source correspondence. Open triangle meshes are accepted.
Faces crossing a plane retain their corners and connectivity; no cut, band,
weld or repair occurs. Active operations refuse polygon faces, invalid
structure, unrepresentable arithmetic and triangles collapsed at stored
precision. Exact orientation predicates distinguish zero area from merely
small triangles. This check does not certify winding or self-intersection.
Any child or body refusal prevents output of the entire displacement node.
All-zero steps evaluate the child unchanged, including polygon faces.

UVs, regions, sharpness and other attributes retain their original IDs and
values. Faces whose corners receive different stored movements lose authored
normal overrides so extraction with `CustomOrDerived` uses the resulting
geometry. Exact two-term differences of stored coordinates determine rigidity;
requested displacement and rounded f64 differences are insufficient. A movement
that rounds away retains normals. Rigid faces retain their overrides, including
faces beside a changed neighbor. This face-local policy can leave different
normals across an otherwise smooth boundary; callers can choose `Derived`
extraction for consistent geometric smoothing. Authored smoothing across
disconnected seams is not reconstructed. UV density is not adjusted. Source maps
are re-pinned to the resulting revision; source chart metrics remain ancestry,
while body sampling, refinement and sweep-validation evidence is cleared.

The evaluator revisits children on cache hits to retain their reports and
occurrence materials. Single-body local results are cached by node content and
policy, independently of ancestor placement; multi-body results are evaluated
in child order. Refusal envelopes are placed in world coordinates.
`vertex_stretch_passes` counts completed mesh passes in the current run and
is zero for a warm hit or an all-zero bypass.

This additive variant uses canonical tag 18 and the text and JSON kind
`stretch_vertices`. Its child and complete ordered step list participate in
fingerprints. The node encoding includes displacement revision 1 for local
evaluation and exact boundary ownership. Existing non-displacement encodings
and evaluated recipes are unchanged, so schema 40 remains valid. Callers
matching `NodeKind` handle the new variant; older text readers reject its
opcode. There is no automatic selection by child body kind and no compatibility
wrapper.

## Consequences

- A stretch is deterministic for identical recipe and policy input and has
  a correct post-deformation AABB.
- Exact rewrites preserve analytic detail; the general path preserves the
  input mesh's resolved detail but cannot recover analytic curves.
- Cache hits replay retained bodies but perform no section or band work, so
  `stretch_faces_split` and `stretch_band_faces` are zero on a warm hit. This
  follows ADR-0004's rule that work counters describe the current run rather
  than replaying the cold run's effort.
- Frontends can express rigid/stretch/rigid furniture zones by nesting
  parallel stretches and can rely on their order.
- `Feature` and evaluation counters gain additive stretch variants/fields.
  `Feature` was already non-exhaustive; `EvalCounters` becomes non-exhaustive
  with this change so future work counters remain additive. Downstream code
  constructing `EvalCounters` with a struct literal must switch to
  `EvalCounters::default()` plus field assignment. This ADR and the `ec-14f5`
  close note are the migration record.
- Successful stretch evaluation changes unchanged recipes that previously
  produced `eval.unimplemented`, so `EVAL_SCHEMA_VERSION` advances from 4 to
  5.
