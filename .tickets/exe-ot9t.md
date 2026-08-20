---
id: exe-ot9t
status: open
deps: []
links: []
type: task
priority: 1
---
# Canonicalize coincident vertices in Boolean seams

## Problem

Stitched Boolean output can contain distinct vertices at identical narrowed
`f32` positions along seam rings, producing zero-length rim edges.

## Fence

Exedra owns canonical mesh identity and attribute-preserving seam cleanup; it
does not own higher-level fillet policy.

## Acceptance

- Seam rings contain no exact-position duplicate vertices.
- Survivor and attribute/provenance merge rules are deterministic and
  documented.
- Deep validation passes and cleanup or rounding no longer needs alias
  workarounds.
- The Boolean oracle remains deterministic for existing valid outputs.
