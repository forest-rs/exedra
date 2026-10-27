# Native authoring in the browser

A live TypeScript authoring workflow: reusable tagged vents, a native blind
recess and a separate pin attached to its native floor. Controls rebuild actual
Rust/Wasm geometry. Coarse/fine display controls reuse the installed native
source, with source-aware surface picking and exact native measures.

From this directory, with Node.js 22.18+ (or 24+), Rust's
`wasm32-unknown-unknown` target, and `wasm-pack` installed:

```sh
npm ci
npm run dev
```

Open **http://127.0.0.1:5174/**. `predev` compiles the local SDK and generated
Wasm package. The first build may download wasm-pack's matching wasm-bindgen
tool. `npm run build` creates the production app; `npm run preview` serves it.
`npm run test:sdk` runs the SDK ownership/authoring regressions.

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

Download buttons export the named **panel** or **pin**, one native STEP solid per
file in millimetres. They do not imply an assembly export and never pass through
the display mesh. The report includes native support/frame tolerance evidence
and native-face to STEP-entity correspondence. Invalid drafts disable download
until a valid rebuild succeeds.

This first SDK sample exposes native extrusions, planar attachments, through
cutouts and blind pockets. It does not expose every Rust modeling operation.
