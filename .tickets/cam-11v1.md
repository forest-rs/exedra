---
id: cam-11v1
status: closed
deps: []
links: []
created: 2026-03-05T09:52:35Z
type: feature
priority: P1
assignee: Bruce Mitchener
tags: [v0.1, api]
---
# MeshEdit chainable selection query steps in plan

Make selection queries explicit fluent steps so MeshEditPlan captures selection transitions deterministically.

## Design

Add MeshEdit step variants and plan payload for region select, region flood, and boundary edge-loop queries. Compile resolves queries against working mesh and stores resulting canonical Selection in plan; preview/apply replay selection transitions from plan.

## Acceptance Criteria

- MeshEdit has chainable selection query methods without requiring &Mesh at call site
- MeshEditPlan encodes query-derived selection transitions
- preview/apply replay query transitions deterministically
- tests cover region/flood/boundary query step chaining

