---
id: ep-we4l
status: open
deps: [ep-oun3]
links: []
created: 2026-03-05T17:31:16Z
type: task
priority: 2
assignee: Bruce Mitchener
tags: [v0.1, primitives, api]
---
# Cap fill policy for rotational primitives

## Design

Introduce a shared CapFill enum (None, Ngon, TriangleFan) and apply it to cylinder generation first. Preserve existing behavior through default mapping. Ensure deterministic cap topology/selection semantics for each fill mode.

## Acceptance Criteria

1) Shared CapFill enum exported from exedra_primitives. 2) cylinder params use CapFill instead of bool capped (or compat shim with deprecation path). 3) Tests cover all cap modes + determinism.

