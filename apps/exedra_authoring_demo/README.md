# Geometry gallery

Editable TypeScript samples: a pocketed plate, collar, rounded ring, solid and
hollow toroidal seals, a flanged sleeve, a turned fitting, solid/hollow swept rails,
a four-section loft, selected outer fillets/inner chamfers, spheres, spherical caps,
pointed cones, machined/fused-lug housings and a machined valve block. Controls rebuild actual
Rust/Wasm geometry. Slider input coalesces to one latest-value build before the
next frame is drawn, without a trailing debounce or reduced interaction quality.
Rendering is requested for edits, display changes, picking, resize and camera
interaction; damping continues until the camera settles. Coarse/fine display controls reuse the installed native
source, with source-aware surface picking and exact native measures.

From this directory, with Node.js 22.18+ (or 24+), Rust's
`wasm32-unknown-unknown` target, and `wasm-pack` installed:

```sh
git clone https://github.com/forest-rs/layerstack ../../.local/partners/layerstack
git -C ../../.local/partners/layerstack checkout --detach "$(node scripts/check-layerstack-partner.mjs --revision)"
npm --prefix ../../.local/partners/layerstack/packages/layerstack ci
npm run check:lockfile
npm ci
npm run dev
```

Open **http://127.0.0.1:5174/** (or use Vite's `--port` option for a parallel preview). `predev` compiles the local SDK and generated
Wasm package in release mode and runs wasm-opt. Dependencies resolve from the public npm registry; no registry account is required.
The lockfile check rejects private registry URLs before CI installs dependencies.
The first build may download wasm-pack's matching wasm-bindgen
tool. `npm run build` creates the production app; `npm run preview` serves it.
`npm run test:sdk` runs the SDK ownership/authoring regressions. After building,
`npm run test:wasm` exercises every sample through the generated Wasm, including
cached/fresh correspondence, held documents, accuracy outcomes and STEP support.

**View TypeScript** opens the selected sample's actual author function, its
parameter types and local helper functions. **Full source** keeps the complete
[authoring file](src/sample.ts) available. Both views are extracted and highlighted
at build time; the browser does not load a TypeScript parser. Geometry/validation live in Rust, reusable descriptions
and lifetimes live in the [SDK](../../packages/exedra/README.md), and Three.js
owns the camera, materials, picking presentation and GPU resource disposal.

Try selecting **vents**, switching coarse/fine quality, and enabling mesh
triangles: the triangle counts change while volume and selected native source
stay fixed. Raise recess depth beyond thickness to see a typed refusal with the
last good model retained; quality changes preserve that refused-draft warning.
Restore a valid depth to recover. An authored parameter change clears stale
selections. **Check rebuild / release** builds and releases twelve revisions and
checks that live document/part ownership returns to baseline with a bounded
compiler cache.

Download buttons export one named native part per STEP file in millimetres when
its geometry is supported. Plane, cylinder, cone, sphere and torus supports export
with supported boundary topology. Ruled supports show the unsupported face/family
reason; multiple shells require explicit solid/cavity ownership and are refused.
The SDK still returns a typed refusal if export is called directly. Structural
support does not guarantee final tolerance, topology or budget validation. They do not imply an assembly export and never pass through
the display mesh. The report includes native support/frame tolerance evidence
and native-face to STEP-entity correspondence. Invalid drafts disable download
until a valid rebuild succeeds.

The SDK exposes native extrusions, planar attachments, through cutouts, blind
pockets, line/arc profile revolutions, smooth sweeps, corresponding-section ruled
lofts and named profile axial-edge finishes. It does not imply arbitrary 3D edge
fillets or automatic section correspondence. Bounds are conservative native support enclosures and can
include space outside trims. Area/volume availability is explicit in the SDK;
unsupported or numerically refused values are never replaced with display estimates.

Timing rows report host-observed Wasm calls, including refused calls. The SDK
separates graph preparation, kernel-call boundaries, buffer transfer, metadata
decoding and cleanup; these are not GPU or presentation timings. After building,
`npm run bench:wasm` performs four warmups and seventeen actual parameter edits
per sample, reporting p50/p95 times, buffer bytes, work/cache counts and released
logical native ownership. It excludes view creation and GPU work.

