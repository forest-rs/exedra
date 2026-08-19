# Brief: `invalidation` (formerly `understory_dirty`) multi-channel dirtiness for Cambium runtime caches

## Decision
Cambium uses `invalidation` to track **Cambium-runtime** cache invalidation with multiple channels (selection, adjacency, UV-derived, operator cache). Exedra remains the source of truth for mesh-derived invalidation.

## Why
Cambium holds many “derived but not authored” caches (adjacency helpers, selection acceleration, operator-local fields). Multi-channel dirty tracking:

- avoids global cache nukes
- keeps recompute budgetable
- supports independent invalidation of unrelated caches
- stays efficient when channels are used sparingly

## Alternatives considered
- **Single dirty flag**: too coarse; recomputes everything too often.
- **Per-operator bespoke dirtiness**: leads to fragmentation and inconsistent behavior.

## Implications
- Channels are defined centrally; adding channels is a memory commitment.
- Mapping from Exedra ChangeSet/DirtySet to Cambium channels is deterministic and conservative.

## Non-goals / deferrals
- Per-corner Cambium dirty marks by default; prefer face/region granularity unless justified.
