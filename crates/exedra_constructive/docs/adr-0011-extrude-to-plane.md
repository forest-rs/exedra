# Forward extrusion to an authored plane

`exedra_constructive::extrude` owns evaluated profile extrusion terminated by a
plane. The direction is the placement's local +Z axis, including its scale and
shear. The target plane is in body coordinates. This additive API does not alter
recipe IR or existing extrusion calls; no caller migration is required.

A profile must lie strictly before the target along that direction. A bounded
profile discretization supplies the extrema of the linear plane-distance
function. For curved profiles the check reserves the requested chord tolerance
multiplied by the length of the plane normal pulled back to profile coordinates.
This conservatively covers unsampled curve interiors; it can refuse a near-plane
curve that finer sampling would accept. Parallel directions, backward targets,
and uncertain/crossing starts are explicit errors.

The internal construction is an ordinary closed extrusion to twice the computed
maximum termination height, followed by the checked plane splitter. Both stored
cap boundaries must remain on their intended sides after f32 realization. The
existing section policy controls cap-plane accuracy, intersection work, and
boundary validity. This reuses one checked implementation for holes, cap
triangulation, seam realization, and closed topology rather than introducing a
second clipping path. It constructs and discards the far half; a future direct
emitter can remove that cost while preserving this public contract.

Walls and the start cap keep ordinary extrusion regions and provenance. The end
cap uses `REGION_CAP_END` with `CapEnd` provenance (since schema 33), section-frame UVs, and
sharp rims. Assembly consumers bind materials by these regions. The result also
returns the terminating section and its measurements. Mesh geometry follows the
sampled profile; no analytic-surface or global self-intersection certificate is
claimed. Recipe-level termination is provided by `NodeKind::ExtrudeToPlane`;
arbitrary target bodies remain a separate extension. Semantic cap selection and
the schema-33 migration are specified in [ADR 0014](adr-0014-semantic-attachments.md).
