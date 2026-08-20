# ADR-0004: Deterministic assembly pattern expansion

- Status: Accepted
- Date: 2026-08-20
- Owners: Cambium maintainers
- Ticket: `cam-5d5y`

## Context

`exedra_assembly` is the concrete structure head. It stores registered parts,
validated instance trees, stable frontend-supplied paths, bindings, and
metadata. Worked scenarios still had to hand-write loops for repeated placed
parts and repeated multi-member structures. Putting those loops in the mesh
kernel would confuse topology edits with workflow expansion; storing
procedural pattern nodes in `exedra_assembly` would turn its concrete model and
frozen interchange into a parameter language.

The first reusable seam only needs deterministic named linear occurrences.
Parameter snapshot binding and mirror recipe composition have distinct
lifecycles and failure modes, so they are intentionally not part of this
slice.

## Decision

Add `cambium::assembly`, backed by the existing workspace
`exedra_assembly` crate.

Fence: `cambium::assembly` owns deterministic repeat/distribute planning and
append-only expansion into an existing concrete `Assembly`; it does not own
assembly storage or interchange, parameter evaluation, constructive recipe
composition, mesh operations, or reflective transforms.

The module exposes:

- linear repeat from a start point and translation step;
- linear distribution between two parent-space anchors under an explicit
  endpoint policy;
- named instantiation of one or more member templates per occurrence,
  including material bindings and opaque metadata.

Ordinals are authored identity. An omission suppresses that ordinal without
renumbering any later occurrence. Callers choose a fixed minimum decimal
width; ordinals grow past that width rather than truncating. Keys are formed
from the exact prefix, ordinal, and optional member suffix.

Expansion order is occurrence-major in strictly ascending ordinal order, then
member slice order. The occurrence placement is composed outside the member's
local placement. All inputs, composed placements, referenced parents, parts,
slots, current sibling collisions, and generated collisions are checked before
the first mutation. Consequently every returned error leaves the input
assembly unchanged. Occurrence, member, and composed placements must all be
finite proper-rigid matrices: reflection, scale, shear, and degeneracy are
typed refusals. Successful mutation is performed on an isolated candidate
clone and replaces the caller's assembly only after the full append succeeds.

Repeat positions use direct ordinal multiplication rather than accumulated
matrix powers. Distribution's `count` is the number of authored slots before
omissions. Endpoint interpolation is:

- include both: `i / (count - 1)`;
- include start: `i / count`;
- include end: `(i + 1) / count`;
- exclude both: `(i + 1) / (count + 1)`.

Endpoint parameters zero and one return the caller's anchors bit-exact.
Interior points use convex interpolation, `(1 - t) * start + t * end`, avoiding
the overflow of `start + t * (end - start)` for large opposing anchors. Every
result is checked finite.

Including both endpoints with one slot is rejected because one authored slot
cannot represent both endpoint identities. Zero slots is an empty expansion.

One planning call accepts at most 65,536 authored occurrence slots, and one
instantiation call appends at most 65,536 concrete instances. The occurrence
and member product is checked before allocation, allocation uses fallible
reservation, and existing plus appended instances must fit the `u32`
`InstanceId` domain. Larger structures remain expressible as multiple
semantically named patterns. Cheap shape gates precede semantic traversal:
empty members first, then occurrence count, member count, and product. An
over-budget shape therefore never scans keys, parts, placements, omissions, or
collisions.

## Mirror sequencing

A reflection is not an assembly placement. Negative-determinant placements
would require winding repair, while evaluating and baking a recipe would lose
recipe identity, policy independence, and provenance. Mirror support therefore
waits for an `exedra_constructive` operation that immutably wraps a recipe root
in `NodeKind::Mirror`. Cambium can then register or reuse a distinct named
recipe-backed part and place it using only proper-rigid transforms.

## Consequences

- `cambium` gains one existing-workspace production dependency on
  `exedra_assembly`; there is no third-party dependency, `unsafe`, or `std`
  widening.
- `exedra_assembly` remains a concrete model. Pattern definitions are not
  retained and `exedra-assembly-v1` is unchanged.
- Existing Cambium callers are source-compatible. The new public module is
  additive.
- Atomic candidate replacement clones the current concrete assembly. This is a
  deliberate first-slice cost in exchange for a strong error contract; a
  future append transaction in the structure head could remove the clone
  without changing pattern semantics.
- The initial translation-only surface is deliberately narrow. Rotation or
  other occurrence families can be added as named operations after their
  composition and identity semantics are explicit.

## Alternatives considered

- **Put patterns in `exedra_assembly`.** Rejected because the structure head
  owns validated concrete state, not workflow evaluation.
- **Keep patterns example-local.** Rejected because repeated scenarios need
  one tested naming, ordering, collision, and atomicity contract.
- **Use reflective instance transforms for mirror.** Rejected because the
  concrete placement seam does not repair winding.
- **Bake mirrored evaluated meshes.** Rejected because it changes content
  identity and discards constructive semantics.
