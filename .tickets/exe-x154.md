---
id: exe-x154
status: closed
deps: [cam-m068]
links: []
created: 2026-03-15T18:06:14Z
type: feature
priority: 2
assignee: Bruce Mitchener
parent: cam-t6z7
tags: [mesh, queries]
---
# Mesh query/helpers for Cambium simplification

Add Exedra query helpers and small edit-adjacent utilities that let Cambium mesh operators stop reimplementing boundary, region, and patch logic.

## Design

Focus on load-bearing mesh-native helpers: boundary loop extraction, connected face patch extraction, region-boundary edge queries, and deterministic loop/ring walkers. Keep them domain-specific to Exedra rather than burying them in Cambium patch helpers.

## Acceptance Criteria

1. Exedra exposes deterministic helpers for boundary loops and connected patches. 2. Region-boundary extraction is available to higher layers. 3. Cambium can migrate at least one operator/query path to the new helpers. 4. Tests cover deterministic ordering and stale-id behavior where relevant.


## Notes

**2026-03-15T23:54:26Z**

Moved more load-bearing mesh query logic into Exedra: Mesh::boundary_loop, Mesh::boundary_loops, and Mesh::selected_face_boundary_loops now own deterministic boundary traversal. Cambium region selection, patch loop extraction, and face-edit flows consume those helpers instead of hand-rolling traversal, with regression tests covering deterministic ordering and stale-id behavior.
