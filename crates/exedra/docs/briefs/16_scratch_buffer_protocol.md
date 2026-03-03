# Brief: Scratch buffer protocol (reusable buffers across Exedra and Cambium)

## Decision
Exedra and Cambium use a shared set of conventions for reusable scratch buffers:

- Scratch is **caller-supplied** (or long-lived engine-owned) and passed into operations.
- `clear()` **retains capacity**; scratch grows as needed but is reused across calls.
- Operations must **not retain references** into scratch after returning.
- Scratch must never influence algorithmic decisions; it is **memory, not state**.
- Peak scratch capacity is measurable and treated as part of performance posture (“wind tunnels”).

This applies to:
- Exedra: `ExtractScratch`, `BooleanScratch` (and similar staging buffers)
- Cambium: `Scratch` in `OpContext`, runner-owned caches/scratch

## Why
Scratch buffers are the key to performance without hidden allocations:

- Avoid allocating in hot loops (triangulation, normal generation, intersection staging).
- Keep memory locality good by reusing contiguous buffers.
- Make performance predictable: allocation behavior becomes explicit and measurable.

A shared protocol prevents style drift between Exedra and Cambium.

## Core conventions (locked)

### 1) Caller-supplied, reusable
- Operations accept `&mut ScratchLike` and may resize buffers as needed.
- The caller controls the lifetime; scratch may live for the duration of an interactive session.

### 2) `clear()` retains capacity
- `clear()` (or equivalent) resets logical length but does not free memory.
- No operation calls `shrink_to_fit()` implicitly.
- Reclaiming memory is an explicit caller decision.

### 3) No references retained
- Operations must not return references into scratch-owned buffers.
- Operations must not store references/pointers/slices into scratch for later use.
- Any data that must outlive the call must be copied into caller-owned storage (or returned as owned values).

### 4) Determinism and correctness
- Scratch contents are treated as uninitialized/garbage unless written during the call.
- Scratch must not affect ordering, tie-breaking, or numeric decisions.
- Algorithms must behave identically regardless of scratch capacity history.

### 5) Boundedness and measurement
- Wind tunnels should record peak scratch capacities (per buffer) and total estimated bytes.
- Large intermediate artifacts should be streamed/dumped outside core scratch when needed.

## Recommended API patterns

### Clear-at-entry (simple v0.1 posture)
- Runner/engine clears scratch at the start of each operation:
  - Cambium: `OperatorRunner` calls `ctx.scratch.clear()`
  - Exedra: extract/boolean entry points clear their own scratch structs (or document expectations)

### Dedicated structs per subsystem
- Exedra keeps subsystem-specific scratch to avoid a single “god scratch”:
  - `ExtractScratch` for extraction/triangulation/normals
  - `BooleanScratch` for broad-phase BVH staging, intersection graph maps, etc.

- Cambium’s `Scratch` focuses on common operator needs and may include reusable `hashbrown` maps/sets.

### Hash maps/sets
- Hash maps/sets are allowed in scratch, with rules:
  - must be cleared and reused, not recreated
  - must not leak iteration order into externally visible outputs (sort if required)

## Non-goals / deferrals
- A unified trait for all scratch types in v0.1 (nice, but not required).
- Automatic memory budgeting in scratch; budgets are handled at the operator/engine level.
