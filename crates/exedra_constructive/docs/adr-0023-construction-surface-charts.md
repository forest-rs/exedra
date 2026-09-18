# ADR-0023: Construction surface charts

Status: accepted

## Decision

Opt-in `SurfaceChart` metadata belongs on the generating recipe node, next to
its source and material bindings. `RecipeBuilder::with_surface_chart` validates
that its metric matches an ordinary extrusion, revolution, loft or sweep. Other
operations refuse the binding. Mesh corner UVs own the realized attribute; constructive
owns its authored metric and source evidence. Render-only callers pay no chart
allocation unless they request it. No material representation enters geometry.

Extrusion walls use sampled profile chord distance and local extrusion height.
Revolution walls use angle times an explicit reference radius, and sampled
profile chord distance. This is a declared rest metric, not an isometry at every
radius. Poles necessarily collapse one chart direction. Caps use the source
profile coordinates. Each chart has a finite nonsingular affine UV transform:
scale, direction, axis exchange and texture phase are explicit.

Loop starts define profile seams (`Loop2::with_seam` moves them). A revolution's
angular seam is its authored local +X meridian, controlled by placement. UV phase
does not move a geometric seam. Distinct corner coordinates share mesh vertices
across seams; reflection reverses geometry winding while preserving coordinates
at their associated corners. Coordinates are emitted from f64 construction data,
checked at the f32 attribute boundary. Distinct chart edges must remain distinct,
and noncollinear face triangles must retain the authored transform's winding.
Checks compare against raw construction coordinates, catching phase cancellation
in f64 too. Stored face charts must remain weakly convex, so corner rotation or
a different valid triangulation cannot reveal an inverted UV triangle. The
exact-predicate coordinate envelope is `1e100`. Coordinates are not recovered
from world axes or
inverted rounded mesh positions. Profile distances use sampled chords, and source
maps retain the policy and loop lengths as original construction evidence.

Chart ancestry is independent of current attribute coverage. Compilation reports
that through `RegionRange::has_uvs`. Mesh Boolean reconstruction currently drops
corner UVs; retained chart ancestry must not claim otherwise. Loft and sweep
charts use the explicit policies below. The exact extrusion stretch rewrite falls back to mapped-mesh stretching when a
chart is authored, so a structural optimization cannot silently erase its UVs.

## Loft and sweep rest policies

Loft charts select `reference_section` explicitly. Its jointly discretized
profile supplies U for corresponding vertices throughout the loft, including
holes with independent authored seams. V spans the authored positive
`rest_length` uniformly over section indices. Smooth samples use their retained
band/parameter evidence. V is therefore not measured station distance, nor an
estimate of the draped surface length. Uneven station spacing and changes in
section intentionally stretch the chart. This policy introduces no new profile
correspondence: existing authored seam/segment correspondence remains required.
Each cap uses its own profile coordinates, independently of the reference section.

Sweep charts use sampled profile perimeter and cumulative sampled centerline
chord length before placement. They support legacy polylines, authored miters,
and analytic curved paths with their existing checks. The profile datum and
frame do not move this metric. Miter cuts preserve longitudinal continuity;
inner/outer rails deliberately stretch relative to the centerline. A closed
path reuses the first geometric ring while its terminal corners carry the full
path length. Its only longitudinal discontinuity is at authored station zero.

`ChartSampling` retains the reference loop lengths and all longitudinal rest
station coordinates, including the closing coordinate for cyclic sweeps. It
continues to describe original generation, independent of subsequent placements
or loss of UV coverage. Existing sweep/loft sampling records retain their own
geometry evidence. Normals follow unchanged geometry/crease rules; corner UVs
follow any reversed face order under reflection. The current publication path
supplies normals and UVs, leaving tangent generation to the rendering consumer.

## Migration

`Node` adds `surface_chart: Option<SurfaceChart>`; normal builder calls remain
unchanged. New immediate charted tessellation entry points use the same bounded
profile sampling and mesh construction as their existing unmapped counterparts.
Text and JSON use a distinct charted-node representation rejected by older
readers. Uncharted recipes retain their existing wire representation. Evaluation
schema 39 invalidates prior caches; chart transforms participate in fingerprints.
Material value edits remain outside recipe geometry identity and reuse buffers.

No new production dependency or unsafe code is required.

Loft/sweep extension: `SurfaceChart` and its interchange DTO gain `Loft` and
`Sweep` variants. `ChartSampling` adds `station_distances` (empty on previous
metrics); update direct struct literals and exhaustive matches. Immediate callers
can use `tessellate_loft_with_chart` and `tessellate_sweep_with_chart`. Unknown
text/JSON metrics fail in older readers, rather than dropping chart intent.
Existing None/Extrude/Revolve canonical bytes and geometry stay unchanged, so
schema 40 remains valid; regression fingerprints are taken from the pre-extension
revision. No dependency, geometry algorithm, material or placement convention
changes are involved.
