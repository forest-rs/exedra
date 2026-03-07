---
id: exe-pv1b
status: closed
deps: [exe-cf8l]
links: []
created: 2026-03-07T14:27:06Z
type: feature
priority: 1
assignee: Bruce Mitchener
parent: exe-jgq6
tags: [v0.1, architecture, api]
---
# Migrate remaining topology edits into exedra::op

Finish the initial topology-op migration so the public kernel catalog covers the core mutation set.

## Design

Add SplitFaceOp, DeleteEdgesOp, and DeleteVerticesOp, reusing EditSession-owned impl helpers for the actual mutation bodies. Reduce EditSession's public topology-edit surface to transitional wrappers only, with docs that point callers to exedra::op as the primary catalog.

## Acceptance Criteria

1) SplitFaceOp, DeleteEdgesOp, and DeleteVerticesOp exist and are exported. 2) EditSession topology methods are thin wrappers or demoted internal helpers. 3) Rustdoc/manual text points kernel callers at exedra::op. 4) Tests cover typed success/failure behavior for the new op entry points.

