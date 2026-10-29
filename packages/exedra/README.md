# Exedra authoring for TypeScript

Describe reusable profiles and native construction in TypeScript; Rust validates,
constructs, queries and realizes the geometry. The SDK supports native extrusions,
through cutouts, blind pockets, planar native-face attachments, line/arc profile
revolutions, smooth sweeps, corresponding-section ruled lofts and selected axial
finishes. Capabilities and measurement availability are explicit.

```ts
import { createSession, extrude, profiles } from "@exedra/authoring";

const vent = profiles.slot({ length: 22, radius: 3 }).tagged("vents");
const outline = profiles.roundedRectangle({ width: 120, height: 80, radius: 6 })
  .tagged("panel").withCutoutsAt(vent, [[-12, 22], [12, 22]]);
const panel = extrude(outline, { height: 12 }).pocket(
  profiles.slot({ length: 48, radius: 8 }).tagged("recess"),
  { end: "end", depth: 5, at: [0, -10] },
);
const pin = panel.floor("recess").workplane().translated([0, -10]).offset(0.5)
  .extrude(profiles.circle({ radius: 3 }).tagged("pin"), { height: 7 });

const session = await createSession({ units: "mm" });
const document = session.build({ panel, pin });
try {
  const native = document.parts.panel;
  const volume = native.measurements.volume;
  if (volume.status === "available") console.log(volume.value);
  console.log(native.floor("recess").workplane());
  const display = native.realize({ tolerance: 0.12 });
  const face = display.pickTriangle(0);
  native.requireSelection(face);
  const step = native.exportStep({ timestamp: "2026-10-27T00:00:00Z" });
  // display contains caller-owned typed arrays; step contains text and correspondence.
} finally {
  document.dispose();
  session.dispose();
}
```

`Profile`, `Solid` and `AuthoringWorkplane` are immutable descriptions with no
native handles. Share them freely: one build lowers shared graph nodes once.
An authoring workplane is a selector awaiting construction. A queried
`NativeWorkplane` is an installed source frame, in document coordinates; it does
not depend on render triangles.

`session.build` installs a whole immutable revision transactionally. On an
`ExedraError`, inspect `diagnostic.code`, `operation`, `message` and optional
node/profile index. Earlier documents remain usable. A missing or ambiguous
named floor, curved workplane, invalid pocket or exceeded work budget produces a
typed refusal rather than a fabricated result. Native measurements have explicit
`available` values or `unavailable` / `numericLimit` diagnoses. Unavailable
integrals do not prevent a valid source from being installed. Available values
are positive and finite; numeric failures are never labelled as unsupported.
Bounds are finite conservative native support enclosures, labelled by
`boundsKind`; they can include space outside trims and are not minimum extents.

Lengths, tolerances, bounds and attachment offsets use explicit document units
(`mm` or `m`); areas and volumes use their square and cube. Geometry is not
rescaled by choosing units. STEP declares the same unit and exports exactly one
named native part, including its placement, independently of display quality.
Its report carries native-ID-indexed topology correspondence to STEP entities.

Calling `realize` at another tolerance keeps the installed native identity. A new
build creates a different revision even for equal geometry. A selection belongs
to one part/revision and is rejected by another; disposing its document also
invalidates native queries and picks. Display arrays and plain reports remain
caller-owned values. Dispose Three.js geometries/materials separately. Dispose
old documents after replacing the view and release the session on teardown.
There is no implicit scheduling or reliance on JavaScript finalizers.

A session retains at most eight documents and eight cached realizations. A
single realization permits at most 250,000 mesh vertices. Graph/profile/placement
budgets are checked before native construction. `session.stats()` exposes
logical native owners and boundary-table payloads: document and cache payloads
can share sources and must not be added as unique memory. These exclude render
buffers, allocator capacity/overhead and GPU allocations.

Build and try the [interactive browser sample](../../apps/exedra_authoring_demo/README.md).
The SDK has no runtime JavaScript dependencies; its generated Wasm package is
built by that app's `prepare:sdk` command. The transport interfaces support hosts
with a different loading arrangement without changing the authoring surface.

Revolutions use profile X as positive radius and Y as height, about local +Y:

```ts
import { placement, revolve } from "@exedra/authoring";
const section = profiles.roundedRectangle({ width: 12, height: 24, radius: 2 })
  .translated([28, 12]).tagged("ring");
const ring = revolve(section, {
  angle: 2 * Math.PI,
  placement: placement({ rotationXYZ: [Math.PI / 2, 0, 0] }),
});
```

