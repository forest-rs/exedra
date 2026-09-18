# exedra_constructive

Constructive geometry head: an immutable, content-addressed recipe IR with
deterministic tessellation into Exedra meshes.

This crate is the retained geometry domain: a *compiler target* for
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

## Profile offsets

`Profile2::offset_with_policy` trims inside corners beside fitted cubics, including
line/cubic, arc/cubic and cubic/cubic joins. `OffsetPolicy::trim_tolerance` and
`max_trim_steps` control the intersection search. `OffsetResult::trims` reports
parameters, positional enclosures and endpoint adjustments; fitted runs retain
their tags and sampling policy. Multiple intersections, tangencies, unresolved
piece-endpoint intersections and consumed geometry fail explicitly.

Migration: add the two policy fields (or use `..Default::default()`), scale
`trim_tolerance` with other dimensional tolerances, and handle `OffsetMethod::Trimmed`,
`OffsetBudget::TrimSteps`, and `OffsetWork::trim_steps`. The former blanket
`OffsetCornerUnsupported` failure is replaced by `OffsetTrimUnresolved` and
`OffsetTrimAmbiguous`. See [numerical trim semantics](docs/adr-0021-cubic-offset-trimming.md).

## Authoring coordinates and diagnosing boundaries

Closed planar surrounds use the ordinary `NodeKind::Sweep` with
`Path3::MiteredPolyline`. Supply `closure: PathClosure::ClosedPlanar { normal }`,
unique stations without a repeated endpoint, `section_origin` in profile
coordinates, and `CapMode::None`. The existing `section_x` controls the starting
roll. The closing corner gets the same bounded miter and local checks as the
other corners, sharing vertices without seam caps. Curved sections and holes
are supported. The `closed_surround` binary in `constructive_probe` exports a GLB
example. Global self-intersection certification remains outside this contract.

`Path3::Curves` accepts the same datum and closure, plus `PathJoin::Smooth` or
`PathJoin::Miter { limit }`. An arched surround can combine straight runs, tangent
arcs and sharp base corners. Closed curves explicitly end at their exact starting
point; no fuzzy snapping closes a gap. Smooth seams share one ring without caps
or a final twist. Sampling records authored corners separately from curved spans.
Migration: existing curved paths add a zero `section_origin`, `PathClosure::Open`
and `PathJoin::Smooth`; schema 38 invalidates old fingerprints. See
[closed curved sweep semantics](docs/adr-0022-closed-curved-sweeps.md).

`SourceMap::sweep_sampling(face)` preserves source sampling policies and curved
path spans through Boolean cuts. These are original construction records,
not validity checks on the Boolean result. See [closed sweep semantics and
migration](docs/adr-0020-closed-planar-sweeps.md).

Hard evaluation failures own the failed node's source string. Smooth-loft
forward-motion refusals also retain both section labels, profile ids, authored
placements, and cubic control geometry with source segment/tag correspondence.
Errors remain inspectable after dropping the recipe. The witness explains the
existing sufficient check; it does not prove a surface self-intersection.

Retained loft sections now use `LoftSection::new(placement, profile)` in place
of tuples. Add `.with_source(builder.source_ref("vessel/shoulder"))` to name a
section independently of its reusable profile. JSON and text round trips retain
these labels. See [failure context and migration](docs/adr-0019-construction-failure-context.md).

Choose `builders::rect_from_corner` or `builders::rect_centered`, and the
corresponding `rounded_rect_*` builders. Circles and rings are centered at the
origin. Centered rectangles compose directly with concentric circular holes.
Migration: the old `rect` and `rounded_rect` names are replaced by their
`*_from_corner` names for identical geometry; existing serialized recipes do
not change.

`Placement3::try_from_orthonormal_axes` checks a finite right-handed frame using
an explicit tolerance. It reports the offending axis or axis pair and measured
length/dot product through `ir::FrameError`; it does not normalize or repair axes.
`from_axes` remains available for general affine placements including scale and
reflection.

