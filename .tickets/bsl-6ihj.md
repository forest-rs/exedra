---
id: bsl-6ihj
status: in_progress
deps: [ec-uoij, exe-zqct]
links: []
created: 2026-08-21T13:40:52Z
type: epic
priority: 1
assignee: Bruce Mitchener
---
# Promote the structural graph into the `joiner` construction crate

## Problem

Nothing in the workspace owns how building elements fit together.
`basilica_ruin` encodes joints as per-part overlap constants;
`basilica_structure_lab` models joints, bearings, and load paths as uncut
hypotheses and defers mating faces to a "joint specimen" ticket that was
never filed. Structure-lab ADR 0002 decides the reusable boundary: a new
crate, `joiner`, owns the construction layer, with construction knowledge in
separate rule-library crates.

Cambium ADR-0002 forbids placeholder crates, so `crates/joiner` is created
together with its first slice. This ticket owns that slice and the deferred
joint-specimen work.

## Fence

`joiner` owns the mechanism only: the element graph; host/fill,
member/member, and element/units relations; the typed rule trait
(`assess` then `instantiate`); the uniform rule output (part edits,
generated parts, contact patches, load-path edges); lowering to
`exedra_assembly` plus constructive edits; and evidence labelling.

It does not own geometry math (`exedra_constructive`, `exedra`),
site/massing/plan layout, statics/FEA/certification, rendering, or an
agent-facing erased parameter boundary. Specific joints, bonds, and profiles
live in `joiner_timber`, `joiner_masonry`, and later rule libraries.

## Terms

- **Mechanism:** the generic graph, rule seam, validation, and lowering owned
  by `joiner`.
- **Knowledge:** a particular joint, bond, or profile implemented by a rule
  library.
- **Fixture:** hand-authored rule output used to exercise the mechanism before
  its rule library exists.
- **Lowering:** compiling a construction graph into an `Assembly` artifact.

## Design

- Seed the mechanism crate from `examples/basilica_structure_lab/src/model.rs`
  (elements, joints, bearings, supports, transfers, evidence); add the
  relation kinds and the rule trait.
- Both sides of every fit derive from one nominal interface expression.
  Clearance is applied only on the receiving side, via the constructive
  profile offset (`ec-uoij`).
- The element graph is the source of truth and the dirty-tracking unit
  (through `invalidation`); the `Assembly` is a compiled artifact. Element
  keys are identity; instance paths derive from them deterministically.
- Stage 1 exercises two seed cases as fixtures so the IR is not shaped by one:
  a member/member truss heel and a host/fill clerestory window. Their concrete
  timber and masonry rules land in stages 2 and 3.
- The lab then becomes a consumer of `joiner`; `basilica_ruin` is rebuilt on
  it afterwards (breaking it is allowed by the North Star).

## Stages

The work ships in three session-sized stages. Stage 1 is the point of no
return for the IR and **stops for review before stage 2 begins**.

1. **Mechanism only.** `crates/joiner` with the element graph, three relation
   kinds, the rule trait and uniform `RuleOutput`, lowering to
   `exedra_assembly`, and the lab's contact/load-path validation running
   over rule output. No concrete rules; the seed of the crate is the lab's
   `model.rs`. The IR must be shown to express *both* seed cases (a truss
   heel and a window opening) as fixtures or doc examples, even though
   neither rule exists yet. Review gate here.
2. **Timber.** `joiner_timber` rules for a complete braced king-post truss;
   the lab and repeated `basilica_ruin` stations consume the fitted recipes.
   Depends on `ec-uoij` and `exe-zqct`.
3. **Masonry.** `joiner_masonry` opening void, sill/lintel/jambs, minimal
   coursing rule; one clerestory window in the lab.

## Acceptance

### Stage 1

- [x] `crates/joiner` exists (`no_std` + `alloc`) with the element graph,
      three relation kinds, the rule trait, the uniform rule output, and a
      crate-local ADR 0001 restating scope from structure-lab ADR 0002.
