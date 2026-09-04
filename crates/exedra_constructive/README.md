# exedra_constructive

Constructive geometry head: an immutable, content-addressed recipe IR with
deterministic tessellation into Exedra meshes.

This crate is a geometry head beside Exedra Ops: a *compiler target* for
pre-mesh construction. It keeps recipe evaluation native until an explicit
conversion produces a mesh.
External geometry frontends build recipes from kurbo-backed 2D profiles and
constructive bodies (declared boxes and cylinders, extrude, revolve, loft,
sweep, CSG, transforms, and instances); evaluation tessellates them into
`exedra_mesh::Mesh` values carrying a full provenance source map, semantic region
and material slots, and an honest fidelity report.

```rust
use exedra_constructive::{
    evaluate::{Fidelity, evaluate},
    ir::{NodeKind, Placement3, PrimitiveSpec, RecipeBuilder},
    tessellate::EvalPolicy,
};

let mut builder = RecipeBuilder::new();
let root = builder
    .add(NodeKind::Primitive {
        spec: PrimitiveSpec::Box {
            size: [1.0, 2.0, 0.5],
        },
        placement: Placement3::IDENTITY,
    })
    .expect("valid box");
let recipe = builder.finish(root).expect("valid recipe");

let result = evaluate(&recipe, &EvalPolicy::default()).expect("evaluation succeeds");
assert_eq!(result.bodies.len(), 1);
assert_eq!(result.report.fidelity_of(root), Some(Fidelity::Exact));
assert!(result.bodies[0].body.mesh.validate_deep().is_empty());
```

Start with `RecipeBuilder` and `NodeKind` to author a recipe, then call
`evaluate` with an explicit `EvalPolicy`. The result contains placed bodies and
a `GeometryReport`; callers should inspect both rather than treating emitted
geometry alone as success. Use the `serde` feature for host-side interchange.

Design commitments (see the [constructive-domain scope](https://github.com/forest-rs/exedra/blob/main/crates/exedra_constructive/docs/adr-0001-constructive-domain-scope.md)):

- **f64 construction, f32 emission.** All construction and evaluation happen
  in f64 (kurbo-native); the single narrowing to `[f32; 3]` happens at mesh
  emission and is documented.
- **Determinism as a contract.** Evaluation trig always routes through
  `libm` — even in std builds — and arc discretization is owned here rather
  than delegated to kurbo's trig, so identical recipes produce bit-identical
  meshes on every platform. Content hashes incorporate an evaluation schema
  version so kurbo upgrades invalidate caches explicitly, never silently.
- **Closed by construction.** Profiles are endpoint-chained cyclic segment
  lists with bulge-parameterized arcs: both endpoints of every segment are
  stored exactly, so loop closure is structural, not tolerance-based.
- **Opaque source identity.** Frontends attach their own source references,
  policy ids, and issue ids; this crate round-trips them through source maps
  and reports without ever parsing them. No source-domain vocabulary lives
  here.
- **Structural mirrors.** `Recipe::mirrored` immutably wraps a frozen recipe
  in a constructive mirror. Existing ids and provenance remain stable, while
  assembly placements stay proper-rigid and mesh winding is repaired during
  constructive evaluation.

## License

Apache-2.0 OR MIT
