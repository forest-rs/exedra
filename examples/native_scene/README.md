# Native scene example

A caller-owned Layerstack stage contains one tagged, pocketed native panel and
1,000 instance placements. Native USD layers own parameters, placements,
appearance and undo. The scene adapter uses the shared geometry compiler and
publishes an ordinary polygon Mesh below the authored source.

```sh
cargo run --release -p native_scene --bin native_scene -- /tmp/native-scene-output 1000
```

The sample performs initial publication, a no-op, attribute and material edits,
a pocket-depth edit, a placement edit, undo, and a display-quality change. Its
assertions check work counts, exact native measurements, tagged floor picking,
source selection scope, original-buffer reuse after undo, and held snapshots
after compiler/encoding eviction and evaluator destruction.

The output directory contains `scene.usda`, `authored.usda`, `generated.usda` and
`work-report.json`. Open `scene.usda` together with its two sibling layers. Units
are explicitly millimetres (`metersPerUnit = 0.001`) and the up axis is Z; there
is no implicit coordinate rescaling. Standard fallback prim types let other USD
readers inspect the custom procedural parent and its ordinary Mesh child.
Opening those files does not execute a generator. Procedural parameters and
polygon output are saved; the original native boundary remains in the running
application, not in an invented USD analytic serialization.

The JSON separates `applyMs` (the native source-edit transaction) from `syncMs`
(the explicit Exedra synchronization). These are observations from one run,
not a controlled benchmark. Work counts distinguish cached input reads,
source lowering, geometry compilation, polygon encoding/validation and actual
publication writes. Native instances share one generated asset; a no-op still
checks the owned property manifest, so zero geometry work does not mean zero CPU
work. Encoding payload bytes exclude allocator overhead and other owners.

Native parent extents enclose conservative analytic support plus emitted points,
with outward float conversion. The Mesh child's extent describes its actual f32
points. Native face IDs are a uniform primvar in generated polygon order;
queries describe that retained producer output, independently of stronger
partners' changes to composed geometry.

## Verification

```sh
cargo test -p exedra_scene -p native_scene --all-features
python examples/native_scene/scripts/validate_usd.py /tmp/native-scene-output
```

The consumer test compares each incremental state with a fresh store snapshot,
empty derived layer, newly composed stage, new generator and fresh compiler.
It compares named composed geometry, attribute types/values/variability and
interpolation metadata, relationships/connections, material resolution and
world transforms. Installation scopes and transient handles are excluded.
Another test deliberately misses a circle extremum with coarse sampling,
checks independent native source bounds and actual Mesh bounds, and demonstrates
that a stronger point override does not rewrite the retained native model.

The optional independent reader script uses existing OpenUSD Python bindings.
It checks composition, units, standard source fallback, topology, finite points,
primvar cardinality, native face IDs, extents, material/shader bindings, instance
placements and proxy geometry. Missing bindings report unavailable and exit 77;
failed checks fail. It does not install tooling or claim general conformance.

`cargo run -p native_scene --bin primitives` retains the smaller built-in
box/cylinder authoring, regeneration, native bounds and undo example.

## Native archive save and reopen

```sh
cargo run -p native_scene --bin native_archive -- /tmp/native-archive-output
cargo test -p native_scene --bin native_archive
```

The example writes `housing.usda` containing a machined body with explicit UV
trim correspondence and four additional valid sources covering cone, sphere,
torus and ruled supports. It drops the original store, parses the saved file into
a fresh store and compares exact topology/tag words and every f64 bit, including
negative zero, an awkward occurrence datum and a next-representable radius.
After reopening, it machines a second bore and writes `housing-second-bore.step`.
Display realization stays in body-local coordinates; the exact large-datum
occurrence frame remains separate and is applied by the f64 STEP exporter.
The archive uses Exedra's versioned ordinary attributes, not the USD-BRep schema.
