---
id: cam-e54o
status: closed
deps: [cam-5np0]
links: []
created: 2026-03-08T05:49:03Z
type: task
priority: P1
assignee: Bruce Mitchener
tags: [v0.5]
---
# Bake derived corner normals into authored overrides

Add an operator that freezes the current derived corner normals into authored overrides for selected faces.

## Design

Input is a canonical face selection plus NormalParams. Use Mesh::derive_corner_normals and write the resulting corner normals into CORNER_NORMAL_OVERRIDE for each selected face corner. Output returns the affected face set. Compile canonicalizes and rejects stale faces.

## Acceptance Criteria

- edit.normal.average exists\n- Writes the current derived corner normals as authored overrides\n- Accepts explicit NormalParams\n- Tests cover stable bake-from-derived behavior

