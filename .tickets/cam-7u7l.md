---
id: cam-7u7l
title: Extrude mode semantics (shell vs keep-source)
status: closed
deps: [cam-mn4h, cam-1xjc, exe-cz8g, cam-xnoi]
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

Current extrude deletes source face, creates side walls + offset cap (open at source location). Implement explicit mode semantics from `cam-xnoi` with forward-compatible naming (for example shell/open-surface vs keep-source/volume-friendly behavior). Define topology outcomes for open vs closed contexts, region/attribute propagation expectations, and constraints for adjacent selections.

## Acceptance Criteria

- Extrude params include explicit mode
- Both modes documented and tested
- Deterministic behavior for canonical selections
- Diagnostics for unsupported inputs
- Naming and mode contract align with `cam-xnoi` semantics matrix
