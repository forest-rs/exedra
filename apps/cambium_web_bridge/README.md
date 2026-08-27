# cambium_web_bridge

Wasm bridge crate that executes deterministic, named Cambium scenarios and
returns step snapshots as JSON for browser demos.

## Inspection payloads (`cambium-inspect-v1`)

`run_inspection_scenario_json(name)` evaluates a constructive scenario and
returns a versioned provenance payload that lets a viewer resolve any
picked triangle to its full construction chain without linking Rust:

```
triangle t -> bodies[b].tri_face[t] -> bodies[b].faces[f]
           -> node (bodies[b].node, scoped by bodies[b].part)
           -> nodes table entry (kind, fingerprint, source, material, issue)
```

The payload also carries the full diagnostics ledger, per-node fidelity
verdicts (`exact` / `policy_defined` / `conflicted` / `envelope_only`),
policy-curve usage, instance placements (16-value column-major matrices),
and aggregated evaluation counters. All lists are emitted in
deterministic order; reruns serialize byte-identically. Evolution within
`cambium-inspect-v1` is additive-only.

Inspection scenarios (`list_inspection_scenarios_json()`):

- `drilled_block`: a cylinder drilled through a slab — the through-hole
  boolean with per-operand provenance on every face.
- `policy_curve`: an underspecified edge realized under a named curve
  policy with a cited spec issue — exercises `conflicted` fidelity.
- `panel_trio`: the multi-instance assembly scene; three placements share
  one provenance-attributed body.

`InspectionSession::new("panel_trio", space)` retains that assembly as one
Addressable runtime across calls. Its `snapshot_json()` method projects the
current host through the same `cambium-inspect-v1` payload, while `space` and
`revision` expose the observation context. The one-shot function remains
available for recipe-only scenarios and compatibility.

For an assembly pick, the instance entry supplies the canonical exact address
and its producing node supplies the material-slot name. The viewer retains
that pair as the typed material-read target; the payload does not invent a
second endpoint syntax.

Current scenarios:
- `stepped_tower`
- `pedestal`
- `boxy_hat`
- `wall_openings`
- `poked_grid`
- `bridge_loops` (select left boundary loop, bridge two parallel loops, then tag the bridge strip)
- `cylinder_normals` (cylinder side faces rebaked from flat to smooth authored normals)
- `region_select_flow`
- `uv_projection_gallery`
- `topology_dissolve_repair` (planar grid strip showing split-edge, dissolve-vertex, then dissolve-edge simplification)
- `primitive_gallery` (quad, box, cylinder, grid, cone, torus, uv_sphere, icosphere)
