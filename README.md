# Exedra

A production-capable geometry kernel and operator stack.

Exedra is the calm geometry foundation. Cambium is the workflow-facing operator
SDK on top of it. The rest of the workspace holds focused construction,
extraction, adapter, test, benchmark, and demo crates that prove those
boundaries without bloating the kernel.

## Workspace

Core crates:

- **[exedra](crates/exedra/)** - Structural half-edge mesh kernel:
  topology, stable IDs, attributes, validation, edit sessions, dirty/change
  summaries, and deterministic triangle extraction.
- **[cambium](crates/cambium/)** - Operator and growth layer:
  compile/preview/apply lifecycle, diagnostics, reports, policy, selections,
  UV projection, face edits, normal edits, and fluent mesh workflows.

Construction and extraction crates:

- **[exedra_primitives](crates/exedra_primitives/)** - Deterministic mesh
  primitive generators such as quads, boxes, grids, cylinders, cones, torus,
  UV spheres, and icospheres.
- **[exedra_analytic](crates/exedra_analytic/)** - Planar analytic topology
  slice that tessellates rectangular frames/openings into Exedra meshes.
- **[exedra_spatial](crates/exedra_spatial/)** - Small spatial primitives:
  AABBs and deterministic flat-octree traversal/refinement.
- **[exedra_qef](crates/exedra_qef/)** - Small QEF solver used by dual
  contouring and related fitting tasks.
- **[exedra_isosurface](crates/exedra_isosurface/)** - Scalar-field seams,
  reference analytic fields, transforms, profile lifts, Hermite intersection
  data, and dual-contouring extraction.

Adapter, test, benchmark, and app crates:

- **[exedra_fidget](crates/exedra_fidget/)** - Thin adapter from Fidget shapes
  into the `exedra_isosurface` field traits.
- **[exedra_testkit](crates/exedra_testkit/)** and
  **[cambium_testkit](crates/cambium_testkit/)** - Deterministic fixtures,
  golden snapshots, and debug dumps.
- **[benchmarks/](benchmarks/)** - Executable wind-tunnel crates for Exedra
  kernel scenarios, QEF solves, render extraction, and Fidget-backed
  field/extraction paths.
- **[apps/cambium_web_bridge](apps/cambium_web_bridge/)** - Wasm bridge for
  deterministic Cambium scenario execution.
- **[apps/cambium_web_viewer](apps/cambium_web_viewer/)** - Three.js viewer for
  the wasm scenario snapshots.

## Architecture

Exedra owns the mesh model:

- **Stable IDs**: index + generation handles for caching and stale-reference
  rejection.
- **Typed attribute layers**: vertex, face, edge, and corner domains.
- **Corner-domain attributes**: UVs and normals for shading discontinuities
  without topological splits.
- **Explicit boundary model**: boundary half-edges use an outside face instead
  of optional hot-path fields.
- **Deterministic extraction**: polygonal meshes to GPU-ready triangle buffers.
- **Edit sessions**: eager mutation with optional ChangeSet and DirtySet output.

Cambium owns workflow orchestration:

- **Operator lifecycle**: compile, preview-on-clone, and apply-in-place.
- **Structured reporting**: deterministic stats, bounded artifacts, diagnostics,
  timings, and plan fingerprints.
- **Selection and tagging**: canonical face/edge/vertex sets and region labels.
- **Mesh edits**: delete/dissolve, bridge, cut, extrude, inset, poke, solidify,
  UV projection, and corner-normal operations.

Implicit and primitive crates stay outside the kernel. They produce or adapt
geometry through explicit Exedra mesh/field boundaries rather than introducing a
scene graph into the core.

## Example Flow

```rust
use cambium::{
    Mesh, OperatorRunner, ValidateMesh, ValidateMeshMode, ValidateMeshParams,
};

fn main() -> Result<(), cambium::OpError> {
    let mesh = Mesh::new();
    let mut runner = OperatorRunner::new();
    let op = ValidateMesh;
    let params = ValidateMeshParams {
        mode: ValidateMeshMode::FastAndDeep,
    };

    let plan = runner.compile(&mesh, &op, &params)?;
    let preview = runner.preview_on_clone(&mesh, &op, &plan)?;
    assert_eq!(preview.report.name, "inspect.validate.mesh");
    Ok(())
}
```

## Glossary

- **Kernel** - The long-lived mesh/topology core in `exedra`.
- **Operator** - A Cambium workflow unit with compile, preview, and apply steps.
- **Attribute domain** - Where data lives: vertex, face, edge, or corner.
- **Field seam** - The trait boundary used by implicit-surface extractors.
- **Wind tunnel** - A small executable benchmark crate outside the core crates.

## Design

See [`docs/exedra_handoff.md`](docs/exedra_handoff.md) and
[`docs/cambium_handoff.md`](docs/cambium_handoff.md) for comprehensive
design specifications.

Design briefs covering specific architectural decisions live in each
crate's `docs/briefs/` directory.

A worked example demonstrating the full pipeline is in
[`docs/worked_example_basilica.md`](docs/worked_example_basilica.md).

## Validation

The intended workspace gates are:

```sh
typos
cargo fmt --all
taplo fmt
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo doc --no-deps
```

## Status

Early development. Working toward v0.1 across the Exedra kernel, Cambium
operator SDK, deterministic primitive generation, implicit-surface extraction,
and local web demo surfaces. Boolean, subdivision, compaction, fuzzing, and
semver-stability work remain tracked as tickets.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or
  <http://opensource.org/licenses/MIT>)

at your option.
