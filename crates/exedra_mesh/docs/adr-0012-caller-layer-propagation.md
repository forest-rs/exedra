# ADR-0012: Caller-Defined Layer Propagation

## Status

Accepted

## Context

Render extraction carries caller-defined attribute layers (ADR-0006,
"carried attribute streams"): second UV sets, vertex colors, wind pivots,
provenance tags. Topology kernels, however, propagated only the built-in
layers, each with hand-written code: `split_edge`, `split_face`,
`flip_edge`, `collapse_edge` and the dissolves left caller-defined values
empty on the elements they created. Worse, deletion cleared only built-in
corner layers, and arena slots are recycled, so a face or corner created
after a deletion could inherit a deleted element's caller-defined value.

## Decision

1. **Rules are declared per layer.** `Propagation` is `Unspecified` (the
   default), `Clear`, `Copy` or `Interpolate`. `Mesh::set_layer_propagation`
   declares it once per layer, next to the layer's registration, rather than
   in `PropagatePolicy`: what a wind pivot or a color means under a split is
   a property of the data, not of one edit. Built-in layers keep their
   `PropagatePolicy` knobs and refuse declarations (`AttrError::Reserved`);
   `u32` and `bool` layers refuse `Interpolate` (`AttrError::NotInterpolable`).
   Declaring a rule writes no value and does not advance the revision.
2. **One code path applies them.** Kernels name, for each element they
   create or rebuild, its source element and, where it lies between two,
   a weighted blend. The attribute store applies each caller-defined layer's
   rule untyped:
   - `split_edge`: the inserted vertex blends its endpoints (copies the
     `from` vertex under `Copy`); child corners keep their parent's corner;
     the parent corners, now at the inserted vertex, blend the corners at
     both ends of their side of the edge. When only one end has a value
     (a sparse layer), `Interpolate` carries that value whichever end it is,
     and `Unspecified` counts it as lost.
   - `split_face`: the new face continues the source face; each diagonal
     corner continues the source corner at its vertex.
   - `flip_edge`: each re-aimed diagonal corner continues the source corner
     at its new vertex (the rule UVs already follow).
   - `collapse_edge`: where a shrinking face's corner at the survivor dies,
     the corner now pointing there continues it (the UV transfer rule). The
     survivor keeps its own vertex values under every rule.
   - `dissolve_edges` and `dissolve_vertices`: values are captured before
     the faces are deleted and restored onto the rebuilt faces, corners
     matched by vertex. A merged face carries a face value only when both
     faces agree, mirroring `FACE_REGION`.
3. **Nothing is silently lost.** `Unspecified` clears what it cannot carry
   and counts it in `ChangeSet::unpropagated_attribute_values`, as does a
   disagreement between fused sources. `Clear` is intentional and uncounted.
4. **Deletion always clears.** Every deleted vertex, half-edge or face
   loses its caller-defined values, whatever the rule, so recycled slots
   start empty. `ChangeSet::cleared_attribute_values` counts the values
   that carried information (sparse values, and dense values other than the
   default). It is a gross count of values on deleted slots, including values
   a kernel carried onto a replacement first (the dissolves, `collapse_edge`);
   `unpropagated_attribute_values` is the loss counter.
5. **Half-edge layers are corner data.** A caller-defined half-edge value
   belongs to the corner at the half-edge's destination vertex within its
   face, as `CORNER_UV` does. Edge-keyed layers stored on one canonical
   half-edge per undirected edge (the `EDGE_SEAM` convention) are outside
   this contract: kernels may move or drop them uncounted.

## Consequences

- Caller-defined layers survive the mesh kernels by an explicit, per-layer
  contract. What a kernel fails to carry is counted as unpropagated; what
  deletion clears is counted separately and in gross.
- `split_face` now sizes dense layers before writing attributes of the new
  face. Previously, a new face extending the face arena silently lost its
  `FACE_REGION` copy.
- Built-in UV and normal-override propagation keep their existing code and
  outputs. Their split-face diagonal rule (a midpoint of the two corners)
  differs from the caller-defined rule (the corner at the diagonal's vertex);
  unifying them would change pinned outputs and is left for a deliberate
  follow-up.
- `exedra_mesh_ops` does not carry caller-defined layers yet, in two classes
  (gap 13b):
  - Operations that edit in place through the kernels (face edits, poke,
    patch and connect operations) delete through `op::delete_faces`, so
    deleted values are cleared and counted; their new elements start empty
    and uncounted, which render extraction reports as attribute fallbacks.
  - Operations that return a fresh `Mesh` built with `MeshBuilder` (Booleans,
    stretch, sections, and reflecting transforms) copy only built-in layers.
    Caller-defined layers and their rules vanish from the result with no
    counter; extraction reports them only as missing layers, and only if
    asked for.
