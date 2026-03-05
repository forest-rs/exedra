---
id: cam-5xnn
title: Region operations (loop selection, flood fill)
status: closed
deps: [cam-t6lk, exe-23ot]
links: []
created: 2026-03-03T06:01:15Z
type: feature
priority: P2
assignee: Bruce Mitchener
tags: [v0.1]
---
# Region operations (loop selection, flood fill)

Implement region selection operations: edge loop selection, face flood fill by tag/material. Foundation for more complex tool workflows.

## Acceptance Criteria

- Edge loop selection finds loops correctly
- Face flood fill by tag works
- Selections in canonical format
- Unit tests on representative meshes
