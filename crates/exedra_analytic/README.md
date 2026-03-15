# exedra_analytic

Experimental analytic geometry head for the Exedra workspace.

This crate is a narrow spike, not a full CAD kernel. The current scope is:

- planar faces,
- line-segment coedges,
- shell/loop/coedge topology,
- deterministic tessellation into `exedra::Mesh`.

The goal is to prove the multi-domain architecture with one honest second
canonical domain, while keeping `exedra` as the polygon head.

## Current limitations

- no curved edges,
- no general trims,
- no booleans,
- no hole loops in a single face yet,
- no reverse conversion from mesh to analytic state.

## License

Licensed under either of Apache License 2.0 or MIT license at your option. See
the workspace root for license files.
