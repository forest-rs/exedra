---
id: exe-tu2n
status: closed
deps: []
links: []
created: 2026-03-16T02:30:32Z
type: feature
priority: 2
assignee: Bruce Mitchener
parent: cam-t6z7
tags: [mesh, queries, regions]
---
# Connected patch and region-boundary mesh queries

Continue the Exedra mesh-head hardening work by moving selected patch adjacency and region-boundary queries out of Cambium patch helpers and into Exedra.

## Design

Add deterministic helpers for connected face patches and/or selected-region boundary extraction in exedra::Mesh, then refactor at least one Cambium path to consume them. Reuse the multi-domain ADR; no new ADR unless ownership changes.

## Acceptance Criteria

1. Exedra exposes at least one new deterministic patch/region query. 2. Cambium migrates a load-bearing caller to it. 3. Tests cover ordering and stale-id behavior. 4. fmt/clippy/tests pass.

## Notes

**2026-03-16T03:06:00Z**

Added `Mesh::connected_face_region` and the public `ConnectedFaceRegionError` surface to move connected face-region traversal into Exedra. Cambium's `flood_fill_faces_by_region` now delegates to that helper instead of owning its own BFS, while preserving the higher-level report shape and error classification. Added Exedra regression coverage for deterministic ordering, region-boundary behavior, and stale/outside seed rejection.

Validation:
- `typos crates/exedra/src/mesh.rs crates/exedra/src/lib.rs crates/cambium/src/region.rs .tickets/exe-tu2n.md`
- `taplo fmt`
- `cargo fmt --all`
- `cargo test -p exedra -p cambium --all-features`
- `cargo clippy -p exedra -p cambium --all-targets --all-features -- -D warnings`
- `cargo doc -p exedra -p cambium --no-deps`
