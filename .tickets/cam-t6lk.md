---
id: cam-t6lk
status: closed
deps: [cam-mrwk]
links: []
created: 2026-03-05T02:03:21Z
type: feature
priority: P2
assignee: Bruce Mitchener
tags: [v0.5, architecture]
---
# Selection bridge type for generic composition

Add a Selection bridge type in Cambium so composition layers can handle face/edge/vertex selections generically while preserving typed sets as canonical APIs.

## Design

Introduce Selection enum (Faces/Edges/Vertices) plus explicit conversion/query helpers. Keep FaceSet/EdgeSet/VertexSet as primary operator param/output types. Define deterministic ordering and domain-mismatch diagnostics.
This ticket is intentionally independent of EditPlan internals; it defines a composition/data-model bridge usable with current runner APIs and future compile/apply flows.

## Acceptance Criteria

- Selection bridge type added with conversion helpers; - canonical typed sets remain supported and preferred; - tests cover deterministic conversions and mismatch handling
