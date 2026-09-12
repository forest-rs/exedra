# `exedra_assembly`

Structure head for the Exedra geometry stack: part definitions (constructive
recipes or baked meshes), instance trees with stable string-key identity,
material slot binding, content-addressed part compilation, and a flat
`RenderList` seam for renderers and exporters. Compiled parts expose
once-per-part, part-local geometry accounting; render lists expose placed,
world-space accounting with instance multiplicity.

```rust
use exedra_assembly::{Assembly, CompilePolicy, PartCompiler, flatten};
use exedra_constructive::{
    ir::{NodeKind, Placement3, PrimitiveSpec, RecipeBuilder},
};

let mut builder = RecipeBuilder::new();
let root = builder
    .add(NodeKind::Primitive {
        spec: PrimitiveSpec::Box { size: [1.0; 3] },
        placement: Placement3::IDENTITY,
    })
    .expect("valid box");
let recipe = builder.finish(root).expect("valid recipe");

let mut assembly = Assembly::new();
let part = assembly.add_recipe_part("box", recipe).expect("unique part key");
assembly
    .add_instance(None, "left", part, Placement3::IDENTITY)
    .expect("unique root key");
assembly
    .add_instance(None, "right", part, Placement3::translate(2.0, 0.0, 0.0))
    .expect("unique root key");

let compiled = PartCompiler::new()
    .compile_parts(&assembly, &CompilePolicy::default())
    .expect("part compiles");
let render_list = flatten(&assembly, &compiled);
assert_eq!(compiled.part(part).unwrap().triangle_count(), 12);
assert_eq!(render_list.triangle_count(), 24);
```

## Main types

- `Assembly` owns part definitions, the instance tree, stable paths, material
  bindings, and opaque instance metadata.
- `PartCompiler` evaluates each distinct part and reuses content-addressed
  results. `CompiledPart` accounting is once per part in part-local space.
- `flatten` resolves placements and bindings into a `RenderList`.
  `RenderList` accounting includes instance multiplicity in world space.

The crate accepts both recipe-backed and baked-mesh parts. It owns their
placement and identity, not their geometry algorithms or rendering.

The optional `serde` feature exposes host-side interchange. Core assembly and
compilation remain `no_std` with `alloc`.

## Composing assemblies

Use `destination.append(&source, "west", placement)` to copy the source's
instance trees into another assembly. The placement applies once, at each
source root. Referenced parts retain their slot order, region mappings and
default materials; instances retain bindings, metadata and part sharing.
Unused part definitions are omitted. The returned `AppendMap` translates
source part and instance handles into destination handles.

`append_selected` takes an instance predicate. Omitting an instance prunes its
whole subtree, and only parts used by retained instances are copied. A key
collision or invalid placement leaves the destination unchanged.

When replacing caller-written copy loops, note that only **part keys and root
instance keys** receive the prefix (`west-frame`). Descendant keys retain their
source spelling, so `frame/insert` becomes `west-frame/insert`. Separate appends
do not intern part definitions; `PartCompiler` still reuses identical geometry
by content. Material keys remain opaque caller-owned strings.

## Compile policy migration

`PartCompiler::compile_parts` and `compile::policy_fingerprint` now accept
`CompilePolicy` in place of `EvalPolicy`. Use `CompilePolicy::default()` for
the existing generated-normal behavior, or wrap an evaluation policy with
`CompilePolicy::from(evaluation)`.

To preserve imported corner normals alongside generated geometry, set
`CompilePolicy::normals` to `NormalsSource::CustomOrDerived`. Missing overrides
use derived normals, including partially authored faces. `CustomOnly` emits
zero normals for missing overrides and is intended for callers that provide
complete coverage. `Derived`, the default, ignores overrides. The policy
applies to both recipe and baked parts and participates in compilation cache
identity.

Compiled baked-part fingerprints now include built-in render attributes.
Persisted compilation content and policy fingerprints must be recomputed.
The v1 JSON interchange still represents baked positions and faces only;
its structural `assembly_fingerprint` is not a rendering cache key.

For mixed-material CSG, `CompiledBody::material_slot` moves to
`RegionRange::material_slot`. A geometric region may now have several ranges,
one for each effective slot. Ranges sort by `(region, slot)`, unassigned first,
and preserve triangle order within each pair. `flatten` resolves each range's
authored slot before considering the part's region/default mappings. Consumers
must iterate all ranges rather than treating a region ID as a unique range key.

See the [structure-head scope](https://github.com/forest-rs/exedra/blob/main/crates/exedra_assembly/docs/adr-0001-structure-head-scope.md)
and `exedra_constructive` for the geometry side of the boundary.

## License

Apache-2.0 OR MIT
