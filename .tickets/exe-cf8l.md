---
id: exe-cf8l
status: closed
deps: []
links: []
created: 2026-03-07T14:27:06Z
type: feature
priority: 1
assignee: Bruce Mitchener
parent: exe-jgq6
tags: [v0.1, architecture, api]
---
# Add exedra::op module and first topology ops

Add the initial exedra::op catalog and migrate the highest-pressure topology edits behind explicit op types.

## Design

Add public exedra::op module and re-export selected op types from lib.rs. First slice should include AddFaceOp, SplitEdgeOp, and DeleteFacesOp with inherent apply(self, &mut EditSession) methods. Move existing session method bodies behind *_impl helpers and let public session wrappers forward temporarily if needed.

## Acceptance Criteria

1) exedra::op module exists and is rustdoc'd. 2) AddFaceOp, SplitEdgeOp, and DeleteFacesOp exist with typed apply methods. 3) Existing topology behavior and tests still pass. 4) Focused tests cover the new op entry points.

