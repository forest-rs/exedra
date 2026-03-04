---
id: cam-inpo
title: Catmull-Clark subdivision operator
status: open
deps: [exe-tezb, exe-0a9w, exe-0wv0, exe-gtg2, cam-26fl]
links: []
created: 2026-03-03T06:01:15Z
type: feature
priority: P2
assignee: Bruce Mitchener
tags: [v0.5]
---
# Catmull-Clark subdivision operator

Implement Catmull-Clark subdivision as an EditOperator. Must preserve UV seams via corner domain, respect crease weights, and produce stable deterministic output. This is the marquee v0.5 operator.

## Acceptance Criteria

- Catmull-Clark subdivision produces correct topology and positions
- UV seams preserved through corner domain
- Crease weights respected
- Deterministic output
- Golden tests
- Wind tunnel benchmark (CWT-3)

