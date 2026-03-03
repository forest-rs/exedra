---
id: exe-tezb
status: open
deps: [exe-dey4, exe-ognv]
links: []
created: 2026-03-03T05:38:32Z
type: feature
priority: 1
assignee: Bruce Mitchener
tags: [v0.5]
---
# split_edge with attribute propagation

Implement split_edge: insert a new vertex on an existing edge, replacing it with two edges. Must propagate all attribute domains according to PropagatePolicy with documented defaults.

## Design

Topology: old edge (h, t) becomes (h0, t0) and (h1, t1) with new vertex v_new.

Attribute propagation defaults:
- Vertex position: midpoint(v0, v1)
- Edge sharpness: copy to both children
- Corner UVs: midpoint of endpoint corner UVs per face
- Custom normal override: clear (force derived)
- Face attributes: unchanged

PropagatePolicy hook allows overriding each domain behavior.

Dirtiness:
- Adjacent faces dirty for triangulation
- Vertex star around endpoints and v_new dirty for derived normals

Must execute inside a Txn and record changes in ChangeSet.

## Acceptance Criteria

- split_edge inserts vertex and splits edge correctly
- All attribute domains propagated per defaults
- PropagatePolicy overrides work
- DirtySet updated correctly
- ChangeSet records created/modified elements
- Unit tests for topology, position interpolation, UV interpolation, sharpness inheritance

