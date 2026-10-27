# Geometry gallery

Editable TypeScript samples: a pocketed plate, collar, rounded ring, solid and
hollow toroidal seals, a flanged sleeve and a turned fitting. Controls rebuild actual
Rust/Wasm geometry. Slider input coalesces to one latest-value build before the
next frame is drawn, without a trailing debounce or reduced interaction quality. Coarse/fine display controls reuse the installed native
source, with source-aware surface picking and exact native measures.

From this directory, with Node.js 22.18+ (or 24+), Rust's
`wasm32-unknown-unknown` target, and `wasm-pack` installed:

```sh
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

The current SDK exposes native extrusions, planar attachments, through cutouts,
blind pockets and line/arc profile revolutions. It does not expose every Rust
modeling operation. Bounds are conservative native support enclosures and can
include space outside trims. Area/volume availability is explicit in the SDK;
unsupported or numerically refused values are never replaced with display estimates.
