# exedra_analytic

Experimental analytic geometry head for the Exedra workspace.

This crate is a narrow spike, not a full CAD kernel. The current scope is:

- planar faces,
- line-segment coedges,
- shell/loop/coedge topology,
- explicit planar opening loops,
- face-level mutation for regions and opening add/remove lifecycle,
- deterministic tessellation into `exedra_mesh::Mesh`.

The goal is to prove the multi-domain architecture with one honest second
canonical domain, while keeping `exedra_mesh` as the polygon head.

## Current limitations

- no curved edges,
- no general trims beyond explicit opening loops,
- no booleans,
- no arbitrary 3D opening editing outside XY-aligned planar faces,
- no reverse conversion from mesh to analytic state.

## License

Licensed under either of Apache License 2.0 or MIT license at your option. See
the workspace root for license files.
