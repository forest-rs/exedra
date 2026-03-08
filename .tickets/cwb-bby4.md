---
id: cwb-bby4
status: closed
deps: [cam-58z3]
links: []
created: 2026-03-08T01:27:02Z
type: task
priority: 2
assignee: Bruce Mitchener
---
# Web scenario uses dissolve edges

Update the topology scenario in the web demo to show dissolve-edge behavior instead of delete-edge hole punching.

## Design

Replace the current edge-deletion step in the topology-oriented scenario with dissolve-edge so the visual story matches the n-gon modeling semantics more clearly. Keep scenario determinism, labels, and stats/fingerprint reporting stable.

## Acceptance Criteria

1) The relevant web scenario uses edit.dissolve.edges. 2) Scenario labels/readme text describe dissolve rather than delete. 3) Bridge tests remain deterministic.

