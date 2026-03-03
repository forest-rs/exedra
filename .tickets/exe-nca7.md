---
id: exe-nca7
status: open
deps: [exe-dc9l]
links: []
created: 2026-03-03T05:22:18Z
type: feature
priority: 0
assignee: Bruce Mitchener
tags: [v0.1, foundation]
---
# Generational arena

Implement Arena<T>, the core storage type backing all mesh element arrays. Provides stable handles via generational indices, tombstone-based deletion, and deterministic iteration in slot order.

## Design

Arena<T> backed by Vec<Slot<T>> where Slot is either Occupied { gen, value } or Free { gen, next_free }.

Key properties:
- insert() returns an Id with the current generation
- remove(id) tombstones the slot (bumps generation, adds to free list)
- get(id) validates generation before returning &T
- Deterministic iteration: always in slot index order, skipping tombstones
- No implicit compaction (see separate compaction ticket)
- Capacity and len tracking for introspection

The arena must work with the Id type from exe-dc9l. Generation validation prevents use-after-free bugs.

Scratch-friendly: no per-element heap allocations. The Vec grows but individual slots are inline.

## Acceptance Criteria

- Arena<T> type exists with insert, remove, get, get_mut operations
- Generation-checked access (stale IDs return None or error)
- Deterministic iteration in slot order (skipping tombstones)
- len() and capacity() introspection
- No std dependency (alloc only for Vec)
- Unit tests for insert/remove/reuse, stale ID rejection, iteration order determinism


## Notes

**2026-03-03T06:17:41Z**

Design brief: crates/exedra/docs/briefs/07_stable_ids_and_compaction.md
