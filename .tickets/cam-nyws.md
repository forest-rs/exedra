---
id: cam-nyws
status: open
deps: [cam-inf3]
links: []
created: 2026-03-04T04:27:35Z
type: task
priority: P2
assignee: Bruce Mitchener
tags: [v0.1, docs]
---
# Rustdoc operator catalog and authoring map

Add a rustdoc-visible operator catalog that helps users discover available operations, parameters, outputs, and expected reporting behavior.

## Design

Extend the doc-only manual with a catalog section/table listing each operator, stable name(), param type, commit/preview behavior, primary stats counters, timing bucket expectations, and common diagnostics. Cross-link to operator types and param structs. Keep this as curated documentation (not generated), updated as operators are added.

## Acceptance Criteria

- Manual contains an operator catalog section with current 0.1 operators
- Each catalog entry links to operator + params rustdoc
- Catalog includes reporting expectations (timings/stats/artifacts)
- Crate root links to catalog and authoring guide
