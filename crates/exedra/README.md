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

It enables `std`, `assembly`, and `ops`; `assembly` also enables
`constructive` because assemblies publicly admit recipe-backed parts.
`analytic`, `isosurface`, `primitives`, `gltf`, and `serde` interchange are
opt-in. `gltf` and `serde` also select `std`. For a `no_std` application,
disable defaults and select `libm` plus the needed heads:

```toml
[dependencies]
exedra = { version = "0.1", default-features = false, features = ["libm", "constructive"] }
```

When `ops` and any of `analytic`, `constructive`, or `assembly` are both
enabled, the matching operation adapter is enabled as well.

## Namespaces

- `exedra::mesh`: mesh topology, attributes, construction, editing, and extraction.
- `exedra::constructive`: immutable recipes, profiles, evaluation, and tessellation.
- `exedra::assembly`: parts, instances, material bindings, compilation, and flattening.
- `exedra::ops`: workflow-oriented mesh operations and enabled adapters.
- `exedra::primitives`: deterministic primitive mesh generators and selections.
- `exedra::analytic`: planar analytic topology and tessellation.
- `exedra::isosurface`: implicit fields and surface extraction.
- `exedra::gltf`: glTF and GLB export.

The primitive, analytic, isosurface, and export namespaces are opt-in. Use the
namespace that owns an operation instead of expecting the facade to add
behavior between domains.

## Mixed geometry sources

```rust
use exedra::{Assembly, Mesh, Recipe};
use exedra::constructive::ir::{NodeKind, Placement3, PrimitiveSpec, RecipeBuilder};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let mut builder = RecipeBuilder::new();
let root = builder.add(NodeKind::Primitive {
    spec: PrimitiveSpec::Box { size: [1.0, 1.0, 1.0] },
    placement: Placement3::IDENTITY,
})?;
let recipe: Recipe = builder.finish(root)?;

let mut assembly = Assembly::new();
assembly.add_recipe_part("body", recipe)?;
assembly.add_baked_part("detail", Mesh::new(), &[])?;
# Ok(())
# }
```

For the boundary and extension rule, see
[`adr-0001-facade-boundary.md`](docs/adr-0001-facade-boundary.md).
