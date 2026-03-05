---
id: cam-qhx4
status: closed
deps: []
links: []
created: 2026-03-05T09:59:41Z
type: feature
priority: P2
assignee: Bruce Mitchener
tags: [v0.1, operator, edit]
---
# Face solidify operator

Add edit.face.solidify operator for explicit shell thickness generation from face selections.

## Design

Operator builds on current face-edit kernels to generate offset shell with side walls and configurable source-face retention mode.

## Acceptance Criteria

- edit.face.solidify operator + params/output exist
- deterministic topology output for simple inputs
- diagnostics for unsupported/non-manifold preconditions
- tests cover open surface and closed surface behavior

