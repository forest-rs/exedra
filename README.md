# Exedra

An actively validated geometry toolkit for building inspectable virtual-world
assets.

The `exedra` crate is the thin application-facing facade. Focused crates retain
the geometry state and algorithms behind it: Exedra Mesh supplies the polygon
kernel, Exedra Constructive retains construction intent, Exedra Assembly owns
placed structure, mesh ops supplies direct geometry operations, and Exedra Edit
adds optional command execution.
Applications can start from one curated namespace while specialist crates stay
small, independently usable, and honest about conversion boundaries.

## Using Exedra

Most applications should start with the facade:

```toml
[dependencies]
exedra = "0.1"
```

Its default features provide the mesh kernel, constructive recipes,
assemblies, and direct mesh operations. Primitive generation, analytic topology,
implicit surfaces, command execution, glTF export, and interchange are opt-in features. Disable
default features and select `libm` for a `no_std` application.

Depend directly on a focused crate when you are implementing a lower-level
algorithm or only need that domain. The public family includes the facade and
the `exedra_*` geometry, support, adapter, and export crates listed below.
Workspace testkits, examples, benchmarks, and the construction experiments are
not published.

## Crate map

Facade and core workflow crates:

- **[exedra](crates/exedra/)** - Feature-gated facade for the stable
  application-facing geometry heads. It contains no geometry implementation.
- **[exedra_mesh](crates/exedra_mesh/)** - Structural half-edge mesh kernel:
  topology, stable IDs, attributes, validation, edit sessions, dirty/change
  summaries, and deterministic triangle extraction.
- **[exedra_mesh_ops](crates/exedra_mesh_ops/)** - Direct mesh modeling and
  geometric queries: face edits, Booleans, edge finishing, sections, frames,
  clearance, transforms, UVs and normals, with source correspondence.
- **[exedra_edit](crates/exedra_edit/)** - Optional mesh command execution:
  compile/preview/apply, stale plans, timings, diagnostics and change reports.
  Command adapters call the same direct algorithms.

Construction and extraction crates:

- **[exedra_constructive](crates/exedra_constructive/)** - Immutable,
  fingerprinted constructive recipes with deterministic tessellation,
  provenance, fidelity reporting, interchange, and evaluation caching.
- **[exedra_assembly](crates/exedra_assembly/)** - Named parts and instances,
  stable paths, material-slot binding, cached compilation, and deterministic
  flattening.
- **[exedra_triangulate](crates/exedra_triangulate/)** - Deterministic,
  dependency-free planar polygon triangulation and exact predicate seams.
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
- **[exedra_gltf](crates/exedra_gltf/)** - Deterministic glTF export for named
  render items, materials, instance metadata, and face-region provenance.
- **[exedra_measurements](crates/exedra_measurements/)** - Exact positive
  lengths, signed offsets, angle magnitudes, and signed angular offsets.
- **[exedra_fidget](crates/exedra_fidget/)** - Adapter from Fidget expression
  shapes to the `exedra_isosurface` field traits.

Workspace-only construction, test, benchmark, and app crates:

- **[joiner](crates/joiner/)** and related `setout`/`joiner_timber`/`joiner_masonry` crates -
  evolving construction and joinery layers kept outside the first public
  package set.
- **[exedra_testkit](crates/exedra_testkit/)** and
  **[exedra_edit_testkit](crates/exedra_edit_testkit/)** - Deterministic fixtures,
  golden snapshots, and debug dumps.
- **[benchmarks/](benchmarks/)** - Executable wind-tunnel crates for Exedra
  kernel scenarios, QEF solves, render extraction, and Fidget-backed
  field/extraction paths.
- **[apps/exedra_edit_web_bridge](apps/exedra_edit_web_bridge/)** - Wasm bridge
  for deterministic Exedra Edit scenario execution.
- **[apps/exedra_edit_web_viewer](apps/exedra_edit_web_viewer/)** - Three.js
  viewer for the wasm scenario snapshots.
