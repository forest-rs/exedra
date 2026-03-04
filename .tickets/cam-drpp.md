---
id: cam-drpp
title: Extrude and inset operators
status: closed
deps: [exe-tezb, exe-al9q]
links: []
created: 2026-03-03T06:01:15Z
type: feature
priority: P1
assignee: Bruce Mitchener
tags: [v0.1]
---
# Extrude and inset operators

Implement extrude (push faces along normals, creating side walls) and inset (shrink face boundary, creating frame). Basic region selection model.

## Acceptance Criteria

- Extrude creates side walls, moves selected faces
- Inset creates frame around selected faces
- Attribute propagation correct
- Unit tests with various face shapes
