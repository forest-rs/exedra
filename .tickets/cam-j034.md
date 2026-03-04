---
id: cam-j034
status: open
deps: [cam-9eop]
links: []
created: 2026-03-04T04:31:35Z
type: feature
priority: 2
assignee: Bruce Mitchener
tags: [v0.5, modeling]
---
# Profile sweep operator (path-driven extrusion)

Implement a deterministic sweep operator that moves a profile along a path to generate surface topology.

## Design

Operator consumes one profile plus an ordered path, generating segment-wise side faces with stable correspondence. v0.5 scope: linear/polyline sweep with deterministic frame policy and explicit limitations. Emit diagnostics for invalid path/profile combinations and keep reporting discipline aligned with other operators.

## Acceptance Criteria

- Operator type + params exist with stable name()
- Deterministic sweep output for same profile/path inputs
- Diagnostics for invalid/degenerate profile/path inputs
- Timings/stats/artifacts integrated
- Tests covering straight path, multi-segment path, and invalid inputs

