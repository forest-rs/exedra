---
id: cam-1rkd
status: closed
deps: [exe-7r9n]
links: []
created: 2026-03-04T17:34:35Z
type: feature
priority: P1
assignee: Bruce Mitchener
tags: [v0.1]
---
# Delete faces operator

Add a typed cambium operator wrapper for Exedra Txn::delete_faces with deterministic canonicalization, structured diagnostics, op stats, and typed output for chaining.

## Design

Expose DeleteFaces operator (name: edit.delete.faces) with params containing canonical face selection and DeletePolicy. Validate/canonicalize face selection, reject OUTSIDE/stale/non-canonical input, call txn.delete_faces, map kernel errors to OpError diagnostics, and return typed output (deleted faces/edges/vertices counts and canonical selection used). No artifact duplication for authoritative output.

## Acceptance Criteria

- DeleteFaces operator implemented and exported; - Typed output includes useful deletion summary; - Canonicalization + stale/non-canonical/OUTSIDE error mapping covered by tests; - Success tests cover deleting one face and multiple faces; - run_commit/run_preview behavior documented via rustdoc example

