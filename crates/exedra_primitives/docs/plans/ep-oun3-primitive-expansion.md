# ep-oun3 Primitive Expansion Pack

## Goals
- Add a shared cap-fill policy for rotational primitives.
- Add deterministic `grid`, `cone`, `torus`, and `icosphere` primitives.
- Keep region/selection contracts consistent with existing primitives.
- Update web primitive gallery to showcase the expanded set.

## Non-Goals
- Subdivision/remeshing kernels.
- New Cambium operators.
- Runtime optimization beyond current primitive-scale workloads.

## Execution Order
1. `ep-we4l`: CapFill enum + cylinder migration.
2. `ep-06tl`: Segmented grid primitive.
3. `ep-1xp0`: Cone primitive using CapFill.
4. `ep-twe4`: Torus primitive.
5. `ep-lbmm`: Icosphere primitive.
6. `cwb-8nej`: Web primitive gallery integration + docs.

## Risks
- Selection semantics drift across primitives.
- Cap fill winding inconsistencies on bottom caps.
- Determinism regressions when adding subdivided meshes.

## Validation
- `cargo fmt --all`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-features`
- `cargo doc --no-deps`
