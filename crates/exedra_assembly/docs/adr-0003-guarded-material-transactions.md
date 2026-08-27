# ADR 0003: Guard material binding transactions

- Status: proposed
- Date: 2026-08-27
- Depends on: ADR 0002 and `forest-rs/exedra#11`

## Context

ADR 0002 deliberately left guarded material editing out of the Addressable
adoption slice. The editing workflow is useful but is a capability addition,
not a prerequisite for replacing Exedra's path and query machinery. Keeping it
in a stacked change makes that cost and contract independently reviewable.

A material read observes an effective value chosen by Exedra's
instance-over-part policy. Editing from that observation must not silently
target a different occurrence, referent, revision, effective value, or material
facet. A multi-target edit must also avoid partial application.

## Decision

`exedra_assembly` adds a domain-specific guarded transaction workflow:

- `BindMaterial` carries a typed material endpoint, proposed binding, and
  `Guard<AssemblySpace, AssemblyReferent, Option<Box<str>>, EditCapability>`;
- validation checks the selected revision, endpoint, semantic referent, guard
  revision, typed capability, effective value, and duplicate targets before
  any write;
- dry runs return the same change and undo evidence without mutation;
- applied non-empty batches commit their validated slot-index writes in place
  and advance the Addressable revision exactly once;
- failed validation leaves both assembly state and revision unchanged; and
- reports retain effective-value changes and prior authored bindings. Undo is
  evidence rather than an executable command because Exedra does not yet expose
  binding removal.

The workflow remains in `exedra_assembly` because material resolution and
binding validity are Exedra policy. It does not move assembly storage or
material mutation into Addressable.

## Consequences

- Callers can preview and atomically apply guarded material batches while stale
  locations, handles, and pins remain visibly stale after a successful apply.
- Application does not clone the `Assembly`; validation makes the subsequent
  slot-index writes infallible under the current representation.
- The validation sequence resembles Addressable's reference mutation
  implementation. This PR remains draft while we decide whether its generic
  revision/guard/atomicity skeleton belongs in Addressable, leaving only
  material policy and application in Exedra.
- Structural, metadata, and part-content authoring continues to use
  `AddressableAssembly::commit`.
