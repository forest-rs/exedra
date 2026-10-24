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
`constructive`. `assembly` selects the same compilation service.
`assembly`, `edit`, `analytic`, `isosurface`, `primitives`, `gltf`, and `serde`
interchange are opt-in. `gltf` and `serde` also select `std`. For a
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

The primitive, analytic, isosurface, and export namespaces are opt-in. Use the
namespace that owns an operation instead of expecting the facade to add
behavior between domains.

## Compilation and structure

Standalone compilation and extraction use `compile` without an `Assembly`.
Existing Assembly compilation and export APIs still work as adapters over the
same service; defaults now select `compile` instead of `assembly`. Enable
`assembly` explicitly when using its root anchor or namespace:

```toml
[dependencies]
exedra = { version = "0.1", features = ["assembly"] }
```

`gltf` and `serde` still select `assembly`.

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
