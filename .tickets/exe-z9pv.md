---
id: exe-z9pv
status: open
deps: [exe-o4iu]
links: []
created: 2026-03-03T05:38:14Z
type: feature
priority: 2
assignee: Bruce Mitchener
tags: [v0.5]
---
# Custom corner normal override layer

Implement the optional corner-domain custom normal override layer. Allows authored normals that override derived normals per-corner. Supports NormalsSource policy: Derived, CustomOrDerived, CustomOnly.

## Design

NormalsSource enum controls extraction behavior:
- Derived: always use computed normals (ignore overrides)
- CustomOrDerived: use custom where present, derived elsewhere
- CustomOnly: only use custom normals (missing = zero/error)

Custom normals are authored data — not reinterpreted on topology changes. When edits create new corners, overrides default to cleared (forces derived) unless PropagatePolicy says otherwise.

Built-in key: exedra::attr::CORNER_NORMAL_OVERRIDE
Storage: sparse (rare — most corners use derived normals)

## Acceptance Criteria

- Custom normal override layer exists as sparse corner-domain attribute
- NormalsSource enum controls extraction behavior
- Extraction respects the source policy
- New corners from edits default to cleared overrides
- Unit tests for each NormalsSource mode


## Notes

**2026-03-03T06:17:41Z**

Design brief: crates/exedra/docs/briefs/05_derived_vs_authored_normals.md
