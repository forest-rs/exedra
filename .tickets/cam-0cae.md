---
id: cam-0cae
status: open
deps: [exe-3ebg]
links: []
created: 2026-03-04T17:42:28Z
type: feature
priority: P1
assignee: Bruce Mitchener
tags: [v0.1]
---
# Delete vertices operator

Add typed Cambium operator wrapper for Exedra delete_vertices with canonicalization, diagnostics mapping, op stats, and typed output for chaining.

## Design

Expose DeleteVertices operator (name: edit.delete.vertices) with params { vertices, policy? }. Canonicalize selection, reject stale/non-isolated input via mapped OpError diagnostics, call txn.delete_vertices, and return typed authoritative output (canonical deleted vertex set). Keep artifacts for diagnostics only.

## Acceptance Criteria

- DeleteVertices operator implemented and exported; - typed output includes canonical deleted vertex set; - stale/non-isolated/non-canonical mapping covered by tests; - run_commit/run_preview behavior documented with rustdoc example

