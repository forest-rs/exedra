# ADR-0001: Construction-layer scope

## Status

Accepted (2026-08-21). Restates and narrows the decision recorded in
[`examples/basilica_structure_lab/docs/adr-0002-joiner-construction-layer.md`](../../../examples/basilica_structure_lab/docs/adr-0002-joiner-construction-layer.md),
which owns the naming, the alternatives considered, and the rule-library
split. This ADR is the crate-local contract and records the IR decisions taken
when the first slice landed.

## Context

Structure-lab ADR 0002 decided that a new crate, `joiner`, owns the
construction layer: above `exedra_assembly`, below any future planning layer,
knowing about building elements and their relations rather than about meshes.
Cambium ADR-0002 forbids placeholder crates, so the crate was created together
with a first slice that carries real semantics.

The seed of that slice is `basilica_structure_lab`'s `model.rs`: an
evidence-labelled graph of elements, members, joints, bearings, supports, and
witnessed load transfers, with three layers of validation. What it lacked was
the generating step — a rule that turns a declared relation between elements
into coordinated geometry on all participants.

## Decision

`joiner` owns the **mechanism** of construction and none of its knowledge.

### Owned

- **The element graph.** Elements with stable frontend-supplied string keys,
  each carrying an opaque role, a material key, an [`OrientedBox`] extent, an
  optional constructive part, an evidence label, and a presence flag.
  Centreline members between named nodes. Ground supports.
- **Three relation kinds**, as first-class siblings in one IR: host/fill,
  member/member, element/units. None is expressed in terms of another.
- **The rule seam.** `Rule::assess` returning a typed `Applicability`, then
  `Rule::instantiate` returning a `RuleOutput`, with strongly typed
  per-rule parameters through an associated type.
- **The uniform rule output**: part edits, generated parts, contact patches,
  load-path edges. The same four for every rule of every relation kind.
- **Validation**: schema and coherence, contact at a documented `1e-9 m`
  tolerance, and load path.
- **Lowering** to an `exedra_assembly::Assembly`.
- **Evidence labelling** on elements, relations, contacts, part edits, and
  rule applications.

### Not owned

Geometry math (`exedra_constructive`, `exedra`); site, massing, and plan
layout; statics, finite-element analysis, capacity, code compliance, or
certification; rendering and export; Cambium's operator lifecycle; and any
erased, document-shaped, or agent-facing parameter boundary. Specific joints,
bonds, and profiles live in rule-library crates.

## IR decisions

These were taken when the first slice landed and are the reason the IR looks
the way it does.

### A member/member relation *is* the joint

The structure lab had both a `Joint` record and, implicitly, the relation it
described. Keeping both would mean keeping two things consistent. Instead the
member/member relation is the joint: it names the node and the members, it
carries the evidence and an opaque fit label, and it is the witness a
`TransferKind::Joint` transfer requires. One record, one truth.

Incidence is a solid claim, not a centreline-intersection claim: the joint node
must lie inside every participating member element's analytic extent. Members
still identify the centreline topology and the participants, but real surface
joints such as a rafter heel on a wall plate need not sit on both centrelines.

### Rule output names participants by key, never by handle

A rule output is authored *before* the parts it generates exist, so a handle
would be meaningless for half of what it refers to. Since keys are already the
identity contract, output uses them throughout, and `Construction::apply`
resolves them. `ElementId` exists only because dirty tracking needs a dense
`Copy` key, and is documented as a handle.

### Generated parts are elements

A generated sill is not a lesser kind of thing than an authored wall: it can
host its own relations, be cut by a later fit, carry load, and be omitted from
a hypothesis. So `RuleOutput::generated` is a list of `Element`s, stamped with
`ElementOrigin::Generated` on merge, and lowering treats them identically.
Applying a rule to a host/fill or element/units relation also appends the
generated keys to the relation's participants, so the relation ends up naming
what actually filled it.

### Rule output and authored fact share one table

