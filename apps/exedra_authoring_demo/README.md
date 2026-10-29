# Geometry gallery

Editable TypeScript samples: a pocketed plate, collar, rounded ring, solid and
hollow toroidal seals, a flanged sleeve, a turned fitting, solid/hollow swept rails,
a four-section loft and selected outer fillets/inner chamfers. Controls rebuild actual
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

[The actual authoring function](src/sample.ts) is also available through **View
TypeScript** in the app. Geometry/validation live in Rust, reusable descriptions
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
its geometry is supported. Plane/cylinder samples export; cone, torus and ruled
supports show a disabled export button with the unsupported face/family reason.
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
binary loads only when this view is mounted. The combined backend owns one
store/stage and attaches the Exedra provider to it; the generic Stage client is
owned by Layerstack and accepts an injected transport. No handles cross the
geometry-only and scene Wasm memories. Both Rust and TypeScript use the exact
same checked Layerstack revision, including clean CI checkout.

The view shows a shared tagged native prototype at actual stage instance paths.
Source parameter edits, one-instance placement, undo, override-layer mute and
quality changes exercise native transactions and explicit geometry sync. Work
counts distinguish source lowering, geometry, polygon encoding/validation and
publication. Discovery remains visible: first stronger opinions or declarations
can scan the composed namespace despite reusing geometry. Numeric transfer counts
exclude metadata. Stage, sync and packing
timings measure Wasm calls including ABI work; client elapsed additionally
includes asynchronous transport overhead. They are observations, not controlled
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