## Native stage view

Select **View → Scene** in the gallery to open the native Layerstack scene application.
`/scene.html` also runs this view on its own. Its separate Wasm
binary loads only when this view is mounted. The official `@forest-rs/layerstack` binding and Exedra extensions share the same
application-owned Stage and combined Wasm module. Typed edits, layer inspection,
change listeners and native geometry operate on that Stage through Layerstack’s
public API. No handles cross the geometry-only and scene Wasm memories. Rust,
TypeScript source and the runtime version report use the same checked Layerstack
revision, including clean CI checkout. The package build uses the combined
module’s actual generated glue; it does not initialize a second Layerstack Wasm.
The Stage opts into provenance for native correspondence. Public layer muting
and typed edit receipts use the official binding; generated geometry stays in
its own layer, so undo changes authored inputs and the next sync republishes the
derived output. Extension refusals use `LayerstackError.code` and `.operation`.

The view shows a shared tagged native prototype at actual stage instance paths.
Source parameter edits, one-instance placement, undo, override-layer mute and
quality changes exercise native transactions and explicit geometry sync. Work
counts distinguish source lowering, geometry, polygon encoding/validation and
publication. Discovery remains visible: first stronger opinions or declarations
can scan the composed namespace despite reusing geometry. Numeric transfer counts
exclude metadata. Stage, sync and packing
timings measure Wasm calls including ABI work; Stage API elapsed also includes
typed client conversion and notification work. They are observations, not controlled
benchmarks. Native accuracy is source-local; GPU occurrence matrices use f32.

Picking maps the actual composed polygon ordinal and instance path to retained
native face tags. Stronger geometry overrides disable that native correspondence.
The renderer deliberately accepts this triangle/normal/native-ID vocabulary;
unsupported topology is refused. USD inspection/download returns the real
caller-owned layers; keep the four named sibling files together. Native BRep
remains runtime data and is not invented as a USD schema.

The app shell can mount into separate viewport/inspector and control slots.
`setActive(false/true)` pauses rendering while preserving one bounded Stage and
its edits/undo. `dispose()` releases it on teardown; changing instance count is
an explicit reset. `/scene.html` is also a standalone integration entry for
testing this same module.

The flanged sleeve has independent bore, sleeve and flange radii, shoulder height,
total height and turn angle. Changing total height leaves shoulder height fixed.
Each field retains its value when switching samples. Invalid dimensions preserve
the last valid model with a diagnostic; values are not silently clamped.


The angled bore sample uses the exact native plate/bore operation. Edit the axis
tilt, radius and minimum edge clearance, inspect its elliptical rims with mesh
triangles, and select the named bore wall. Its STEP file keeps native ellipse and
cylinder geometry. Refused edits preserve the last valid display.

Construction checker coordinates are available on revolutions, sweeps and lofts.
Rest radius, rest length, reference section and station controls change the
construction chart without changing native identity. An unsupported chart or
invalid metric is diagnosed explicitly; no mesh projection is substituted.

`npm run bench:wasm -- --charts` repeats the same edit workload with available
construction charts and includes UV transfer bytes in the reported buffer total.

`npm run test:views` checks lazy Scene initialization, retained Stage lifetime,
late completion, retries and teardown independently of GPU execution.

The flanged sleeve starts with a radius-2 profile fillet at its concave shoulder.
Its generated face is an exact torus; selecting **Chamfer** produces an exact cone.
The size control is independent of shoulder height, total height and all radii.
Zero leaves the sharp source section. Trims that consume an adjacent edge are
refused by the native profile operation, preserving the last valid display.
**View TypeScript** shows the reusable `cornerAt(...).fillet/chamfer(...)` call.

`profile-pipeline-fixtures.mjs` can compare unchanged native fingerprints, measures,
face metadata, render-buffer hashes and deterministic GLB bytes across generated
Wasm packages. The sharp sleeve fixture deliberately remains sharp while the
interactive sleeve defaults to a rounded shoulder.

