# ADR 0002: The `joiner` construction layer

## Status

Accepted (2026-08-21). Supersedes the ADR 0001 consequence that the
structural graph "stays example-private until a later ticket demonstrates a
reusable boundary": this ADR records that boundary and the crate that will
own it. The crate itself is created only with an earned first slice, per
Exedra Ops ADR-0005.

## Context

The workspace has a geometry stack but no construction layer:

- `exedra_mesh` is the mesh kernel; `exedra_constructive` compiles one part's
  recipe (profiles, extrude/revolve/loft/sweep, n-ary CSG through the mesh
  boolean pipeline, provenance down to profile segments) into meshes;
  `exedra_assembly` arranges parts as instances under stable paths with
  material bindings; `exedra_ops` is the mesh-operator SDK plus
  deterministic placement patterns.
- Nothing owns *how building elements fit together*. `basilica_ruin`
  encodes joints as per-part constants (`KING_POST_RAFTER_OVERLAP`,
  `ROOF_CLEARANCE`) that make boxes overlap or clear by hand-tuned amounts.
  `basilica_structure_lab` models joints, bearings, supports, and witnessed
  load transfers as *uncut* hypotheses with anchor-contact only; its README
  defers mating faces to a "follow-on joint specimen ticket".
- The structure lab's pipeline — parameters and evidence, to a structural
  graph, to validation, to one assembly instance per element, to emit — is
  already the shape of the missing layer. What it lacks is the generating
  step: a rule that turns a declared relation between elements into
  coordinated geometry on all participants, plus the contacts that
  validation consumes.
- The same generating step is needed far beyond trusses: a window voids a
  wall and is filled by sill, lintel, jambs, and reveals; a wall decomposes
  into courses and units under a bond; an arch decomposes into voussoirs
  over centering; ornament is applied to a substrate. Whole-structure
  generation is composition of such fittings.

## Decision

A new workspace crate, **`joiner`** (`crates/joiner/`), is the construction
layer. It sits above `exedra_assembly` and below any future planning layer,
and it knows about building elements and their relations — not about meshes.

### Name

