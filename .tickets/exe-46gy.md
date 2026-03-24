---
id: exe-46gy
status: closed
deps: [exe-yekg]
links: []
created: 2026-03-24T07:36:28Z
type: task
priority: 2
assignee: Bruce Mitchener
---
# Reuse Exedra Fidget Staging Buffers

Reuse staging buffers inside exedra_fidget bulk evaluators so eval_points and eval_gradients stop allocating fresh vectors on every call.

## Design

Own only adapter-internal scratch reuse in crates/exedra_fidget/src/field.rs. Preserve the public ScalarField and SpecializableField behavior while moving repeated point splitting into reusable buffers stored alongside the cached bulk evaluators. Depend on the new exedra_fidget benchmark crate for before/after measurements.

## Acceptance Criteria

1. exedra_fidget reuses point and gradient staging buffers across repeated eval_points/eval_gradients calls. 2. Existing adapter behavior and tests stay unchanged. 3. The new benchmark crate is rerun in release and the ticket note records the before/after timings plus tradeoffs. 4. typos, cargo fmt --all, cargo test -p exedra_fidget -p <bench-crate>, cargo clippy -p exedra_fidget -p <bench-crate> --all-targets --all-features -- -D warnings, and cargo doc -p exedra_fidget -p <bench-crate> --no-deps pass.

## Notes

**2026-03-24T07:58:00Z**

Reworked `crates/exedra_fidget/src/field.rs` so the cached float and gradient evaluators now own reusable `x/y/z` staging buffers instead of allocating fresh vectors on every `eval_points` and `eval_gradients` call. The bulk-eval calls now operate directly on those buffers and no longer clone the cached tapes on the hot path. Added a regression test that exercises repeated point and gradient evaluation across changing batch sizes to make sure the reused buffers stay valid after shrinking and growing. No ADR was added because this is an internal adapter-local performance change with no ownership or public semantic change. Measured against `exedra_fidget_bench`, release timings improved from `219.038 µs` to `190.714 µs` on `vm_sphere_eval_points`, from `767.193 µs` to `740.354 µs` on `vm_toothed_torus_eval_points`, from `477.721 µs` to `474.065 µs` on `vm_toothed_torus_eval_gradients`, and from `109.101 ms` to `95.168 ms` on the end-to-end `vm_toothed_torus_dual_contour` scenario. Validation: `typos crates/exedra_fidget/src/field.rs .tickets/exe-46gy.md`; `cargo fmt --all`; `cargo test -p exedra_fidget -p exedra_fidget_bench`; `cargo clippy -p exedra_fidget -p exedra_fidget_bench --all-targets --all-features -- -D warnings`; `cargo doc -p exedra_fidget -p exedra_fidget_bench --no-deps`; `cargo run --release -p exedra_fidget_bench`.
