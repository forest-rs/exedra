---
id: exe-yekg
status: closed
deps: []
links: []
created: 2026-03-24T07:36:28Z
type: task
priority: 2
assignee: Bruce Mitchener
---
# Add Exedra Fidget Wind Tunnel

Add a top-level benchmark crate for exedra_fidget field evaluation and one representative meshing scenario.

## Design

Keep benchmark code out of core crates. Measure the adapter seams that the profiler pointed at: eval_points, eval_gradients, and one end-to-end dual_contour path on a Fidget-authored shape. Report release timings and enough shape/point counts to compare before and after adapter changes.

## Acceptance Criteria

1. A new top-level benchmark crate exists for exedra_fidget. 2. It measures at least eval_points and eval_gradients on representative Fidget-authored shapes. 3. It includes one end-to-end extraction scenario. 4. Results print release timings and scenario sizes. 5. typos, cargo fmt --all, cargo test -p <new-crate>, cargo clippy -p <new-crate> --all-targets --all-features -- -D warnings, and cargo doc -p <new-crate> --no-deps pass.

## Notes

**2026-03-24T07:45:00Z**

Added `benchmarks/exedra_fidget_bench` as a top-level executable wind tunnel for the profiled `exedra_fidget` adapter seams. The benchmark covers VM-backed sphere `eval_points`, toothed-torus `eval_points`, toothed-torus `eval_gradients`, and one end-to-end toothed-torus `dual_contour` extraction path so adapter-internal changes can be correlated with meshing impact. Initial release baseline on this machine: `vm_sphere_eval_points` best/avg `200.042/219.038 µs` on `32768` samples, `vm_toothed_torus_eval_points` `718.875/767.193 µs` on `32768` samples, `vm_toothed_torus_eval_gradients` `467.292/477.721 µs` on `16384` samples, and `vm_toothed_torus_dual_contour` `107.948/109.101 ms` with `9904` active cells and `19744` output faces. No ADR was added because this is benchmark infrastructure only and does not change crate ownership or public semantics. Validation: `typos Cargo.toml benchmarks/exedra_fidget_bench/Cargo.toml benchmarks/exedra_fidget_bench/src/main.rs .tickets/exe-yekg.md`; `cargo fmt --all`; `taplo fmt`; `cargo test -p exedra_fidget_bench`; `cargo clippy -p exedra_fidget_bench --all-targets --all-features -- -D warnings`; `cargo doc -p exedra_fidget_bench --no-deps`; `cargo run --release -p exedra_fidget_bench`.
