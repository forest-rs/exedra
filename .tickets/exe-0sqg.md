---
id: exe-0sqg
status: open
deps: []
links: []
created: 2026-03-05T02:27:26Z
type: task
priority: P1
assignee: Bruce Mitchener
tags: [v0.1, test, architecture]
---
# Adjacency index consistency cross-check mode

Add a debug/test consistency mode that rebuilds adjacency from topology and asserts equality with maintained index state after representative kernel edits.

## Design

Implement internal checker utilities to derive adjacency from authoritative topology and compare against maintained index structures. Expose checker for tests and optional debug assertions in edit-heavy paths. Keep deterministic comparison ordering and diagnostics for first mismatch.

## Acceptance Criteria

- Adjacency checker utility exists and compares maintained index vs rebuilt view; - representative edit-kernel tests invoke checker after mutations; - mismatch diagnostics include vertex/edge context; - no impact on release-mode behavior