`placement` snapshots fixed-axis X/Y/Z rotation angles in radians, then
translation; Rust constructs and validates the rigid frame. `profiles.polygon`
accepts an authored closed line outline without repeating its first vertex.
Winding, self-crossings and work budgets are checked in Rust. Full and partial
revolutions retain cylinders, cones and regular toroidal walls. Full-turn sections
with supported axis boundaries also retain spherical walls and pointed cone tips;
partial turns touching the axis are refused. Unsupported STEP
surfaces produce an `exchange.unsupportedSurface` diagnostic; the source stays
usable for display and native queries. The current exporter supports plane,
cylinder, cone, torus and sphere faces with supported boundary topology; ruled
faces remain unsupported.

`sphericalCap` preserves the requested sphere radius and cap height in Rust,
including shallow domes, hemispheres, taller caps and a full sphere at height
`2 * radius`:

```ts
import { sphericalCap, placement } from "@exedra/authoring";
const dome = sphericalCap({ radius: 25, height: 12,
  placement: placement({ rotationXYZ: [Math.PI / 2, 0, 0] }),
});
```

Its base starts at local Y=0 and its tip is at Y=`height`; the placement above
makes that direction world +Z. Rust constructs the canonical center-defined
profile and ordinary native revolution. The installed part still reports
`family: "revolution"`, supports construction charts, and exports its exact
spherical wall and planar base independently of the display mesh.
Native dimension or floating-point height/rim resolution refusals use
`revolve.cap.*` diagnostics with the original `radius` and `height`; numeric
resolution refusals also expose `axisOffset` or `rimRadius`. No epsilon ring or
accidental full sphere is substituted. Earlier documents remain usable.

Each `document.buildTiming`, `display.timing`, exported `StepFile.timing` and
queried workplane `timing` reports host-observed elapsed milliseconds. Refused
operations expose the same evidence through `ExedraError.timing` while preserving
the original diagnostic. `kernelMs` measures the Wasm method boundary (including
its binding work); `sdkPrepareMs` covers graph lowering/JSON requests;
`transferMs` covers taking typed arrays or STEP text; `metadataMs` covers reading
and decoding metadata; `cleanupMs` covers releasing temporary Wasm handles.
`totalMs` spans the observed SDK operation, including result wrapping. These are
synchronous host CPU/elapsed measurements, not GPU time or presented-frame latency.

`session.lastOperation` retains only one immutable timing value, with its
operation, outcome and document/part scope. Reading timings or `stats()` does not
record additional operations. Held timing values remain plain immutable evidence
after later edits or disposal; they do not retain another native model owner.

`part.stepSupport` reports structural STEP geometry support with a typed face/family
reason when unavailable. A supported result still requires final export validation
of placement, tolerance, topology and output budgets.

Native sweeps and ruled lofts use immutable descriptors too:

```ts
const path = paths.start([0, 0, 0], { tangent: [0, 1, 0], sectionX: [1, 0, 0] })
  .lineBy([0, 20, 0])
  .tangentArc({ radius: 40, angle: Math.PI / 2, normal: [0, 0, 1] });
const rail = sweep(profiles.circle({ radius: 6 }).tagged("rail"), path);
const transition = loft([
  { profile: section, placement: placement() },
  { profile: section, placement: placement({ translation: [5, 0, 60] }) },
]);
const rounded = extrude(profiles.rectangle({ width: 70, height: 50 }).tagged("body"), { height: 20 })
  .axialEdges("body").fillet({ radius: 5, feature: "round" });
```

Import `paths`, `sweep`, `loft` and `placement` from the SDK. Paths lower once per
shared descriptor, using native line/arc validation. Sweeps currently require a
supported smooth open path and section. Loft sections must have corresponding
line/arc segment structure; unsupported integrals remain explicitly unavailable.
Named axial-edge finishes apply to extrusion profile corners, including selected
hole corners. They do not imply general rim or arbitrary 3D edge fillets. Kernel
errors preserve the construction node and operation; a failed document build
never registers a partial result.

Construction UVs are display policy over an installed native source:

```ts
import { charts, uvTransform } from "@exedra/authoring";
const grid = uvTransform({ matrix: [[1 / 20, 0], [0, 1 / 20]] });
const display = document.parts.rail.realize({
  tolerance: 0.12,
  chart: charts.sweep({ wall: grid, caps: grid }),
});
// Attach display.uvs to a two-component render UV attribute.
```

