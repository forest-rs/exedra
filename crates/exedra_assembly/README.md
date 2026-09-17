# `exedra_assembly`

Structure head for the Exedra geometry stack: part definitions (constructive
recipes or baked meshes), instance trees with stable string-key identity,
material slot binding, content-addressed part compilation, and a flat
`RenderList` seam for drawables with exact placed bounds. Compiled parts expose
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

## Spatial frames

`Assembly::add_frame(parent, key, placement)` adds a real spatial frame without
registering geometry. Frames can parent other frames or part instances, and
carry the same stable paths and metadata. A part that evaluates to empty geometry
remains a part instance; it is not converted into a frame.

`Instance::part()` now returns `Option<PartId>`. Geometry consumers must handle
`None`; material binding on a frame returns `AssemblyError::NoPart`. Use
`add_instance` unchanged for geometry-bearing occurrences. `flatten` visits
children of frames and emits only their geometry.

Assembly interchange requires every instance to supply `part`: an integer
associates geometry and explicit `null` denotes a frame. Omitting the field is
an error. Structural assembly fingerprints distinguish frames from
geometry-bearing instances; geometry and compilation policy fingerprints remain
independent of the instance tree.

## Composing assemblies

Use `destination.append(None, &source, "west", placement)` to copy the source's
instance trees into another assembly. Both append methods now take an explicit
parent as their first argument: pass `None` for the previous root-level behavior,
or `Some(frame)` to attach a composed assembly below an existing instance.
The placement applies once, at each
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

## Retaining compiled geometry

Keep one `PartCompiler` across evaluated snapshots, and rebuild lightweight
assembly records with the new parent/local placements and material bindings.
`compile_parts` uses source-content and policy fingerprints, so new local part
handles do not prevent reuse. Equal content shares `Arc<CompiledPart>` and its
geometry report. Changed dimensions compile only new content; a changed policy
uses a separate cache entry.

`CompiledParts` and `PartCompiler` are `Send + Sync`. A worker can retain the
compiler and publish snapshots to other threads. `CompiledParts::clone` copies
only the handle tables; geometry and reports retain shared `Arc` ownership.
Compiler mutation still requires exclusive access. This uses `alloc::sync::Arc`
and requires pointer-width atomics, available on the supported native and
`wasm32-unknown-unknown` targets; `std` is not required for the ownership model.

`CompiledParts::part` and `CompiledParts::parts` now expose `Arc<CompiledPart>`
handles, and `CompileError::NoGeometry::report` uses `Arc<GeometryReport>`.
Replace explicit `Rc::clone`/`Rc::ptr_eq` calls with their `Arc` equivalents.
`CompiledParts::report` continues to return a borrowed `GeometryReport`.

`CompiledParts::validate_for(&assembly)` rejects mismatched part counts and
source content. It accepts pose, metadata and material edits. The snapshot's
`policy_fingerprint()` identifies its evaluation/extraction settings. Content
checking includes hashing baked mesh attributes, so use it at export or snapshot
boundaries; reading parent/local placements directly requires no geometry walk.
`flatten` remains the explicit way to measure exact placed bounds.

`mark_part_changed(id)` refers to the **last successful compilation's** local
handle assignment. It evicts that content/policy entry, including shared aliases;
it does not identify a persistent occurrence or discard all historical variants.
Freshly rebuilt assemblies do not need invalidation marks for correctness.

Use `cached_entries()` and `clear_cache()` to control cache retention without
resetting lifetime counters. Clearing releases the cache and its bookkeeping,
including pending marks. Previously returned `CompiledParts` retain shared
ownership and remain usable, so their memory lasts until those snapshots drop.
Reports remain available on hits, and matching content does not turn a partial
evaluation into complete geometry.

## Shared geometry queries

Use `PartCompiler::compile_snapshot` when an operation needs both rendering and
inspection. Its `EvaluationSnapshot` shares evaluated topology and source maps
with the render extraction, captures instance placements in `render()`, and
provides the ordinary buffers/reports through `compiled()`. All geometry is
part-local; the captured render items supply occurrence world placements.

`body_by_source(part, "panel")` finds a unique producing source label without
retaining a prior body's numeric index. `BodyLookupError` distinguishes unknown
parts, missing labels, and ambiguous labels with match counts. Labels belong to
the producing node, not to an inferred search through erased Boolean history.

A `SnapshotBody` can resolve a `WorkplaneAttachment`, extract a section, or expose
its immutable geometry. A `SnapshotWorkplane` can produce a checked `PlanarPatch`
for circular-footprint clearance queries. Workplane checks include snapshot and
body identity: clones share scope, but a fresh compilation starts a new scope,
even when all geometry came from cache. Re-resolve attachment intent after edits;
do not serialize a transient workplane or face index as a persistent reference.

Snapshots and body/workplane handles survive cache eviction. `compile_parts`
still serves render-only callers without retaining topology. Upgrading a
render-only cache entry to a query snapshot requires one evaluation; subsequent
queries and snapshot cache hits do not repeat recipe evaluation. See the
[attachment and clearance contract](../exedra_constructive/docs/adr-0014-semantic-attachments.md)
and the `material_gallery` `semantic_attachments` example for the complete flow.

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

`CompilePolicy::uvs` selects the UV source the same way. `UvSource::CustomOnly`,
the default, emits zero for corners without an authored UV and leaves their
ranges without coverage. `UvSource::CustomOrBoxProjected { scale }` box-projects
every unauthored corner on its face's dominant-axis plane, multiplied by
`scale`, using the same math as the `uv.box` operator; authored UVs are never
overwritten. Meshes authored in meters get one UV unit per meter with
`scale: 1.0`. Adding the UV source to the policy advanced `policy_fingerprint`
to the `assembly-compile-v2` prefix, so persisted policy fingerprints must be
recomputed; defaults still produce the same render bytes as before.

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

### UV coverage

`RegionRange::has_uvs` records whether all emitted triangle corners in the range
have a finite UV after `CompilePolicy::uvs` is applied. Under the default
`UvSource::CustomOnly` that means finite authored UVs: render buffers still use
zero for missing coordinates, and consumers can distinguish that fallback from
intentionally authored zero. Under `UvSource::CustomOrBoxProjected` unauthored
corners count when their emitted projected coordinates are finite, which
compilation checks on the emitted buffer rather than assumes: a finite position
times a finite scale can still overflow. A non-finite authored UV still
disqualifies its range, because projection fills missing values and never
repairs authored ones. When migrating
hand-built `RegionRange` values, supply `has_uvs` from the source attribute
coverage under the policy in force; do not infer it from nonzero render
coordinates. Compilation populates it automatically, independently of material
binding, and the glTF exporter relies on it to refuse textured materials
without texture coordinates.
