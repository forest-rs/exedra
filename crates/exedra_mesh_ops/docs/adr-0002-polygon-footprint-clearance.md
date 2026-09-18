# Polygon footprint clearance

Mesh operations own containment and boundary distances on a checked planar
material domain. Constructive binds source features; assembly's existing
snapshot workplane exposes the same query. No recipe or cache encoding changes.

`PlanarPatch::polygon_clearance` accepts one simple filled polygon in patch-local
units, with either winding and no repeated endpoint. Concave footprints and
supports, support holes and exact contact are supported. Footprint holes are
outside this slice. The polygon is validated under explicit vertex, separation
and work limits; no normalization or repair changes the authored boundary.

Containment and nearest boundary distance are separate results. A footprint can
cover a support hole while all of its edges remain clear; a crossing can have
zero distance while violating containment. Violation witnesses distinguish
transverse crossings, exterior boundary intervals and covered hole boundaries.
`classify` always rejects containment violations, regardless of decision
tolerance. A contained touching footprint has zero clearance and can classify
within tolerance. Negative requirements and nonfinite inputs fail explicitly.

The query validates nonadjacent footprint edges, measures every boundary pair,
and partitions nontransversely intersecting edges at opposing boundary vertices.
It classifies open intervals and checks excluded holes, including coincident
boundaries' filled sides. Exact-sign predicates determine incidence and winding;
coordinate ordering treats signed zeros equally. Crossing coordinates use
outward-rounded interval arithmetic, refusing positional enclosures wider than
`64 * f64::EPSILON * coordinate_scale` per axis rather than returning an unstable
witness. Distances remain f64 measurements. Rounded interval
samples too close to a boundary for reliable classification return NumericLimit.
Witness ties follow support loop/edge then footprint-edge order.

The input patch retains its existing planarity and snapshot evidence. Distances
do not certify an analytic surface, thickness, collision freedom, or the minimum
motion required to repair a footprint. Work counts cover segment-pair and
point/segment visits; temporary interval storage is bounded by authored vertex
and checked patch-boundary counts. Exhaustion returns no partial result.

## Migration

Additive query, result, witness, policy and error types in `clearance`; existing
circular queries are unchanged. Constructive reexports the common policy/error
and binds the generic evidence types. Existing snapshot lifetime and stale-source
checks remain unchanged. No dependency or serialization migration is required.
