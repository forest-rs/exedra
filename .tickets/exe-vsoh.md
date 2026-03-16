---
id: exe-vsoh
status: closed
deps: []
links: []
created: 2026-03-16T18:02:44Z
type: task
priority: 2
assignee: Bruce Mitchener
---
# Selected face patch topology query

Move selected patch boundary/interior edge classification and incident-vertex extraction out of Cambium patch helpers and into exedra::Mesh.

## Design

Add a deterministic selected-face patch topology query in exedra::Mesh that reports boundary edges, shared interior edges, incident vertices, and whether the patch boundary lies on the mesh boundary. Refactor Cambium patch::region to use it while keeping Cambium-owned normals/UV/source-attr enrichment local.

## Acceptance Criteria

1. Exedra exposes a deterministic selected-face patch topology query. 2. Cambium patch::region consumes it instead of classifying edges itself. 3. Tests cover deterministic ordering and stale/outside rejection. 4. fmt/clippy/tests/doc pass.

## Notes

**2026-03-16T18:28:00Z**

Added `Mesh::selected_face_patch_topology` plus the public `SelectedFacePatch*` data types to Exedra, and rewired `Mesh::selected_face_boundary_loops` to reuse that shared patch classification instead of re-deriving boundary edges independently. Cambium `patch::region` now consumes the kernel query and only layers normals, UVs, and source-edge attributes on top.

One integration subtlety mattered here: Cambium’s patch-edge contract historically used the “next corner” half-edge orientation rather than the raw face-loop corner orientation. The Exedra helper now preserves that contract so downstream inset/extrude wiring stays stable.

Validation:
- `typos crates/exedra/src/mesh.rs crates/exedra/src/lib.rs crates/cambium/src/patch/region.rs .tickets/exe-vsoh.md`
- `taplo fmt`
- `cargo fmt --all`
- `cargo test -p exedra -p cambium --all-features`
- `cargo clippy -p exedra -p cambium --all-targets --all-features -- -D warnings`
- `cargo doc -p exedra -p cambium --no-deps`
