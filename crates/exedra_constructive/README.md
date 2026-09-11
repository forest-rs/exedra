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

## Materials through Booleans

Slots are opaque recipe-local IDs. CSG preserves the slot of each surviving
source face independently of its geometric `FACE_REGION`:

- **Difference:** retained minuend surfaces keep the minuend's assignment;
  exposed cutter surfaces keep the cutter's assignment, with winding reversed.
  Several cutters are unioned in operand order before subtraction.
- **Union:** exterior pieces keep their source assignments; internal surfaces
  disappear. Where equally oriented surfaces coincide, the earlier operand wins.
- **Intersection:** each retained boundary piece keeps its source assignment.
  Equally oriented coincident surfaces again use the earlier operand.
- Oppositely oriented contact surfaces disappear in union/intersection;
  difference retains the minuend's touching surface and its assignment.

CSG does not create a separate cap material: a newly exposed closing surface
comes from an operand face, including an extrusion's cap, and carries that
face's slot. Groups used as operands are unioned in child order. Nested CSG,
mirrors, transforms, instances and mesh stretch preserve face assignments;
stretch bands inherit the surface they extend. Unsupported geometry still
produces typed diagnostics and an envelope-only result.

Read `PlacedBody::material_for_face(face)` for the effective slot. The sparse
`TessellatedBody::face_materials` map stores authored face overrides;
`PlacedBody::material` is the occurrence's default for missing entries.
An unassigned surface inherits the nearest ancestor assignment, or remains
unassigned for assembly region/default binding. Ancestor defaults are resolved
after geometry caching, so changing them cannot reuse another occurrence's slot.

`RecipeBuilder::material_slot` interns equal names in first-registration order.
CSG neither renumbers that table nor merges distinct IDs; unused declarations
remain in the recipe. Only surviving faces have output assignments. Assembly
compilation emits ranges in `(region, slot)` order, unassigned first. GLB export
deduplicates resolved material keys in first-use order; callers may deliberately
bind several slots to the same key.

Migration: callers that read `PlacedBody::material` as a whole-body assignment
must resolve each face instead. Evaluation schema 16 invalidates cached
fingerprints because mixed-slot CSG now emits geometry. Exporting a bare `Mesh`
discards this constructive slot map; use assembly compilation to retain it.
`SourceMap::new` and `face_features()` use live element iteration order, not
arena slot indices. Lookup by ID handles deleted slots through a sorted live-ID
table (O(log n)); use the lookup methods rather than indexing the feature slice.

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

## Convex edge finishes

`NodeKind::EdgeFinish` applies a `RoundPolicy` to one completely evaluated
child body. Use `EdgeSelection::SharpEdges` for every authored sharp edge, or
`RegionBoundaries(vec![[a, b]])` for an explicit boundary between stable face
regions. Region pairs are sorted and deduplicated before fingerprinting.
Each pair must identify one connected boundary with one source feature per
side; missing and ambiguous targets are errors.

After a Boolean, use `OperandBoundaries` to qualify each region by the operand
that produced it. A box panel and box cutter can keep their ordinary region
numbers. For the recessed-door example, operand 0 is the panel and operand 1
is the cutter:

```rust
use exedra_constructive::edge_finish::{EdgeSelection, OperandRegion};

let rim = EdgeSelection::OperandBoundaries(
    [1, 2, 5, 6].map(|region| [
        OperandRegion { operand: 0, region: 4 }, // panel front (-Y)
        OperandRegion { operand: 1, region },   // cutter wall
    ]).to_vec(),
);
```

Operand indices follow the producing CSG node's declared `operands` list,
including operands that contributed no faces. They are local to that Boolean;
a later CSG operation assigns new operand indices. Transforms and edge finishes
retain the attribution. This does not select leaf nodes inside grouped or
nested operands. If one qualified pair names disconnected boundaries, selection
still refuses; separate cutters should be separate operands when they need
separate selection. Equal region numbers on different operands are valid.

A rail with all sharp edges filleted uses the whole-body selector:

```rust
use exedra_constructive::edge_finish::{EdgeSelection, RoundPolicy};
use exedra_constructive::ir::{NodeKind, Placement3, PrimitiveSpec, RecipeBuilder};

let mut recipe = RecipeBuilder::new();
let rail = recipe.add(NodeKind::Primitive {
    spec: PrimitiveSpec::Box { size: [0.09, 0.2, 2.0] },
    placement: Placement3::IDENTITY,
}).unwrap();
let mut policy = RoundPolicy::fillet(0.006);
policy.chord_tolerance = 0.00002;
let finished = recipe.add(NodeKind::EdgeFinish {
    child: rail,
    selection: EdgeSelection::SharpEdges,
    policy,
}).unwrap();
let recipe = recipe.finish(finished).unwrap();
```

Dimensions use the recipe's units. Finishing happens in the child's coordinate
space before outer transforms or assembly placement. A scale outside the
finish scales the finished shape; a scale inside changes the input geometry
before the radius is applied. Finished bodies are cached independently of
ancestor material defaults. All target and policy fields participate in the
recipe fingerprint.

Replacement faces retain their source feature, region and material slot. New
bands and corner patches inherit the first contributing face in ascending
input-ID order; an explicit `RoundPolicy::region` overrides only their region.
Opaque slots still resolve through the existing per-face material API. Source
features refer to the child body, while the finish node can carry its own
`with_source` identity. Generated vertices take the first incident source
feature in feature order; surviving vertices retain their previous feature.

Fillet normals follow the sampled circular/spherical surfaces; chamfers and
transverse end rims remain hard. Set assembly `CompilePolicy::normals` to
`NormalsSource::CustomOrDerived`. Surviving corners retain their exact UVs, and
trimmed faces interpolate their source charts. Bands and patches project onto
the first source face's UV chart, matching material ownership; chart boundaries
are marked as seams. Projection stretches textures toward perpendicular
tangencies and can fold beyond them; it does not provide an arc-length unwrap.
Incomplete or non-finite charts supply no new UVs; untextured inputs remain untextured.
`finish_edges` exposes the same operation for
a standalone tessellated body and returns typed failures without changing it.

The supported examples include a box rail with all convex edges filleted, a
box-like foot with a selected exposed edge chamfered, and the square rim of a
Boolean-cut door recess. A rounded 2D profile
extruded along the rail remains a different operation: its end perimeters
stay sharp. Concave targets, oversized radii, boundary edges, affected
non-planar faces and unsupported junctions are refused. Open chain ends must
meet one end face; CSG can split that face and make the end unsupported.
Closed rims with unambiguous operand/region pairs avoid these selection and
end-face limitations. For a square rim, set `policy.max_tangent_turn` to `FRAC_PI_2`;
the default remains 0.7 radians. Consecutive edges with a shared planar flank
and equal dihedral angles receive exact miters. Fillet normals retain the
crease between adjoining cylinders, and automatic band counts also bound
chord error along the elliptical miter seam. Operand-qualified selection avoids
region collisions without changing the supported rounding geometry. General
concave-edge blends and self-intersecting offset strips remain outside this scope.

Migration for qualified selection: existing `SharpEdges` and `RegionBoundaries`
calls keep their behavior, encoding, and fingerprints; evaluation schema remains
21. Exhaustive matches must handle `EdgeSelection::OperandBoundaries` and
`EdgeFinishError::AmbiguousOperandSelection`. Rust DTO callers wrap numeric
boundary lists in `EdgeBoundariesDto::Regions`; qualified lists use `Operands`.
The JSON `boundaries` field retains numeric pairs or accepts pairs of
`{"operand": 1, "region": 5}` objects. Older readers reject these objects;
they cannot silently fall back to finishing every sharp edge. Constructive
text uses the explicit `operand_boundaries` selector.

Migration: evaluation schema 21 invalidates cached output for planar-flank
miters, including existing polygonal drill rims. It follows schema 20's UV
transfer change, which removed the unconditional `eval.edge_finish.uv_deferred`
note. Regenerate
persisted constructive text with the current schema. Existing recipes need no
field changes. Callers that supply their own unwrap can continue overwriting
the finished faces' UVs; new faces no longer necessarily have missing UVs.
The runnable comparison is
`cargo run -p material_gallery --bin edge_finishes -- target/edge-finishes`.
Exhaustive `NodeKindDto` matches must handle its new `EdgeFinish` variant.

## License

Apache-2.0 OR MIT
