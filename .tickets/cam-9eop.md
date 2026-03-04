---
id: cam-9eop
status: open
deps: []
links: []
created: 2026-03-04T04:31:35Z
type: feature
priority: 2
assignee: Bruce Mitchener
tags: [v0.5, api, docs]
---
# Profile section model and selection contract

Define profile-oriented input model and selection contract for profile-based modeling operators (loft/sweep) in Cambium.

## Design

Add a profile representation contract for ordered cross-sections (face-loop or edge-loop derived) with deterministic ordering and correspondence requirements. Define validation/preconditions, canonicalization behavior, and diagnostics for mismatched profile counts/topology. Document how profile selections map from existing FaceSet/EdgeSet and region tags.

## Acceptance Criteria

- Profile model contract documented in ticket notes and reflected in API shape proposal
- Deterministic ordering/correspondence rules defined
- Precondition/diagnostic taxonomy for invalid profile inputs defined
- Linked as dependency for loft/sweep operator tickets

