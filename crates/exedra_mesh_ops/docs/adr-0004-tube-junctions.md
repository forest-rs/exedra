# Tube junctions

Mesh operations own the skin that joins open tube ends at a branch node
(`junction`). Constructive recipes and application toolkits (for example a
tree generator's hero forks) supply the rings and bind provenance; this crate
only needs ring positions, their boundary order, and the node.

## Decision

1. **Topology comes from arm directions, not ring points.** Each ring is an
   arm from the node through its centroid. The arm graph is the convex hull of
   the arm directions on the unit sphere, computed with exact `orient3d` over
   at most `MAX_JUNCTION_RINGS` directions (quartic in the ring count). Hulls of the ring points
   themselves are avoided on purpose: ring vertices are cocircular by
   construction, the most degenerate input a hull can get, and f32 mesh
   positions are never exactly coplanar, so the hull's topology would depend
   on rounding.
   - Coplanar directions (always the case for three arms) split the sphere
     along their circle into two faces.
   - When the directions are not coplanar and the node is not strictly inside
     their hull, every arm lies in one open hemisphere; a virtual arm opposite
     their mean closes the sphere, and the faces around it merge into one.
   - Adjacent facets within `MERGE_ANGLE` (3°) merge into one polygonal
     crotch, so evenly spread arms do not produce sliver crotches whose
     diagonal is arbitrary. Merging is bounded: every merged facet stays
     within `MERGE_ANGLE` of the group's mean normal, so a chain of slightly
     bent facets cannot grow into one wide crotch.
2. **Rings divide into runs by crotch direction.** Around each arm, the
   incident crotches' outward directions cut the ring into one contiguous run
   of vertices per neighbour. Crotch directions stay well defined for
   antipodal neighbours (a T junction), where neighbour directions would not.
   Each vertex joins exactly one run.
3. **Bridges and crotches.** Each graph edge becomes a strip between the two
   facing runs, with `min(p, q)` quads and `|p − q|` triangles spread evenly.
   Each crotch polygon alternates ring edges and bridge side edges and becomes
   a fan of quads around one new vertex: the polygon centroid, domed a quarter
   of the way toward its mean radius along the crotch direction. Two rings
   form one closed strip, aligned by the rotation that carries the first
   ring's inward axis onto the second's outward axis (a bent tube's
   rotation-minimising frame); rung lengths alone cannot rule out a twist
   when one ring is much smaller than the other.
4. **The skin claims the rings' boundary half-edges.** Faces traverse ring
   edges in boundary order, so they weld to the tubes with no seam in the mesh
   topology. Faces are added in a greedy order that joins each face to the
   existing fan at every one of its vertices along an edge, as the kernel's
   manifold checks require. The plan fixes that order
   (`JunctionPlan::attach_order`); a skin without one is a planning refusal.
5. **Refusals are typed and happen before any edit.** The construction
   assumes the rings bound a convex region around the node and that each
   bridge follows the arc between its arms. Planning refuses:
   - collars closer than their angular radii widened by
     `COLLAR_CLEARANCE` (a quarter of the sum). Angular radii are measured
     per vertex from the node, so tilted rings report the cone they occupy;
   - a bridge whose shorter great-circle arc passes within another arm's
     angular radius (`BridgeCrossesArm`), as when three arms fan across one
     side of the node. Nearly antipodal arms have no shorter arc and are
     covered by the run check below;
   - a ring with a bridged neighbour's run vertex beyond its plane
     (`RingBeyondNeighbour`): the convexity precondition, violated by an
     acute branch whose ring lies farther out than its neighbour's;
   - a neighbour whose direction, where it has a well-defined azimuth
     around the ring's axis, lies deeper in another run than the middle of
     the run next to its own (`RunMisaligned`), as for rings tilted across
     their fork. A neighbour may sit past its own run's span: the excess
     grows with ring tilt and crowding, and clean skins show tens of degrees;
   - a quad folded along both diagonals, whose rungs cross (`SkinTwisted`),
     as between a tiny and a large ring; no face-to-face test sees a fold
     inside one quad;
   - skin faces that intersect each other (`SkinSelfIntersects`), tested
     exactly with quads split along both diagonals, as a backstop for folds
     the geometric checks do not anticipate, such as very coarse rings.
     Crotch centers are rounded to the mesh's `f32` before the test, so it
     covers the positions the mesh will hold. A triangle with exactly
     collinear vertices is treated as its edges;
   - rings that open toward the node, are not star-shaped around their axis,
     or are too coarse to give each neighbour a vertex, degenerate arm
     graphs, invalid charts (including a parent whose first vertex lies on
     its axis), and skins without a manifold insertion order.

   The exact backstop in `plan_junction` covers the skin alone: a plan has
   no tube walls. `add_junction` additionally tests every skin face exactly
   against the tube faces incident to the ring vertices (`SkinCrossesTube`),
   before any edit. The geometric checks are load-bearing for a plan without
   a mesh and deliberately conservative: measured on tree-like forks, many
   of the `RingBeyondNeighbour` and `OverlappingCollars` refusals would have
   planned clean skins. With the wall test in place they could be relaxed
   for `add_junction` later, once it is shown to catch what they catch.

   A seeded fuzz in the tests checks that every accepted junction, capped,
   is a closed, manifold, genus-zero shell with no self-intersection and no
   twisted quad. A
   kernel refusal while adding faces is not expected for an accepted plan;
   should one happen, the crotch center vertices and faces added before it
   remain, as the crate's eager operations do not roll back.
6. **One documented chart.** An optional `JunctionChart` continues a parent
   tube's cylindrical chart across the skin: `U` by angle around the parent
   axis from its first vertex, `V` by distance along it. Faces crossing the
   angle seam keep `U` continuous; child rings meet the skin at explicit
   seams. The parent's first vertex is its ring's vertex 0 for
   `plan_junction`, and the from-vertex of the seed's OUTSIDE half-edge for
   `add_junction`. Every skin face reports whether it belongs to a bridge or a crotch.

## Consequences

- `plan_junction` works on positions alone, so a constructive node can plan
  from sweep rings before any mesh exists.
- The skin is piecewise flat and quad-dominant. It does not smooth the crotch
  into a saddle; callers wanting a softer blend subdivide or relax the result.
- The cylindrical chart distorts near the parent axis, for example across a
  saddle between two children that passes over it.
