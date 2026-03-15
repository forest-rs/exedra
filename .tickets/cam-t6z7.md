---
id: cam-t6z7
status: open
deps: []
links: [exe-h2rh, exe-xgtv]
created: 2026-03-15T18:05:44Z
type: epic
priority: 2
assignee: Bruce Mitchener
tags: [architecture, v1.0]
---
# Multi-domain geometry architecture epic

Establish a Hydra-style geometry architecture: Exedra remains the polygon head, sibling crates own analytic/implicit/points domains, and Cambium orchestrates domain-native operators plus explicit conversions.

## Design

Own the cross-crate architecture in Cambium. Deliver an ADR, crate-by-crate blueprint, operator domain model, and the first execution slices: strengthen Exedra mesh primitives and prove one second canonical domain via an analytic->mesh path. Reuse existing exedra kernel tickets where they already cover needed work.

## Acceptance Criteria

1. Owning ADR defines domain boundaries and conversion policy. 2. Blueprint names crate/module layout and ticket sequence. 3. Cambium gets an explicit domain/orchestration model. 4. First mesh-head improvements are scheduled against Exedra. 5. One analytic MVP slice is defined with deterministic tessellation into Exedra mesh.

