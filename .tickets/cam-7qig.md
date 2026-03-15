---
id: cam-7qig
status: closed
deps: [cam-m068]
links: []
created: 2026-03-15T18:06:03Z
type: feature
priority: 2
assignee: Bruce Mitchener
parent: cam-t6z7
tags: [architecture, operators]
---
# Domain-aware operator taxonomy and orchestration model

Extend Cambium from a mesh-only operator catalog to an orchestrator over multiple canonical geometry domains without inventing a fake universal geometry abstraction.

## Design

Add explicit operator domain metadata and a taxonomy that can represent mesh, analytic, implicit, points, and convert families. Keep domain-native semantics primary; conversions are explicit steps with declared data loss. Preserve current mesh operator stability while opening room for new heads.

## Acceptance Criteria

1. Cambium defines operator domains explicitly. 2. Taxonomy/docs cover mesh, analytic, implicit, points, and convert families. 3. Current mesh operators remain supported. 4. Conversion steps are modeled as first-class operations.


## Notes

**2026-03-15T23:54:26Z**

Added explicit OperatorDomain metadata to Cambium operators via EditOperator::domain(), covering mesh/analytic/implicit/points/convert families with a non-breaking Mesh default and operator-level tests. This gives the runtime an explicit domain taxonomy without inventing a fake universal geometry API.
