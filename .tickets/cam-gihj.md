---
id: cam-gihj
title: Mark edge sharp operator
status: closed
deps: [cam-ibof, exe-k3nb]
links: [exe-k3nb]
created: 2026-03-03T06:00:47Z
type: feature
priority: P1
assignee: Bruce Mitchener
tags: [v0.1]
---
# Mark edge sharp operator

Implement the mark-edge-sharp selection/tagging operator. Sets the edge sharpness attribute on selected edges. Simple operator that validates the EditOperator + attribute write path.

## Acceptance Criteria

- Implements EditOperator
- Sets edge sharpness on selected edges
- ChangeSet correctly reflects changes
- Unit tests


## Notes

**2026-03-03T06:37:44Z**

Worked example: docs/worked_example_basilica.md — step 6 (shade.sharpness.from_angle) tags sharp edges by dihedral angle across the whole basilica mesh.

**2026-03-03T17:46:14Z**

Implemented MarkEdgeSharp operator with deterministic edge canonicalization, stale-edge precondition errors, and change accounting. Added tests for set/clear semantics and duplicate selection canonicalization. Validated with cargo fmt --all, cargo clippy --workspace --all-targets --all-features -- -D warnings, cargo test --workspace --all-features.
