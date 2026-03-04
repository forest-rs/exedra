---
id: exe-08f4
title: Txn::add_face preflight/stitch performance pass
status: open
deps: []
links: []
created: 2026-03-04T13:49:23Z
type: task
priority: P2
assignee: Bruce Mitchener
tags: [v0.1]
---
# Txn::add_face preflight/stitch performance pass

Optimize add_face preflight and boundary stitching for repeated face creation workloads (extrude/inset and future patch operators).

## Design

Current add_face uses O(degree * total_half_edges) global scans (has_undirected_edge/find_boundary_half_edge) and always calls global stitch_outside_loops. Replace with localized vertex-fan/outgoing lookups and scoped boundary restitching on affected components. Keep deterministic behavior and manifold checks intact.

## Acceptance Criteria

- Replace global edge scans with localized lookups
- Avoid whole-mesh OUTSIDE restitch when only local boundary changed
- Preserve add_face error semantics and deterministic behavior
- Bench test shows reduced cost on repeated face creation
