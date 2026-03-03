---
id: exe-5nj1
title: Explicit EDGE_SEAM attribute (edge-domain bool)
status: open
deps: [exe-17rj, exe-cbv1]
links: []
created: 2026-03-03T07:07:12Z
type: Explicit EDGE_SEAM attribute (edge-domain bool)
priority: P1
assignee: Bruce Mitchener
---
# Explicit EDGE_SEAM attribute (edge-domain bool)

Define an explicit edge-domain boolean attribute EDGE_SEAM that marks edges as UV seams. This is distinct from implicit UV discontinuity detection. Operators like mark_seam set this tag; UV projection operators read it to know where to cut. A derived utility can check whether an edge is an implicit seam (different UV values on corners across the edge) but the authoritative seam tag is the explicit attribute.

## Design

EDGE_SEAM is an edge-domain bool attribute, stored sparse (default false). Cambium mark_seam operator sets it. UV operators consult it for cut placement. Separate utility: Mesh::is_uv_discontinuous(edge) checks corner UV values across the edge for implicit seam detection. The explicit tag takes precedence over implicit detection for operator logic. Storage: sparse layer since most edges are not seams.

## Acceptance Criteria

EDGE_SEAM attribute defined as a builtin edge attribute. Can be set/queried per edge. mark_seam operator (cam-10no) uses this attribute. UV operators can read seam tags. Utility for implicit UV discontinuity detection exists separately. Sparse storage, default false.

