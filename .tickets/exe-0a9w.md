---
id: exe-0a9w
title: split_face with attribute propagation
status: open
deps: [exe-dey4, exe-ognv]
links: []
created: 2026-03-03T05:39:00Z
type: feature
priority: P1
assignee: Bruce Mitchener
tags: [v0.1]
---
# split_face with attribute propagation

Implement split_face / insert_diagonal: split one face into two by inserting a diagonal edge. Must propagate attributes according to PropagatePolicy.

## Design

Topology: one face becomes two faces connected by a new diagonal edge.

Attribute propagation defaults:
- Face attributes: both faces copy original (material/region)
- Corner UVs: existing corners keep UVs, new diagonal corners copy from corresponding vertex corner in original face
- Custom normal override: existing keep, new diagonal corners cleared
- Edge (new diagonal): smooth (not sharp) unless policy forces sharp

Dirtiness: both new faces dirty, incident vertices dirty for derived normals.
Must execute inside a Txn.

## Acceptance Criteria

- split_face splits a face into two via diagonal
- Attribute propagation correct per defaults
- PropagatePolicy overrides work
- DirtySet and ChangeSet updated
- Unit tests for quad split, n-gon split, UV preservation


## Notes

**2026-03-03T06:17:41Z**

Design brief: crates/exedra/docs/briefs/05_derived_vs_authored_normals.md

**2026-03-03T06:27:28Z**

Design brief: crates/exedra/docs/briefs/13_edit_propagation_model.md
