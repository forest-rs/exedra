---
id: exe-m5xf
status: closed
deps: [exe-pv1b]
links: []
created: 2026-03-07T14:27:06Z
type: task
priority: 2
assignee: Bruce Mitchener
parent: exe-jgq6
tags: [v0.1, docs, api]
---
# Adopt exedra::op catalog in Exedra docs and Cambium-facing examples

Update docs and examples so kernel callers discover exedra::op first instead of learning topology edits solely from EditSession methods.

## Design

Refresh exedra crate docs/manual/transaction examples to show EditSession as transaction host plus exedra::op::* as the kernel operation catalog. Keep Cambium architecture docs aligned with the boundary but do not force immediate internal adoption in every operator.

## Acceptance Criteria

1) Exedra rustdoc/manual references exedra::op as the kernel op catalog. 2) EditSession docs are reframed as transaction host/bookkeeping context. 3) At least one example shows applying a kernel op through EditSession. 4) Docs remain consistent with the ADR fence.

