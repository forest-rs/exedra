# Exedra Compile

Content-addressed geometry compilation and retained queries, independently of
assembly or scene composition. Supply borrowed `PartInput` records containing
caller-owned diagnostic keys and recipes, baked meshes or installed native models. The service owns no
hierarchy, occurrence placements, appearance binding or persistent scene identity.

`PartCompiler::compile_parts` produces triangle buffers and complete recipe
reports. `compile_snapshot` additionally retains topology and source maps for
sections, semantic surface inspection, attachments and clearance queries.
`GeometrySnapshot` contains geometry only; callers retain placement separately.

Inputs use snapshot-local `PartId` handles. `CompiledParts::validate_for` checks
counts and captured source identity without evaluating or rescanning geometry. Content and policy hashes
retain their existing encodings; moving ownership does not invalidate compiled
content identities. Diagnostic keys are excluded from sharing.

Cloned geometry snapshots share their tables in constant time. Fresh snapshots
create fresh selection scopes even when geometry is reused. Snapshot bodies,
workplanes and inspected surface evidence survive compiler eviction and remain
`Send + Sync`. Re-resolve authored attachment intent after geometry edits.

`GeometrySnapshot::part` retains one input without retaining unrelated input
buffers. Body handles and per-part views share the parent snapshot's query scope.

The compiler defaults to explicit caller-controlled retention. Use
`with_capacity` for an entry budget, `clear_cache` to release entries, or
`mark_part_changed` to evict content from the last successful input ordering.
A single pending set replaces channel-based invalidation. Cache eviction never
invalidates held snapshots. Counters distinguish compilation, hits, evictions,
triangle extraction and attribute/tangent fallbacks; reports on cache hits retain
the original evaluation measurements. Render-only entries retain no topology;
upgrading one to a query snapshot evaluates once.

For mesh inspections that must match compiled triangles, use
`CompilePolicy::extraction_params()`. Compilation uses robust polygon
triangulation; the kernel's default is a fan. The returned settings also expose
normal selection, UV fallback, carried attributes and tangents. Stored-UV quality
surveys do not synthesize missing-coordinate fallbacks.

Existing Assembly compilation is an adapter over this service. Standalone
callers can use these records and snapshots directly without creating an
`Assembly`. The crate is `no_std` with `alloc` and requires
pointer-width atomics; the `std` feature forwards host support to dependencies.

## Retained native models

Install an immutable analytic construction with `NativeSource::from(extrusion)`
and `PreparedSource::new(PartSource::Native(native.clone()))`. Extrusions,
revolutions, sweeps and ruled lofts retain their original typed geometry and source
roles. Canonical source identity is captured once at installation, independently
of display tolerance. Cloning the source shares that installation.

`CompilePolicy::native` explicitly controls native realization accuracy and work
budgets. Compile the same prepared input at coarse and fine tolerances; each
snapshot retains its mesh and `native_realization()` report/correspondence while
`SnapshotBody::native()` exposes the original immutable constructor. Native area
and volume queries avoid meshing; unsupported measures, currently ruled-loft
area/volume, return a typed availability error. Use the original typed extrusion for tagged floor queries and generic-body CAD export.
`NativeSource::face_tags(face)` yields distinct authored tags without allocation,
including both source-section labels on loft walls.
`NativeSource::planar_frame(face)` queries the oriented supporting plane across
construction families and explicitly refuses curved faces.
`conservative_bounds()` encloses native supporting domains after occurrence
placement, independently of display tolerance. It can include space outside
trims, partial angular faces and rotated source boxes; it is not a tight extent
or a material-classification query.

`native_render_face(triangle)` follows the actual grouped render index buffer.
It returns a source-scoped native selection, without inferring provenance from
render regions or requiring a draw group per native face. Native selections
survive tolerance changes and eviction. A freshly installed equal-content source
can share cached render buffers but rejects the earlier source's native selections.
Mesh workplanes and other mesh query selections retain their existing per-snapshot
scope. The mesh source map describes the realized mesh; native provenance comes
from the explicit native correspondence record.

`CompileCounters::native_realizations` counts real cache-miss/upgrade work.
`native_storage()` surveys distinct sources retained by cache entries separately
from display buffers. Its boundary payload byte count excludes construction
records, allocator overhead and spare capacity; it is not total heap residency.
Held sources and snapshots can outlive cache eviction and remain usable.

## Input identity migration

Replace `PartInput { key, source }` literals with `PartInput::new(key, source)`.
The constructor captures content identity once; its immutable borrow prevents
source edits while the input exists. For repeated compilation, install a source
in `PreparedSource::new(source)` and use `PartInput::prepared(key, &prepared)`.
Prepared sources keep geometry and fingerprint together, so cache hits and
validation do not scan baked attributes again. To edit, consume with
`into_source`, then install a new prepared source. Assembly uses this same
installation boundary for part registration and replacement.

## License

Apache-2.0 OR MIT

Native measure results are positive and finite when available. Unsupported family queries
return `NativeMeasureError::Unavailable`; a supported calculation failing to produce a positive finite result returns `NumericLimit`, naming the family and measure.
Other queries on that immutable source remain available.

## Native construction charts

Set `CompilePolicy::native_chart` to a compatible `SurfaceChart` when compiling
native revolution, sweep or loft sources. The default uncharted path allocates
no chart coordinates or policy box. Extrusion and bored-plate chart requests
return `NativeChartError::Unavailable`; a mismatched chart operation, invalid
transform, unrepresentable metric or failed mapping retains its typed diagnosis
and the input part key. Recipes keep their node-authored charts; baked inputs
ignore this native-only setting.

`NativeSource::check_chart` provides constant-time scalar/family/reference-section
preflight. Actual metric and mapped-coordinate checks happen on a cache miss.
Loft section-index or datum-distance stations reuse the shared chart metric;
native profile/path distances come from source curves, not display chords.
All chart transforms, reference metrics and station modes join the policy key.
Native source content and selection identity do not change with the chart.

A retained snapshot's `NativeRealization::chart()` borrows the applied policy,
identical to the validated request. The policy box is shared with cached geometry;
queries do not clone it. Uncharted records carry only an empty pointer-sized field.
Authored corner UVs can split render vertices while native topology stays shared.
`SnapshotBody::native_render_face` follows the compiler's final triangle order,
including UV splitting and region regrouping. Render-only compilation retains
buffers; use `compile_snapshot` for native policy/correspondence inspection.
