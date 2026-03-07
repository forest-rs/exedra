---
id: cam-an35
status: closed
deps: []
links: []
created: 2026-03-07T18:57:02Z
type: epic
priority: P1
assignee: Bruce Mitchener
---
# Cambium patch-region modeling substrate

Add internal Cambium region/loop utilities for patch-style face editing so modeling operators share deterministic boundary extraction, loop handling, duplication, and reconnection helpers instead of duplicating mesh surgery.

## Design

Internal-only Cambium substrate layered above exedra::op. Focus on selected face regions, deterministic boundary loop discovery, loop ordering/orientation helpers, region duplication/topology helpers, and migration of face-edit operators to the shared substrate. Avoid introducing a public Patch object or a second plan system.

## Acceptance Criteria

- Epic ticket links the first substrate tickets and migration tickets.\n- Cambium gains shared internal modules for face-region and boundary-loop work.\n- Existing face-edit operators are migrated to the substrate without changing public operator IDs or outputs.\n- Validation/tests/docs reflect the new internal architecture.

