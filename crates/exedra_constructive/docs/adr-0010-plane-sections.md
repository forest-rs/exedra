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

`PlaneSection::to_profiles` converts each filled region into an owned `Profile2`
and its unchanged placement. `profile_section::profiles_from_mesh_section` provides
the same conversion for plain mesh sections, retaining face IDs instead of
construction features. This adapter belongs in constructive, which owns the
profile vocabulary; mesh operations remain independent of constructive.

The conversion preserves every boundary sample, cyclic starting point, winding,
hole and region order. It emits line segments without fitting curves or choosing
correspondence between independent loft sections. Segment tags index an owned
source table across the outer loop and holes of each profile. These sources need
their original mesh/body or snapshot context; they are not persistent selectors
or a retained dependency on the section's source geometry. Reusing the profile
after the source changes does not recompute the section.

Extraction establishes simple separated boundaries and correct nesting. Callers
editing public section data must preserve those invariants. Conversion checks
frame validity, source counts, polygon degeneracy, tag limits and ordinary profile
construction requirements; it does not repeat all budgeted section-topology checks.
Invalid input returns a typed error without a partial profile list.

Reusing section samples exposed two gaps in extrusion, loft and sweep caps:
ear clipping could discard collinear rim samples or produce a thin triangle that
collapses at mesh precision. Those caps now retain a successful ear-clipped cover
when it uses every boundary sample and its triangles survive placement/narrowing.
Otherwise they reuse the boundary-preserving Delaunay cover already used by plane
cuts, without inserting new vertices. Still-unrepresentable cap triangles are
typed failures. This does not certify the whole body's geometry.

The conversion API is additive and its profiles use normal recipe serialization
and fingerprints. The cap correction changes evaluation semantics: schema 36
invalidates prior cache entries. Migration: reevaluate cached recipes and regenerate
schema-stamped text. Retained plane cuts already use the shared geometry
implementation through `NodeKind::PlaneCut`.

The `plane_cut` example exports an obliquely cut asymmetric loft, separated capped
halves, and a section outline. Volume is measured against the same robust face
triangulation that defines the cut surface.
