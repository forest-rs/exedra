---
id: cam-tzew
status: open
deps: []
links: [exe-lopy]
created: 2026-03-03T06:01:15Z
type: feature
priority: 1
assignee: Bruce Mitchener
tags: [v0.1, infra]
---
# cambium_testkit crate

Create cambium_testkit workspace crate. Provides golden snapshot formats, debug dump serialization/deserialization, operator test corpus. Uses std.

## Acceptance Criteria

- Crate exists in workspace
- Golden snapshot format defined
- Debug dump serialization works
- Can be used from cambium tests


## Notes

**2026-03-03T06:37:44Z**

Worked example: docs/worked_example_basilica.md — defines per-step artifact expectations and golden test posture. Region constants (REGION_WALL_OUTER etc.) live in testkit.
