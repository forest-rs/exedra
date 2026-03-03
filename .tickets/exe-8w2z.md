---
id: exe-8w2z
title: delete_faces kernel primitive
status: open
deps: [exe-dey4]
links: []
created: 2026-03-03T07:06:40Z
type: delete_faces kernel primitive
priority: P1
assignee: Bruce Mitchener
---
# delete_faces kernel primitive

Add `delete_faces(faces: &FaceSet, policy: DeletePolicy) -> ChangeSet` as a kernel edit primitive. Needed for boolean cleanup, operator workflows, and general mesh editing. Must go through the transaction system and produce a proper ChangeSet with dirty bits.

## Preconditions

- Input `FaceSet` must be canonical (sorted, deduplicated)
- Input must not include `FaceId::OUTSIDE` — reject with a structured error
- All face IDs must be live (not already dead) — reject with error

## Design

### Key identity: `from(h)`

Half-edges store only `to` (destination vertex). The origin vertex is derived:

```
from(h) = half_edge(prev(h)).to
```

This is needed throughout deletion to construct boundary half-edges. Since prev is derived by walking in v0.1 (see exe-ei01), this is an O(k) operation per half-edge where k is face degree.

### Algorithm (deterministic, process faces in increasing stable-id order)

**Phase 1: Classify edges**

For each face being deleted, walk its half-edge loop. For each half-edge `h`:
- Let `t = twin(h)`
- If `t.face` is also being deleted → **both-sides-deleted**: no boundary needed, both `h` and `t` will be removed
- If `t.face` is a surviving interior face → **one-side-deleted**: `t` needs a new boundary twin
- If `t.face` is OUTSIDE → **was-already-boundary**: the existing boundary half-edge `t` is also removed (the boundary retracts)

**Phase 2: Remove faces and their half-edges**

For each deleted face (in stable-id order):
1. Mark the face arena slot dead (generation bump)
2. For each half-edge in the face loop: only delete half-edges whose `face` is being deleted — never touch the surviving twin

**Phase 3: Create boundary half-edges (the critical part)**

For each one-side-deleted edge where surviving twin `t` goes from `v0 → v1` (i.e., `t.to = v1`):
1. Create a new boundary half-edge `b` with:
   - `b.to = v0` (i.e., `from(t)`, the reverse direction)
   - `b.face = FaceId::OUTSIDE`
   - `b.twin = t`
2. Set `t.twin = b`

**Phase 4: Stitch OUTSIDE boundary loops**

New boundary half-edges must be linked into closed OUTSIDE loops via `next` pointers. Deterministic stitching rule:

For a boundary half-edge `b` (face=OUTSIDE), `next(b)` is the boundary half-edge that continues the boundary cycle. Walk from `to(b)` around the vertex using surviving interior half-edges to find the next boundary half-edge departing from that vertex. When multiple boundary loops are created (e.g., deleting non-adjacent faces), each loop is independent. Start each loop from the smallest half-edge ID not yet assigned.

Also fix up any pre-existing OUTSIDE boundary loops that were disrupted by the deletion (was-already-boundary edges).

**Phase 5: Vertex `out` pointer fixup**

For surviving vertices whose `out` half-edge was deleted:
- Reassign `out` to another valid **outgoing** half-edge from that vertex
- "Outgoing" means a half-edge whose origin is the vertex: `from(h) == v`
- Use the vertex star walk (via twin/next on surviving half-edges) to find a candidate

**Phase 6: Isolated vertex cleanup**

After all fixup, check for vertices with no remaining incident half-edges.

```
enum DeletePolicy {
    /// Remove isolated vertices (conservative cleanup). Preferred default.
    CleanupIsolated,
    /// Keep isolated vertices with `out` set to sentinel/INVALID.
    KeepIsolated,
}
```

`CleanupIsolated` is the expected default for most callers.

### ChangeSet

The resulting ChangeSet records:
- `deleted_faces`: all deleted face IDs
- `deleted_half_edges`: all deleted half-edge IDs (from deleted face loops + removed boundary half-edges)
- `deleted_vertices`: all deleted vertex IDs (if CleanupIsolated)
- `created_half_edges`: all new OUTSIDE boundary half-edges created in Phase 3
- Dirty bits (conservative):
  - `dirty_faces`: all surviving faces adjacent to deleted faces (faces containing the surviving twins)
  - `dirty_vertices`: all vertices incident to any deleted or newly created half-edge
  - `dirty_corners`: corners of affected surviving faces (extraction splitting and derived normals may change)

### Corner-domain attribute cleanup

Half-edges are corner IDs (CornerId == HalfEdgeId). Deleting half-edges implicitly deletes their corner attributes. New boundary half-edges get no corner attributes (they belong to OUTSIDE).

## Acceptance Criteria

- Input validation: rejects OUTSIDE in FaceSet, rejects dead face IDs
- Delete one face from a closed box → 5-face open mesh with correct OUTSIDE boundary loop
- Delete two adjacent faces → correct boundary loops around the opening
- Delete two non-adjacent faces → two separate boundary loops, each correctly closed
- Both-sides-deleted: deleting all faces sharing an edge removes both half-edges, no boundary created for that edge
- Delete a face from an already-open mesh → boundary loops merge/extend correctly
- Isolated vertices cleaned up with `CleanupIsolated` policy
- Isolated vertices preserved (out=sentinel) with `KeepIsolated` policy
- Surviving vertex `out` pointers reference valid outgoing half-edges
- Surviving half-edge twins point to valid half-edges (either interior or OUTSIDE)
- All OUTSIDE boundary loops are closed (following next eventually returns to start)
- ChangeSet records created boundary half-edges (not just deletions)
- ChangeSet dirty bits include adjacent surviving faces, incident vertices, and affected corners
- Resulting mesh passes `validate_fast()` in all cases
- Works through the transaction system
- Deterministic: same canonical FaceSet + policy → same result, same boundary loop ordering

