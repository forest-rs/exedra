# `exedra_mesh_ops`

Direct mesh modeling and geometric queries over `exedra_mesh`.

Operations take plain meshes, explicit parameters and operation-specific policy.
Their typed results carry geometric evidence and source correspondence for callers
to interpret. They require no recipe, runner, clock, or application state.

The kernel owns topology and primitive edits. Constructive owns authored recipes
and semantic provenance. Applications own execution and history.

See the [boundary and migration decision](docs/adr-0001-mesh-operation-boundary.md).

## Choosing an operation

| Need | Module |
| --- | --- |
| Extrude, inset, solidify, cut a rectangular opening | `face_edit` |
| Bridge or subdivide selected faces | `bridge`, `poke` |
| Join open tube ends at a branch node | `junction` |
| Boolean solids or finish selected edges | `boolean`, `round` |
| Section or split a mesh with capped halves | `section` |
| Resolve an authored planar frame or measure clearance | `workplane`, `clearance` |
| Transform or stretch a mesh | `transform`, `stretch`, `stretch::stretch_vertices` |
| Project a mesh loop, measure boundaries, inspect bounds | `planar`, `measure`, `inspect` |
| Select regions, author normals or UVs | `region`, `normal_edit`, `uv` |

Each operation documents its supported inputs and failure semantics. Kernel
edit sessions are eager; use a clone when an application needs transactional
behavior. Geometric source IDs are transient evidence, not persistent feature
selectors. The crate-level rustdoc includes a direct edit example.

## Texture coordinates

`uv::map_uvs` authors either built-in corner UV channel on loaded or generated
meshes. Choose existing coordinates, arbitrary planar frames, signed box faces,
per-face planes, cylindrical, spherical, toroidal or perspective projection,
then apply an affine UV transform. Box layouts independently swap and flip axes
on each face pair; authored corner normals select sides when available.
`UvMappingOptions` chooses source/target channels, seam lifting and pole recovery.
Angular correction operates per face and preserves topology.

Use `inspect_uvs` for coverage, finite bounds and offending corner IDs;
`preview_uvs` computes the same report as authoring without mutation. Reports
include writes, unchanged/skipped corners, normal fallbacks, seam lifts, pole
repairs, coarse angular faces and retained scratch capacity. All selections and
coordinates are checked before the first write. Unchanged bit patterns create
no dirty corners. Finish the caller's edit session to advance revision and
obtain its change output.

Retain `UvAuthoringScratch` and call `map_uvs_with_scratch` for repeated work.
Gathering uses small batches for linear modes and reusable per-face buffers for
angular/normal-dependent modes. Sorted pending writes merge into sparse storage
without per-corner insertion shifts; small selected-face edits avoid scanning
the whole authored layer. Scratch capacity is explicit and can be released.
The returned report owns its selected-face list separately.

`UvTransform::rotate_offset_scale` uses radians and applies rotation, offset,
then nonuniform scale. Zero and negative scales are supported. `then` composes
in call order; `around_pivot` and `fit_bounds` provide explicit authoring helpers.
Existing mapping refuses missing coordinates. Use projection with
`write_missing_only` to fill gaps, or select distinct source/target channels to
copy and transform coordinates while retaining the original channel.

Migration: existing `project_planar`, `project_box`, and `project_cylinder` keep
their scale-then-offset semantics. The new mapping API is opt-in. Exhaustive
`UvError` matches must handle `Mapping`, `IncompatibleLayer` and `BulkWrite`.
`UvOutput` adds mapping evidence and finite bounds and implements `PartialEq`
instead of `Eq`; struct literals can use `..Default::default()`.

`uv::quality::inspect_uv_quality` measures the stored triangle map in texels per
position unit, using explicit image dimensions and triangulation. Reports expose
principal densities, directional anisotropy, mirrored orientation, missing or
degenerate coordinates, optional acceptance bounds, and bounded source-corner
witnesses. Reports retain the channel, image dimensions, triangulation and limits,
and expose the surface area outside limits as well as triangle counts. This
distinguishes ill-conditioned slivers from large distorted patches. Mirroring is permitted unless explicitly forbidden. `visit_uv_quality`
streams the same triangle results for heatmaps without storing a whole-mesh
report. Select faces and retain `UvQualityScratch` for local repeated inspection.

