---
id: cam-4x8o
status: open
deps: [exe-dc9l]
links: []
created: 2026-03-03T05:56:07Z
type: feature
priority: 0
assignee: Bruce Mitchener
tags: [v0.1, foundation]
---
# DiagnosticsSink and Diagnostic

Implement structured diagnostics infrastructure. Diagnostics are bounded, deterministically ordered, and severity-classified. Programmatic identity via enums, not strings.

## Design

DiagnosticsSink: bounded storage with deterministic ordering

Diagnostic { level: DiagLevel, code: DiagCode, message: String, spans: Vec<DiagSpan> }

DiagLevel: Note, Warn, Error
DiagCode: PreconditionFailed, NonManifoldInput, MissingRequiredAttribute, NumericToleranceIssue, InternalInvariantViolation, Cancelled, BudgetExceeded

DiagSpan: Vertex(VertexId), HalfEdge(HalfEdgeId), Face(FaceId), Corner(CornerId)

Overflow handling (deterministic, severity-aware):
- Retain all Error first, then Warn, then Note
- Within each level, retain earliest insertion order
- Truncate at max_diagnostics

v0.1 does NOT attempt automatic deduplication.

Module layout: diag.rs

## Acceptance Criteria

- DiagnosticsSink with bounded capacity
- Severity-aware overflow handling
- Deterministic ordering
- DiagCode enum covers documented codes
- DiagSpan references Exedra element IDs
- Unit tests for overflow with mixed severity levels

