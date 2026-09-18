# ADR-0023: Construction surface charts

Status: accepted

## Decision

Opt-in `SurfaceChart` metadata belongs on the generating recipe node, next to
its source and material bindings. `RecipeBuilder::with_surface_chart` validates
that its metric matches an ordinary extrusion or revolution. Other operations
refuse the binding. Mesh corner UVs own the realized attribute; constructive
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
corner UVs; retained chart ancestry must not claim otherwise. Smooth loft and
sweep charts need their own explicit rest policies and are outside this slice.
The exact extrusion stretch rewrite falls back to mapped-mesh stretching when a
chart is authored, so a structural optimization cannot silently erase its UVs.

## Migration

`Node` adds `surface_chart: Option<SurfaceChart>`; normal builder calls remain
unchanged. New immediate charted tessellation entry points use the same bounded
profile sampling and mesh construction as their existing unmapped counterparts.
Text and JSON use a distinct charted-node representation rejected by older
readers. Uncharted recipes retain their existing wire representation. Evaluation
schema 39 invalidates prior caches; chart transforms participate in fingerprints.
Material value edits remain outside recipe geometry identity and reuse buffers.

No new production dependency or unsafe code is required.
