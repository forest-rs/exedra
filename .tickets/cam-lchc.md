---
id: cam-lchc
status: closed
deps: []
links: []
created: 2026-03-05T09:44:35Z
type: feature
priority: P1
assignee: Bruce Mitchener
tags: [v0.1, api]
---
# MeshEdit domain-generic fluent selection state

Refactor MeshEdit to carry Selection (faces/edges/vertices) through compile/preview/apply so fluent APIs can compose across domains.

## Design

Store Selection in MeshEdit/MeshEditPlan, keep current face edit steps, and enforce per-step domain requirements at plan/apply time with structured OpError diagnostics. Preserve deterministic plan fingerprinting and update docs/tests.

## Acceptance Criteria

- MeshEdit and MeshEditPlan use Selection instead of face-only state
- Face-only steps fail with clear PreconditionFailed when selection is not faces
- Fingerprinting includes selection domain+content deterministically
- Existing and new tests pass

