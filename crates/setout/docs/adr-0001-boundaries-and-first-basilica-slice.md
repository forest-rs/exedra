# ADR-0001: Setout boundaries and the first basilica slice

## Status

Accepted for implementation planning on 2026-08-23. Implementation has not
started. Ticket `set-qckq` owns the first milestone. The `crates/setout`
directory is not a workspace crate until that milestone supplies an earned,
working slice.

This is the milestone's one owning ADR. Slice-level implementation rationale,
API examples, and validation detail belong in crate rustdoc and commit messages;
they do not require child tickets or one ADR per crate.

## Context

The Exedra workspace now has a real construction layer. `joiner` owns elements,
construction relations, analytic extents, typed rule application, contact and
load-path validation, dirty channels, and lowering to `exedra_assembly`.
`exedra_math` owns deterministic floating-point vector arithmetic over plain
arrays. The basilica structure lab consumes `joiner`.

The remaining defect is upstream of construction. Roof dimensions, endpoints,
and world-space anchors are still authored or recomputed independently. They
agree by convention instead of following from one set of premises. The basilica
also needs to explain the historical basis of those premises without making
historical reconstruction vocabulary universal arithmetic policy.

The long-term design adds deterministic generative expansion, but the first
milestone must reach a real basilica contact seam before that broader layer
lands. It must preserve the final architecture rather than introduce a temporary
directed formula graph.

## Fences

`setout` owns deterministic propagation from explicit root claims and decisions
to typed quantities and structural provenance; it explicitly does not own
construction knowledge, reconstruction policy, geometry realization, or spatial
generation policy.

`setout_reconstruction` owns historical source catalogues, warrants, assessment,
recommendations, and annotated explain; it explicitly does not mutate a setout
evaluation or silently choose an operative claim.

`setout_generate` owns deterministic topology expansion into immutable network
fragments and opaque domain payloads; it explicitly does not own construction
elements, arithmetic propagation, or consumer evidence policy.

`setout_joiner` owns translation between setout definitions/evaluations and
concrete joiner declarations, analytic extents, and rule parameters; it
explicitly does not own propagation, construction rules, reconstruction policy,
or joiner validation.

`joiner` remains independently usable. It does not depend on any setout crate.

## Invariants

1. Stable semantic keys, never container indices or displayed ordinals, are
   durable identity.
2. Persisted lengths and exact point components are integer iota values.
3. Definitions are immutable during planning and evaluation.
4. Every determined claim has structural ancestry to roots and explicit
   decisions.
5. One evaluation has at most one operative claim per quantity; challengers are
   retained but require a counterfactual evaluation to propagate.
6. Policy analyses may propose decisions but never apply them.
7. A durable claim selection contains a semantic producer directive for plan
   compilation and an expected structural claim key for non-rebinding
   validation.
8. A durable discrete selection names a stable candidate key, never an ordinal.
9. Missing or changed selection targets remain visible orphans.
10. A setout-managed joiner element receives its analytic extent from one
    resolved binding. The frontend does not author a second placement or extent.
11. Joiner evidence remains an explicit construction assertion. A
    reconstruction assessment does not silently rewrite it or choose a rule.
12. Exact setout points cross into floating geometry once. `setout_joiner` uses
    `exedra_math` for the fixed-order vector operations that construct a joiner
    frame and fingerprints the lowered bits separately.
13. Dirty mappings are expressed in stable quantity and element keys; a value
    change that lowers to identical floating bits does not dirty geometry.
14. No crate lands without an independent fixture proving its responsibility.

## Options considered

### Add setout directly to joiner

This would place geometry bindings, setout contributors, and reconstruction
projection in `joiner` itself. It was rejected because it would make the generic
construction mechanism depend on one planning system, weaken replaceability, and
couple joiner's public API to an engine that has not yet landed.

### Keep all integration in the basilica frontend

This would preserve crate independence but duplicate key mapping, strict
consumption, extent construction, provenance bindings, and dirty-channel mapping
at every future construction consumer. It was rejected because the seam is
general and already has two independently meaningful sides.

### Build a narrow directed graph before the final engine

This would reach one numeric demo quickly but would postpone multi-way methods,
conflict retention, durable decisions, and provenance identity. It was rejected
because those semantics determine storage and planning; retrofitting them would
replace the first implementation rather than extend it.

### Chosen: a vertical long-term spine with a sibling adapter

Land the consumer-neutral core and the `setout_joiner` adapter together in the
first milestone, proved by neutral fixtures and one exact-endpoint basilica path.
Land reconstruction assessment as the next reviewable slice in that same
milestone. Deterministic generative expansion follows only with its own topology
fixture and construction consumer.

