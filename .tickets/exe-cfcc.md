---
id: exe-cfcc
status: closed
deps: []
links: []
created: 2026-03-24T06:17:50Z
type: task
priority: 2
assignee: Bruce Mitchener
---
# Render extraction wind tunnel

Add a top-level benchmark crate for Mesh::to_trimesh scenarios so render-extraction changes can be measured on representative meshes, including high-split implicit outputs.

## Design

Own this as performance infrastructure, not an algorithm rewrite. Keep benchmark code out of core crates. Start with executable-style timings like exedra_qef_bench and include at least one synthetic seam-heavy mesh plus one imported implicit-style OBJ or generated mesh shape.

## Acceptance Criteria

1. A new top-level benchmark crate exists for exedra render extraction. 2. It exercises Mesh::to_trimesh on at least two representative scenarios, including a split-heavy case. 3. Results include triangle/render-vertex counts and elapsed timings in release mode. 4. The crate is wired into the workspace and documented enough to run. 5. typos, cargo fmt --all, cargo test -p <new-crate>, cargo clippy -p <new-crate> --all-targets --all-features -- -D warnings, and cargo doc -p <new-crate> --no-deps pass.


## Notes

**2026-03-24T06:23:43Z**

Added benchmarks/exedra_render_bench as a top-level executable wind tunnel for Mesh::to_trimesh. The crate exercises three scenarios: a smooth torus baseline, a synthetic UV-split sphere with per-face UV variation, and a split-heavy implicit toothed torus generated through exedra_fidget + exedra_isosurface. Each scenario reports mesh face/vertex counts, output triangle/render-vertex counts, split counters, and best/average release timings. Initial release baseline on this machine: torus_smooth best/avg 27.4/27.8 ms, uv_sphere_face_uv_splits 182.6/183.4 ms, implicit_toothed_torus 1.328/1.328 s. Validation: typos benchmarks/exedra_render_bench/src/main.rs benchmarks/exedra_render_bench/Cargo.toml .tickets/exe-cfcc.md Cargo.toml; cargo fmt --all; cargo test -p exedra_render_bench; cargo clippy -p exedra_render_bench --all-targets --all-features -- -D warnings; cargo doc -p exedra_render_bench --no-deps; cargo run --release -p exedra_render_bench.
