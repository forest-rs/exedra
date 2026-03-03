---
id: cam-6vcu
title: PolicySet and sub-policies
status: closed
deps: []
links: []
created: 2026-03-03T05:57:06Z
type: feature
priority: P1
assignee: Bruce Mitchener
tags: [v0.1, foundation]
---
# PolicySet and sub-policies

Implement the Cambium policy system. PolicySet is a single object carrying all higher-level policy knobs. Passed into OpContext.

## Design

PolicySet { quality, uv, boolean, propagate, limits, validate }

QualityPolicy { mode: QualityMode, budget: Option<WorkBudget> }
QualityMode: Preview, Commit
WorkBudget { max_faces, max_corners, max_millis (advisory) }

LimitsPolicy { max_diagnostics, max_artifact_items, max_artifact_bytes }
ValidatePolicy { validate_on_preview, validate_on_commit, fail_on_error }
UvPolicy { default_scale, default_offset, allow_overwrite_existing }
BooleanPolicy { preview_params, commit_params } (uses exedra::BooleanParams)

propagate: exedra::PropagatePolicy (Cambium overrides Exedra defaults per-tool)

Policies are plain structs with Copy where practical. Identical policies produce identical results.

Module layout: policy.rs

## Acceptance Criteria

- PolicySet struct with all sub-policies
- Default values documented and sensible
- Copy/Clone where practical
- Used by OpContext
- Unit tests for default construction


## Notes

**2026-03-03T06:21:10Z**

Design brief: crates/cambium/docs/briefs/07_budget_and_cancellation_semantics.md

**2026-03-03T06:37:44Z**

Worked example: docs/worked_example_basilica.md — step 10 (ruin.damage.chip_edges) is budgetable in preview mode via max_faces/max_corners.
