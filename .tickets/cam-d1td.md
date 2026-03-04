---
id: cam-d1td
status: open
deps: [cam-9eop]
links: []
created: 2026-03-04T04:31:35Z
type: feature
priority: 2
assignee: Bruce Mitchener
tags: [v0.5, modeling]
---
# Loft operator (profile-to-profile surface generation)

Implement a deterministic loft operator that bridges two or more compatible profiles into a connected surface.

## Design

Operator consumes validated profile sections and generates connecting faces with deterministic winding and region tagging. Support at least equal-segment profiles in v0.5; reject incompatible profile topology with structured diagnostics. Integrate with runner reporting: timings (select/compute/attrs), stats counters, and bounded artifacts.

## Acceptance Criteria

- Operator type + params exist with stable name()
- Deterministic loft topology for compatible profile pairs
- Structured diagnostics for incompatible profile inputs
- Report/timing/counter conventions followed
- Tests for deterministic output and validation invariants

