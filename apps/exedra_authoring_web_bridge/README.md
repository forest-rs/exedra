# Native authoring Wasm bridge

The browser adapter for immutable profile/construction descriptions, retained
native documents and binary display buffers. Used by the
[TypeScript SDK](../../packages/exedra/README.md) and
[interactive sample](../exedra_authoring_demo/README.md).

Rust owns validated profile geometry, native construction, queries, realization
and STEP exchange. The bridge owns document handles, per-document named feature
tables, bounded compiler caching and structured diagnostics. Hosts choose units,
request display quality, own transferred arrays and explicitly release documents.
The request graph is a private application wire seam, not a persistent format.

`AuthoringKernel` creates at most eight immutable documents and caches at most
eight realizations. It preflights references/depth/work limits, lowers shared
profile and construction nodes once, then atomically registers named root
sources after metadata is validated. Refusals leave older documents intact.
Releasing the final document clears the compiler cache. Metadata preserves
fingerprints as strings and native IDs as bounded u32 numbers.

`NativeBuffers` transfers packed f32 positions/normals and u32 indices/native
face IDs through wasm-bindgen; geometry never travels as JSON number arrays.
Triangle correspondence comes from the compiler's final extraction order.
Small metadata/reports use JSON. Generated handles are freed explicitly by the
SDK, including error paths. `NativeStepFile` exports an independent native part
and carries native-to-STEP correspondence and tolerance evidence.

```sh
cargo test -p exedra_authoring_web_bridge
cargo clippy -p exedra_authoring_web_bridge --all-targets --all-features -- -D warnings
```

Build the browser package with the demo's `npm run prepare:sdk` command. The
bridge is a Wasm host adapter; portable native/core checks remain on their owning
crates.

Retained solid nodes lower to the analytic rectilinear Boolean and convex planar
halfspace kernels. The SDK exposes `union`, `intersect`, `subtract` and
`intersectHalfSpace`; their support, frame, budget and empty-result contracts are
documented in the [SDK guide](../../packages/exedra/README.md). Typed operation
reports retain source correspondence in intermediate authoring-node scope.

After building the SDK and Wasm, verify the actual public boundary with:

```sh
node --experimental-strip-types apps/exedra_authoring_demo/scripts/smoke-solid-operations.mjs
```
