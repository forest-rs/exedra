# External Frontend Integration Guide

How an external geometry frontend targets `exedra_constructive`. The
contract has two equal surfaces: the Rust API (this crate as a
dependency) and the `exedra-recipe-v1` JSON interchange (emit recipes from
any language). Both rebuild through the same validating builder, and
round-trip **content fingerprints** prove the wire preserved intent.

## The pipeline, end to end

```text
your spec data ──▶ your compiler (exact arithmetic, spec tables, policies)
                        │  builds
                        ▼
                RecipeBuilder ──finish()──▶ Recipe (frozen, fingerprinted)
                        │  evaluate() / cambium::constructive::apply_recipe()
                        ▼
        Evaluation { bodies: [mesh + SourceMap], report: GeometryReport }
```

Your compiler owns: parsing, exact-decimal closure arithmetic (convert to
f64 at this boundary — i64 thousandths-of-mm convert exactly), spec-issue
tables, curve-policy realizations, and all spec vocabulary. This crate
owns: validated profiles, deterministic tessellation, provenance, and
honest fidelity reporting. Nothing spec-specific crosses the boundary —
your identifiers travel as opaque interned strings.

## Building a recipe

```rust
use exedra_constructive::builders;
use exedra_constructive::evaluate::evaluate;
use exedra_constructive::ir::{CapMode, NodeKind, Placement3, RecipeBuilder};
use exedra_constructive::tessellate::EvalPolicy;

let mut b = RecipeBuilder::new();

// Opaque identity: your spec's names, never parsed here.
let source = b.source_ref("yourspec:part/7#body");
let front = b.material_slot("front");

let profile = b.add_profile(builders::rounded_rect(600.0, 400.0, 40.0)?);
let node = b
    .with_source(source)
    .with_material(front)
    .add(NodeKind::Extrude {
        profile,
        placement: Placement3::IDENTITY,
        height: 720.0,
        caps: CapMode::Both,
    })?;
let recipe = b.finish(node)?;

let result = evaluate(&recipe, &EvalPolicy::default())?;
assert_eq!(result.bodies.len(), 1);
# Ok::<(), alloc::boxed::Box<dyn core::error::Error>>(())
```

Everything validates at insertion with typed errors — hostile input never
panics (fuzz-enforced). Wrong winding is an error with `Loop2::reversed`
as the deliberate fix; orientation is never changed implicitly.

## Mirroring a recipe for assembly

Use immutable recipe composition when an assembly needs a handed counterpart:

```rust
use exedra_constructive::ir::Plane3;

let mirrored = recipe.mirrored(Plane3 {
    normal: [1.0, 0.0, 0.0],
    distance: 0.0,
})?;
# Ok::<(), alloc::boxed::Box<dyn core::error::Error>>(())
```

The returned recipe keeps every existing node and intern-table id, then wraps
the old root in one unbound `Mirror` node. Its fingerprint is deterministic
and distinct; the original recipe is unchanged. Register it as a distinct
recipe-backed part and place both parts with proper-rigid assembly transforms.
Do not encode the handedness as a negative-determinant instance placement or
evaluate and bake the recipe into an opaque mesh first: those alternatives
either require winding repair at the wrong layer or discard constructive
identity, policy independence, and provenance.

## Identity: fingerprints vs source references

| Identity | Assigned by | Changes when | Use for |
|---|---|---|---|
| `Fingerprint` (128-bit) | this crate, content-addressed | any content or schema change | caching, plan staleness, wire verification |
| `SourceRef` string | your compiler | never (your choice) | provenance continuity across edits |

`EVAL_SCHEMA_VERSION` is folded into every fingerprint: a crate upgrade
that changes evaluation output invalidates caches explicitly.

## Spec ambiguity: policies and issues

When your spec underdetermines a curve, realize it under a **named
policy** and say so:

- `let p = b.curve_policy("yourspec.front-transition@1");` then
  `Seg2::policy(to, p, SegKind::Arc { bulge })`.
- Evaluation reports the node `Fidelity::PolicyDefined(p)` and attributes
  every `(node, policy)` pair in `report.policy_curves`.

When an authored source contradicts itself, cite the issue:

- `b.with_issue(b.source_ref("yourspec.issue.nonclosing-profile"))` before
  `add`. The node still builds (viewers need something on screen) but
  reports `Fidelity::Conflicted(issue)` — build, but confess.

CSG nodes use the mesh Boolean pipeline for handled configurations. A refused
configuration reports `Fidelity::EnvelopeOnly`, an `eval.csg.unsupported`
diagnostic, the pipeline diagnostics, and operand-union bounds instead of
invented geometry.

## Regions and materials

- `FACE_REGION` (u32, on the mesh) uses a stable documented mapping:
  `0` start cap, `1` end cap, `2 + k` for global source segment `k`
  (outer-loop segments first, then each hole's, in order).
- Per-segment identity beyond regions rides `SegTag` (your u32) through
  the `SourceMap`: face → `Feature::Wall { loop_index, seg }` → your tag.
- Material slots are interned names bound per node (`with_material`),
  inherited semantics and per-instance rebinding arrive with the assembly
  head; rebinding never re-tessellates.

## Provenance queries

`body.source_map` answers both directions in O(1)/O(log n):
`face_feature(face)`, `vertex_feature(vertex)`, `faces_for(feature)`.
Maps are pinned to the mesh revision — editing the mesh afterwards makes
lookups fail typed (`StaleSourceMap`), never silently. After rigid
instancing the map is re-pinned automatically.

## The wire format

Enable the `serde` feature and use `interchange::{to_dto, from_dto}`.
Policy: additive-only within v1; unknown *fields* are ignored, unknown
node *kinds* hard-error (recipes are executable content); floats survive
bit-exactly; curve segments are explicit records independent of kurbo.
The frozen corpus (`goldens/recipe_v1.frozen.json`) is the schema's
compatibility test — vendor it in your repository as an integration
anchor, along with any gallery fixtures you rely on.

## Determinism contract

Identical recipe bits + identical policy + identical crate version ⇒
bit-identical meshes on every platform (trig is libm-always; predicates
are exact; no hash-order iteration). Signatures
(`exedra_testkit::trimesh_signature`) are safe golden material.

## Checklists

Before shipping a compiler against this crate:

- [ ] All spec vocabulary stays in your repository (names here are opaque).
- [ ] Exact spec arithmetic happens before the f64 boundary.
- [ ] Underspecified curves use `curve_policy` + `Seg2::policy`.
- [ ] Contradictory shapes use `with_issue` and still build.
- [ ] Wire consumers verify fingerprints after `from_dto`.
- [ ] Golden tests pin your shapes with mesh goldens + report snapshots.
