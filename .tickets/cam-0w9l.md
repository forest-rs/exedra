---
id: cam-0w9l
status: open
deps: [cam-4x8o, cam-f0wg]
links: []
created: 2026-03-03T05:56:47Z
type: feature
priority: 0
assignee: Bruce Mitchener
tags: [v0.1, foundation]
---
# OpError and OpErrorKind

Implement structured operator error types. Errors are classifiable by kind with attached diagnostics and artifacts for context.

## Design

OpError { kind: OpErrorKind, diagnostics: Vec<Diagnostic>, artifacts: Artifacts }

OpErrorKind:
- PreconditionFailed (empty selection, missing layer)
- InvalidMesh (mesh invalid for this operator)
- MissingAttribute (required layer missing or wrong domain)
- NumericFailure (tolerance, degeneracy, NaN/Inf)
- BudgetExceeded (preview budget hit; retry with higher budget or commit)
- Cancelled (user/orchestration cancellation)
- InternalInvariantViolation (bug)

OpError is distinct from Exedra errors. Cambium wraps Exedra errors by mapping into OpErrorKind and attaching original artifacts.

Module layout: error.rs

## Acceptance Criteria

- OpError struct with kind, diagnostics, artifacts
- OpErrorKind enum covers all documented variants
- Wrapping helpers for Exedra errors
- Display/Debug implementations

