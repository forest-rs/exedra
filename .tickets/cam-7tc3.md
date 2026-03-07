---
id: cam-7tc3
status: closed
deps: [cam-an35, cam-2fau]
links: []
created: 2026-03-07T18:57:21Z
type: task
priority: P1
assignee: Bruce Mitchener
---
# Deterministic boundary loop extraction for face regions

Add internal Cambium utilities that turn face-region boundary edges into deterministic boundary loops with stable ordering and orientation helpers for modeling operators.

## Design

Build on region boundary classification to walk OUTSIDE-adjacent boundary edges into one or more deterministic loops. Define stable loop ordering and orientation utilities suitable for wall/frame generation. Keep behavior internal and deterministic; do not introduce public patch objects.

## Acceptance Criteria

- Internal loop helper extracts one or more boundary loops from region boundary edges.\n- Loop traversal is deterministic across identical mesh state and input selection.\n- Stable loop ordering is documented in code comments/tests.\n- Tests cover single loop, multiple disjoint loops, and adjacent multi-face regions.\n- No public API changes.

