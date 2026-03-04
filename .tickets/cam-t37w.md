---
id: cam-t37w
status: closed
deps: [exe-7r9n]
links: []
created: 2026-03-04T17:34:31Z
type: feature
priority: P1
assignee: Bruce Mitchener
tags: [v0.1]
---
# Delete edges operator

Add a typed cambium operator wrapper for Exedra Txn::delete_edges with deterministic canonicalization, structured diagnostics, op stats, and typed output for chaining.

## Design

Expose DeleteEdges operator (name: edit.delete.edges) with params containing canonical edge selection and DeletePolicy. Validate/canonicalize edge selection, reject stale/non-canonical input, call txn.delete_edges, map kernel errors to OpError diagnostics, and return typed output (deleted edges/faces/vertices counts and canonical selection used). No artifact duplication for authoritative output.

## Acceptance Criteria

- DeleteEdges operator implemented and exported; - Typed output includes useful deletion summary; - Canonicalization + stale/non-canonical error mapping covered by tests; - Success tests cover interior and boundary edge deletion paths; - run_commit/run_preview behavior documented via rustdoc example

