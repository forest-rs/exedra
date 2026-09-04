# exedra_ops

Deterministic mesh operations and typed workflow adapters for Exedra.

Exedra Ops provides higher-level modeling operations on top of
[`exedra_mesh`](https://crates.io/crates/exedra_mesh), the structural mesh kernel. It is `#![no_std]`
compatible (with `alloc`) and designed for both interactive and
procedural workflows.

```rust
use exedra_ops::{Mesh, OperatorRunner, ValidateMesh, ValidateMeshMode, ValidateMeshParams};

let mesh = Mesh::new();
let mut runner = OperatorRunner::new();
let operation = ValidateMesh;
let params = ValidateMeshParams {
    mode: ValidateMeshMode::FastAndDeep,
};

let plan = runner.compile(&mesh, &operation, &params).expect("valid plan");
let preview = runner
    .preview_on_clone(&mesh, &operation, &plan)
    .expect("preview succeeds");
assert_eq!(preview.report.name, "inspect.validate.mesh");
```

Start with `OperatorRunner` when you need compile/preview/apply lifecycle and
structured reporting. Use `MeshEdit` for a fluent sequence of supported mesh
edits. Drop to `exedra_mesh::op` when you are implementing a lower-level
kernel operation rather than an application workflow.

The default feature set is the focused `std` mesh-operation surface. Enable
`analytic` for analytic editing and analytic-to-mesh conversion,
`constructive` for recipe workflows, or `assembly` for pattern expansion. The
application-facing `exedra` facade selects the common constructive, assembly,
and operations bundle. The `profile_section` module is included by
`constructive`; `convert` is included by either `analytic` or `constructive`,
with its items gated independently. `std` and `libm` can be selected
independently and are forwarded only to enabled adapters.

## Core concepts

- **Edit operators** apply changes via Exedra edit scopes and can return
  structured ChangeSets when the runner requests recorded changes. This is the
  primary execution path.
- **Preview/commit** separation is first-class — preview may be
  budgeted and approximate, commit is reproducible.
- **Operator reports** with deterministic stats, bounded artifacts, and
  severity-aware diagnostics are mandatory.
- **Region tagging** enables operator composition: earlier steps tag
  faces with semantic IDs, later steps select by region.

## Current mesh surface

- **UV generation**: planar, box, and cylinder projection.
- **Selection and tagging**: edge sharpness, seam marking, face
  region assignment.
- **Topology edits**: delete and dissolve, boundary-loop bridge, rectangular
  face cut, extrude, inset, poke, and solidify.
- **Normals**: clear, face, derived, and smoothing operations.
- **Boolean orchestration**: preview/commit staging over Exedra Mesh's Boolean
  pipeline.

## Design

- [SDK boundary](https://github.com/forest-rs/exedra/blob/main/crates/exedra_ops/docs/adr-0001-operator-sdk-surface.md) — the mesh workflow
  surface and its relationship to the kernel.
- [Cross-domain boundary](https://github.com/forest-rs/exedra/blob/main/crates/exedra_ops/docs/adr-0005-exedra-ops-and-cross-domain-boundary.md)
  — native-head ownership and the future procedural-network seam.
- [Design briefs](https://github.com/forest-rs/exedra/tree/main/crates/exedra_ops/docs/briefs) — focused decisions on specific topics
  (preview/commit, timing, budget semantics, etc.).
- [Worked example](https://github.com/forest-rs/exedra/blob/main/docs/worked_example_basilica.md) — a Byzantine
  basilica ruin pipeline demonstrating the full stack.

## License

Licensed under either of Apache License 2.0 or MIT license at your
option. See the workspace root for license files.
