# ADR-0001: Direct mesh operations below constructive and applications

Status: accepted.

## Decision

`exedra_mesh` owns structural topology, attributes, traversal and primitive edits.
`exedra_mesh_ops` composes these into reusable modeling and geometric queries.
It has no dependency on constructive recipes, assembly or execution machinery.
Constructive evaluation maps geometric correspondence to authored provenance;
application execution wraps direct operations without owning their algorithms.

Keep operation-specific limits, failures, scratch and evidence explicit. Preserve
attribute transfer, numerical behavior and failure guarantees while moving code.
Eager kernel edits do not imply rollback. Prepared data must validate its source
state independently of any application runner.

## Migration

- Import Boolean APIs from `exedra_mesh_ops::boolean` instead of
  `exedra_mesh::boolean`, and rounding APIs from `exedra_mesh_ops::round` (also
  re-exported at the mesh-operations crate root).
- The kernel exposes `Mesh::half_edges()` for deterministic traversal, including
  outside half-edges; modeling algorithms do not access topology arenas.

- `exedra_math::{Placement3, Plane3}` owns shared geometric representations;
  constructive IR re-exports the same types. Rotation constructors preserve
  constructive's explicit libm choice even when the std feature is unified.

- `exedra_mesh_ops::poke::poke_faces` performs direct face subdivision with
  explicit propagation policy. `PokeFacesPlan` offers inspectable preparation and
  checks captured dependencies before applying, including unfinished edits.
  The legacy operator delegates to it and owns only runtime reporting.

- `exedra_mesh_ops::measure` measures plain oriented polygon boundaries.
  Constructive patch/section measurements delegate and keep their semantic owner
  for frame and provenance. Arithmetic and refusal behavior are unchanged.

- `measure::signed_volume` measures existing indexed triangle buffers with a
  translated reference and compensated f64 summation. It identifies invalid
  indices/nonfinite positions and does no topology validation or tessellation.
  `SignedVolume::transformed` transports the measurement by an affine determinant
  and reference-point transform, preserving the supplied winding. This separates
  body orientation from occurrence scale/reflection. See the
  [authoring contract](../../exedra_constructive/docs/adr-0018-authored-geometry-diagnostics.md).

- `exedra_mesh_ops::transform::transform` places plain meshes and returns typed
  numeric, structure and rebuild errors. Face/vertex iteration order supplies
  explicit source correspondence. Reflection re-winds faces and carries built-in
  attributes and caller-defined layers verbatim; non-reflecting clones retain
  every attribute (ADR-0003).
  Constructive retains its error vocabulary and adds `InvalidMesh` for invalid
  source structure previously assumed to have been validated upstream.

- `exedra_mesh_ops::section::{section_mesh, split_mesh}` accepts plain meshes.
  Sections identify their input edge-owning faces. Split halves return output
  face/vertex source maps, including generated caps and triangle-edge
  intersections with directed interpolation parameters. Triangle edges may be
  triangulation diagonals; the API does not invent mesh-edge identity. Constructive
  binds this correspondence to features and material slots and retains its
  stale-source checks. Numeric realization checks and shared work limits remain
  in the geometry implementation.
- Shared plane/edge interpolation lives in `exedra_math`; planar ring contact
  predicates live in `exedra_mesh_ops::polygon`. Profile validation delegates
  after adapting its curve-library points, without adding a curve dependency to
  mesh operations.

- `exedra_mesh_ops::workplane` resolves authored frames on explicit face IDs and
  exposes geometric patch analysis with shared work counters and measured failure
  evidence. Constructive owns semantic surface selection and augments those
  failures with requested-selector context. Its workplane types are re-exports.
- `exedra_mesh_ops::clearance::PlanarPatch` checks and stores planar boundaries
  from plain meshes. Geometric source IDs can be replaced with caller evidence
  through `try_map_sources`, which moves checked point buffers unchanged.
  Constructive binds features this way; containment and distance calculations
  have one owner. Its `CircleClearance` and `BoundaryWitness` are aliases carrying
  that evidence. `CircleClearance::classify` now returns the geometric clearance
  error type; construction and query methods on the constructive patch retain
  their existing error type, including stale semantic source maps.

- `exedra_mesh_ops::stretch::stretch_mesh` accepts a plain mesh, authored plane,
  signed length, placement and geometric seam policy. Its output identifies
  surviving source vertices/faces, generated seam vertices and band faces, plus
  whether topology was rebuilt. Constructive keeps exact recipe recognition and
  binds feature/material evidence to those results. Rigid paths preserve IDs,
  custom attributes and existing source-map origins; rebuilt paths preserve the
  same built-ins and feature attribution as before, and carry caller-defined
  layers by position (ADR-0003). Raw calls now validate inputs
  previously checked by recipes, and whole-mesh translation refuses f32 overflow
  instead of ignoring a failed vertex-position update.

- `exedra_mesh_ops::face_edit` owns direct extrusion, inset, solidification and
  rectangular cuts. `bridge` owns geometric loop pairing and strip construction;
  it reuses the kernel boundary traversal. `selection` owns canonical typed sets.
  Prepared inset, cut, poke and bridge data checks captured source topology,
  positions and consumed attributes independently of the runner, including
  unfinished edits. Geometry returns work counts and orientation evidence;
  runtime adapters turn those into reports and diagnostics. Shared patch helpers
  have one owner; the duplicated ops patch module has been removed.
