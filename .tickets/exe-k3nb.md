---
id: exe-k3nb
status: open
deps: [exe-17rj, exe-cbv1]
links: [cam-gihj]
created: 2026-03-03T05:29:34Z
type: feature
priority: 1
assignee: Bruce Mitchener
tags: [v0.1]
---
# Edge sharpness attribute

Implement the edge-domain sharpness flag or weight. Edges are represented canonically as (h, twin(h)) — no separate EdgeId in v0.1. Sharpness controls derived normal computation (v0.5) and is used by Cambium for seam marking.

## Design

Edge sharpness:
- Domain: Edge (canonical pair of half-edges)
- Type: bool (flag) initially; may evolve to f32 weight for crease/bevel later
- Storage: per canonical edge; accessor takes a HalfEdgeId and canonicalizes via min(h, twin(h)) or similar deterministic rule
- No separate EdgeId type in v0.1 (spec locks this)

Canonical edge identity:
- Edge is identified by the pair {h, twin(h)}
- Need a deterministic canonical form for storage (e.g. store on the half-edge with smaller slot index)
- This is important for determinism: same edge accessed via either half-edge must return the same sharpness

Used by:
- v0.5 derived normal computation (sharp edges create hard shading boundaries)
- Cambium seam marking operators
- Subdivision (crease weights)

## Acceptance Criteria

- Edge sharpness attribute exists
- Accessible from either half-edge of a pair
- Canonical edge identity is deterministic
- Default is smooth (not sharp)
- Unit tests for set/get from both half-edges of a pair


## Notes

**2026-03-03T06:21:10Z**

Design brief: crates/exedra/docs/briefs/10_attribute_storage_hybrid_dense_sparse.md
