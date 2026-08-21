# ADR-0001: Structure-head scope

## Status

Accepted (2026-08-19).

## Context

The workspace's multi-domain architecture (cambium ADR-0002) gives each
geometry domain a dedicated head with a narrow responsibility and explicit,
lossy conversions between heads. `exedra_constructive` compiles a single
part's recipe into one or more meshes; nothing in the workspace owned the
level above it: many parts, arranged and repeated, with materials bound per
use. External runtimes need that level to remain representation-neutral,
deterministic, and cache-friendly.

## Decision

`exedra_assembly` is the structure head. It owns exactly:

- **Part definitions.** A part is a constructive `Recipe` or a baked
  `exedra::Mesh`, registered under an opaque frontend-supplied `PartKey`
  string. Parts declare material slot names and an optional mapping from
  tessellation regions (`FACE_REGION` values) to slots.
- **Instance trees.** Instances reference a part, carry an f64 `Placement3`,
  slot-to-material-key bindings, and an opaque metadata bag. Each instance
  has a frontend-supplied string key unique among its siblings.
- **Calm common-case defaults.** Registering a recipe with exactly one
  material slot makes that slot the part-wide default. Zero-slot and
  multi-slot recipes remain explicit. Deterministic instance traversal is
  available both as a value slice and as `(InstanceId, &Instance)` pairs.
- **Identity.** An `InstancePath` — the key sequence from the root — is the
  stable identity contract: the same path across re-evaluations denotes the
  same logical part. Indices are never identity. Frontends choose keys;
  this crate only guarantees their stability semantics.
- **Compilation and sharing.** Parts compile (tessellate) once per distinct
  (content fingerprint, policy fingerprint) pair; instances share compiled
  results. Dirty tracking runs through the `invalidation` crate so a changed
  part invalidates only its own entry.
- **The render seam.** `flatten()` produces a flat `RenderList` of
  (instance path, world placement, part reference, per-region index ranges
  with resolved material keys). Renderers and exporters consume this and
  nothing deeper.

It owns none of:

- **Geometry math.** Tessellation, booleans, discretization live in
  `exedra` / `exedra_constructive`. This crate never inspects coordinates
  beyond composing placements.
- **Parameter models, conditional logic, pricing.** External runtimes
  evaluate their specification languages and hand this crate finished
  recipes, placements, and bindings.
- **Rendering / file formats.** Exporters (e.g. `exedra_gltf`) are separate
  leaf crates over the `RenderList`.

## Consequences

- Material rebinding is a structure-level edit: it can never trigger
  re-tessellation, because geometry identity excludes bindings.
- The crate is `no_std` + `alloc`; interchange (a host-side format) sits
  behind a `serde` feature that implies `std`, matching the
  `exedra-recipe-v1` policy.
- Cross-part geometry operations (a boolean between two instances) are out
  of scope; frontends express those inside a single recipe instead.

### Migration note

`add_recipe_part` now resolves unmapped regions through the recipe's only
slot when exactly one slot is declared. Callers that deliberately wanted a
one-slot recipe to resolve no material must instead declare that intent with
a multi-slot recipe or clear the material at their consuming boundary.

## Alternatives considered

- **Folding assembly into `exedra_constructive`.** Rejected: the recipe IR
  is a per-part compiler target with content-addressed identity; grafting
  mutable scene structure onto it would entangle two lifecycles the tenets
  require to stay separable.
- **Index-based instance identity.** Rejected: external runtimes re-emit
  scenes wholesale; only stable keys survive re-evaluation.
