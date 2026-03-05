---
id: cam-4x8a
status: open
deps: []
links: []
created: 2026-03-05T09:59:41Z
type: feature
priority: P2
assignee: Bruce Mitchener
tags: [v0.1, operator, inspect]
---
# Selection summary inspect operator

Add an inspect operator that reports selection domain/counts and optional lightweight topology stats for diagnostics and scripting.

## Design

Operator accepts a Selection plus optional detail level, emits deterministic counters/artifacts summarizing counts by domain and simple validity flags without mutating mesh.

## Acceptance Criteria

- inspect.select.summary operator exists with stable name()
- accepts Selection bridge and reports deterministic summary output
- no mesh mutation side effects
- tests cover face/edge/vertex selections and empty selection

