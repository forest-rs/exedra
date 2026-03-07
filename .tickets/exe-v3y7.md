---
id: exe-v3y7
status: closed
deps: [exe-clpr]
links: []
created: 2026-03-07T15:14:31Z
type: task
priority: 2
assignee: Bruce Mitchener
parent: exe-pdum
tags: [v0.1, docs, api]
---
# Tighten Exedra docs for the strong mutation fence

Tighten ADR/manual docs after the strong fence lands so the public model is unambiguous.

## Design

Update ADR-0004 and Exedra manual docs to state that MeshBuilder owns construction, exedra::op owns mutation, and EditSession is transaction hosting/bookkeeping only. Remove transitional wording.

## Acceptance Criteria

1) ADR-0004 reflects the strong fence. 2) Manual/crate docs no longer mention transitional mutation wrappers. 3) Examples use exedra::op for mutation.
