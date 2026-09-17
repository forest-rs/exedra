# Plane sections and capped cuts

Geometry implementation now lives in `exedra_mesh_ops`; constructive retains
feature/material binding and source-map checks. See the
[operation boundary](../../exedra_mesh_ops/docs/adr-0001-mesh-operation-boundary.md).

`exedra_constructive::section` owns section extraction and capped splitting of
an evaluated body. It operates on the mesh's robust face triangulation, not
the analytic recipe that produced the mesh. Stretch shares ordered edge/plane
intersection arithmetic with this operation. No production dependency is added.

One preparation pass classifies vertices, intersects straddling triangle edges,
and assembles directed section loops. The same edge identity and narrowed point
are used by both halves and their caps. Containment organizes disconnected
outer loops and holes; touching, intersecting, or inconsistently wound loops
are refused. Boundary-preserving cap triangulation keeps collinear samples so
side walls and caps share exactly the same boundary.

Original face-boundary samples omitted by robust triangulation are restored
only when exactly collinear in 3D. Projection-only collinearity on a nonplanar
face is refused rather than choosing a different surface. Section separation,
winding, and nesting are checked again after f32 realization, within the same
pair-work budget; rounding must not merge distinct boundaries.

The public operations accept a plane in body coordinates, an explicit distance
tolerance, and finite triangle, section-vertex, and intersection-check budgets.
Contacts within tolerance are errors, including planes through vertices, edges,
or coplanar faces. A disjoint plane succeeds with an empty section and one empty
half. Input topology must be closed and oriented, and output topology must remain
closed. Distant self-intersection is not certified.

Surviving faces retain source features, regions, material slots, seams, sharpness,
UVs, and corner normals. New caps and cut vertices have dedicated features. Cap
region and material are caller-authored; cap UVs use section-frame coordinates.
Source sampling/realization evidence is cleared on derived bodies.

This additive API requires no caller migration. It adds evaluated-body operations
without changing recipe formats or cache identity. Recipe-level cuts can use this
same implementation in a later slice.

The `plane_cut` example exports an obliquely cut asymmetric loft, separated capped
halves, and a section outline. Volume is measured against the same robust face
triangulation that defines the cut surface.
