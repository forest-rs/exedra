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

See the [structure-head scope](https://github.com/forest-rs/exedra/blob/main/crates/exedra_assembly/docs/adr-0001-structure-head-scope.md)
and `exedra_constructive` for the geometry side of the boundary.

## License

Apache-2.0 OR MIT
