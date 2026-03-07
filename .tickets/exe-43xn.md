---
id: exe-43xn
status: closed
deps: []
links: []
created: 2026-03-07T15:14:16Z
type: feature
priority: 1
assignee: Bruce Mitchener
parent: exe-pdum
tags: [v0.1, architecture, api]
---
# Add authored-write ops and remove remaining public mutation helpers

Add authored-write kernel ops and remove the remaining public mutation helpers from Mesh/EditSession.

## Design

Introduce `exedra::op` authored-write functions for adding vertices and setting built-in authored attributes. Use them from Cambium and Exedra tests/docs. Remove public mutation helpers on Mesh/EditSession so mutation discovery goes through `exedra::op`; keep `MeshBuilder` as the construction API.

## Acceptance Criteria

1) Authored-write `exedra::op` functions exist and are exported. 2) Public Mesh/EditSession mutation helpers are removed or made internal. 3) Cambium/Exedra compile against the new op catalog. 4) Tests cover at least one success/failure path per new op family.
