---
id: exe-gtg2
title: Vertex sharpness attribute for subdivision corner classification
status: open
deps: [exe-0wv0]
links: []
created: 2026-03-04T05:53:49Z
type: feature
priority: 2
assignee: Bruce Mitchener
tags: [v0.5]
---
# Vertex sharpness attribute for subdivision corner classification

Add an optional f32 vertex-domain sharpness attribute. Allows explicit corner classification override for subdivision: a vertex with high sharpness is pinned as a corner regardless of incident edge sharpness count. Without this, corner classification is derived purely from edge sharpness (2+ sharp edges = corner), which doesn't cover authored corner pins or smooth-despite-sharp overrides.

## Design

Built-in key: attr::VERTEX_SHARPNESS (Domain::Vertex, f32). Storage: sparse (most vertices use derived classification). Default: absent = derive from edge sharpness. Value semantics: 0.0 = smooth override, f32::INFINITY = corner pin, intermediate = semi-sharp vertex (decays per subdivision level). Mesh accessors: vertex_sharpness(VertexId) -> Option<f32>, set_vertex_sharpness(VertexId, f32). Txn wrappers with vertex dirty marking. Subdivision classification rule: if vertex sharpness is present, use it; otherwise count incident sharp edges (0=smooth, 1=dart, 2=crease, 3+=corner).

## Acceptance Criteria

1) VERTEX_SHARPNESS built-in key exists. 2) Mesh::vertex_sharpness/set_vertex_sharpness accessors work. 3) Txn wrappers mark vertex dirty. 4) Sparse storage, absent = derive. 5) Round-trip tests. 6) cargo clippy/test pass.
