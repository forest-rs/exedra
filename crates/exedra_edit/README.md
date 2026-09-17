# exedra_edit

Mesh command execution, preview and reporting for editor applications.

Use [`exedra_mesh_ops`](../exedra_mesh_ops/) for direct modeling and geometric
queries. `exedra_edit` adds command preparation, stale-plan checks, clone-based
preview, timings, diagnostics and change reports. Both paths call the same
geometry algorithms. The mesh kernel owns topology and primitive edits.

This crate is `#![no_std]` with `alloc`: choose the default `std` feature or
`libm`. The `exedra` facade exposes it through the opt-in `edit` feature.
Constructive evaluation, analytic edits and assembly expansion use their native
crates directly; there are no cross-domain runner adapters.

```rust
use exedra_edit::{Mesh, OperatorRunner, ValidateMesh, ValidateMeshMode, ValidateMeshParams};

let mesh = Mesh::new();
let mut runner = OperatorRunner::new();
let operation = ValidateMesh;
let params = ValidateMeshParams {
    mode: ValidateMeshMode::FastAndDeep,
};

let plan = runner.compile(&mesh, &operation, &params).expect("valid plan");
let preview = runner
    .preview_on_clone(&mesh, &operation, &plan)
    .expect("preview succeeds");
assert_eq!(preview.report.name, "inspect.validate.mesh");
```

Use `OperatorRunner` for compile/preview/apply and structured reporting.
Use `MeshEdit` for a sequence of supported mesh commands. Edit sessions are
**eager**: in-place failures can leave partial changes; dropping a session does
not roll them back. Clone-based preview leaves the original mesh unchanged.

The command adapters cover face edits, bridge, deletion/dissolve, normals,
UVs, tagging, inspection and Boolean orchestration. They retain runtime reports
while direct mesh-operation results retain geometric evidence.

UV projection uses each corner's destination vertex, matching render extraction.
Migration: regenerate mappings authored by the earlier projection operators,
which shifted UVs by one corner around each face. Operator parameters are unchanged.

`uv.box` shares its plane selection and projection with `exedra_mesh`
(`dominant_box_plane`, `project_corner_box`), which render extraction also uses
under `UvSource::CustomOrBoxProjected`. For a mesh without authored UVs,
authoring with `uv.box` at a given scale and no offset, then extracting with the
default UV policy, yields the same render buffers as extracting the unmodified
mesh under that policy. Where authored UVs exist the paths differ: extraction
keeps them, while `uv.box` overwrites them unless `write_missing_only` is set.

Migration: dominant-axis selection now normalizes the face normal before
applying `normal_epsilon`, so it depends on orientation only. Very small or very
large faces that used to fall back to the `+Z` plane now project on their true
dominant plane. Regenerate box mappings on such meshes if the old output was
relied upon.

## Design

See the [boundary and migration decision](../exedra_mesh_ops/docs/adr-0001-mesh-operation-boundary.md)
and the rustdoc manual for command authoring, selections and reporting.

## License

Licensed under either of Apache License 2.0 or MIT license at your
option. See the workspace root for license files.
