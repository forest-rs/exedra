---
id: exe-ccxf
status: open
deps: []
links: [et-o66p, et-c9eu, et-jmpb]
created: 2026-08-21T13:39:07Z
type: feature
priority: 2
assignee: Bruce Mitchener
---
# Replace one mesh face with a validated patch

Give topology-edit consumers one atomic kernel operation that replaces an interior face with a triangulated or otherwise partitioned patch and reports explicit one-to-many lineage.

## Design

Exedra owns topology mutation, validation, attribute propagation, and source-face to replacement-face identity. The operation must preflight the complete patch before mutation, preserve the source boundary, reject non-manifold or inconsistent patches atomically, and avoid a generalized revision-history framework. The exact public result shape is a human-gated design decision before implementation.

## Acceptance Criteria

A documented no_std kernel operation atomically replaces one live interior face with a boundary-compatible patch; returns deterministic replacement face IDs associated with the source face; propagates face and boundary attributes by explicit policy; records ordinary ChangeSet creation/deletion; rejects stale, malformed, non-manifold, and partial candidates without mutation; safe Rust and no new dependency.


## Notes

**2026-08-21T13:58:08Z**

Architecture gate: use a dedicated additive face-to-triangle-patch operation whose return value carries source-to-created-face lineage; leave ChangeSet unchanged. A future ChangePlan concept could own reusable preflight/acceptance and produce a ChangeSet, but it is explicitly outside this slice.
