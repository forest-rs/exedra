---
id: exe-rwps
status: closed
deps: []
links: []
created: 2026-03-24T07:09:49Z
type: task
priority: 2
assignee: Bruce Mitchener
---
# Swap Sampled Hot Maps To Hashbrown

Replace sampled BTreeMap hot paths in dual contour triangle bookkeeping and render extraction with hashbrown maps, then remeasure the render wind tunnel.

## Design

Scope is limited to the sampled hot maps in crates/exedra_isosurface/src/dual_contour.rs and crates/exedra/src/render.rs. Preserve deterministic output ordering and public semantics while reducing map insertion overhead in release-mode meshing and render extraction.

## Acceptance Criteria

1. Dual contour triangle bookkeeping and render extraction use hashbrown in their hot-path lookup structures without changing public behavior. 2. Existing tests stay green. 3. exedra_render_bench is rerun in release and the ticket note records before/after timings plus any tradeoffs.

## Notes

**2026-03-24T07:23:00Z**

Swapped the sampled hot-path lookup structures from `alloc` B-trees to `hashbrown` maps/sets in exactly two places: `exedra::render::to_trimesh` now uses `HashMap<RenderVertexKey, u32>` and `HashMap<VertexId, VertexVariants>` for render-vertex reuse and split tracking, and `exedra_isosurface::dual_contour::try_mark_triangle` now uses `HashSet<[u32; 3]>` plus `HashMap<(u32, u32), u8>` for duplicate-triangle and edge-incidence bookkeeping. The output order remains traversal-driven, so the hash-based containers do not affect public determinism. No ADR was added because this is an internal performance-only container swap with no ownership or semantic change. Release wind-tunnel averages improved from 3.31 ms to 2.25 ms on `torus_smooth`, from 5.62 ms to 3.50 ms on `uv_sphere_face_uv_splits`, and from 23.91 ms to 15.08 ms on `implicit_toothed_torus`. Validation: `typos Cargo.toml crates/exedra/Cargo.toml crates/exedra_isosurface/Cargo.toml crates/exedra/src/render.rs crates/exedra_isosurface/src/dual_contour.rs .tickets/exe-rwps.md`; `cargo fmt --all`; `taplo fmt`; `cargo test -p exedra -p exedra_isosurface -p exedra_render_bench`; `cargo clippy -p exedra -p exedra_isosurface -p exedra_render_bench --all-targets --all-features -- -D warnings`; `cargo doc -p exedra -p exedra_isosurface -p exedra_render_bench --no-deps`; `cargo run --release -p exedra_render_bench`.
