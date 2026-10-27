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
revolutions retain cylinders, cones and regular toroidal walls. Unsupported STEP
surfaces produce an `exchange.unsupportedSurface` diagnostic; the source stays
usable for display and native queries. The current exporter supports plane and
cylinder faces, not cone, torus or ruled faces.

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
