---
id: cam-ibof
title: OperatorRunner (run_commit and run_preview)
status: open
deps: [cam-wu7q, cam-6vcu, exe-dey4, exe-cbv1]
links: [cam-ezlm, cam-vt4j]
created: 2026-03-03T05:58:07Z
type: feature
priority: P1
assignee: Bruce Mitchener
tags: [v0.1, foundation]
---
# OperatorRunner (run_commit and run_preview)

Implement OperatorRunner — the orchestrator that standardizes commit vs preview execution, transaction management, and result shaping.

## Design

OperatorRunner { persistent caches, reusable scratch }

run_commit:
1. Clear scratch
2. Create transaction: mesh.begin()
3. Call op.apply(&mut txn, params, ctx)
4. Commit transaction -> ChangeSet
5. Optional validation (if policy.validate.validate_on_commit)
6. Return OpResult { change_set, report }

run_preview:
1. Clone input mesh (v0.1 strategy; optimize later)
2. Run commit path on the clone
3. Return (preview_mesh, report)

Preview contract (locked):
- run_preview always returns owned preview mesh + OpReport
- Never mutates the committed base mesh
- Future optimization must preserve deterministic results

Runner-level timing buckets: op.apply, txn.commit, validate

Requires Mesh: Clone from Exedra (exe-cbv1).

Module layout: runner.rs

## Acceptance Criteria

- OperatorRunner exists with run_commit and run_preview
- run_commit: transaction lifecycle correct, returns OpResult
- run_preview: clones mesh, returns independent preview
- Scratch cleared at start of every run
- Optional validation respects policy
- Runner-level timing buckets recorded
- Unit tests for both paths with a simple test operator


## Notes

**2026-03-03T06:21:10Z**

Design brief: crates/cambium/docs/briefs/01_preview_commit_first_class.md, crates/cambium/docs/briefs/02_edit_operators_primary.md

**2026-03-03T06:27:28Z**

Design brief: crates/exedra/docs/briefs/14_exedra_cambium_boundary_contract.md

**2026-03-03T06:36:03Z**

Design brief: crates/exedra/docs/briefs/16_scratch_buffer_protocol.md

**2026-03-03T06:37:44Z**

Worked example: docs/worked_example_basilica.md — preview/commit is used interactively around ruin steps 8-10. Preview clones the mesh; commit produces ChangeSet for incremental extraction.
