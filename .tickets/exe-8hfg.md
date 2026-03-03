---
id: exe-8hfg
title: Vertex positions (required dense layer)
status: closed
deps: [exe-17rj, exe-cbv1]
links: []
created: 2026-03-03T05:27:51Z
type: feature
priority: P1
assignee: Bruce Mitchener
tags: [v0.1, foundation]
---
# Vertex positions (required dense layer)

Implement the required vertex-domain position attribute layer. This is the one attribute layer that every mesh must have. Stored as dense Vec<[f32; 3]> indexed by VertexId.

## Design

Positions are the canonical required attribute:
- Domain: Vertex
- Type: [f32; 3] (plain array per ADR-0001; no glam dependency in API)
- Storage: dense Vec, indexed by vertex slot index
- Always present; every valid vertex has a position
- Built-in key: exedra::attr::VERTEX_POSITION

This is the first concrete attribute layer and validates the attribute system design from exe-17rj.

## Acceptance Criteria

- Position layer exists as a dense vertex-domain attribute
- Type is [f32; 3]
- Every vertex has a position (no Option)
- Accessible via built-in key VERTEX_POSITION
- Integrates with the attribute system from exe-17rj
- Unit tests for position get/set


## Notes

**2026-03-03T06:17:41Z**

Design brief: crates/exedra/docs/briefs/01_corner_attributes_and_extraction.md

**2026-03-03T10:25:45Z**

Implementation summary (2026-03-03): wired VERTEX_POSITION as required dense vertex attribute in practice by adding mesh-level vertex APIs (add_vertex, vertex_position, set_vertex_position) that keep position storage synchronized with vertex capacity and write positions eagerly on vertex creation. Added regression test covering required position get/set semantics; existing attributes tests cover dense layer behavior and built-in key access. Validation run: typos, cargo fmt --all, taplo fmt, cargo clippy --workspace --all-targets --all-features -- -D warnings, cargo test --workspace --all-features, cargo doc --no-deps.
