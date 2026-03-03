---
id: exe-203r
status: open
deps: []
links: []
created: 2026-03-03T05:37:17Z
type: task
priority: 1
assignee: Bruce Mitchener
tags: [v0.1, decision]
---
# Decide OUTSIDE face representation

Decide whether FaceId::OUTSIDE is a real arena entry with a valid Face record, or a sentinel constant treated specially. The spec says pick one and document. Both approaches have tradeoffs.

## Design

Option A: Real arena entry
- OUTSIDE is the first face inserted into the arena (well-known slot)
- Has a valid Face record with edge pointing to one boundary half-edge
- Pro: uniform traversal code, no special cases in get/iteration
- Con: degree field is meaningless (boundary may have many disconnected loops), must skip OUTSIDE in face iteration for rendering/extraction

Option B: Sentinel constant
- OUTSIDE is a compile-time constant (e.g. FaceId with index=u32::MAX, gen=1)
- Not in the arena; checked explicitly
- Pro: no wasted arena slot, face iteration naturally skips it
- Con: every face access needs a branch or separate code path

This must be decided before or during exe-cbv1 (Mesh struct). The choice affects validation, iteration, and extraction code.

## Acceptance Criteria

- Decision documented in an ADR or as an update to the boundary model section
- Implementation in exe-cbv1 aligns with the decision
- Validation code aligns with the decision


## Notes

**2026-03-03T06:17:41Z**

Design brief: crates/exedra/docs/briefs/02_outside_face_boundary_model.md
