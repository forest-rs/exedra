# exedra_assembly

Structure head for the Exedra geometry stack: part definitions (constructive
recipes or baked meshes), instance trees with stable string-key identity,
material slot binding, content-addressed part compilation, and a flat
`RenderList` seam for renderers and exporters. Compiled parts expose
once-per-part, part-local geometry accounting; render lists expose placed,
world-space accounting with instance multiplicity.

See `docs/adr-0001-structure-head-scope.md` for the scope contract and
`exedra_constructive` for the geometry side of the boundary.
