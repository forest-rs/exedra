# Planar workplanes on evaluated bodies

`exedra_constructive::workplane` owns planar patch selection, authored frame
validation, and body/local coordinate conversion. The mesh kernel continues to
own face topology and triangulation. There are no new dependencies, recipe nodes,
or changes to existing signatures; this additive API requires no migration.

Selection names one face, a face region, or an existing operand-qualified region.
All selected faces must be edge-connected. Unqualified regions spanning Boolean
operands are ambiguous even when coplanar. Holes and open planar patches are
supported. Face IDs and revisions are scoped to one logical mesh, as with source
maps; callers must not use revision equality to identify unrelated meshes.

The caller supplies both origin and preferred X direction in body coordinates.
The face area-vector sum supplies Z following the mesh's winding. This removes
triangulation-dependent axis/origin choices without inventing correspondence or
assuming all meshes are outward-oriented solids. X is projected into the plane,
Y is Z cross X, and the frame is right-handed and orthonormal. Zero or nearly
normal X is refused. The origin must lie within the caller's distance tolerance
and is projected onto the selected plane; it may lie outside the patch boundary.

Corner positions are promoted from f32 to f64 before geometric arithmetic.
Robust face triangulation must succeed, each face must have positive area along
the aggregate normal, and every selected corner must lie within the distance
tolerance. The reported deviation is checked against the final projected origin.
Face and corner limits bound selected-patch work; resolving a region scans the
body's face list. This is a planarity check, not a global solid or overlap proof.

Workplanes retain their source revision and expose an explicit stale check.
Resolved frames do not follow topology edits or recipe reevaluation automatically.
[ADR 0014](adr-0014-semantic-attachments.md) adds explicit retained attachment
intent, resolved afresh on each evaluation. Strict-origin calls retain their
existing semantics. Curved tangent frames and arbitrary face tracking remain
separate operations.
