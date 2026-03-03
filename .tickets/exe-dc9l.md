---
id: exe-dc9l
status: open
deps: []
links: []
created: 2026-03-03T05:21:25Z
type: feature
priority: 0
assignee: Bruce Mitchener
tags: [v0.1, foundation]
---
# Stable ID type (index + generation)

Implement the core stable handle type used for all mesh element references. This is the most fundamental type in Exedra — everything else builds on it.

## Design

Public type: Id { index: u32, gen: NonZeroU32 }

Distinct types (newtypes or phantom-tagged generic — not plain type aliases):
- VertexId
- HalfEdgeId (== CornerId)
- FaceId

Requirements:
- Copy, Clone, Eq, PartialEq, Hash, Debug derives
- no_std compatible (core only, no alloc needed)
- Sentinel support: FaceId::OUTSIDE as a reserved constant
- NonZeroU32 for generation enables Option<Id> niche optimization

Open question: whether to use a single generic Id<Domain> with a phantom tag or separate newtypes. Phantom-tagged generic reduces boilerplate; newtypes allow per-domain methods. Plain type aliases are ruled out — no type safety.

## Acceptance Criteria

- Id type exists with index: u32 and gen: NonZeroU32
- VertexId, HalfEdgeId/CornerId, FaceId are distinct types
- FaceId::OUTSIDE sentinel constant exists
- All ID types are Copy + Clone + Eq + PartialEq + Hash + Debug
- Option<Id> is the same size as Id (niche optimization)
- No std dependency
- Unit tests for equality, hashing, sentinel identity, and niche optimization


## Notes

**2026-03-03T06:17:41Z**

Design brief: crates/exedra/docs/briefs/03_determinism_contract.md, crates/exedra/docs/briefs/07_stable_ids_and_compaction.md
