---
id: cam-ul4v
title: Face region/material tagging (tag.face.region)
status: closed
deps: [cam-ibof]
links: [cam-u9zk]
created: 2026-03-03T06:00:47Z
type: feature
priority: P1
assignee: Bruce Mitchener
tags: [v0.1]
---
# Face region/material tagging (tag.face.region)

Implement face region/material tagging as a first-class v0.1 capability. This is the glue that lets operators compose: earlier steps tag faces with semantic region IDs, later steps select by region to scope their work.

## Design

The face-domain "region id" layer:
- Type: u32 (simple, extensible, no enum lock-in)
- Domain: Face
- Storage: dense (most faces will have a region tag)
- Default value: 0 (untagged)

Exedra provides the ability to store face-domain attributes with typed keys and deterministic iteration. The region id layer should be a built-in key (`exedra::attr::FACE_REGION` or similar) — decide carefully, as this becomes part of the kernel contract.

Cambium/testkit defines demo-level region constants (REGION_WALL_OUTER, REGION_DOME, etc.) as vocabulary for the basilica pipeline and other demos. These are NOT kernel vocabulary — they live in cambium or testkit.

Operator: `tag.face.region`
- Assigns a region id to a canonical FaceSet
- Implements EditOperator
- Writes face-domain region attribute via Txn
- Produces standard OpReport with faces_processed counter

This is foundational for the basilica worked example where every step uses region tags to scope subsequent operations (extrude walls by footprint region, add openings on outer walls, UV project by region, ruinize by region, etc.).

## Acceptance Criteria

- Face-domain region id layer exists (u32, dense)
- Built-in attribute key for region id (in Exedra)
- tag.face.region operator implements EditOperator
- Sets region id on canonical FaceSet
- ChangeSet correctly reflects dirty faces
- Default region id is 0 (untagged)
- Region constants for demos live in cambium/testkit, not exedra
- Unit tests for tagging, overwriting, and default value


## Notes

**2026-03-03T06:37:44Z**

Worked example: docs/worked_example_basilica.md — every step uses region tags (REGION_FOOTPRINT, REGION_WALL_OUTER, REGION_DOME, etc.) to scope subsequent operators. Region tagging is the composability mechanism.

**2026-03-03T13:29:12Z**

API note: added exedra::attr::FACE_REGION built-in dense key and Txn::set_face_region() to support transaction-scoped operator writes to face-domain attributes.
