# Exedra

A production-capable geometry kernel and operator stack.

This workspace contains two core crates:

- **[exedra](crates/exedra/)** — Structural half-edge mesh kernel
  (topology, attributes, deterministic extraction, B-rep booleans).
- **[cambium](crates/cambium/)** — Operator and growth layer
  (subdivision, procedural modeling, UV generation, operator stacking).

Exedra is the calm, stable foundation. Cambium moves faster on top of it.

Experimental domain spikes may sit beside the core crates when they earn a
clear architectural slice. The current example is
**[exedra_analytic](crates/exedra_analytic/)**, a planar analytic-topology MVP
that tessellates deterministically into Exedra meshes.

## Architecture

Exedra provides a polygonal half-edge mesh with:

- **Stable IDs** (index + generation) for safe caching and external references.
- **Typed attribute layers** across vertex, face, edge, and corner domains.
- **Corner-domain attributes** (UVs, normals) enabling shading discontinuities
  without topological splits.
- **Explicit boundary model** with an outside face — no `Option` in hot fields.
- **Deterministic extraction** to GPU-ready triangle buffers.
- **Transactions** producing ChangeSets and DirtySets for incremental workflows.

Cambium provides:

- **Composable operators** with preview/commit execution modes.
- **Structured reporting** (deterministic stats, bounded artifacts, diagnostics).
- **UV generation** (planar, box, cylinder projection).
- **Region tagging** for operator composition via semantic face labels.

Both crates are `#![no_std]` compatible (with `alloc`).

## Design

See [`docs/exedra_handoff.md`](docs/exedra_handoff.md) and
[`docs/cambium_handoff.md`](docs/cambium_handoff.md) for comprehensive
design specifications.

Design briefs covering specific architectural decisions live in each
crate's `docs/briefs/` directory.

A worked example demonstrating the full pipeline is in
[`docs/worked_example_basilica.md`](docs/worked_example_basilica.md).

## Status

Early development. Working toward v0.1 (canonical kernel + deterministic
extraction for Exedra, operator runtime + UV generation for Cambium).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or
  <http://opensource.org/licenses/MIT>)

at your option.
