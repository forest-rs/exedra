---
id: ep-lbmm
status: open
deps: [ep-oun3]
links: []
created: 2026-03-05T17:31:16Z
type: task
priority: 2
assignee: Bruce Mitchener
tags: [v0.1, primitives]
---
# Icosphere primitive

## Design

Add deterministic icosphere primitive with subdivision levels. Preserve stable vertex/face ordering for fixed params and provide semantic selections (faces.all, optional seam contract if present).

## Acceptance Criteria

1) IcoSphereParams + icosphere() added. 2) Subdivision levels produce expected face growth. 3) validate_fast/deep pass. 4) Determinism test included.