The sphere, spherical-cap and pointed-cone samples use full-turn native
revolutions. Sphere and cone author ordinary reusable center-arc/line profiles;
the cap delegates its original radius and height to Rust's canonical section
builder. Radius and height remain independent, including hemispheres and taller
caps. Invalid dimensions retain the previous display with a typed diagnostic.
The inspector keeps the same measurements, kernel timings, face picking,
construction checker and exact STEP download paths. Show mesh triangles to
inspect welded poles, and change display quality without changing native identity.

The maintained Wasm inventory checks exact analytic measures, emitted closed
oriented meshes, unit normals, coarse/medium/fine accuracy, charted poles and
explicit collapsed-vertex STEP correspondence. It also checks recovery from an
invalid cap height and from a representable height too small to preserve its rim.

The Machined housing sample authors a flanged polygon blank followed by four
retained-body cylinder cuts. Thirteen independent controls cover stock, flange,
main passage, two mounts, service port and material clearance. Selecting a named
passage shows its exact native cylinder, local tool axis, radius, rim identities
and certified clearance. Build, realization and STEP timings are actual SDK/Wasm
calls. Intersecting/invalid edits preserve the last valid displayed model and
prevent stale export. The generated-release-Wasm smoke suite checks every prior
gallery case plus housing, independent parameter edits, selections across quality
changes, STEP, failed drafts, repeated release and arbitrary native-body import
with explicit face-local trims. Native archive layer save/reopen is currently a
Rust `exedra_scene` adapter; this geometry view does not claim browser-native USD
BRep editing or persistence.

Fused-lug housing is a second path through the same controls and machining
sequence: two actual native rectilinear unions attach left and right box lugs to
a central box blank. The SDK exposes the Boolean arrangement work, source-face
origins and complete source images through `part.solidOperations`. Merged faces
can retain several source feature names. Exact Boolean integrals plus analytic
machining deltas use `basis: "nativeBooleanAndAnalyticMachining"`.

## Native housing machining

**Machined housing** uses stepped polygon stock and four retained-body cylinder
cuts. **Fused-lug housing** starts with three box blanks, performs two native
unions, then applies those cuts. Both expose 13 independent dimensions and named
stock, lug and passage selections. Selecting a passage shows its native axis,
radius, rims and clearance evidence. Quality changes retain the native selection.

An overlapping service passage produces a structured clearance refusal, retains
the last valid model and disables STEP export until corrected. The inspector
distinguishes constructor-derived machining measures from native Boolean measures.
`solidOperations` and `machining` provide typed operation-local correspondence.
The Boolean kernel accepts orthogonal planar/line bodies; drilling accepts
disjoint cylindrical passages through two parallel planar exit patches.

`npm run test:wasm` also checks union, intersection, subtraction, convex halfspace
cuts, cavities/disconnected outputs, placed operands and refused-edit recovery.
Native archive save/reopen is separately available through the Rust
`exedra_scene::native` adapter; the browser scene view does not yet expose it.


The **Machined valve block** sample combines a union of overlapping horizontal
cylinders, a stepped vertical passage through nonparallel planar faces and four
blind fastener holes. Eleven controls change its actual retained geometry. The
joined chamber has no internal wall; the counterbore has a shared annular shoulder
and each blind hole has a native floor. Select each named feature, vary display
quality, or open `/?sample=valveBlock&wireframe=1` to inspect its mesh triangles.
Near-tangent chambers and invalid bore depths retain the last valid revision.
`smoke-machining.mjs` checks every control, typed recovery, feature correspondence,
STEP export and the fractional-width fused-lug regression using release Wasm.

**Inspect native boundary** runs an explicit timed topology/trim inspection.
Certified and unsupported trim arrangements are counted separately; a local
manifold result does not certify global self-intersection, outward shell
orientation or material containment. Inspection never runs implicitly on slider
edits. The SDK exposes the full per-face report and effective work policy.
The Rust `native_scene --bin native_archive` example saves this compound valve
as retained native geometry in USDA, reopens it in a fresh store with exact-bit
comparisons, adds another blind service bore and exports STEP. This native
archive witness is separate from the gallery's scene layer download.
