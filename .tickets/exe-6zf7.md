---
id: exe-6zf7
status: closed
deps: [exe-cfcc]
links: []
created: 2026-03-24T06:17:50Z
type: task
priority: 2
assignee: Bruce Mitchener
---
# Accelerate render vertex dedup in to_trimesh

Replace Mesh::to_trimesh linear scans for render-vertex reuse and split tracking with indexed lookup structures so large extracted meshes do not spend tens of seconds in render extraction.

## Design

Keep the public ExtractParams/ExtractStats surface stable. Use data structures that still work under no_std + alloc. Depend on the benchmark ticket for measurement and regression checks.

## Acceptance Criteria

1. `to_trimesh` no longer uses global linear scans for render-vertex lookup on the hot path.
2. Split counting semantics remain unchanged on existing tests.
3. Wind-tunnel results show a material improvement on the split-heavy scenario.
4. Public API/docs remain accurate.
5. `typos`, `cargo fmt --all`, `cargo test -p exedra`, `cargo clippy -p exedra --all-targets --all-features -- -D warnings`, `cargo doc -p exedra --no-deps`, and the render wind tunnel pass.

## Notes

**2026-03-24T06:28:13Z**

Replaced Mesh::to_trimesh hot-path linear scans with two indexed structures built from alloc-only collections: a BTreeMap<RenderVertexKey, u32> for render-vertex reuse and a per-topology-vertex BTreeMap<VertexId, VertexVariants> for local UV/normal variant tracking. Public ExtractParams/ExtractStats semantics stayed unchanged, and the existing render tests all stayed green. Measured against the new render wind tunnel, release timings improved from 27.8 ms to 3.3 ms on the smooth torus, from 183.4 ms to 5.6 ms on the UV-split sphere, and from 1.328 s to 23.9 ms on the split-heavy implicit toothed torus. Validation: cargo fmt --all; cargo test -p exedra; cargo clippy -p exedra --all-targets --all-features -- -D warnings; cargo doc -p exedra --no-deps; cargo run --release -p exedra_render_bench.