The name follows the craft rather than the material or the geometry: a
joiner is the tradesperson whose work is fitting parts to each other. The
reading "the thing that joins things" survives every scale the crate is
meant to cover — members into joints, fillings into openings, units into
bonds, ornament onto substrates, bays into structures. `joiner` and
`forest-rs/joiner` were free on crates.io and GitHub on 2026-08-21.
Rejected: `heartwood` (taken; also Radicle's protocol name), `tectonics`
(exact in architectural theory, but collides with geology and with the taken
`tectonic`), `millwright` (honest about breadth, long), material-biased
names (`ashlar`, `stereotomy`, `timberwork`, `purlin`), and `trabeation`
(implies post-and-lintel and quietly excludes arches).

### Domain

The domain object is the **building element** and the **relations** between
elements. Three relation kinds are first-class siblings in one IR; none is
expressed in terms of another:

| Relation | Example | Rule output |
|---|---|---|
| **Host / fill** | a window voids a wall; sill, lintel or relieving arch, jambs, reveals fill the void | cut on the host part, generated fill parts, bearing contacts |
| **Member / member** | rafter heel on tie beam; king-post foot; brace end | coordinated cuts on every participating member, mating faces |
| **Element / units** | wall → courses → units under a bond; arch → voussoirs | decomposition into generated parts and their contact graph |

Every rule application, regardless of kind, yields the same four things:

1. **Part edits**: constructive operations (retain / remove solids, in
   `exedra_constructive` terms) appended to each participant's recipe.
2. **Generated parts**: new recipes with a role (sill, peg, voussoir, …),
   material, and placement.
3. **Contact patches**: mating and bearing faces with a declared meaning
   (bearing, side fit, shoulder, mortar bed, clearance only).
4. **Load-path edges**: witnessed transfers in the structure lab's sense,
   so the existing contact and load-path validation consumes rule output
   without knowing which rule produced it.

### Rules

- A rule exposes `assess(context) -> Applicability` and
  `instantiate(context, params) -> RuleOutput`. `assess` is the normal
  entry point: a rule rejects itself — wrong topology, member too thin,
  incompatible material or grain, required capability missing — as a typed
  result, never a panic or a silently degenerate cut.
- Parameters are strongly typed per rule. No erased, document-shaped, or
  agent-facing parameter boundary exists in the core; if one is ever
  earned it is an additive adapter at the registry edge, not the native
  interface.
- **Shared interface.** Both sides of a fit are derived from one nominal
  interface expression. The receiving side is the clearance offset of that
  expression; the inserted side retains it. A mortise is never a separately
  generated hole that happens to use similar dimensions. This is what makes
  fit a provable property rather than a coincidence of constants.
- **Evidence class travels with everything.** Elements, relations, and
  rule applications carry the lab's evidence source key and class
  (`Observed`, `DocumentedReconstruction`, `RegionalAnalogy`,
  `ModernEngineeringInference`). This is carried from day one; it is cheap
  to carry and expensive to retrofit.

### Mechanism versus knowledge

`joiner` owns the **mechanism** only: the element graph, the three relation
kinds, the rule trait and its uniform output, lowering to
`exedra_assembly` plus constructive edits, the contact and load-path graph,
and evidence labelling. It contains no knowledge of any particular joint,
bond, or profile.

Construction **knowledge** lives in separate rule-library crates that plug
in — `joiner_timber`, `joiner_masonry`, and later others. A consumer that
needs four timber joints must not inherit a dependency on thirty, nor on
stone.

### Lowering and identity

- The element graph is the source of truth. An `exedra_assembly::Assembly`
  is a compiled artifact of it, exactly as a mesh is a compiled artifact of
  a recipe. Structural connectivity is never encoded through parent-child
  placement (unchanged from ADR 0001).
- Element keys are the stable identity contract; the assembly instance
  paths they produce are derived and deterministic.
- The element is the dirty-tracking unit, through the `invalidation`
  crate: moving one window re-cuts one wall part and re-validates its
  bearings and nothing else.

### Boundaries

`joiner` owns none of:

- **Geometry math.** Cuts, offsets, tessellation, booleans are
  `exedra_constructive` and `exedra_mesh`. The only Exedra-side change this
  layer requires is a **profile offset** operation in
  `exedra_constructive` (clearance generation); see the linked ticket. The
  reserved `Stretch` node is *not* a dependency: member length is an
  ordinary recipe parameter and content-addressed caching already handles
  re-evaluation.
- **Site, massing, and plan layout.** "Where do the aisles go" is planning,
  not joining. If that layer is ever built it is a sibling crate that
  *emits* an element graph for `joiner` to realize.
- **Statics, FEA, capacity, code compliance, certification.** Unchanged
  from ADR 0001: validation is schema, coherence, contact, transfer
  witness, and load path.
- **Rendering and export.** Consumers of the compiled assembly.
- **Exedra Ops' operator lifecycle.** `joiner` uses `exedra_ops::assembly`
  placement patterns for stationing (truss bays, window rhythm) and
  otherwise talks to `exedra_constructive` and `exedra_assembly` directly.
  Exedra Ops stays the mesh-operator SDK; it does not become the construction
  SDK as well.

### Staged first slice

The first slice is staged so the mechanism can be reviewed before construction
knowledge depends on it, while still exercising both relation families so the
IR is not shaped by one:

1. **Mechanism and fixtures**: create `joiner`, migrate the structure lab to
   consume it, and express a member/member truss heel and a host/fill window as
   hand-authored rule-output fixtures. This is the review gate.
2. **Timber knowledge**: implement enough rules for one complete braced
   king-post truss in `joiner_timber`, then replace the overlap constants in
   `basilica_ruin`.
3. **Masonry knowledge**: implement the clerestory opening and minimal
   coursing rule in `joiner_masonry`.

Stage 2 completed on 2026-08-25 without changing this boundary. The timber
library fits housed heels, a keyed king-post through-tenon, both ends of the
compression struts, and paired principal-rafter bearings at the king head.
The tie relation names the shoulder on the tie top while the post extent and
member include the exposed tenon tip below it. Full-section housed bearings
name the carried member endpoint and cut only the carrier. These public
geometric contracts, minimum-relish checks, and load mechanisms are documented
beside the rules and pinned by constructive evaluation tests. The lab renders
isolated fits plus assembled and exploded complete trusses; `basilica_ruin`
reuses the same fitted recipes in every intact station.

The upper strut bearing records the rafter as physically carried by the strut
but deliberately adds no directed transfer. That contact closes the
rafter/strut/king force triangle; representing every internal force as a
downstream edge would create a cycle and misrepresent `joiner`'s acyclic
support explanation as a statics model. The heel uses the same contact-only
principle at the other edge of the truss triangle.

The secondary-roof extension keeps that boundary and adds two deliberately
different rules. Purlins remain full-section while a through trench edits the
principal rafter; purlins again remain full-section while an underside seat
edits each common rafter. Setout authors an overlap equal to the cut depth,
and each rule validates that contract before removing it. The common crossing
footprint is an internal geometry helper, not a public generic-notch rule:
edit ownership, load direction, end-relish checks, and stable rule identity
remain specific to the two structural roles. The lab's former `ridge-member`
is renamed `purlin-*-upper` because it lies below the apex; no apex joint or
longitudinal purlin scarf is implied by this slice.

Cut depths, minimum bearing, remaining depth, and end relish enter each rule
as exact `Length` values. A rule lowers that parameter family together when it
meets the floating participant extents; the shared crossing helper remains
private analytic geometry and does not become an authored-unit layer.

Applicability is deliberately independent of a rule's default parameters. It
checks whether the authored roles, crossing, and overlap can support that joint
form; instantiation then checks the caller's cut depth against the authored
overlap and enforces remaining depth, bearing size, and clear timber beyond
both ends. This lets a valid custom-depth crossing be assessed without making
the default dimensions part of the construction graph's semantics.

The structure lab's `model.rs` seeded the mechanism crate. The stage-1 fixtures
proved that both relation families fit the mechanism before the rule libraries
existed; stage 2 replaces the timber fixture geometry with concrete rules.

## Consequences

- The "joint specimen" work deferred by ADR 0001 is owned by the `joiner`
  promotion ticket, not by a lab-private extension.
- `basilica_ruin`'s per-part joint constants become a known defect with a
  planned replacement, not an accepted design.
- `exedra_constructive` gains one small, generally useful primitive
  (profile offset) and otherwise stays ignorant of construction.
- `exedra_assembly` ADR-0001's "cross-part geometry operations are out of
  scope" stays true: `joiner` performs them by editing recipes *before*
  registration, never by booleans between instances.
- Rule libraries can be authored, versioned, and licensed independently of
  the mechanism, which matters for knowledge sourced from databases and
  suppliers with their own terms.

## Alternatives considered

- **Extend `exedra_ops`.** Rejected: Exedra Ops' contract is the mesh operator
  lifecycle; a construction vocabulary would double its responsibility and
  pull building semantics into every Exedra Ops consumer.
- **Extend `exedra_assembly`.** Rejected: the structure head is a
  representation-neutral instance tree for external runtimes; building
  elements and rules are a different lifecycle with a different dependency
  surface (evidence, rule libraries, validation).
- **Keep growing the structure lab in `examples/`.** Rejected: the lab has
  already reached the reusable boundary; continuing there would make the
  Basilica the owner of a general mechanism.
- **One crate holding mechanism and all rules.** Rejected per the
  modularity and replaceability tenets; see mechanism versus knowledge.
- **Erased, document-shaped rule parameters in the core** (an agent-facing
  registry from the start). Rejected: keeps the core calm and typed; the
  erasure boundary is additive if and when it is earned.
