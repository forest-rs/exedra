# `exedra_analytic`

Narrow analytic geometry head for Exedra. It retains planar topology and
face-level provenance until an explicit tessellation into `exedra_mesh::Mesh`.

```rust
use exedra_analytic::{RectFrameParams, TessellateParams, rect_frame_xy};

let shell = rect_frame_xy(&RectFrameParams::default()).expect("valid frame");
let output = shell
    .to_exedra_mesh(&TessellateParams::default())
    .expect("planar shell tessellates");

assert_eq!(shell.faces().len(), 1);
assert_eq!(output.mesh.faces().count(), 8);
assert!(output.mesh.validate_deep().is_empty());
```

Use `AnalyticShellBuilder` for arbitrary planar faces, `AnalyticShell` for
retained topology and edits, and `TessellatedShell` for the mesh plus its
analytic-face provenance.

This crate is deliberately not a full CAD kernel. The current scope is:

- planar faces,
- line-segment coedges,
- shell/loop/coedge topology,
- explicit planar opening loops,
- face-level mutation for regions and opening add/remove lifecycle,
- deterministic tessellation into `exedra_mesh::Mesh`.

The goal is to prove the multi-domain architecture with one honest second
canonical domain, while keeping `exedra_mesh` as the polygon head.

The default `std` feature selects native floating-point support. For `no_std`,
disable defaults and enable `libm`.

## Current limitations

- no curved edges,
- no general trims beyond explicit opening loops,
- no booleans,
- no arbitrary 3D opening editing outside XY-aligned planar faces,
- no reverse conversion from mesh to analytic state.

## License

Licensed under either of Apache License 2.0 or MIT license at your option. See
the workspace root for license files.
