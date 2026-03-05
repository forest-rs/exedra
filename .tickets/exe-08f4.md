---
id: exe-08f4
title: Txn::add_face preflight/stitch performance pass
status: closed
deps: [exe-23ot]
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
- Bench test or focused perf test shows reduced cost on repeated face creation (wind tunnel regression scenario may land separately)

## Notes

**2026-03-05T04:14:05Z**

Replaced add_face whole-mesh OUTSIDE restitch with vertex-scoped restitch (stitch_outside_loops_for_vertices) keyed by loop vertices. Preflight now uses maintained indexed lookups from exe-23ot for boundary reuse/non-manifold checks. Added local consistency checks via existing adjacency cross-check tests. Validation: cargo clippy --workspace --all-targets --all-features -- -D warnings; cargo test --workspace --all-features.