These are f64 differential measurements of piecewise affine triangles, not a
certificate of global overlap, smooth-surface fidelity or an unknown rest chart.
Match the renderer's triangulation when inspecting its output. Robust polygon
fallbacks are reported and excluded; the kernel may allocate for robust polygons.
Returned revisions belong to the caller's source mesh, not a cross-mesh cache key.

Migration: quality inspection is additive; existing UV authoring behavior and
reports are unchanged. Thresholds are opt-in and image dimensions have no implicit
model-unit conversion. The default `[1, 1]` reports repeats per position unit.

## Edge finishing

`round_sharp_edges` selects authored sharp edges. `round_edges` accepts an
explicit transient edge set and returns `RoundResult`: work counters and the
input faces responsible for each replacement, strip, or corner patch. Twins
and duplicate targets are canonicalized; failures leave the input mesh
byte-identical. Stable construction targets belong in the caller, not in mesh IDs.

Both operations author radial fillet normals and retain hard chamfer/end
boundaries. Extract with `NormalsSource::CustomOrDerived`. Unchanged faces keep
all attributes; rewritten faces preserve their regions and valid authored
normals at surviving corners. New faces use the requested region or the first
source face's region. Source faces are listed in ascending input-ID order.

Surviving corners keep their exact UVs. New corners interpolate the source
face's robust triangulation; bands and patches project onto the first source
face's chart, matching material ownership. Textures stretch toward perpendicular
tangencies and can fold on surfaces turning beyond them.
Different charts meet at explicit edge seams. Incomplete or non-finite source
charts supply no new UVs. Untextured inputs remain untextured; callers can use
the face provenance to apply a different mapping after finishing.

Explicit segments must be in `1..=256` and control both strip bands and corner
radial layers. Otherwise chord tolerance controls the arc and triangle surfaces,
including corner interiors. A tolerance requiring more than 256 bands or layers
is an error. Finer spherical patches cost more triangles; use an explicit count
or a coarser tolerance when that tradeoff suits the caller.

Radius and clearance decisions use the stored mesh coordinates. Faces that
collapse at final f32 precision, rewritten faces that reverse orientation, and
edge trims that cross are refused. Convex trihedral corners
and gently turning chains are supported. Adjacent edges sharing a planar
flank and equal dihedral angles meet at an exact miter, preserving the setback
or radius on both edges. Set `max_tangent_turn` to `FRAC_PI_2` for square rims;
the default remains 0.7 radians. Fillet miters keep a crease between cylinders,
and automatic band counts account for their elliptical seam curves.

Straight concave chains between planar flanks and perpendicular planar end caps
also support fillets and chamfers.
These fill an internal corner with material; fillet normals point toward the
cylinder center in the void. Collinear chain subdivisions and triangulated end
caps are supported. The existing cap faces retain their ownership and UVs; added
cap triangles extend the first incident cap face's chart and material. Concave
turns, closed rings, and junctions with other selected chains are refused.
Separate convex and concave chains can finish together in one atomic pass.

Concave clearance uses the swept triangle between the original corner and its
two tangencies. It conservatively refuses obstructions even just beyond the
fillet arc. End tangencies must fit before the next existing cap-boundary vertex;
finishing does not dissolve collinear Boolean subdivisions to gain clearance.

Migration for concave finishing: call signatures and policy encoding are
unchanged. Previously refused straight concave selections now add material;
`ConcaveEdge` identifies the remaining unsupported turns and junctions.

Migration: existing `round_sharp_edges` calls retain their return type. Use
`round_edges` when selection or provenance is needed, and choose
`CustomOrDerived` to consume the new normal overrides. Callers that relied on
silently clamped band counts must choose a supported count or tolerance.
Exhaustive `RoundError` matches must handle the new `InvalidEdge` variant.

## Polygon footprints

`PlanarPatch::polygon_clearance` checks a simple filled polygon against the whole
material domain, including concavities and holes. Inspect containment separately
from nearest boundary distance: a plate can enclose a hole while its edges remain
clear. Results include footprint/source-boundary witnesses and bounded query work.
See [polygon clearance semantics](docs/adr-0002-polygon-footprint-clearance.md).

## License

Apache-2.0 OR MIT
