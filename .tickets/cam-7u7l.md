---
id: cam-7u7l
title: Extrude mode semantics (shell vs keep-source)
status: open
deps: [cam-mn4h, cam-1xjc, exe-cz8g]
links: []
created: 2026-03-04T12:54:57Z
type: feature
priority: P1
assignee: Bruce Mitchener
tags: [v0.1]
---
# Extrude mode semantics (shell vs keep-source)

Add explicit extrude mode semantics so users can choose one-sided shell behavior or keep-source behavior when extruding faces.

## Design

Current extrude deletes source face, creates side walls + offset cap (open at source location). Add mode enum in params, e.g. RemoveSource (current), KeepSource (thickness-like prism from selected patch). Define topology outcomes, region propagation, and constraints for adjacent selections.

## Acceptance Criteria

- Extrude params include explicit mode
- Both modes documented and tested
- Deterministic behavior for canonical selections
- Diagnostics for unsupported inputs
