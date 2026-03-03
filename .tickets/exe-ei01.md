---
id: exe-ei01
status: open
deps: [exe-2752]
links: []
created: 2026-03-03T07:06:41Z
type: Derive prev(h) by walking; no stored prev pointer in v0.1
priority: P1
assignee: Bruce Mitchener
---
# Untitled

Decision: do not store a prev half-edge pointer in the topology records for v0.1. Instead, provide a prev(h) accessor that derives the previous half-edge by walking the face loop (next chain) until it wraps around. This saves 4-8 bytes per half-edge and simplifies the mutation surface. If profiling shows prev traversal is a bottleneck, we can add a stored prev pointer later as an optimization.

## Design

HalfEdge record stores: next, twin, vertex, face. No prev field. Mesh::prev(h) walks: start at next(h), follow next until next(current) == h, return current. Cost is O(k) where k is face degree — acceptable for v0.1 since most faces are triangles or quads (k=3-4). This is a conscious trade-off: simpler mutation (no prev maintenance) vs slower prev lookup. Document this decision in the topology records.

## Acceptance Criteria

HalfEdge struct has no prev field. Mesh::prev(h) method exists and returns correct result. prev(h) works for triangles, quads, and ngons. Decision documented in exe-2752 or topology docs.