When failed triangulation finds an actual boundary contact,
`TessellateError::ProfileBoundaryContact` identifies both authored segments,
their optional tags, the sampled edge indices, and crossing versus touching.
A ground-reaching opening belongs in the outer boundary as a notch; it cannot
be a touching hole. Curve diagnostics describe the operation's sampled chords.
An unexplained triangulation failure keeps its original error instead of
inventing a boundary culprit. See the
[authoring contract](docs/adr-0018-authored-geometry-diagnostics.md).

`workplane::face_workplane` builds an orthonormal frame on a face or connected
planar region. Callers author the origin and X direction in body coordinates;
face winding supplies +Z. Operand-qualified regions disambiguate reused Boolean
region numbers. Workplanes expose local/body transforms, placements for local
feature offsets, selected face IDs, measured planarity, and a revision check.
Resolved workplanes are snapshots. `WorkplaneAttachment` retains a semantic
surface selector, projection line, and preferred X direction, then resolves a
fresh workplane after recipe changes. No transient face ID is retained.
Nonplanar, disconnected, degenerate, or ambiguously selected patches are refused.
This additive API needs no migration. See the `face_workplane` GLB example for
holes and collars positioned on an angled panel using face-local coordinates.

`extrude::extrude_to_plane` creates a closed extrusion along a profile
placement's +Z axis, stopping at an authored plane in body coordinates. Holes,
wall regions, and source features survive; the terminating cap has the ordinary
end-cap region and `Feature::CapEnd` provenance. The operation computes its own extent
and refuses parallel, backward, crossing, or numerically unrepresentable cases.
It returns the body and the terminating section with accuracy/work measurements.
`NodeKind::ExtrudeToPlane` retains the same operation in a recipe.
`NodeKind::PlaneCut` retains one capped half-space of a child. Planes use node-local
coordinates after child transforms, before ancestor transforms.
The `extrude_to_plane` example in `constructive_probe` exports hollow supports
terminating at one inclined plane.

For evaluated bodies, `section::section_body` extracts planar regions with holes
and per-edge source features. `section::split_body` returns both capped halves
of the triangulated mesh, with an explicit plane, distance tolerance, finite
work budgets, and caller-selected cap region/material. Contacts within tolerance
are typed failures. These operations preserve surviving surface attributes and
clear source sampling evidence on derived bodies; they do not certify distant
self-intersections. See the `section` rustdoc example and the `plane_cut` binary
in `constructive_probe` for an oblique cut through a smooth loft.

`PlaneSection::to_profiles` turns each filled region into an owned `Profile2`,
placement and segment-source table, ready for extrusion, sweeping or authored
loft correspondence. Holes and disconnected regions remain distinct, and every
polyline sample is preserved. `profile_section::profiles_from_mesh_section`
accepts sections from `exedra_mesh_ops` with the same conversion. These are
copies of evaluated boundaries, without a retained dependency on the source body.

## Retained attachments and clearance

`NodeKind::OnWorkplane { support, child, attachment }` evaluates one complete
support body, resolves the authored workplane, and places the child in that
frame. The support is not emitted by the attachment node: include it in a group
or Boolean when needed. `EvalPolicy::workplane` supplies accuracy and work limits.
`GeometryReport::attachments` records resolved local frames and planarity evidence.

`SurfaceSelector::EndCap` follows original terminal-cap provenance through profile
changes, placement, and Boolean face splits. `SourceEndCap("panel/body".into())`
qualifies that role by the opaque source label on its generating recipe node;
`SourceStartCap` does the same for the start cap. Name the extrusion or other
surface-generating node, not a transform, instance, or Boolean wrapper. Qualified
labels must be unique among reachable recipe nodes. Whole-recipe fingerprints
include this naming context; node fingerprints still support subtree reuse. A shared definition counts
once; disconnected occurrences still fail the connected-patch check. Missing,
disconnected, nonplanar, and ambiguous targets are refused without a fallback.
`SourceMap::surface_origin` exposes original feature and source independently of
`face_feature`, which retains immediate Boolean operand attribution. Difference
may reverse a surviving surface's winding without changing its ancestry. Other
topology-changing operations do not yet promise to retain this additional ancestry.
See [ADR 0015](docs/adr-0015-boolean-surface-provenance.md).

