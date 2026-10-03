# Exedra Scene

Optional native [LayerStack](https://github.com/forest-rs/layerstack) scenes
over independently usable Exedra construction. LayerStack owns authored scene
structure, composition, transforms, materials, references and undo. Exedra owns
geometry intent, deterministic evaluation, source correspondence and reports.
There is no second scene tree and no dependency from the geometry crates back
to this integration.

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

`SceneEvaluator::sync` evaluates that intent and publishes conventional polygon
`Mesh` children named `__exedra_geometry`. A successful snapshot retains each
source's recipe, evaluated mesh, source map and report for inspection. Native
mesh properties include points, polygon counts and indices, extent, face-varying
normals, face regions and complete corner UVs when present. Polygon subdivision
is disabled explicitly. Geometry stays source-local; query native transforms
for placed measurements and native bindings for appearance.

Run the complete author/edit/evaluate/undo example from the workspace:

```sh
cargo run -p native_scene
```

## Generated-layer contract

Supply an empty, exclusively owned generated layer as the weakest local
sublayer, with identity time offset. Keep that layer and evaluator paired with
the same live stage and store. External generated-layer edits are refused.
Author source intent and appearance overrides in stronger layers.

Each explicit sync discovers supported inputs beneath the requested root. It
shares geometry by recipe content and evaluation policy, writes only changed
outputs and removes outputs whose sources disappeared. References reuse
identical content; local dimension overrides specialize it. Native instanceable
references currently refuse generation because descendant authoring requires
a separate prototype-publication design. Reserved output paths with unrelated
authored content and construction beneath generated children are refused.

Preflight or geometry failures preserve the previous generated layer; useful
cache entries may remain. Publication is one atomic native transaction. Undo
source edits and sync again to restore their geometry. Generated changes are
derived data and do not belong in the source undo stack.

Stronger appearance opinions survive regeneration. Stronger indexed attributes
or topology overrides remain the caller's responsibility when topology changes.
This integration does not validate or repair those overrides.

## Work and lifetime

Discovery currently scans the subtree on every sync. Geometry evaluation and
publication are content incremental. `SyncStats` counts discovery, evaluations,
reuse, mesh writes/removals and point writes. Constructive reports retain the
original evaluation measurements on cache hits; they are not current-sync work.

The cache defaults to 128 content/policy pairs and supports an explicit entry
budget. Numeric payload bytes are measurable, but exclude topology, reports,
maps and allocation overhead. Snapshots share immutable geometry with `Rc` and
may retain it after eviction or evaluator destruction. Native property buffers
are shared with published specs. No scheduler or renderer resources are owned.

## Adoption

This is an additive scene-authoring experiment. Existing `Recipe`, `evaluate`,
`Assembly`, part compilation, extraction and glTF export APIs remain unchanged
and do not require LayerStack. Enable the facade's `scene` feature or depend on
`exedra_scene` directly when a caller needs native scene composition.

The two primitive schemas demonstrate the boundary; they are not a complete
recipe vocabulary, arbitrary asset importer or replacement for every assembly
workflow. Recipe operators and native-instance publication need further work.
The crate supports `no_std` with `alloc`; its explicit `std` feature forwards
host support to dependencies. Scene dependencies are pinned to the revision
used by the current Lightweald integration so native types can be shared.

## License

Apache-2.0 OR MIT