- [x] Lowering produces an `exedra_assembly::Assembly` whose instance paths
      derive deterministically from element keys.
- [x] The lab's contact and load-path validation runs over merged rule output.
- [x] `basilica_structure_lab` consumes `joiner`; its README no longer defers
      mating faces.
- [x] Evidence source and class are carried on every element, relation, and
      rule application.
- [x] Both relation families are exercised by deterministic fixtures.

### Remaining stages

- [x] `joiner_timber`: housed heels, keyed king-post-to-tie through-tenon,
      housed strut ends, and paired rafter-head bearings; interfaces are
      derived from participant sections and fit class is a typed parameter.
- [ ] `joiner_masonry`: opening void plus sill/lintel/jambs and a minimal
      coursing rule.
- [ ] Definition of Done (typos, fmt, taplo, clippy `-D warnings`, doc, and
      tests) passes for every later stage.

## Stage 1 gate

Accepted on 2026-08-23. The mechanism crate and lab migration are complete;
concrete timber and masonry knowledge remain stages 2 and 3. The two seed
fixtures prove member/member and host/fill rule output, lowering, contact
validation, and load-path validation without embedding either rule in the
mechanism.

Review fixed atomic duplicate rejection and transfer dirtiness, and made two
model invariants explicit in the crate ADR: member joints use analytic-extent
incidence rather than centreline intersection, and finite orthonormal extent
frames may have either handedness. glTF retains reflected node transforms; the
lab's baked OBJ reverses triangle winding when the world determinant is
negative, with `roof-covering-north` as a regression fixture.

Stage 2 dependencies `exe-zqct` and `ec-uoij` are closed.

Validation passed: `typos`, `cargo fmt --all`, `taplo fmt`, workspace clippy
with warnings denied, all-feature workspace tests, `cargo doc --no-deps`, and
`cargo check -p joiner --no-default-features`.

## Notes

**2026-08-25 — Stage 2**

The timber slice now fits one complete braced king-post truss rather than two
isolated specimens. The tie is suspended by an exposed through-tenon and
transverse timber key; the contacts route tie load through the key and tenon
slot instead of claiming a shoulder carries tension. Struts bear into the king
above the tie and into the rafters, and the rafter heads bear in two separate
housings. The 360 mm reference king head preserves a web between the paired
120 mm pockets. These checks establish geometry and intended transfer only,
not connection capacity or certification.

The lab evaluates every finished member, exports assembled and exploded truss
checkpoints, and retains named load paths to ground. `basilica_ruin` composes
the joinery once for each canonical north member, derives the south rafter and
brace through winding-correct constructive mirrors, and repeats seven
proper-rigid parts per intact station, including the generated key. The mirror
lives in each counterpart recipe rather than an assembly placement or an
exporter-specific mesh rewrite.

The combined post recipe exposed one remaining Boolean topology case: a face
split by a shoulder chain can also contain a closed blind-housing loop. The
splitter now assigns each loop to exactly one open-chain partition and re-faces
the ring and disk atomically; a direct regression and the complete post recipe
pin the behavior. No joiner-specific cutter reordering or geometric nudge was
introduced.

Validation passed on 2026-08-25: `typos`; `cargo fmt --all --check`;
`taplo fmt --check`; warnings-denied all-target/all-feature workspace clippy;
all-feature workspace tests; workspace rustdoc without dependencies; and the
`joiner_timber` tests with default features disabled. The structure lab
exported a clean 34-element, 18-joint construction, and Blender regenerated all
ten semantic checkpoints from those exported meshes.

**2026-08-23T02:00:31Z**

`OrientedBox` is `Placement3` plus an extent. It has no home in `exedra_assembly`
(instances carry no declared extent; `Aabb3` is a compiled envelope, the opposite
direction). Candidate: an oriented extent type in `exedra_constructive` beside
`Placement3`, with `joiner::OrientedBox` a re-export or thin wrapper. Trigger:
setout Slice D, when exact-endpoint bindings become a second consumer. Do not
move it on one consumer.