An attachment origin is the intersection of its selected plane and the authored
infinite line `anchor + t * projection`. Both signs of `t` are allowed. The
preferred X direction controls roll. All inputs use node-local coordinates;
ancestor transforms place the completed result. The origin may lie outside the
patch, so attaching a frame alone does not prove a feature fits.

`clearance::PlanarPatch::from_workplane` extracts and checks the selected patch's
outer boundary and holes. Reuse the patch to measure circular footprints with
`circle_clearance(center, radius)`. The result gives a signed containment margin,
nearest boundary point, source edge/features, and measured work.
`classify(minimum, tolerance)` distinguishes satisfied, violated, and within the
decision tolerance. A near-threshold result is not silently accepted.

Measurements use the evaluated polygonal boundary in orthonormal workplane
coordinates. They do not certify analytic curve error, three-dimensional bore
clearance, or wall thickness. Crossings, contact, invalid nesting, stale source
maps and exhausted work budgets are explicit errors.

Run `cargo run -p material_gallery --bin semantic_attachments` for four retained,
Boolean-drilled panels: baseline, resized, tilted, and a narrow panel with two
clearance violations. It exports canonical recipes, a measured report, and a GLB
with colored mounts and nearest-boundary witness strokes. The displayed bodies
and measurements come from one shared query snapshot.

Migration: schema 34 invalidates earlier evaluation fingerprints and canonical
text. Workplane selectors and attachments now own optional source labels and
are `Clone` rather than `Copy`; clone them when reusing owned values. Existing
operand-qualified regions keep their immediate-Boolean meaning.

## Inspecting surfaces and diagnosing attachments

`workplane::inspect_surfaces` inventories surviving cap and region selectors.
Each entry includes original source labels and roles, unknown-ancestry counts,
connected patches with snapshot-local face IDs, measured planes, and a selection
status. It does not pick an anchor, roll, or replacement target. Several selectors
can cover the same faces. This inventory covers semantic cap/region selections;
individual transient face queries remain available through `face_workplane`.

A successful entry means one geometrically valid planar patch under the recorded
inventory policy. Authored frame inputs and resolution budgets still need checking.
Absent selectors are not evidence of why a surface is missing: it may have been
removed, never authored, or lost its provenance. Nonplanar or ambiguous entries
remain inspectable, with evidence; exhausted global budgets fail the whole query.
`SurfaceInventoryPolicy` limits scanned faces, corner visits across overlapping
selectors, and output selector count. The result retains effective policy and work
statistics.

`WorkplaneAttachment::resolve` and `face_workplane` now return `WorkplaneFailure`.
Its `selection` is the requested selector, `kind` is the existing coarse category,
and `evidence` distinguishes duplicate source labels, disconnected patches, mixed
Boolean operands, measured nonplanarity, and the exact exhausted resource/limit.
Other categories carry `WorkplaneEvidence::None`. Face IDs in evidence refer to the
queried body; use assembly snapshot ownership before applying them elsewhere.
Retained `OnWorkplane` errors carry the same failure in `TessellateError::Attachment`.

