# ADR-0001: Constructive Domain Scope

- Status: Accepted
- Date: 2026-08-19
- Owners: Exedra maintainers

## Context

Exedra's downstream machinery — the half-edge kernel, edit sessions,
Cambium's operator lifecycle, primitives with semantic regions — is solid,
but nothing upstream of the mesh exists: no curves, no pre-mesh profiles, no
constructive operations, no recipe model. External spec compilers (parametric product specifications, living in
separate repositories by decision) need a spec-agnostic construction
representation they can target, with the mesh as its *output*, not its
source model.

Cambium ADR-0002 (multi-domain geometry architecture) already prescribes the
shape of the answer: a new sibling head with explicit, lossy,
provenance-carrying conversion into the mesh domain — never an extension of
the mesh kernel, and never a universal geometry abstraction.

Extending `exedra_analytic` was considered and rejected: analytic is a
*mutable editable* planar-topology domain in f32 with mesh-lifecycle
semantics and an ADR that deliberately excludes curves. The constructive
domain is an *immutable evaluated recipe* domain in f64 where caching,
provenance, and deterministic re-evaluation are the primary concerns.
Different lifecycle, scalar policy, and API temperament; forcing them
together would break both.

## Decision

`exedra_constructive` owns the fourth geometry head:

- **kurbo-backed 2D profiles.** Profiles are endpoint-chained cyclic segment
  lists (`Seg2`: line, bulge-parameterized arc, cubic, policy-defined
  segment), closed *by construction* — segment `i` runs from segment
  `i-1`'s endpoint, so no open state exists. Arcs store both endpoints
  exactly (bulge = tan(sweep/4), center derived), which kurbo's
  center-parameterized `Arc` cannot guarantee (endpoints via cos/sin).
  kurbo (pinned, re-exported) is the *only* curve math engine — bulge→arc
  conversion, cubic flattening, areas, winding, bounding boxes, affine
  transforms all route through it; no parallel curve math is permitted
  except bulge-arc discretization itself. Profile construction rejects hole
  pairs that overlap, contain one another, or touch; curved boundaries use
  kurbo's cubic representation and a documented relative flattening
  tolerance for that topology check.
- **Profile operations** are construction-time functions over `Profile2`,
  not IR nodes: their output is an ordinary segment profile that hashes and
  evaluates like any other, so adding one never touches
  `EVAL_SCHEMA_VERSION`. Today that is the generic builders (rectangle,
  rounded rectangle, L, circle, ring) and **offset** — a signed distance
  with an explicit corner policy (round, or miter with a limit), used for
  clearance geometry. Offset keeps the exactness contract above: lines
  offset to parallel lines and bulge arcs to concentric arcs with the same
  center and sweep, with no flattening and libm-only arithmetic; only
  cubics go through kurbo's offset fitting, which is documented as outside
  the cross-platform bit-identity contract because kurbo's fitting trig
  dispatches to `std` under feature unification. Degenerate results
  (collapsed arcs and loops, self-intersection, undercut material, holes
  reaching the outer loop) are typed `ProfileError`s; consistent with the
  no-auto-repair non-goal, self-intersection is detected and rejected,
  never healed, and no loop-trimming or offset-cleanup algorithm is in
  scope. Self-intersection, undercut, and hole contact are decided from
  kurbo-flattened rings at a documented relative tolerance.
- **The constructive node set**: extrude, revolve (partial sweeps, cap
  flags, open shells), loft, sweep along polyline or planar paths, planar
  faces, grid surfaces, primitives (via `exedra_primitives` specs), imported
  mesh leaves, n-ary CSG, transform/mirror/instance/group, and a reserved
  stretch node. All public enums are `#[non_exhaustive]`. There is **no 3D
  curve type**: planar profiles plus placements plus polyline paths cover
  the parametric-spec domain; a spatial path variant can arrive additively if ever
  earned. Loft evaluation rejects sections whose placed geometry remains
  coplanar, because such a recipe cannot produce a volumetric solid.
- **Two-layer identity.** A content-addressed node hash (Merkle over the
  canonical byte encoding, stamped with [`EVAL_SCHEMA_VERSION`]) keys caches
  and fingerprints; a frontend-assigned opaque source identity keys
  provenance continuity across recipe edits. The canonical encoding — f64
  bit patterns, explicit tags, documented layout — is the compatibility
  contract; serde comes later as a feature, never as the contract.
- **Explicit placement construction.** `Placement3` stores a row-major 3x4
  affine matrix, while `from_axes` accepts local basis vectors as matrix
  columns plus a translation. Common axis rotations use `libm` constructors;
  callers do not need to hand-author matrix layouts.
