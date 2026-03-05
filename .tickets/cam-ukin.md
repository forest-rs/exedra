---
id: cam-ukin
status: closed
deps: []
links: []
created: 2026-03-05T09:49:04Z
type: feature
priority: P1
assignee: Bruce Mitchener
tags: [v0.1, api]
---
# MeshEdit support for edge/vertex operator steps

Extend MeshEdit fluent chain with edge and vertex domain operators now that Selection is domain-generic.

## Design

Add MeshEdit step variants and plan variants for mark.edge.seam, mark.edge.sharp, edit.delete.edges, edit.delete.vertices. Enforce domain preconditions (edges/vertices) with structured OpError at plan time, and propagate selection outputs deterministically.

## Acceptance Criteria

- MeshEdit methods for seam/sharp/delete_edges/delete_vertices exist
- compile/preview/apply support new step plans
- domain mismatch returns PreconditionFailed with clear diagnostic
- tests cover success + mismatch

