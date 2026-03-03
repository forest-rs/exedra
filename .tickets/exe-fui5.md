---
id: exe-fui5
status: open
deps: [exe-qs69]
links: []
created: 2026-03-03T05:49:44Z
type: feature
priority: 2
assignee: Bruce Mitchener
tags: [v0.9, boolean]
---
# BooleanScratch and reusable allocations

Implement BooleanScratch — reusable scratch buffers for the boolean pipeline. BVH scratch, intersection staging, hashbrown maps. All reused across stages and across boolean calls.

## Design

BooleanScratch {
  // BVH construction scratch
  // Intersection segment staging
  // hashbrown maps for connectivity
  // Classification scratch
  // Stitching scratch
}

Must be caller-supplied and reusable. No allocations in hot loops.
Clear between uses but retain capacity.

## Acceptance Criteria

- BooleanScratch type exists with appropriate buffers
- Reused across pipeline stages
- No allocations in hot loops during boolean execution
- clear() retains capacity

