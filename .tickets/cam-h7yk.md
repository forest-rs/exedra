---
id: cam-h7yk
title: Face-edit attribute propagation parity
status: open
deps: [exe-cz8g]
links: []
created: 2026-03-04T12:54:57Z
type: task
priority: P2
assignee: Bruce Mitchener
tags: [v0.1]
---
# Face-edit attribute propagation parity

Bring extrude/inset attribute propagation in line with propagation expectations (beyond face.region).

## Design

Current face-edit ops set face.region only. Add explicit policy and behavior for edge seam/sharpness and corner UV handling on generated topology, with conservative defaults when source data is missing.

## Acceptance Criteria

- Documented propagation behavior for generated faces/edges/corners
- Tests for seam/sharpness/UV propagation or explicit clearing defaults
- No topology regressions (validate_fast/deep)
