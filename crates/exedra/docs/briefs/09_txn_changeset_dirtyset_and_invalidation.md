# Brief: EditSession → ChangeSet → DirtySet, and how it plays with `invalidation` (formerly )

## Decision
All Exedra mutations happen in an explicit eager **edit scope** (`EditSession`).
Finishing a recorded edit scope produces a deterministic **ChangeSet**
containing created/deleted IDs and a conservative **DirtySet** for invalidation.
Cambium uses Exedra’s ChangeSet/DirtySet as the **source of truth** for
mesh-derived invalidation, and uses `invalidation` (formerly `understory_dirty`) only for
**Cambium-runtime caches and workflow state**.

## Why
Incremental systems rot when invalidation is “inferred”:

- It’s easy to miss dependencies
- Caches get subtly wrong
- Debugging becomes superstition (“try clearing caches”)

By making ChangeSets explicit, you make incremental extraction, derived data recomputation, and debugging architectural, not optional.

`invalidation` becomes an accelerator for Cambium’s derived/cached state (selections, adjacency caches, operator-local fields) without confusing the kernel boundary.

## Alternatives considered
- **Direct mutation without recorded change output**: simplest code at first, but dirtiness becomes ad-hoc and brittle.
- **Cambium infers kernel dirtiness**: duplicates kernel logic in the operator layer; guaranteed drift.
- **Single global dirty channel**: too coarse; loses the ability to budget recomputation and makes UIs feel “mushy.”

## Implications
- Exedra defines what “dirty faces/vertices/corners” mean for extraction and derived data.
- Cambium preview/apply runners can use the same operator logic: edits → finish recorded scope → consume `DirtySet`.
- Cambium maps Exedra dirty sets into a small number of `invalidation` channels deterministically and conservatively.

## Non-goals / deferrals
- Perfect minimal dirty sets initially. Conservative is acceptable; refine based on profiling and wind tunnels.
