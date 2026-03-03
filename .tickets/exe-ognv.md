---
id: exe-ognv
status: open
deps: [exe-17rj]
links: []
created: 2026-03-03T05:40:23Z
type: feature
priority: 1
assignee: Bruce Mitchener
tags: [v0.5]
---
# PropagatePolicy and edit propagation defaults

Implement PropagatePolicy — the policy struct that controls how attributes are propagated during topology edits. Defines defaults for each domain and allows callers (typically Cambium) to override.

## Design

PropagatePolicy {
  position_split: PositionSplit (Midpoint, WeightedMidpoint)
  uv_split: UvSplit (Midpoint, CopyFromSide)
  normal_override_split: NormalOverrideSplit (Clear, CopyFromSide, Average)
  face_attr_split: FaceAttrSplit (Copy, CopyAndTag)
  edge_attr_split: EdgeAttrSplit (Inherit, Clear, SplitWeights)
}

Consumed by split_edge, split_face, collapse_edge, flip_edge.
Exedra provides sensible defaults; Cambium may override per-tool.

Concrete defaults will emerge from implementation of the edit primitives — this ticket captures the framework and default values.

## Acceptance Criteria

- PropagatePolicy struct exists with all domain sub-policies
- Default() provides documented, sensible defaults
- Edit primitives accept optional PropagatePolicy
- Each policy variant is tested via edit operations

