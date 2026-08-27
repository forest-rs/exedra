# exedra_assembly

Structure head for the Exedra geometry stack: part definitions (constructive
recipes or baked meshes), instance trees with stable structured addresses,
material slot binding, content-addressed part compilation, and a flat
`RenderList` seam for renderers and exporters.

For selection and revision-aware authoring, call `Assembly::into_addressable`
with a host-assigned `SpaceId`. The resulting `AddressableAssembly` uses
`addressable_tree` directly for exact, relative, and pinned resolution and for
explicit tree queries. It also exposes revision-scoped `InstanceId` handles,
explains effective materials, and provides a revisioned `commit` gateway for
post-binding structural, metadata, material, or part-content authoring.
Extraction returns the revision needed to resume the same space safely. The
top-level `addressable_assembly` example is the complete Basilica tour.

See `docs/adr-0001-structure-head-scope.md` for the scope contract and
`docs/adr-0002-addressable-api.md` for the runtime addressing boundary.
`exedra_constructive` owns the geometry side of the structure boundary.
