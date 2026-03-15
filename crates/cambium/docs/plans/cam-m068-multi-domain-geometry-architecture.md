## Scope

- Define a durable multi-domain geometry architecture for the workspace.
- Keep `exedra` as the polygon editing head and strengthen it so Cambium mesh
  operators become thinner and less bespoke.
- Add one narrow second head (`exedra_analytic`) as the proof that explicit
  sibling domains beat a fake universal geometry core.
- Establish how Cambium orchestrates domain-native operators and explicit
  conversions without flattening semantics.

## Non-goals

- No attempt to replace `exedra` as the canonical polygon domain.
- No universal "geometry trait" that erases mesh/analytic/implicit/points
  differences.
- No broad CAD scope in the first analytic slice: no NURBS, no general trims,
  no heroic booleans.
- No simultaneous addition of analytic, implicit, and points heads before one
  second head has earned the pattern.

## Fence

- `exedra` owns editable polygon topology, authored mesh attributes, and
  deterministic extraction; it explicitly does not own exact analytic surface
  semantics, implicit field evaluation, or point-cloud semantics.
- `cambium` owns orchestration, operator runtime, and cross-domain workflow
  composition; it explicitly does not pretend all geometry domains share the
  same canonical operator substrate.

## Why This Shape

The repo already has a real polygon kernel and a growing workflow layer. The
architecture decision is not "mesh or topology"; it is whether the workspace
should:

1. keep pretending mesh can cover every future geometry mode, or
2. admit multiple canonical domains and make conversion explicit.

For a Houdini-like future, the second option is stronger:

- mesh-native operations remain honest and effective,
- analytic/implicit/points work can exist without corrupting polygon APIs,
- conversion steps become intentional and diagnosable instead of hidden.

## Canonical Domains

### 1. Polygon domain (`exedra`)

Primary use:
- direct polygon editing,
- UV/seam/sharpness/normal authoring,
- deterministic extraction,
- mesh-first procedural modeling.

Load-bearing invariants:
- stable IDs,
- explicit edit scopes,
- attribute domains are first-class,
- extraction is deterministic,
- conversions out of mesh are explicit.

### 2. Analytic domain (`exedra_analytic`)

First-slice use:
- planar face shells,
- line edges,
- exact profile/opening semantics before tessellation,
- deterministic tessellation into `exedra::Mesh`.

Load-bearing invariants:
- shells/loops/coedges are first-class,
- geometry references are explicit,
- tessellation is deterministic for fixed params,
- provenance survives the conversion seam where possible.

### 3. Deferred domains

Deferred until the analytic head proves the pattern:
- implicit field domain,
- points/curves domain.

The repo already has open implicit-surface tickets. Those remain valid, but the
multi-domain architecture must not assume every future head arrives at once.

## Crate Graph

### Existing crates

- `crates/exedra`
  - polygon topology kernel
  - edit session + change tracking
  - attribute domains
  - deterministic extraction
- `crates/cambium`
  - operator runtime
  - workflow/fluent composition
  - reports, diagnostics, policy

### New crates/modules

- `crates/exedra_analytic`
  - canonical analytic topology/geometry head
  - narrow MVP: planar faces + line edges
  - deterministic tessellation into `exedra`

Potential later siblings:
- `crates/exedra_implicit`
- `crates/exedra_points`

### Ownership rule

Each domain crate owns its canonical state. `cambium` coordinates work across
domains but does not collapse them into one shared authoring model.

## Cambium Operator Model

Cambium should become domain-aware, not domain-agnostic.

### Operator domains

Introduce explicit operator domain metadata:
- `Mesh`
- `Analytic`
- `Implicit`
- `Points`
- `Convert`

### Taxonomy direction

Keep existing stable operator IDs for mesh operators. Add domain metadata first,
then grow the catalog honestly:

- mesh-native examples:
  - `inspect.validate.mesh`
  - `edit.face.inset`
  - `mark.edge.seam`
- future analytic-native examples:
  - `analytic.profile.opening`
  - `analytic.face.offset`
- conversion examples:
  - `convert.analytic.mesh`
  - `convert.implicit.mesh`

Stable IDs and domains should be distinct concepts:
- the ID tells users what operation they are invoking,
- the domain metadata tells the runtime which canonical head owns the state.