- **Immutable mirror composition.** `Recipe::mirrored` clones a frozen recipe
  and appends one unbound `Mirror` root over its prior root. Existing node and
  table ids, fingerprints, source identity, and provenance bindings remain
  stable; the original remains unchanged. Planes must admit finite,
  non-degenerate normalization at construction. Winding correction remains a
  constructive-evaluation concern, so assembly placements stay proper-rigid
  and callers neither reflect instances nor evaluate and bake opaque meshes.
  General recipe reopening, table merging, assembly naming, and part reuse are
  deliberately outside this operation.
- **Deterministic evaluation with honest reporting.** Evaluation is a pure
  function of recipe and policy. Output includes the mesh, a bidirectional
  source map down to profile-segment granularity, a region/material-slot
  table, and a fidelity report (`Exact`, policy-defined, conflicted,
  envelope-only) whose policy and issue identifiers are opaque
  frontend-supplied values — the mechanism for spec ambiguity lives here,
  the spec-specific tables live in the frontends' repositories.
- **Primitive evaluation.** Declared boxes and cylinders evaluate through
  `exedra_primitives`, explicitly selecting its `libm` backend (which takes
  precedence if Cargo also unifies `std`). Constructive fixes centering, caps,
  and axes to the public IR contract, copies backend semantic regions into
  `FACE_REGION`, and records them as `Feature::PrimitiveRegion`. The additive
  provenance and `TessellateError::PrimitiveSegmentLimit` variants are the
  migration note for downstream users: both enums are non-exhaustive, and
  primitive regions use each backend primitive's documented local namespace.
  Explicit cylinder segmentation is bounded by
  `EvalPolicy::discretize.max_segment_edges` before allocation.
  Making these previously declared nodes evaluable changes output for an
  unchanged recipe, so it advances `EVAL_SCHEMA_VERSION` from 3 to 4.

### Scalar policy

Construction and evaluation are f64 end to end (kurbo-native). Exact-decimal
spec arithmetic is the frontends' job; i64 thousandths-of-millimeter
values convert exactly into f64. The narrowing to `[f32; 3]` happens exactly
once, at mesh emission, with round-to-nearest-even, and is recorded in the
report. For `exedra_primitives`, its f32 parameter conversion is that same
mesh-emission boundary; values that cannot remain finite and positive in f32
fail with `TessellateError::NonFiniteGeometry`. Exedra's ADR-0001 (`[f32; N]`
public math) and `NumericPolicy` are
untouched; the tessellator emits welded topology and does not lean on
mesh-side merge tolerances.

### Determinism policy

- Trig always routes through `libm` — the dependency is non-optional and is
  used even in std builds. This extends `exedra_primitives` ADR-0002
  (either/or backend with an error budget) to a stricter contract:
  bit-identical results across platforms, because content hashes, caches,
  and goldens depend on them.
- kurbo's flatten path is sqrt/arithmetic only (audited, `ec-c3ii`) and is
  safe to call; kurbo's arc trig dispatches to `std` when the `std` feature
  is unified in by any downstream crate, so **the deterministic evaluation
  path never calls kurbo arc trig** — arc discretization is owned here.
- kurbo is pinned exactly; any upgrade bumps [`EVAL_SCHEMA_VERSION`], which
  is folded into every content hash, so caches and goldens invalidate
  explicitly.
- Discretization counts derive from parameters via integer math, never from
  accumulated floats; no iteration depends on hash order.

## Non-goals

- No NURBS or analytic surface kernel; no new curve mathematics beyond
  kurbo plus policy-discretized spec curves.
- No exact/rational arithmetic kernel; determinism plus typed failure over
  heroic robustness.
- No 3D curve kernel.
- No scene graph or cross-part assembly (a sibling assembly head owns
  that); CSG happens only inside a recipe.
- No spec vocabulary: no shape numbers, parameter names, or spec citations
  in code, tests, or goldens.
- No serde in the compatibility contract; the canonical encoding is the
  contract and serde derives are a convenience feature.
- No auto-repair of invalid profiles: typed rejection, never silent fixes.

## Consequences

- External frontends get a semver-conscious integration surface; mistakes
  in the canonical encoding are breaking changes, so it ships with golden
  coverage from the first slice.
- `cambium` gains a real non-Mesh operator domain via an explicit
  conversion seam (its own ADR when that lands).
- One more workspace crate with a heavier dependency (kurbo) — accepted and
  recorded against the dependency-creep tenet in `ec-c3ii`'s audit.
