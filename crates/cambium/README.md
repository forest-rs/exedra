# cambium

Operator and growth layer for the exedra mesh kernel.

Cambium provides higher-level modeling operations on top of
[exedra](../exedra/), the structural mesh kernel. It is `#![no_std]`
compatible (with `alloc`) and designed for both interactive and
procedural workflows.

## Core concepts

- **Edit operators** apply changes via Exedra transactions and return
  structured ChangeSets. This is the primary execution path.
- **Preview/commit** separation is first-class — preview may be
  budgeted and approximate, commit is reproducible.
- **Operator reports** with deterministic stats, bounded artifacts, and
  severity-aware diagnostics are mandatory.
- **Region tagging** enables operator composition: earlier steps tag
  faces with semantic IDs, later steps select by region.

## Planned operators

- **UV generation**: planar, box, and cylinder projection.
- **Selection and tagging**: edge sharpness, seam marking, face
  region assignment.
- **Subdivision**: Catmull-Clark (v0.5).
- **Modeling**: extrude, inset, bevel workflows (v0.5).
- **Boolean orchestration**: preview/commit staging over Exedra's
  boolean pipeline (v0.9).

## Design

- [Handoff spec](../../docs/cambium_handoff.md) — comprehensive design
  document covering operator runtime, policies, diagnostics, and
  milestone plan.
- [Design briefs](docs/briefs/) — focused decisions on specific topics
  (preview/commit, timing, budget semantics, etc.).
- [Worked example](../../docs/worked_example_basilica.md) — a Byzantine
  basilica ruin pipeline demonstrating the full stack.

## License

Licensed under either of Apache License 2.0 or MIT license at your
option. See the workspace root for license files.
