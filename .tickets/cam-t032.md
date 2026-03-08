---
id: cam-t032
status: closed
deps: [cam-5np0]
links: []
created: 2026-03-08T05:49:03Z
type: task
priority: P1
assignee: Bruce Mitchener
tags: [v0.5]
---
# Bake face normals into authored overrides

Add an operator that writes flat face normals as authored corner overrides on selected faces.

## Design

Input is a canonical face selection. For each selected face, compute the deterministic face normal and write it to every corner override on that face. Degenerate faces report an operator error instead of inventing normals.

## Acceptance Criteria

- edit.normal.face exists\n- Writes one face-aligned normal override per selected face corner\n- Degenerate faces are reported cleanly\n- Tests cover flat shading on a quad/ngon