Migration: evaluation schema 35; regenerate evaluation caches and schema-stamped
text. Match `failure.kind` for existing error-category handling and inspect
`failure.evidence` for recovery. Workplane corner limits now count both connectivity
and geometric analysis, visiting every selected corner twice on success. Increase
explicitly tuned limits accordingly. No recipe node or selector encoding changes.
See [ADR 0016](docs/adr-0016-surface-inspection.md).

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
stay sharp. Oversized radii, boundary edges, affected non-planar faces and
unsupported junctions are refused. Convex open chain ends must meet one end
face; CSG can split that face and make the end unsupported.
Closed rims with unambiguous operand/region pairs avoid these selection and
end-face limitations. For a square rim, set `policy.max_tangent_turn` to `FRAC_PI_2`;
the default remains 0.7 radians. Consecutive edges with a shared planar flank
and equal dihedral angles receive exact miters. Fillet normals retain the
crease between adjoining cylinders, and automatic band counts also bound
chord error along the elliptical miter seam. Operand-qualified selection avoids
region collisions without changing the supported rounding geometry. General
concave corner blends and self-intersecting curved strips remain outside this scope.

Straight concave shoulders of through housings and notches can be filleted or
chamfered using the same selections. They add material into the internal corner,
with the fillet's cylinder center on the void side. Collinear chain subdivisions
and triangulated end caps perpendicular to the chain are supported; existing cap
materials and UVs are retained, and new cap triangles inherit the first incident
cap face's chart and material. Concave bends, closed rings, and mixed corner junctions remain
explicit refusals. Separate convex and concave chains can finish in one pass.
The setback must fit before the next existing end-cap boundary vertex. Nearby
geometry entering the swept triangle enclosing the added arc is conservatively
refused, even if it lies just beyond the arc itself.

Migration for concave finishing: selections and recipe encoding are unchanged,
but straight concave targets that previously refused now add material.
Evaluation schema 23 invalidates cached geometry and older text dumps.
Schema 24 additionally invalidates robust triangulations that contained
almost-collinear ears; those faces now retry with constrained Delaunay
diagonals while preserving their boundary vertices.

Migration for qualified selection: existing `SharpEdges` and `RegionBoundaries`
calls keep their selection behavior and encoding. Exhaustive matches must handle
`EdgeSelection::OperandBoundaries` and
`EdgeFinishError::AmbiguousOperandSelection`. Rust DTO callers wrap numeric
boundary lists in `EdgeBoundariesDto::Regions`; qualified lists use `Operands`.
The JSON `boundaries` field retains numeric pairs or accepts pairs of
`{"operand": 1, "region": 5}` objects. Older readers reject these objects;
they cannot silently fall back to finishing every sharp edge. Constructive
text uses the explicit `operand_boundaries` selector.

Migration: evaluation schema 22 invalidates cached output for rim finishes that
previously refused because replacement faces temporarily pinched a boundary
during insertion. Separate pocket rims can now finish together or in sequence.
It follows schema 21's planar-flank miters and schema 20's UV
transfer change, which removed the unconditional `eval.edge_finish.uv_deferred`
note. Regenerate
persisted constructive text with the current schema. Existing recipes need no
field changes. Callers that supply their own unwrap can continue overwriting
the finished faces' UVs; new faces no longer necessarily have missing UVs.
The runnable comparison is
`cargo run -p material_gallery --bin edge_finishes -- target/edge-finishes`.
Exhaustive `NodeKindDto` matches must handle its new `EdgeFinish` variant.

Polygon mounting footprints use `PlanarPatch::polygon_clearance` with an explicit
work policy. Results distinguish containment from nearest boundary distance and
carry source witnesses for crossings, exterior intervals and covered holes.
The `footprint_clearance` example queries retained snapshots and exports a GLB.

## License

Apache-2.0 OR MIT

### Planar measurements

`PlanarPatch::measure()` and `PlaneSection::measure()` report material area,
total perimeter (including holes), area centroid, local XY bounds, and edges
examined. Holes subtract area; disconnected section regions contribute together.
An empty section has zero area/perimeter and no centroid or bounds. A centroid
can lie in a hole or outside a concave region.

Queries reuse stored polygonal boundaries without allocation or retessellation.
Lengths use body units and areas use squared body units. Coordinates use the
owner's frame; retain the patch or section for that frame and source evidence.
Results describe evaluated polygons, not analytic curved-surface measurements.
See [the measurement contract](docs/adr-0017-planar-measurements.md).
