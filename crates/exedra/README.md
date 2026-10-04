# Exedra

Exedra is the application-facing facade for the Exedra modeling suite. It
offers stable, domain-named entry points while leaving each behavior in its
own owning crate.

The facade owns feature selection, namespace curation, and three root anchors:
`Mesh`, `Recipe`, and `Assembly`. It does not implement geometry, topology,
evaluation, assembly state, workflows, or conversion rules.

It is a curated entry point, not a mirror of every workspace crate. Specialist
support libraries, backend adapters, and domain-specific layers remain direct
dependencies until they earn a stable application-facing namespace.

## Feature selection

`mesh` is always available. The default feature set is suited to a typical
native application:

```toml
[dependencies]
exedra = "0.1"
```

It enables `std`, `compile`, and `mesh_ops`; `compile` also enables
`constructive`. `assembly` and `scene` select the same compilation service.
`assembly`, `edit`, `analytic`, `isosurface`, `primitives`, `scene`, `gltf`, and
`serde` interchange are opt-in. `gltf` and `serde` also select `std`. For a
`no_std` application, disable defaults and select `libm` plus the needed heads:

```toml
[dependencies]
exedra = { version = "0.1", default-features = false, features = ["libm", "constructive"] }
```

Direct modeling and queries use `mesh_ops`. Enable `edit` only when you need
mesh command execution, preview and runtime reports. Native domains expose
their own operations and conversions directly.

## Namespaces

- `exedra::mesh`: mesh topology, attributes, construction, editing, and extraction.
- `exedra::constructive`: immutable recipes, profiles, evaluation, and tessellation.
- `exedra::compile`: independent compilation, extraction and retained geometry queries.
- `exedra::assembly`: parts, instances, material bindings, compilation, and flattening.
- `exedra::mesh_ops`: direct mesh modeling, geometric queries and correspondence.
- `exedra::edit`: opt-in mesh command execution, preview and reporting.
- `exedra::primitives`: deterministic primitive mesh generators and selections.
- `exedra::analytic`: planar analytic topology and tessellation.
- `exedra::isosurface`: implicit fields and surface extraction.
- `exedra::gltf`: glTF and GLB export.
- `exedra::scene`: optional native LayerStack composition, typed construction
  and explicit mesh regeneration.

The primitive, analytic, isosurface, and export namespaces are opt-in. Use the
namespace that owns an operation instead of expecting the facade to add
behavior between domains.

## Native scenes

Enable `scene` to author construction on a native LayerStack stage. It also
enables `compile` and `constructive`; the evaluator lowers composed parameters
through the shared caller-owned compiler and publishes conventional polygon
meshes. Transforms, references, appearance and source undo use native scene APIs.

```toml
[dependencies]
exedra = { version = "0.1", features = ["scene"] }
```

Standalone compilation and extraction use `compile` without an Assembly or
LayerStack. Existing Assembly compilation and export APIs still work; defaults
now select `compile` instead of `assembly`. Enable `assembly` explicitly when
using its root anchor or namespace. `gltf` selects independent compilation; `serde` selects recipe interchange.
Neither selects Assembly or LayerStack. Existing Assembly export callers select
`assembly,gltf`; Assembly interchange callers select `assembly,serde`. See the
[scene contract](../exedra_scene/) and
[native scene example](../../examples/native_scene/) for the initial box and
cylinder recipes, shared-stage publication and current limits.

The [caller-owned export example](../../examples/caller_owned_export/) serializes
a recipe, compiles it, resolves a hierarchy and materials, and exports GLB using
only `gltf,serde`. The exporter owns format records and encoding; callers own
authored structure and appearance resolution.

## Mixed geometry sources

```rust
use exedra::{Mesh, Recipe};
use exedra::compile::{CompilePolicy, PartCompiler, PartInput, PartSource};
use exedra::constructive::ir::{NodeKind, Placement3, PrimitiveSpec, RecipeBuilder};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let mut builder = RecipeBuilder::new();
let root = builder.add(NodeKind::Primitive {
    spec: PrimitiveSpec::Box { size: [1.0, 1.0, 1.0] },
    placement: Placement3::IDENTITY,
})?;
let recipe: Recipe = builder.finish(root)?;

let body = PartSource::Recipe(recipe);
let detail = PartSource::Baked(Mesh::new());
let inputs = [
    PartInput::new("body", &body),
    PartInput::new("detail", &detail),
];
let geometry = PartCompiler::new().compile_snapshot(&inputs, &CompilePolicy::default())?;
assert_eq!(geometry.compiled().parts().len(), 2);
# Ok(())
# }
```

For the boundary and extension rule, see the
[facade boundary decision](https://github.com/forest-rs/exedra/blob/main/crates/exedra/docs/adr-0001-facade-boundary.md).

## Status

The 0.1 API is intentionally early. The facade identifies the supported crate
boundaries and common flow; it does not imply universal Boolean, analytic, or
implicit-surface coverage. Domain crates document their current refusal and
fidelity contracts.

## License

Apache-2.0 OR MIT
