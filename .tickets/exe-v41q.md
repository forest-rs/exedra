---
id: exe-v41q
status: open
deps: [exe-qs69]
links: []
created: 2026-03-03T05:49:05Z
type: feature
priority: 2
assignee: Bruce Mitchener
tags: [v0.9, boolean]
---
# Boolean diagnostics and failure taxonomy

Implement structured boolean error types and diagnostic artifacts. Failures must be explainable, not just error codes.

## Design

BooleanFailureKind enum:
- NonManifoldInput
- SelfIntersectionDetected
- CoplanarAmbiguity
- ToleranceExceeded
- NumericalInstability
- InternalInvariantViolation

BooleanError { kind, artifacts: BooleanArtifacts }

BooleanArtifacts:
- Intersection segments/polylines
- Suspect triangles/edges lists
- Manifold violation reports
- Tolerance decision summary
- Per-stage timing and stats

All artifact lists deterministically ordered.
Artifacts must be bounded and streamable for large meshes.

## Acceptance Criteria

- BooleanFailureKind enum covers documented failure modes
- BooleanError returns structured artifacts sufficient to diagnose
- Artifacts are bounded and deterministically ordered
- Stage timing and stats captured
- Unit tests trigger each failure kind with appropriate diagnostics


## Notes

**2026-03-03T06:21:10Z**

Design brief: crates/exedra/docs/briefs/06_staged_booleans_with_artifacts.md
