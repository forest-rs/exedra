# Native scene browser bridge

This unpublished application module owns one native Layerstack store and stage,
plus the shared Exedra geometry compiler and scene adapter. Layerstack owns
composition, explicit-layer transactions, inverse receipts, queries and mute
controls. Exedra owns the native construction provider, compilation and derived
Mesh publication. The generic TypeScript client lives in Layerstack; it talks to
this same stage through an injected transport.

`SceneKernel(instances, realm)` installs the shared native panel provider and
authors base, override and generated layers with millimetres and Z up. Pass a
unique host-generated realm such as `crypto.randomUUID()` for each backend,
including separate Wasm module instances. Stage/store/layer/receipt IDs retain
that realm and use strings. The demo never exchanges raw geometry handles with
the separate geometry-only Wasm module.

`request` accepts Layerstack's version-one generic envelope and the separate
`exedra.scene` extension on the same transport. Source authoring and geometry
synchronization are explicit operations. Native transaction failures are atomic;
generator or publication failure preserves last-good output. Request size,
transaction/query sizes, instance count and retained inverse receipts are
bounded. Release unused receipts or dispose the stage to release ownership.

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
JSON work; client elapsed also includes asynchronous transport overhead. No
clock or DOM dependency enters the core adapter.

The web build checks out the TypeScript partner at the exact Layerstack git
revision used by all three Rust dependencies. The geometry-only SDK does not
import this runtime; the Scene module and its separate Wasm binary load lazily.