The tradeoff is a larger first milestone than a formula DAG. In return, every
public identity and boundary belongs to the intended long-term system, and the
first result removes real duplicated authoring rather than demonstrating an
isolated engine.

## Decision identity

A `ClaimKey` remains the structural identity of a realized claim. It is not
enough by itself to compile a fresh counterfactual plan because a challenger may
not exist until a relation check executes. Durable selection therefore records:

- a semantic producer: external root, generated root, relation method, or solver
  output;
- the expected `ClaimKey` observed when the decision was authored.

The producer directs planning. Evaluation reconstructs the selected claim and
checks the expected key. Changed ancestry produces `OrphanedDecision`; it never
silently binds the same producer name to a different claim.

Discrete knowledge stores candidates with stable `CandidateKey` values. A
candidate key is semantic within its producing claim: mathematical alternatives
derive it from canonical value identity, while named choice domains use stable
option identity. Ordinals are presentation data only. A candidate decision also
names the producing claim selection, so a surviving option from a different
claim cannot capture an old decision.

## Joiner integration

`setout_joiner` depends on `setout`, `joiner`, and `exedra_math`; the reverse
dependencies do not exist. An optional reconstruction bridge may be earned when
`setout_reconstruction` lands, without adding reconstruction vocabulary to the
base adapter.

The adapter owns:

- stable mappings between setout path keys and joiner's flat key grammar;
- construction declarations whose geometry fields are setout bindings;
- contributors that translate construction relations into network fragments;
- strict resolution of exact endpoints, dimensions, and typed rule parameters;
- construction of `joiner::OrientedBox` values from resolved bindings;
- claim/support links retained beside materialized joiner records;
- mapping evaluation deltas to joiner's `GEOMETRY`, `CONTACT`, and `LOAD_PATH`
  channels.

Joiner's analytic extent contract remains intact. For a setout-managed element,
the adapter materializes that claim from the resolved binding, so it is not a
second frontend-authored truth. Joiner continues to validate contacts, relation
incidence, compiled recipes, and load paths against the materialized extent.

Joiner evidence and reconstruction assessment remain distinct. Joiner evidence
states the basis of a construction assertion and may participate in rule
suitability. Reconstruction assessment interprets quantity provenance. A bridge
may validate shared source identities or deliberately project an accepted
assessment into a new construction scenario, but it never updates joiner evidence
or applies a rule implicitly.

## First milestone

The first milestone is the exact basilica roof spine, delivered as reviewable
slices rather than one unreviewable commit.

1. `setout`: typed quantities; exact iota `Length` and `Point3`; checked
   `Rational`; stable claim, support, candidate, and decision identity;
   multi-way `Sum`, `ScaleLength`, `OffsetLength`, `Equal`, `Pitch`, and integer
   `Pythagorean`; plan compilation; evaluation; local conflicts;
   counterfactuals; strict access; explain; fingerprints; from-scratch oracle.
2. `setout_joiner`: segment-member and box bindings; one-time exact-to-float
   lowering; stable key mapping; delta-to-dirty mapping; the basilica roof
   fixture deriving rafter foot, head, and slope length without a duplicated
   frontend world coordinate; joiner contact and recipe validation still pass.
3. `setout_reconstruction`: source and calculation axes; limiting-premise
   assessment; explicit proposals; analysis-only invalidation; annotated roof
   explain. It does not automatically replace joiner evidence.

`setout_generate` follows with stable invocations, labeled choices, strict staged
expansion, orphaned overrides, and incremental equivalence. It does not land as a
placeholder in the first milestone.

## Extension points

- Bounded measurement knowledge extends method capability and planning; it is not
  represented as floating-point tolerance.
- `setout_fiksi` may adapt explicit simultaneous solver groups; fiksi is never a
  silent fallback or a core dependency.
- Spatially demand-driven world refinement may add region, semantic-detail, and
  budget inputs to generation. Parent/child fragment identity and boundary
  contracts must let refinement add knowledge without replacing entity identity.
- `joiner::OrientedBox` consolidation moves only if the adapter proves a genuine
  second owner for the representation; exact endpoint binding alone does not force
  it into a lower crate.
- A shared provenance-analysis trait waits for a second real analysis.

## Migration and dependency notes

There is no existing setout public API to migrate. `joiner` requires no public API
change for the initial adapter. If implementation reveals that a joiner API must
change, the change stops for a migration note and explicit approval.

The maintainer has pre-approved production dependencies on `joto_constants`,
`joto_format`, and `joto_parse`. Parsing and formatting remain adapters or feature
gates; exact iota representation belongs in the core. Any other new production
dependency, including a fingerprint implementation, still requires the normal
dependency decision.