- `exedra_mesh_ops::normal_edit` offers one `NormalEditPlan` for clearing, flat,
  derived and selection-local smooth normals. Prepared derived normals capture
  all faces incident to selected vertices, so changing a neighboring face cannot
  silently reuse old normals. Existing runtime plan names alias this geometry
  plan, and runtime fingerprinting uses its public inspection methods.
- Extraction regressions exposed incorrect corner-index attribution: reversed
  extrusion caps paired UVs/tags to the wrong source vertices/edges, and rectangular
  cuts shifted outer-edge tags by one corner. Transfer now follows source identity
  independently of output winding. Cut preparation fingerprints therefore change
  when distinct edge tags expose that corrected correspondence. Inset rejects
  zero factors as its existing contract requires. Nonfinite consumed geometry or
  attributes fail before face mutation; eager kernel failures still imply no
  rollback.
- Rectangular-cut preparation now refuses nonplanar sources, off-plane origins
  or corners, nonperpendicular frame axes and boundary contact. These inputs
  previously passed the projection-only checks and could generate warped or
  zero-width frames. Fixed tolerances are documented on preparation; supported
  cuts inside concave quads retain their existing behavior. New refusals are
  `NonPlanarFace`, `RectangleOffPlane`, or the existing frame/containment errors.

- `exedra_mesh_ops::uv` owns planar, signed box and cylinder projection, with
  typed failures and processed-face, write/skip and fallback evidence. Runtime
  operators delegate and own diagnostic wording/artifacts. Projection formulas
  and box/render-extraction equivalence are unchanged. Nonfinite parameters,
  geometry and computed UVs are now refused before any write. Runtime timing
  uses one `project` bucket around the direct operation; the old internal
  `select`/`compute`/`attrs` buckets are no longer exposed.
- `inspect` provides read-only bounds and selection summaries; `region` provides
  region/flood/boundary queries. They take immutable meshes without edit sessions.
  Query results use `SelectionQueryStats` instead of unrelated runtime counters.
  Bounds now refuse absent/nonfinite positions and f32 overflow instead of
  returning incomplete or nonfinite summaries. `selection::resolve_edge_set`
  owns twin-to-canonical edge resolution, shared by runtime attribute adapters.
- Primitive deletion, dissolve and attribute writes remain in `exedra_mesh::op`.
  `delete_edges` now returns the sorted source face IDs it removed instead of
  `()`, eliminating duplicated incident-face traversal in the runner. Callers
  that require unit can discard this result. Read-only dissolve selection
  checks share the kernel's execution preflight; the duplicated runner checks
  are removed. They do not promise transactional execution.
- `boolean::preview_intersections` owns surface-curve preview and
  `components::remove_small_components` owns explicit component cleanup. The
  runtime retains timing/diagnostic conversion and optional cleanup sequencing.
  Empty curve previews do not establish disjoint solids or Boolean no-ops.
  Component cleanup reports removed source faces and propagates kernel refusal
  instead of ignoring a failed deletion; runtime commit errors include `Cleanup`.
- Unused runtime quality/work-budget, default-UV and placeholder Boolean policy
  fields/types are removed. They never controlled execution. Use the actual
  operation parameters and supported per-operation work limits; remaining
  runtime policy covers propagation, reporting capacity and validation.

- Mesh-loop conversion moves from `exedra_ops::profile_section` to
  `exedra_constructive::profile_section`. Its geometric projection lives in
  `exedra_mesh_ops::planar`, sharing distance measurement with workplane analysis.
  Newell-normal/centroid frames and root ordering are preserved. This is not a
  least-squares fit or authored loft correspondence. Disconnected edge order,
  absent/nonfinite positions and invalid tolerances now return typed failures.
- The unused analytic and constructive runner wrappers are removed. Call
  `AnalyticShell::{set_face_region, add_rect_opening_xy, remove_opening,
  to_exedra_mesh}` and constructive `evaluate::{evaluate, evaluate_with_cache}`
  directly. Native outputs already retain typed diagnostics, provenance, cache
  counters and fidelity; recipe fingerprints remain available on `Recipe`.
  There is no second preview/apply recipe lifecycle or diagnostic-to-prose copy.
- Concrete repeat/distribute and named expansion move from `exedra_ops::assembly`
  to `exedra_assembly::pattern`. They retain f64 3D placements, authored ordinal
  omissions, endpoint policy, explicit limits and atomic candidate replacement.
  These immediate conveniences differ from `setout_generate`'s exact unit-aware
  labeled fragments and regeneration deltas. Neither crate depends on the other;
  applications lower semantic layout items to concrete assembly instances.
- The remaining mesh command runner is renamed `exedra_edit`, with companion
  testkit and web demo names updated. It owns plans, preview, timing, reporting
  and command policy; it depends on the geometry crates, never conversely.
  Native-domain adapter features are removed. Replace the facade's `ops` feature
  and namespace with opt-in `edit` for command execution, or default `mesh_ops`
  for direct geometry. Facade defaults are `std`, `assembly`, `mesh_ops`.
  No heterogeneous runtime or new application framework is introduced.
