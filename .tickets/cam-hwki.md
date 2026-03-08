---
id: cam-hwki
status: closed
deps: [cam-rd95]
links: []
created: 2026-03-08T18:32:32Z
type: task
priority: 2
assignee: Bruce Mitchener
---
# Bridge two equal-length boundary loops

Add `edit.bridge.loops` in Cambium. Inputs are two canonical boundary edge loops of equal length. Build a quad strip between them with deterministic start alignment and loop orientation handling. Use existing boundary-loop query output and patch helpers where possible.

## Design

Use Cambium loop helpers, not Exedra patch objects. Require equal edge counts in v0.1. Deterministic alignment rule: anchor on minimum loop vertex pair under lexicographic position/ID rule, with explicit orientation choice that preserves outward winding where possible.

## Acceptance Criteria

1. Operator bridges two equal-length boundary loops into a valid quad strip. 2. Compile rejects stale, non-boundary, overlapping, or unequal-length loops. 3. Typed output includes created bridge faces. 4. Fluent API support exists. 5. Tests cover ring-to-ring and hole-to-hole style cases.
