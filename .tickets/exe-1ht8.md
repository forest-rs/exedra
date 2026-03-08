---
id: exe-1ht8
status: closed
deps: []
links: []
created: 2026-03-08T08:07:11Z
type: task
priority: 2
assignee: Bruce Mitchener
---
# Fix smooth cylinder side normal derivation

Derived corner normals on cylinder side faces are grouping incorrectly, producing repeated vertical bands instead of a continuous radial sweep. Add a focused regression test and fix the smooth-group traversal.

## Design

Use an exedra_primitives cylinder fixture to assert that side-face corner normals follow the expected radial direction and that adjacent side faces around the ring contribute to one continuous smooth fan while the cap rim remains hard. Fix the corner-neighbor/group traversal in normals.rs rather than patching extraction or the viewer.

## Acceptance Criteria

- Regression test reproduces the bad cylinder side normals before the fix
- Derived normals on smooth cylinder side corners are radial and continuous around the ring
- Cap/side boundary remains hard
- Workspace quality gates pass