`part.chartKind` identifies structural native chart support, or is `null` when
unavailable. `charts.revolve` uses an explicit rest radius; `charts.loft` uses an
explicit reference section, rest length and station rule. Native distances follow
exact source curves. Coordinate transforms map those lengths to texture repeats;
offsets change texture phase. Chart settings do not change source identity,
workplanes or topology. Invalid or unsupported settings produce typed refusals.

`display.uvs` is a caller-owned `Float32Array`, or `null` for uncharted output.
Corner seams can split render vertices; `maxVertices` limits both native mesh
construction and delivered render vertices, so attribute seams can cause a typed
budget refusal. Native-face correspondence remains parallel to final render
triangles. Report `chart` records the policy actually applied. Report `work` and
`curveRefinement` describe the generation retained with the output, including on
cache hits; current-call compilation/cache counters and timing are separate.
Always inspect `withinTolerance`, even if a refinement improved its bound.

Construction caps support independent attachments across families:

```ts
const peg = rail.cap("end").workplane().offset(0.5)
  .extrude(profiles.circle({ radius: 2 }).tagged("peg"), { height: 5 });
const document = session.build({ rail, peg });
const nativePlane = document.parts.rail.cap("end").workplane();
```

Authoring selectors and installed native selections remain distinct. A full
revolution has no end caps and reports `selection.missingCap`. Queried workplanes
come from exact supporting geometry; their datum is parameter zero, independent
of display triangles and texture coordinates.


A plain rectangular extrusion can retain an exact angled through bore:

```ts
const plate = extrude(profiles.rectangle({ width: 100, height: 70 }), { height: 20 })
  .throughBore({
    point: [0, 0, 10], direction: [Math.sqrt(3), 0, 1], radius: 8,
    separation: 2, feature: "bore",
  });
const document = session.build({ plate });
const bore = document.parts.plate.bore;
```

`point` and `direction` use the source construction frame, before its authored
placement. Inputs are snapshotted; subsequent caller mutations do not alter the
descriptor. `separation` is minimum remaining material to the plate sidewalls,
separate from session numeric separation and display tolerance. The first bore
operation accepts an unpocketed rectangular plate; unsupported plates, near-parallel
axes and insufficient clearance return distinct diagnostics with the failed node
and relevant numeric evidence. Elliptical rims and the cylindrical wall remain
native, including in supported STEP export. `part.bore` reports local axis inputs,
document-space evidence, rim semiaxes and native wall identity.

Closed outlines can preserve exact construction centers:

```ts
const section = profiles.loop([
  { kind: "line", to: [3, 0] },
  { kind: "line", to: [4, 0] },
  { kind: "arc", to: [3, 1], center: [3, 0], clockwise: false, feature: "rim" },
]);
const turned = revolve(section, { angle: Math.PI });
const plate = extrude(section, { height: 2 });
```

Each segment starts at the preceding endpoint, cyclically. The direction chooses
minor or major circular travel; full circles need two arcs. Inputs are copied
and immutable. Rust derives radius and sweep without moving the center or
endpoints; the same outline feeds the existing native operations. A radius
mismatch returns `profile.arc.endpointRadii` with profile/segment, feature,
`startRadius`, `endRadius` and `radiusRoundoffBound`. The bound covers local
arithmetic, independently of session separation or distance from the origin.
`profile.arc.input` and `profile.arc.numericLimit` distinguish invalid source
intent from unrepresentable derived geometry. Winding and arrangement validation
remain the same as for other profiles.

Reusable profile corner treatments select positions in profile coordinates:

```ts
const outline = profiles.polygon([
  [7, 0], [22, 0], [22, 5], [18, 5], [18, 30], [7, 30],
]).tagged("sleeve");
const rounded = outline.cornerAt([18, 5]).fillet(2, { feature: "shoulder" });
const chamfered = outline.cornerAt([18, 5]).chamfer(1, { feature: "shoulder" });
const part = revolve(rounded);
```

`cornersAt([[x, y], ...])` treats several corners simultaneously. Selections
snapshot their points and optional `{ tolerance }`; the default search distance
is the session's numeric separation. Exactly one boundary vertex must match each
position, including holes. No nearest vertex or snapped position is guessed.
Select on the profile before translating it, or use the translated coordinates.

The same treated profile can be reused in extrusion, revolution, sweep, loft and
cutout templates. Straight edges retain their tags; generated arcs/chamfers take
the supplied feature or the common adjacent tag. Whole-feature intent survives
profile treatment and translation. Geometry, validation and segment provenance
stay in the shared Rust profile library.

