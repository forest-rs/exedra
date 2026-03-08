# cambium_web_bridge

Wasm bridge crate that executes deterministic, named Cambium scenarios and
returns step snapshots as JSON for browser demos.

Current scenarios:
- `boxy_hat`
- `wall_openings`
- `poked_grid`
- `cylinder_normals` (cylinder side faces rebaked from flat to smooth authored normals)
- `region_select_flow`
- `uv_projection_gallery`
- `topology_dissolve_repair` (planar grid strip showing split-edge, dissolve-vertex, then dissolve-edge simplification)
- `primitive_gallery` (quad, box, cylinder, grid, cone, torus, uv_sphere, icosphere)
