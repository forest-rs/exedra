# Plan: exe-o4iu + exe-z9pv + exe-o1ar normals slice

## Goals
- Add deterministic derived corner normals in Exedra.
- Add authored corner-normal overrides with explicit source policy.
- Extend render extraction to emit real normals and split render vertices on normal discontinuities.
- Keep the public API calm and explicit.

## Non-goals
- Tangent generation.
- Incremental normal recomputation/cache infrastructure.
- Normal editing operators above the kernel authored-write layer.

## Steps
1. Add derived normal types/params and deterministic normal computation over corners.
2. Add the sparse authored override layer and authored-write op.
3. Propagate authored overrides through topology edits using explicit policy.
4. Extend render extraction params/stats/output to consume derived and authored normals.
5. Split render vertices on UV or normal discontinuity.
6. Update docs and close the tickets atomically with the code.

## Risks
- Introducing hidden policy in extraction instead of explicit params.
- Regressing determinism via unstable traversal or float handling.

## Validation
- typos
- cargo fmt --all
- taplo fmt
- cargo clippy --workspace --all-targets --all-features -- -D warnings
- cargo test --workspace --all-features
- cargo doc --no-deps
