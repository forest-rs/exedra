# Exedra Scene

Optional native [LayerStack](https://github.com/forest-rs/layerstack) scenes
over independently usable Exedra construction. LayerStack owns authored scene
structure, composition, transforms, materials, references and undo.
[`exedra_compile`](../exedra_compile/) owns geometry compilation, cache retention,
extraction and query snapshots. This crate lowers native inputs and publishes
native mesh properties. It has no geometry evaluator or separate geometry cache.

## Authoring and evaluation

Register `schemas::Domain::UsdGeom` and `register_schemas` in the same registry
builder, then compose a `LiveStage` with that registry. Register `UsdShade`
when authoring materials. `composition` and `schemas` expose the native APIs;
their path and token handles belong to the caller's store.

`define_primitive` prepares an ordinary native `Transaction`. Apply it through
the live stage, edit properties through native transactions, and apply native
inverse transactions for undo. The initial construction vocabulary is:

| Type | Parameters | Local geometry |
| --- | --- | --- |
| `ExedraBox` | `exedra:size`: varying `double3`, positive extents | Minimum corner at the origin |
| `ExedraCylinder` | `exedra:radius`, `exedra:height`: varying positive `double`; `exedra:segments`: uniform `uint`, at least three | Capped +Z cylinder, base center at the origin |

Both types derive from `Xform`. Dimensions have no implicit fallback. Composed
default values or explicitly requested native time samples lower to ordinary
immutable recipes. Placement remains native scene data, separate from geometry.

Supply an explicit `exedra_compile::PartCompiler` and `CompilePolicy` to
`SceneEvaluator::sync`. The same compiler can serve standalone geometry, Assembly
adapters or multiple native stages. Geometry contains no native token or path
handles. Each successful sync gets a fresh query scope while sharing immutable
cached geometry. Standalone and native callers can retain the same topology,
source maps, reports and extracted triangle buffers.

Publication creates conventional polygon `Mesh` children named
`__exedra_geometry`. Native properties include points, polygon counts and indices,
extent, face-varying normals, face regions and complete corner UVs when present.
Normal selection and UV projection use the shared compilation policy and mesh
math. Carried attributes and tangents remain available in the shared extraction
buffers; the initial native publisher does not author those extra streams.
Subdivision is disabled explicitly. Query native transforms for placed
measurements and native material bindings for appearance.

Run the complete author/edit/compile/publish/undo example:

```sh
cargo run -p native_scene
```

## Generated-layer contract

The caller owns the `LiveStage`, store, registry, source layers and layer order.
Supply an initially empty, exclusively owned generated local sublayer, with an
identity time offset. It must occur once and cannot be the root layer; it need
not be the weakest layer. Keep that layer and evaluator paired with the same
live stage and store. External generated-layer edits are refused.

Partners can share the stage and geometry compiler, each publishing to its own
generated layer and subtree. Exedra never installs a private stage or reorders
the caller's layers. Place appearance overrides above the relevant generated
layer when they should win composition.

Each explicit sync discovers supported inputs beneath the requested root. The
shared compiler reuses geometry by source content and compilation policy; native
publication writes only changed outputs and removes obsolete output specs.
References reuse identical content, and local dimension overrides specialize it.
Native instanceable references currently refuse generation because descendant
authoring requires a separate prototype-publication design. Reserved output paths
with unrelated authored content and construction beneath generated children are
refused.

Preflight or geometry failures preserve the previous generated layer. Compilation
and encoding caches may retain useful entries. Publication is one atomic native
transaction. Undo source edits and sync again; derived generated changes do not
belong in the source undo stack. Stronger appearance opinions survive regeneration.
Stronger indexed attributes or topology overrides remain the caller's
responsibility when topology changes.

## Work and lifetime

Discovery currently scans the subtree on every sync. Geometry compilation and
publication are content incremental. `SyncStats` separates compilation/reuse,
native polygon encoding, mesh writes/removals and point writes. Constructive
reports retain original evaluation measurements on cache hits.

The evaluator retains at most 128 native property encodings by default, with an
explicit entry budget available. `cached_payload_bytes` measures their numeric
buffers, excluding allocation overhead. `clear_cache` releases these encodings;
it does not clear the caller's geometry compiler. Use that compiler's capacity,
clear/eviction methods and counters to control geometry retention. Held geometry
snapshots use `Arc` and survive cache eviction or compiler destruction. Native
property buffers are shared with published specs. No scheduler or renderer
resources are owned.

## Migration

`sync` now accepts a caller-owned `PartCompiler` before the time argument and a
`CompilePolicy` instead of `EvalPolicy`. Use `CompilePolicy::from(evaluation)` to
retain evaluation settings. Native scenes no longer own a geometry cache.

Replace `SourceGeometry::evaluation()` access with `body(index)` or
`body_by_source(label)` for retained topology and queries, `report()` for complete
diagnostics, and `compiled()` for extraction buffers. `recipe()` still exposes
lowered intent. Body handles use the same query and stale-selection contracts as
standalone compilation. Refusal reports use `Arc`; hard compilation failures are
`SceneErrorKind::Compile` and retain shared-service diagnostic context.

Existing direct construction and Assembly compilation remain independent of
LayerStack. Enable `scene` for native composition; enable `compile` for standalone
compilation and queries, or `assembly` for structural workflows. The two primitive
schemas demonstrate the boundary; broader recipe operators and native-instance
publication remain future work.

The crate supports `no_std` with `alloc`. Scene dependencies remain pinned to the
revision used by the current Lightweald integration so native types can be shared.

## License

Apache-2.0 OR MIT
