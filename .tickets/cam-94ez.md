---
id: cam-94ez
title: Bounds inspect operator
status: closed
deps: [cam-tdg4]
links: []
created: 2026-03-04T16:09:59Z
type: feature
priority: P2
assignee: Bruce Mitchener
tags: [v0.1]
---
# Bounds inspect operator

Add an inspect.bounds operator that computes axis-aligned bounds for whole mesh or selected faces and returns typed bounds output for chaining.

## Design

Operator name: inspect.bounds. Input supports whole mesh or canonical face selection. Output should be typed (authoritative) and include min/max/centroid/diagonal. Keep report/artifact channels for diagnostics/debug only. Deterministic iteration/order.

## Acceptance Criteria

- inspect.bounds operator implemented
- Typed output includes min/max/centroid/diagonal
- Supports whole mesh and face selection
- Deterministic tests for empty/single-face/multi-face cases
- Rustdoc includes usage example