Treatments currently require adjacent straight segments. Smooth vertices, arc
junctions, repeated selections and trims that consume adjacent edges return
distinct `profile.corner.*` diagnostics. Errors identify the profile, selected
position or local segment and available numeric trim evidence. Corner evidence
is immutable; invalid edits never register a partial native document.

`nativeBody(description, { placement })` authors exact boundary tables directly,
without choosing a constructor family. The description includes f64 vertices,
shared bounded line/circle/ellipse edges, oriented loops and trimmed analytic
surfaces, with optional per-face feature names. Supporting frames are explicit
3×4 rigid matrices. Inputs are copied immediately; Rust validates references,
closure and support agreement when building a document. Such installed parts
report `family: "body"` and neutral `boundary` face roles. Native selections,
conservative bounds, display realization and supported STEP export remain
available. Constructor charts, cap selectors and exact integrals are explicitly
unavailable. Boundary validation does not certify global self-intersection or
nested-shell containment.

`solid.subtractThroughCylinder(options)` applies the generic retained-body
machining operation. It supports two parallel planar exits, polygonal and full
conic exit trims, and separated planar/cylindrical boundaries. Repeated cuts
preserve existing native IDs and feature tags; conservative checks explicitly
refuse tangency, intersecting cuts and unsupported boundaries. Tool points and
directions use the stock's body-local coordinates; its occurrence placement is
applied once to display, queries and STEP. `throughBore` retains the earlier
rectangular-plate constructor convenience.

`part.machining.operations` exposes each tool's local axis/radius, generated
wall/rim/seam IDs, source-to-result face correspondence, certified clearance,
native support gap, removed volume and surface-area delta. All nested SDK
metadata is immutable. When authoring started from a constructor with known
exact integrals, `part.measurements` carries
`basis: "constructorAndAnalyticMachining"`: its value is the constructor integral
plus the validated analytic operation deltas. A generic `NativeSource` by itself
has no construction-history evidence and continues to report those integrals as
unavailable; importing arbitrary `nativeBody` tables does not fabricate them.

Retained planar solids support exact native set operations:

```ts
const stock = extrude(profiles.rectangle({ width: 40, height: 30 }).tagged("stock"), { height: 12 });
const lug = extrude(profiles.rectangle({ width: 12, height: 10 }).tagged("lug"), {
  height: 12, placement: { translation: [22, 0, 0] },
});
const fused = stock.union(lug);
const common = stock.intersect(lug);
const remaining = stock.subtract(lug);
const clipped = stock.intersectHalfSpace({
  point: [0, 0, 6], normal: [0, 0, 1], feature: "cut-plane",
});
const document = session.build({ fused, common, remaining, clipped });
const evidence = document.parts.fused.solidOperations;
```

`union`, `intersect` and `subtract` operate on axis-aligned orthogonal planar
polygons with line boundaries in the left operand's construction space. Both
operands must have exactly equal orientation matrices; independent translations
are converted into that space once. A shared placement is applied once to the
result. Different orientations return `boolean.frameMismatch`, including
near-axis trigonometric rotations; no coordinate is snapped. Curves, oblique
faces, insufficient distinct-plane spacing and nonmanifold contacts are explicit
refusals. Input bodies must already represent embedded, outward-oriented regular
solids. The bridge admits at most 32,768 arrangement cells and 4,000,000 source
face tests per operation. Its numeric gap and minimum spacing use the session's
separation, independently of display tessellation.

`intersectHalfSpace` retains points satisfying
`dot(position - point, normal) <= 0` in the body's local construction space.
It accepts a strictly convex planar solid with convex line faces; holes, curves,
nonconvexity and planes within session separation of existing vertices are
refused. The new cap receives `feature`. Source tags survive through native face
correspondence. A complete removal returns `halfspace.empty`; an empty Boolean
returns `boolean.empty`, because document parts require positive geometry.
Failed builds leave previously installed documents usable.

`part.solidOperations` exposes native work, outcome and typed source
correspondence. Each entry identifies its authoring graph node and operands;
face/edge/vertex IDs belong to those intermediate operation bodies, not
necessarily the final part. Boolean evidence includes material components,
cavities, shells, exact volume/area and outward-rounded arithmetic bounds.
Output faces combine all contributing source tags from `faceSources`, including
reversed difference faces. Boolean volume/area are native measurements;
clipped generic bodies deliberately report integral measurements unavailable.
No display mesh is used for any operation or integral evidence.
