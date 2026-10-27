# Exedra Scene

Optional native [LayerStack](https://github.com/forest-rs/layerstack) scenes over
independently usable Exedra construction. The host owns the stage, store, layer
order, edit targets and scheduling. LayerStack owns composition, retained input
evidence and validated polygon publication. Exedra lowers construction intent;
[`exedra_compile`](../exedra_compile/) owns geometry compilation, cache retention,
extraction and query snapshots. There is no second scene tree or geometry service.

## Authoring and evaluation

Register `schemas::Domain::UsdGeom`, `schemas::Domain::UsdProc` and
`register_schemas` in one registry builder, then compose a `LiveStage` with that
registry. Register `UsdShade` when authoring materials. `composition` and
`schemas` expose native APIs; their handles belong to the caller's store.

`define_primitive` prepares a native transaction. Apply it through the live stage,
edit properties through native transactions, and apply their inverses for undo.
The initial vocabulary consists of typed `GenerativeProcedural` recipes:

| Type | Parameters | System | Local geometry |
| --- | --- | --- | --- |
| `ExedraBox` | `primvars:exedra:size`: varying positive `double3` | `exedra:box` | Minimum corner at the origin |
| `ExedraCylinder` | `primvars:exedra:radius`, `primvars:exedra:height`: varying positive `double`; `primvars:exedra:segments`: uniform `uint`, at least three | `exedra:cylinder` | Capped +Z cylinder, base center at the origin |

Both types inherit native transforms through `GenerativeProcedural` and
`Boundable`. Dimensions have no fallback. `define_primitive` authors the system
explicitly; the registered types also supply its schema fallback. Other USD
applications need the Exedra schema definitions or a host-authored
`fallbackPrimTypes` mapping to interpret these custom types as procedurals.
Opening or composing a stage does not execute construction.

Create `SceneEvaluator::new(&store, root, edit_target)`, then supply the shared
`PartCompiler`, requested `Time` and complete `CompilePolicy` to `sync`.
LayerStack's `Procedural` retains composed input reads and an immutable lowered
`PreparedSource`. Missing inputs, samples, layer offsets and schema fallbacks use
native resolution. Equal consumed values reuse the recipe after masked edits.
The shared compiler hashes installed intent once and reuses geometry by content
and policy. It can also serve standalone callers, Assembly or other stages.
Transforms and materials remain separate from part-local geometry.

Successful snapshots retain immutable geometry inputs, topology, source maps,
applicable native/constructive evidence, extraction buffers and ordinary queries.
Native and standalone callers can share the same immutable compiled geometry,
without native handles in the compiler.

Run the complete author/edit/compile/publish/undo example:

```sh
cargo run -p native_scene
```

## Host generators and native sources

`SceneEvaluator::register_generator(schema, revision, evaluator)` accepts the
existing Layerstack `ProceduralEvaluator` contract with `Output = PreparedSource`
and `Error = SceneErrorKind`. Register the host's typed schema, derived from
`GenerativeProcedural`, in the caller's native registry. Read values through
`ProceduralInputs`; consumed values and missing inputs use Layerstack's retained
composition evidence. The evaluator returns geometry and performs no publication
side effects. Recipe, native and baked inputs use the same shared compiler.

Each registration creates a distinct immutable definition. Replacement resets
tracked source bindings even if the caller repeats a revision label. It retains
published-property manifests and last-good output; a failed replacement leaves
those opinions intact and can be corrected or retired. Revision fields are host
inspection labels, not hashes of arbitrary captured state. Re-register when
captured configuration, external resources or code change. `generators()`
inventories installed definitions; source snapshots record definition metadata
and can compare shared definition identity without retaining callback captures.
Exact type matches precede inherited matches; ambiguous inherited definitions
are refused. Built-in schemas keep their original meaning, including authored
cylinder segments.

This atomic batch does not order generators that consume generated meshes.
Reading a reserved generated-output subtree is refused. Layerstack's procedural
graph owns inter-producer ordering/resource workflows where those are required;
this adapter does not introduce another dependency or invalidation engine.

The [native scene example](../../examples/native_scene/) authors a tagged native
panel and repeated native instances, edits parameters/placement/appearance,
queries exact source measurements, picks generated polygons, and exports native
USD layers plus structured work evidence. Its consumer tests compare incremental
results against clean regeneration and retain old snapshots after eviction.

## Publication and ownership

The caller supplies an explicit native `EditTarget`. It may select the root
layer, a shared sublayer or a mapped reference/variant site. Exedra never creates
a private stage, installs layers or reorders the stack. The target layer must
exist. Multiple evaluators may share a layer when their output sites are distinct.

Publication uses `ValidatedMesh::prepare` to create or update conventional
polygon `Mesh` children named `__exedra_geometry`. Exedra owns the published
property manifest: points, polygon counts and indices, extent, subdivision mode,
face-varying `primvars:normals`, uniform `primvars:exedra:region`, native source
face IDs in uniform `primvars:exedra:nativeFace` when present, and complete
`primvars:st` UVs when available. Conversion transfers owned numeric buffers into
shared native storage and validates each immutable encoding once. Subdivision is
explicitly disabled. The boundable source also owns a derived `extent` at the
publication target; native bounds and instancing use it without a procedural
extent plugin. For native sources this encloses conservative analytic support
and emitted points, rounded outwards into the USD float extent; recipes and baked meshes use
the emitted geometry. The Mesh child always keeps its actual f32 bounds. Parent
extents prune descendant bounds according to native USD semantics. The native ID
array and `published_native_face` address the retained producer output in
published polygon order, not regrouped render triangle order. Stronger
partners may independently replace composed geometry/provenance. Source-native
queries do not claim to describe those overrides. Carried attributes and tangents
remain available in the compiler's extraction buffers; this publisher does not
author them yet.

Preparation inspects the actual authored site on every sync. It repairs missing
or replaced owned values without reevaluating unchanged construction or scanning
cached geometry. Other properties, material relationships, prim metadata,
children and opinions in other layers survive regeneration. Incompatible owned
property declarations refuse the complete update. Initial publication refuses
already occupied geometry properties; this evaluator does not infer ownership
of previous generated output after restarting. Keep the evaluator with its
publication manifests, or explicitly retire its output before creating a new one.

When a source disappears, only its owned properties are retired. A mesh
declaration created by this evaluator is removed only when no partner opinions
remain. Partners retain their own lifetime and cleanup responsibilities. Stronger
appearance opinions remain native USD data; maintaining indexed attributes after
topology changes belongs to their author.

Publish reusable construction at an authored asset source. Native instanceable
references consume its generated children; instance roots and descendants are
not independently evaluated or authored. Ordinary noninstanceable references
can specialize composed construction parameters locally. To vary construction
per native instance, author another asset source; use native transforms and
material overrides for placement and appearance. `PointInstancer` can also place
the published assets. Construction beneath reserved generated children is refused.

All sources are evaluated and prepared before one atomic transaction is applied.
Consumed input evidence is checked against the current scene before publication;
target-generation guards protect prepared authoring separately. Evaluation or
preparation failures preserve previously published opinions. Useful cache entries
may remain. Undo source edits and sync again; derived publication changes do not
belong in the source undo stack.

## Work and lifetime

The evaluator captures store affinity at construction. Moving the store is safe;
replacing either interner or supplying a foreign stage/store is refused before
interpreting handles. A replacement live stage in the same store refreshes
membership and retained input evidence.

Discovery traverses the requested subtree initially and after relevant structural
changes or change-history expiry. Ordinary parameter, transform and material edits
reuse membership. Current query evidence skips composed input resolution; equal
resolved inputs skip recipe lowering. Cached native snapshots skip polygon
validation and extent derivation. Publication still inspects owned properties,
including samples and metadata, to detect output replacement.

`SyncStats` separates discovery, source lowering, input resolution/reuse,
compilation/reuse, encoding/validation, publication-property checks, mesh updates,
retirement, point writes and source-bound writes. It includes input evidence
refreshed after publication changes ancestor composition records. Reports retain
original geometry evaluation measurements on compiler cache hits.
`SceneSnapshot::changes` exposes this
sync's native publication report; feed it and prior source-edit reports to native
retained consumers such as `BoundsCache`.

The encoding cache holds at most 128 entries by default; `with_capacity` selects
another budget. `cached_payload_bytes` estimates its numeric buffers, excluding
allocator overhead and owners retained by layers or snapshots. `clear_cache`
releases encodings, not input bindings or the caller's compiler. Reencoding after
eviction or release can rebind equal output to new buffer owners and author an
update. Use the shared compiler's own retention controls for geometry. LayerStack
exposes separate authored/composed change-history budgets and counters. History
loss refreshes discovery; current query evidence can still reuse recipes.

## Geometry sources

`SourceGeometry::input` retains the compiler's immutable `PreparedSource`.
`recipe` and `report` return `None` for sources without constructive evaluation;
`native` exposes an original `NativeSource` when present. Retained body queries
use the same public compiler APIs as standalone callers. Scene placement and
material opinions remain native stage data.

The adapter is `no_std` with `alloc`. Direct geometry, standalone compilation and
Assembly do not depend on Layerstack. Broader authored construction vocabularies,
sampled mesh publication and renderer resources remain separate host concerns.
The workspace pins one exact Layerstack revision; cooperating libraries must use
matching native types.

## License

Apache-2.0 OR MIT
