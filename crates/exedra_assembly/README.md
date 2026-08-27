# exedra_assembly

Structure head for the Exedra geometry stack: part definitions (constructive
recipes or baked meshes), instance trees with stable structured addresses,
material slot binding, content-addressed part compilation, and a flat
`RenderList` seam for renderers and exporters.

For selection and guarded editing, call `Assembly::into_addressable` with a
host-assigned `SpaceId`. The resulting `AddressableAssembly` uses
`addressable_tree` directly for exact, relative, and pinned resolution and for
explicit tree queries. It also exposes revision-scoped `InstanceId` handles,
explains effective materials, and applies guarded material transactions
atomically without cloning the complete assembly. Use its `commit` gateway for
post-binding structural, metadata, or part-content authoring; extraction returns
the revision needed to resume the same space safely. The top-level
`addressable_assembly` example is the complete Basilica tour.

See `docs/adr-0001-structure-head-scope.md` for the scope contract,
`docs/adr-0002-addressable-api.md` for the runtime addressing boundary, and
`docs/adr-0003-guarded-material-transactions.md` for the proposed editing
workflow.
`exedra_constructive` owns the geometry side of the structure boundary.