## Exedra Mesh-Head Improvements

The mesh head should gain stronger kernel support so Cambium stops owning as
much low-level topology logic.

### Query helpers

Add deterministic mesh-native helpers in `exedra`:
- boundary loop extraction,
- connected face patch extraction,
- region-boundary edge queries,
- loop/ring walkers where the semantics are truly kernel-level.

Why:
- current Cambium patch/query helpers are load-bearing but live one layer too
  high for long-term reuse,
- mesh-native operators need a smaller, calmer substrate.

### Edit primitives

Keep building the Exedra op catalog with local, composable primitives:
- `collapse_edge`
- `flip_edge`
- `duplicate_faces`
- `detach_faces`

Not every one of these must land immediately, but they are the right direction:
smaller kernel edits, thinner Cambium compositions.

### Compile/apply honesty

Cambium plans need source-state binding.

Required changes:
- bind `EditPlan` to mesh revision or equivalent source state,
- reject stale plan replay,
- separate topology-only signatures from full mesh-state signatures,
- keep compile/apply only where precomputation is meaningful.

This improves the current mesh head because it turns the runner contract into a
real seam rather than optimistic reuse.

## Analytic MVP

### First-slice types

`exedra_analytic` should start with:
- `ShellId`
- `LoopId`
- `CoedgeId`
- `VertexId`
- `Plane`
- `LineEdge`
- `AnalyticShell`

Scope constraints:
- only planar faces,
- only line edges,
- no general trims,
- no curved edges,
- no boolean kernel yet.

### First conversion seam

Add deterministic analytic-to-mesh tessellation:
- fixed winding rules,
- fixed vertex ordering rules,
- explicit tessellation params,
- provenance mapping into mesh face regions / edge semantics where applicable.

### First demo

Use one narrow scenario:
- wall with rectangular opening.

Author it twice:
- mesh-native in `exedra`/`cambium`,
- analytic-native in `exedra_analytic` then tessellate to `exedra`.

Compare:
- authoring semantics,
- topology correctness,
- extraction output,
- future editability.

## Sequencing

### Phase 1. Architecture artifacts

- `cam-m068`: ADR + blueprint (this plan)

Result:
- one owning architecture record,
- one execution map,
- no ambiguity about domain ownership.

### Phase 2. Mesh-head hardening

- `exe-x154`: mesh query/helpers for Cambium simplification
- existing `exe-h2rh`: collapse/flip primitives
- `cam-aou6`: honest source-bound plans

Result:
- stronger polygon core,
- thinner mesh-native Cambium ops,
- better operator/runtime contracts.

### Phase 3. Domain-aware Cambium

- `cam-7qig`: explicit operator domains and orchestration model

Result:
- Cambium can coordinate multiple heads without flattening semantics.

### Phase 4. Analytic MVP

- `cam-6z7d`: analytic head MVP + deterministic tessellation

Result:
- one real second canonical domain,
- explicit proof that the Hydra architecture pays for itself.

### Phase 5. Deferred heads

- existing implicit-surface epic `exe-xgtv`
- future points head if and only if a concrete workflow demands it

Result:
- growth only after one second head validates the model.

## Ticket Map

- `cam-t6z7` Multi-domain geometry architecture epic
- `cam-m068` ADR and blueprint for multi-domain geometry architecture
- `cam-7qig` Domain-aware operator taxonomy and orchestration model
- `cam-6z7d` Analytic head MVP and deterministic tessellation path
- `exe-x154` Mesh query/helpers for Cambium simplification
- `cam-aou6` Bind compiled plans to source mesh state
- linked existing work:
  - `exe-h2rh` collapse_edge and flip_edge
  - `exe-xgtv` implicit surface meshing epic

## Risks

- A fake universal abstraction may appear if operator domains are modeled only
  in docs and not reflected in runtime boundaries.
- Exedra can become architecture landfill if mesh-native helpers are not
  separated from analytic ambitions.
- The analytic head can sprawl into CAD theater unless its first slice remains
  brutally small.
- Conversion seams can overclaim preserved semantics if provenance and data-loss
  rules are not explicit.

## Validation

- `typos`
- `cargo fmt --all`
- `taplo fmt`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-features`
- `cargo doc --no-deps`
