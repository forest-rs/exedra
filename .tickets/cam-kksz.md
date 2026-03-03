---
id: cam-kksz
status: open
deps: [exe-qa74]
links: []
created: 2026-03-03T06:01:15Z
type: feature
priority: 2
assignee: Bruce Mitchener
tags: [v0.9]
---
# Boolean orchestration (preview/commit)

Implement Cambium-level boolean orchestration: preview vs commit staging, region restriction (AABB overlap), result tagging, cleanup. Wraps Exedra boolean pipeline with workflow-level concerns.

## Design

Preprocessing:
- Region restriction: operate only where AABBs overlap
- Preview: fast/approximate boolean; Commit: robust boolean

Postprocessing:
- Tag result patches (inside/outside, source id)
- Cleanup: remove tiny components (policy-driven)

Must keep failure artifacts visible and debuggable.

## Acceptance Criteria

- Preview boolean works with reduced quality/scope
- Commit boolean uses full pipeline
- Result patches tagged with source info
- Failure artifacts surfaced cleanly
- Unit tests for preview vs commit differences

