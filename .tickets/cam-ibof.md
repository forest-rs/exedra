---
id: cam-ibof
status: open
deps: [cam-wu7q, cam-6vcu, exe-dey4, exe-cbv1]
links: []
created: 2026-03-03T05:58:07Z
type: feature
priority: 0
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