`Construction::apply` merges an output into the *same* contact and transfer
tables a frontend authors into. Downstream, a rule-produced bearing and a
declared one are indistinguishable — which is precisely what lets validation
consume rule output without knowing that rules exist. Provenance is not lost:
`Construction::applications` records which rule fitted which relation, and
generated elements name the application that made them.

### Invalid states are rejected at insertion, not reported by validation

`Construction` is append-mostly and validated at every mutation, like
`exedra_assembly::Assembly`: keys are non-empty, free of `/`, and unique
within their category, and every reference resolves at the moment it is made.
Every insertion checks uniqueness before changing its map, and transfer
insertion checks the whole edge before marking either endpoint dirty, so a
rejected mutation is observationally atomic.
`Construction::apply` pre-flights an entire application, so a rejected one
merges nothing. This is stronger than the structure lab, which reported
duplicate and dangling keys from `validate`; those codes no longer exist
because those states are no longer representable. Validation keeps everything
that genuinely needs the whole graph.

### The extent is an analytic claim, distinct from the part

An element declares an `OrientedBox` extent *and* may carry a constructive
recipe. Contact and load-path validation measure the extent; lowering compiles
the recipe and places it by the extent's own frame. `Element::new` derives the
part from the extent so the two start as one expression. This is what keeps
`joiner` out of geometry math: it never tessellates anything to find out where
a face is, and it says so — a contact is an *analytic anchor* claim, exactly
the claim the structure lab already made.

An extent frame may be right- or left-handed. Both are finite orthonormal
frames, and lowering preserves the handedness in the explicit placement. This
lets paired building elements share local construction coordinates without
silently rewriting a reflected frame. A renderer that retains the instance
transform can communicate its negative determinant downstream; a renderer that
bakes the transform into vertices must reverse triangle winding. The structure
lab's OBJ path does the latter and pins the reflected north roof covering as a
regression fixture.

### Contact meaning decides what can carry load

A contact patch declares a meaning: bearing, side fit, shoulder, mortar bed,
or clearance only. Bearing, shoulder, and mortar bed can witness a contact
transfer; side fit and clearance cannot. A clearance-only patch is the inverse
assertion — it is validated for *non*-penetration rather than coincidence — so
"these deliberately do not touch" is a statement the model can make and prove
rather than an omission.

### Part edits are composed into one n-ary node before registration

`exedra_assembly` ADR-0001 puts cross-part geometry operations out of scope,
so a cut is never a boolean between placed instances. Lowering folds each
participant's consecutive same-kind edits into a single
`Csg { Difference | Intersection, [base, tool, ...] }` node on that
participant's own recipe, and freezes it before registration. Tools are
expressed in the *target's* local frame, which is how both sides of a fit can
derive from one nominal expression without this crate ever inverting a matrix.

### Recipe splicing is explicit, and its two limits are typed

Composing edits means grafting a frozen tool recipe into another recipe's
builder, remapping every interned id. Two things cannot be done and are
reported rather than guessed at: `NodeKind` is `#[non_exhaustive]`, so a node
kind added upstream yields `LowerError::UnsupportedToolNode`; and profile
segments reference curve policies by index, so a tool whose policies cannot
re-intern to the same indices yields `LowerError::PolicyRemapUnsupported`.

## Consequences

- The structure lab becomes a consumer of this crate rather than the owner of
  a general mechanism, keeping only its basilica-specific hypothesis authoring
  and its OBJ emission.
- Rule libraries can be authored, versioned, and licensed independently.
- Incremental lowering is *not* implemented in the first slice. The dirty
  channels and their marking are, and `Construction::take_dirty` drains them
  deterministically; a consumer can already re-lower selectively. Making
  `lower` itself incremental is follow-on work.
- Lowering depends on the Boolean pipeline handling an n-ary difference,
  including cutters that touch, overlap, or share a projection. That behavior
  is owned and regression-tested in `exedra`; rule libraries continue to emit
  the ordinary n-ary operation and must not reorder, separate, or otherwise
  disguise interacting cutters.
