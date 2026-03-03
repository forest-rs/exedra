# exedra

Structural half-edge mesh kernel.

Exedra is a production-capable, `#![no_std]` polygonal mesh kernel
providing:

- **Half-edge topology** with stable generational IDs and an explicit
  boundary model (outside face, no `Option` in hot fields).
- **Typed attribute layers** across vertex, face, edge, and corner
  domains. Corner attributes (UVs, normals) enable shading
  discontinuities without topological splits.
- **Deterministic extraction** to GPU-ready triangle buffers, with
  render-vertex splitting on attribute discontinuities.
- **Transactions** that produce ChangeSets and DirtySets for incremental
  workflows.
- **Validation** (fast and deep) with structured error reporting.
- **B-rep booleans** (planned) as a staged pipeline with diagnostic
  artifacts.

Exedra is designed to be calm, stable, and long-lived — higher-level
modeling operations live in [cambium](../cambium/).

## Design

- [Handoff spec](../../docs/exedra_handoff.md) — comprehensive design
  document covering topology, attributes, invariants, APIs, and
  milestone plan.
- [Design briefs](docs/briefs/) — focused decisions on specific topics
  (boundary model, determinism, attribute storage, etc.).
- [ADRs](docs/) — architectural decision records.

## License

Licensed under either of Apache License 2.0 or MIT license at your
option. See the workspace root for license files.
