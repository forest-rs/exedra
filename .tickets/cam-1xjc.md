---
id: cam-1xjc
title: Face-edit multi-face adjacency support
status: closed
deps: []
links: []
created: 2026-03-04T12:55:23Z
type: feature
priority: P2
assignee: Bruce Mitchener
tags: [v0.1]
---
# Face-edit multi-face adjacency support

Support adjacent selected faces for extrude/inset by handling shared-border collapse and manifold-safe patch behavior.

## Design

Current preflight rejects selections where faces share an edge. Add patch-aware processing that computes outer boundary loops of the selected region, avoids duplicate internal walls, and preserves manifold topology. Output should be deterministic and aligned with face-edit semantics contracts used by extrude/inset mode work.

## Acceptance Criteria

- Adjacent face selections are supported for extrude/inset
- No duplicate walls across internal shared edges
- Deterministic boundary-loop construction for selected patches
- Tests for adjacent/non-adjacent/mixed selections
- Behavior is documented as compatible with `cam-xnoi` / `cam-7u7l` mode semantics

## Notes

**2026-03-05T08:35:38Z**

Implemented adjacency-aware extrude/inset patch behavior: internal shared edges collapsed (no duplicate internal walls/frames), outer boundary-only wall/frame generation, shared generated vertices across adjacent faces, deterministic edge counting, and new adjacent-selection regression tests.
