# Native scene browser bridge

The official Layerstack binding owns the native document, composition, typed
transactions, undo receipts, queries, asset loading and persistence. Exedra owns
only construction, compilation, derived Mesh publication and display caches.
Both bindings are linked into one Wasm module and borrow the same document.

Open `scene_root()` through the official `Layerstack` client with identifier
`scene.usda` and `provenance: true`. Use `stage.withNative` to construct
`SceneKernel(nativeStage, instances, realm)` with a unique host-generated realm
such as `crypto.randomUUID()`. Construction supplies the three named dependency
layers and authors the demo assembly. The official registry supplies standard
schemas; the root's fallback type metadata identifies the custom procedural.

`sync(nativeStage)`, `pack(nativeStage)` and `pick(nativeStage, ...)` borrow the
caller's official stage for that call. Store identity rejects another document
before mutation. No pointer or borrowed view is retained by the geometry kernel.
Disposing the kernel leaves the stage usable; the official client guards calls
after stage disposal. `set_display_tolerance` affects realization only.
Layer muting, editing, undo, inspection and export use the official client
directly. Native errors use the official binding's structured error record:
`scene.provenance` requires provenance at creation, `scene.stage_affinity`
rejects another document, `scene.stale_pick` requires current display scope,
and `scene.geometry` reports failed geometry evaluation. `Stage.withNative`
preserves these codes and attaches the caller's operation name.

Native publication mutates the official document's store, so `save` observes the
actual dirty layer generations. All four named layers can be inspected,
downloaded and reopened through the official API. Generator or publication
failure preserves last-good output. Instance count and geometry work remain
bounded by their existing native policies.

`pack` extracts actual composed USD Mesh values and instance world transforms,
not a JavaScript scene tree or a second geometry model. Immutable array storage
identity skips revalidation and repacking; numeric buffers transfer once through
an owned `ScenePacket`. Placement changes emit only changed f64 matrices. Missing
property inventories retain conservative transform queries; comparisons with
last-sent matrices still suppress unchanged transfers. Complete inventories use
Layerstack's schema-owned transform-property rule. A real ancestor transform
updates descendants.
New declarations or first stronger opinions can also trigger conservative stage
membership/discovery scans. Zero geometry work is reported separately from
those scans and does not imply constant host work for every placement edit.

The first renderer accepts triangle polygons with face-varying normals and
uniform native face IDs; other composed topology is refused explicitly. Native
picking requires the current source scope, an actual instance path and unchanged
producer provenance. Stronger geometry overrides do not become native geometry
or inherit its accuracy report. Source-local deviation includes native
realization emission; the renderer's f32 occurrence matrices are a separate
approximation. Numeric transfer counts exclude metadata and allocator overhead.

Before using the shared prototype, the view verifies each current native
prototype's actual mesh proxy buffers and authored provenance against the
retained source. Equal-content sources may share numeric allocations while
retaining different identities. Foreign geometry, extra boundables and
unsupported child-mesh transforms or interpolation return
`scene.instanceCorrespondence`; picking performs its own current-instance check.
Prototype IDs never survive composition revisions. Successful verification is
cached only for the same native change revision and source scope, with
`prototypeChecks`/`instanceChecks` reporting its work. Failed checks do not
advance last-sent geometry or transform state.

The app observes stage-call, synchronization, packing and buffer-copy elapsed
times separately with its host clock. Wasm-call timings include their ABI and
report encoding; client elapsed includes official wrapper decoding. No
clock or DOM dependency enters the core adapter.

The web build checks out the TypeScript partner at the exact Layerstack git
revision used by the Rust dependencies. The geometry-only SDK does not
import this runtime; the Scene module and its separate Wasm binary load lazily.
