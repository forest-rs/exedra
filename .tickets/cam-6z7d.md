---
id: cam-6z7d
status: closed
deps: [cam-m068]
links: []
created: 2026-03-15T18:06:03Z
type: feature
priority: 2
assignee: Bruce Mitchener
parent: cam-t6z7
tags: [architecture, analytic]
---
# Analytic head MVP and deterministic tessellation path

Prove the second canonical geometry domain with a narrow analytic crate and deterministic analytic->mesh conversion into Exedra.

## Design

Scope the MVP hard: planar faces, line edges, shells/loops/coedges, deterministic tessellation into exedra::Mesh, and one end-to-end demo such as a wall with an opening. Do not attempt general trims, NURBS, or heroic booleans in the first slice.

## Acceptance Criteria

1. New analytic crate/module boundary is specified. 2. MVP topology/geometry types are documented. 3. Deterministic analytic->mesh conversion contract exists. 4. One demo scenario is defined for comparison against mesh-native authoring.


## Notes

**2026-03-15T18:19:34Z**

Landed the first exedra_analytic spike as a new workspace crate. Scope is intentionally narrow: planar faces, line-segment coedges, shell/loop/coedge topology, deterministic tessellation into exedra::Mesh, and a rect_frame_xy helper that proves analytic authoring of a wall-opening style frame before mesh conversion. Added crate-local ADR docs/adr-0001-planar-mvp-scope.md, README, tests for planar validation and deterministic tessellation, and workspace wiring. Also updated the root README to surface exedra_analytic as an experimental domain spike. Validation: typos README.md crates/exedra_analytic crates/cambium/docs crates/cambium/src; cargo fmt --all; cargo test -p exedra_analytic --all-features; cargo clippy -p exedra_analytic --all-targets --all-features -- -D warnings; cargo doc -p exedra_analytic --no-deps; cargo test --workspace --all-features; cargo clippy --workspace --all-targets --all-features -- -D warnings.
