---
id: cam-rd95
status: closed
deps: []
links: []
created: 2026-03-08T18:32:32Z
type: epic
priority: 2
assignee: Bruce Mitchener
---
# Boundary loop bridge substrate and operator

Add a Cambium bridge operator built on the patch/loop substrate. First slice is deterministic bridging between two boundary loops with equal vertex counts and explicit loop selection. This should set up loft without overcommitting to resampling or twist heuristics yet.

## Design

Own this in Cambium. Use existing boundary-loop extraction and Exedra kernel ops. Keep Exedra unchanged unless a clear kernel gap appears.

## Acceptance Criteria

1. Ticket tree exists for the first bridge slice and follow-on loft dependencies. 2. The first implementation ticket is scoped to equal-count loop bridging with deterministic pairing. 3. Notes explicitly defer resampling/twist-generalization.
