---
id: cam-bfrq
status: open
deps: []
links: []
created: 2026-03-04T00:40:37Z
type: task
priority: 2
assignee: Bruce Mitchener
tags: [v0.1, api]
---
# Add edge-specific op report counter and migrate edge tag ops

Edge tagging operators currently reuse counters.corners_written to report canonical edge sparse writes. This is semantically overloaded and can mislead telemetry consumers. Add an explicit edge write counter and update seam/sharp operators.

## Design

Extend SmallCounters with edges_written (or edge_attrs_written) while keeping backwards-compatible counters for now. Update edge_mark::apply_edge_bool_tag to increment edge counter; keep corners_written unchanged unless a migration policy is chosen. Document semantics in rustdoc and changelog/ticket note.

## Acceptance Criteria

1) New edge counter exists in OpReport stats. 2) seam/sharp ops report edge writes via the new field. 3) Tests updated to assert the new metric. 4) clippy/test pass.

