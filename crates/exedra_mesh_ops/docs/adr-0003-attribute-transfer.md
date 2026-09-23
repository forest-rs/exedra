# ADR-0003: Attribute transfer through mesh operations

## Status

Accepted

## Context

`exedra_mesh` kernels carry caller-defined layers under each layer's
declared `Propagation` rule (`exedra_mesh` ADR-0012). Mesh operations built
on top did not: in-place operations left their new elements empty, and
operations that rebuild a fresh `Mesh` (Booleans, stretch, sections,
reflecting transforms) dropped caller-defined layers and their rules
entirely. Booleans also dropped built-in corner UVs, so a textured operand
lost its chart wherever it was cut (constructive ADR-0023, "Chart
ancestry").

Every one of these operations already knows where its output came from:
in-place edits capture their source faces and corners before editing,
rounding assigns each rewritten face an owning source face, and every
rebuild reports an output face's source face and places its corners on that
face.

## Decision

1. **Values follow the correspondence the operation already reports**, under
   each layer's own rule, through the public `exedra_mesh` capture API
   (`Mesh::capture_attributes`, `op::restore_attributes`,
   `Mesh::adopt_attribute_layers`).
2. **In-place operations** capture values before they delete anything:
   - extrude, inset and solidify: walls, frames, caps and inner faces take
     their source face's values; each corner takes the source corner at the
     same vertex; copied vertices take their source vertex's values.
   - `cut_rect_face`: every generated face takes the source face's values;
     frame corners at the outer vertices take their source corners. The
     inner corners and vertices have no source and start empty.
   - `poke_faces`: fan triangles take the source face's values, outer
     corners the source corners, and the center corner and vertex all
     source corners and vertices with equal weights.
   - rounding: faces take their owning source face's values; a corner at a
     surviving vertex takes the source corner there; a corner or vertex at a
     new point is weighted barycentrically in the owner.
   - `bridge_boundary_loops` has no source face for its strip, like regions
     and UVs; nothing is deleted, so nothing is lost.
   - `remove_small_components` deletes only and reports the cleared values in
     `ComponentCleanup::cleared_attribute_values`.
3. **Rebuilds** adopt every source layer (storage, type, default and rule)
   onto the result, then write values by position: each output corner lies
   on its source face and is sampled there. A corner at a source corner's
   exact position selects that corner alone, and a corner on a face-loop
   edge (within `2^-20` of its length, a few `f32` ulps of narrowing)
   interpolates the edge's ends. Both checks walk the face loop, so corners
   the robust triangulation drops as collinear (T-vertices) still sample
   exactly. Other points take barycentric weights in the robust
   triangulation, from the containing triangle or the triangle the point
   overshoots least, clamped and renormalized: arbitrary data is never
   extrapolated.
   - stretch samples at each output vertex's unmoved source position; a
     generated vertex takes its first face's sample.
   - sections sample surviving faces by position and intersection vertices
     along their source edge. Caps have no source face and start empty.
   - Booleans sample each output face in its operand's original face. Cut
     surfaces come from the cutting operand and carry its values. Output
     vertices that are original operand vertices are identified by the
     stitch's identity maps (`BooleanOutput::vertex_provenance`), never by
     position. Each operand writes only its own layers: at its original
     vertices by identity, at every other output vertex its faces reach by
     the point on its first such face. Identity samples are written after
     point samples, so an operand vertex on the other operand's face keeps
     its own values; within each pass operand A writes last, so for a layer
     name both operands carry A's value (or its absence) wins. A name only
     one operand carries always keeps that operand's value. The
     same weights interpolate built-in corner UVs, so Booleans now keep UV
     charts; a source face with an incomplete or non-finite chart gives its
     output faces no UVs and is counted in `BooleanStats::uv_unmapped_faces`.
     Operands that register one layer name with different storage, type,
     default or rule fail with `BooleanError::AttributeLayerConflict`.
   - reflecting transforms only re-wind faces. Every output element is one
     source element, so values are written verbatim
     (`Mesh::capture_attributes_verbatim`), whatever the rule, exactly as the
     non-reflecting clone keeps them. Handedness-sensitive values are not
     adjusted; callers storing such data correct it after a reflection.
4. **Loss is counted.** Values an `Unspecified` rule cannot carry are counted
   in the operation's report: the caller's `ChangeSet` for in-place
   operations, and `unpropagated_attribute_values` on `StretchStats`,
   `CutMesh`, `BooleanStats` and `RoundStats` for rebuilds. As in the kernel
   contract (ADR-0012 in `exedra_mesh`), the count is per written element,
   not per distinct source value: one source face's value lost onto several
   output faces counts once for each. A source face whose triangulation is
   entirely degenerate has no interior to sample: a rebuilt corner or vertex
   sampled there (on no loop edge) starts empty and counts once when the
   source carries layers of its domain.

## Consequences

- Caller-defined layers now survive every operation in this crate by the
  same declared contract as the kernels.
- Booleans keep corner UVs. Constructive CSG passes them through unchanged.
- Transfer runs only when a source carries caller-defined layers (or, for
  Boolean UVs, a UV layer); otherwise the operations do no extra work.
- Built-in normal overrides are still not carried through Booleans.
- Stretch previously wrote each rebuilt corner's UV and normal override onto
  the neighbouring corner of its face. That is fixed separately; the transfer
  uses the corrected corner mapping.