- **[examples/](examples/)** - Standalone constructive, basilica, and structural
  integration scenarios kept outside the core crates.
  **[yingzao_fashi_pavilion](examples/yingzao_fashi_pavilion/)** builds a timber pavilion
  with exact setting out, fitted brackets, shared parts, and material variants.
  **[moon_gate_courtyard](examples/moon_gate_courtyard/)** places it in a garden
  with a coursed masonry entrance, procedural planting, pierced stones and drainage.

## Architecture

The `exedra` facade owns dependency selection, curated namespaces, and
end-to-end entry documentation. No implementation crate depends on it, and it
does not define geometry state, algorithms, scheduling, or conversion
semantics.

Exedra Mesh owns the mesh model:

- **Stable IDs**: index + generation handles for caching and stale-reference
  rejection.
- **Typed attribute layers**: vertex, face, edge, and corner domains.
- **Corner-domain attributes**: UVs and normals for shading discontinuities
  without topological splits.
- **Explicit boundary model**: boundary half-edges use an outside face instead
  of optional hot-path fields.
- **Deterministic extraction**: polygonal meshes to GPU-ready triangle buffers.
- **Edit sessions**: eager mutation with optional ChangeSet and DirtySet output.

`exedra_mesh_ops` owns reusable modeling algorithms and geometric queries.
Constructive binds their source correspondence to authored provenance.
`exedra_edit` wraps direct operations with command plans, clone-based preview,
timings, diagnostics and change reports.

Native heads retain their own values, algorithms and conversions. Use
`exedra_mesh_ops` for direct geometry and `exedra_edit` for editor command
execution. A heterogeneous procedural network belongs in an application toolkit
above these crates.

Implicit and primitive crates stay outside the mesh kernel. They produce or
adapt geometry through explicit mesh/field boundaries rather than introducing
a scene graph into the core.

## Example Flow

```rust
use exedra::{Mesh, mesh::PropagatePolicy};
use exedra::mesh_ops::face_edit::{extrude_faces, ExtrudeFacesParams, ExtrudeMode};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut mesh = Mesh::from_polygons(
        &[[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [2.0, 1.0, 0.0], [0.0, 1.0, 0.0]],
        &[&[0, 1, 2, 3]],
    )?;
    let faces = mesh.faces().collect();
    let mut edit = mesh.edit();
    let (_, result) = extrude_faces(&mut edit, &ExtrudeFacesParams {
        faces, mode: ExtrudeMode::KeepSource, distance: 0.5,
    }, &PropagatePolicy::default())?;
    assert_eq!(result.cap_faces.len(), 1);
    let _: () = edit.finish();
    Ok(())
}
```

## Glossary

- **Facade** - The leaf-only `exedra` crate that selects and names public heads.
- **Mesh kernel** - The long-lived mesh/topology core in `exedra_mesh`.
- **Operator** - An Exedra Edit mesh command with compile, preview, and
  apply steps.
- **Attribute domain** - Where data lives: vertex, face, edge, or corner.
- **Field seam** - The trait boundary used by implicit-surface extractors.
- **Wind tunnel** - A small executable benchmark crate outside the core crates.

## Design

The current direction and capability boundaries live in
[`ROADMAP.md`](ROADMAP.md). Durable architectural decisions live in each
owning crate's `docs/adr-*.md` files; implementation-specific design briefs
remain beside their owning crates.

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

Exedra is early and evolving, but the workspace already contains a usable
facade, deterministic mesh kernel, and focused constructive, field-extraction,
assembly, inspection, and export layers.

The project does not claim universal Boolean coverage, general manifold dual
contouring, subdivision, CAD-grade exact arithmetic, structural analysis, or
semver stability. Unsupported and ambiguous cases are expected to return typed
diagnostics where the current contract permits them.

The deliberately small forward plan lives in [`ROADMAP.md`](ROADMAP.md). It
prioritizes kernel correctness, a complete constructive-to-assembly asset path,
inspectable interchange, field-extraction stabilization, measured incremental
workflows, and scenario-driven geometry-quality extensions. Structural and
historical showcase experiments remain separate from the core roadmap.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or
  <http://opensource.org/licenses/MIT>)

at your option.
