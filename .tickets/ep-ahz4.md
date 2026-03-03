---
id: ep-ahz4
status: open
deps: []
links: [ep-od6p, ep-wbxp]
created: 2026-03-03T17:19:58Z
type: task
priority: 2
assignee: Bruce Mitchener
---
# Selection semantics audit for seam/rim sets

Audit and tighten the semantic contract for seam/rim selection sets emitted by cylinder and uv_sphere so downstream operators can rely on stable meaning, not just determinism.

## Design

Document exact inclusion rules and orientation expectations for edges.seam, edges.rim_top, and edges.rim_bottom. Confirm whether sets should represent chains, boundary edges, or canonical representative edges for undirected rims. Add conformance tests against known meshes and update primitive docs with examples.

## Acceptance Criteria

Selection semantics are explicitly documented; tests verify emitted sets against the documented contract for capped/uncapped cylinder and uv_sphere; no ambiguous/implicit behavior remains in code comments or public docs.

