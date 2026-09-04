# exedra_ops_web_viewer

Local-first Three.js viewer for deterministic Exedra Ops scenario snapshots.

This viewer is the interactive web demo surface for Exedra Ops scenarios. It runs
named scenarios in wasm, then visualizes ordered step
snapshots with deterministic metadata (plan fingerprints, mesh signatures,
stats, diagnostics).

## Prerequisites

- `node` + `npm`
- `wasm-pack`

## Run

```sh
cd apps/exedra_ops_web_viewer
npm install
npm run dev
```

`predev` builds wasm bridge assets from `../exedra_ops_web_bridge` into
`src/wasm_pkg/`.

## Scenario list

The scenario picker currently includes:

- `boxy_hat`: inset + extrude progression over a quad.
- `wall_openings`: wall, rectangular cuts, deletes to open door/window, solidify.
- `poked_grid`: planar grid with three deterministic poke-and-raise steps for terrain-style topology buildup.
- `bridge_loops`: select one open boundary loop, bridge two parallel loops into a quad strip, then tag the strip region.
- `cylinder_normals`: cylinder side faces rebaked from flat to smooth authored normals with unchanged geometry.
- `region_select_flow`: tag faces, select/flood by region, inspect deterministic counters.
- `uv_projection_gallery`: planar/box/cylinder projection pass sequence.
- `topology_dissolve_repair`: planar grid-strip topology story showing split-edge, dissolve-vertex, then dissolve-edge simplification.
- `primitive_gallery`: quad, box, cylinder, grid, cone, torus, uv-sphere, and icosphere constructor outputs.

## Provenance Inspector

`inspector.html` (linked from the main page) is a separate inspection
instrument over the bridge's `exedra-ops-inspect-v1` payloads: it renders
inspection scenarios (`drilled_block`, `policy_curve`, `panel_trio`) with
region coloring and topology edges on a dark instrument layout, and
clicking any face surfaces its full provenance chain — instance path,
part, node (kind + fingerprint), feature attribution, region, source
reference, material slot, issue citation, and fidelity verdict — beside
a summary readout and the exact diagnostics ledger. The selected face
highlights in the accent color. Reloading a scenario re-renders the
byte-identical payload.

## Viewer Controls

- Scenario picker: choose and rerun a named flow.
- Step slider: scrub deterministic snapshots.
- `Wireframe`: enable triangulation wireframe on shaded mesh.
- `Region Colors`: proxy region coloring mode (v0.1 approximation).

## Metadata Panels

- `Operator`: stable operator name for that step.
- `Fingerprint`: deterministic plan fingerprint (`null` for pure snapshot steps).
- `Mesh signature`: deterministic mesh signature after the step.
- `Stats`: compact deterministic counters and creation/deletion summaries.
- `Diagnostics`: emitted operator diagnostics for the step.

## Static Build + Publish

Build deployable static assets:

```sh
cd apps/exedra_ops_web_viewer
npm run build
```

Smoke-check the output bundle:

```sh
npm run smoke:dist
```

Preview the generated static site locally:

```sh
npm run preview
```

The deployable output is `apps/exedra_ops_web_viewer/dist/` and includes:

- `index.html`
- hashed JS/CSS assets
- wasm bundle from `exedra_ops_web_bridge`

## Troubleshooting

- `exedra_primitives requires either std or libm` during wasm build:
  ensure the bridge keeps `libm` enabled explicitly for `exedra_mesh`,
  `exedra_ops`, and `exedra_primitives`; the bridge uses that deterministic
  backend on every target.
- `wasm-opt` permission issues in restricted environments:
  run build with appropriate permissions or configure wasm-pack accordingly.
- Empty/blank viewer:
  ensure `npm run build:wasm` succeeds and `src/wasm_pkg/` is populated.
