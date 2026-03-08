---
id: ep-dnwg
status: closed
deps: []
links: []
created: 2026-03-08T12:38:26Z
type: task
priority: 2
assignee: Bruce Mitchener
---
# Primitive feature edges are sharp by default

Author semantic edge sharpness for primitives so default shading reflects modeling intent instead of relying on auto_sharp heuristics.

## Design

Set sharpness explicitly in primitive constructors where feature boundaries are semantically obvious: box outer edges, cylinder cap rims when capped, and cone bottom rim when capped. Keep side seams smooth. Document the contract in crate docs/ADR.

## Acceptance Criteria

Box extracts with hard outer edges by default; capped cylinder extracts with hard cap rims and smooth side seams; capped cone extracts with hard bottom rim and smooth side faces; tests cover default extracted normals/sharpness semantics; docs mention the default sharp-edge policy.

