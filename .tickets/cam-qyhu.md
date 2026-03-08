---
id: cam-qyhu
status: closed
deps: [cam-5np0]
links: []
created: 2026-03-08T05:49:03Z
type: task
priority: P1
assignee: Bruce Mitchener
tags: [v0.5]
---
# Clear authored corner normals

Add an operator that clears authored corner normal overrides on selected faces so extraction falls back to derived normals.

## Design

Input is a canonical face selection. For each selected face corner, clear attr::CORNER_NORMAL_OVERRIDE through exedra::op::set_corner_normal_override. Output returns the affected face set. Compile canonicalizes and rejects stale faces.

## Acceptance Criteria

- edit.normal.clear exists\n- Clears overrides on all corners of selected faces\n- Compile canonicalizes and rejects stale faces\n- Tests cover typed output and fallback-to-derived behavior

