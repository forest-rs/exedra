---
id: cam-cuo5
status: open
deps: []
links: []
created: 2026-03-05T09:59:41Z
type: feature
priority: P1
assignee: Bruce Mitchener
tags: [v0.1, operator, select]
---
# Boundary edge loop selection operator

Add a select.* EditOperator wrapper for deterministic boundary edge-loop selection so query workflows fit compile/apply runner paths.

## Design

Wrap select_boundary_edge_loop helper in an EditOperator with params { seed_edge } and EdgeSet output, preserving deterministic selection and diagnostics.

## Acceptance Criteria

- select.edge_loop.boundary operator exists with stable name()
- compile/apply path returns canonical EdgeSet output
- diagnostics for stale/interior seed edge
- tests cover success and rejection

