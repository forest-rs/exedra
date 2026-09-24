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
6. **Charts continue the tubes' own UVs.** `JunctionOptions::ring_uvs`
   gives, per ring edge, the UVs the tube face across it holds at both ends
   (`add_junction` reads them from the mesh with `continue_uvs`). Skin
   corners on ring edges receive exactly those values, so the texture is
   continuous across every skin/tube boundary whatever chart the tubes use.
   New vertices take the discrete harmonic extension of the ring values.
   - A tube's chart usually jumps across one ring edge (its U seam). The
     jump continues inside the skin along a cut: a breadth-first path of
     faces to one sink face, chosen farthest from ring 0 (typically the
     parent, which puts the sink between the branches), then farthest from
     every ring. Cut edges carry the jump in the harmonic equations, so the
     extension is smooth across them; faces on the cut are lifted onto one
     side. The texture jumps across one side of each cut by the tube's own
     seam jump, invisible when that jump is a whole number of repeats.
   - The sink face absorbs whatever the tubes' jumps fail to cancel. A trunk
     and branches that each wrap once cannot be charted without such a
     point: going around the trunk's collar is homologous to going around
     all the branches' collars, so the periods must balance or the chart
     must be singular somewhere. The texture swirls around the sink.
   - Without smoothing, a bridge face can span both rings, and a cut through
     it leaves its far ring edge off by the jump. Smoothing leaves no face on
     two rings.
   - V is whatever the tubes give; charts whose V is arc length through the
     junction (branches starting where the parent ends plus the gap) blend
     without compression.
   This replaces a single cylindrical chart around a parent axis, which left
   explicit seams at every child ring and distorted near the parent axis.
7. **Smoothing is opt-in refinement plus thin-plate fairing.**
   `JunctionSmoothing { rows, tangents }` splits every non-ring edge into
   `rows` segments. Ring edges are never split: the tube faces own them.
   Bridge quads become stacks of quads, bridge triangles stacks ending in a
   triangle, and crotch quads rows that shorten toward the crotch center.
   Every new vertex, crotch centers included, is placed by minimizing the sum
   of squared umbrella Laplacians over new and ring vertices, solved with
   conjugate gradients. A ring vertex's umbrella includes a ghost neighbour
   inside its tube, against the wall tangent at the vertex's mean skin edge
   length, so the minimum leaves each ring along the tube wall: tangent
   continuity in the discrete sense, not an exact G1 patch.
   - Tangents default to each ring's inward axis for plans; `add_junction`
     measures the tube walls (mean direction from off-ring neighbours).
   - Weighting the ring terms more heavily was tried and rejected: it trades
     the ring crease for folds at tight crotches (more `SmoothedSkinTwisted`
     refusals and larger creases).
   - A Catmull-Clark style subdivision was rejected because it splits ring
     edges (a T-junction with the tubes) and only approximates the ring
     positions.
   - The coarse skin must pass every check first. The smoothed skin is then
     checked again with the same exact backstops and refused as
     `SmoothedSkinTwisted` or `SmoothedSkinSelfIntersects`, so a caller can
     fall back to the coarse skin; `add_junction` tests the smoothed skin
     against the tube walls. Both refusals are real backstops, reached by
     tests: a fuzz junction folds a quad at two rows with measured tube
     tangents, and caller tangents aimed across the rings pull rows from two
     branches through each other (`SmoothedSkinSelfIntersects`) or fold a
     crotch quad (`SmoothedSkinTwisted`). The seeded fuzz covers smoothed
     junctions on straight and leaning tubes at one to six rows; a junction
     the coarse skin accepts is either smoothed cleanly or refused with a
     smoothed-skin variant.
   - Measured refusal rate: of 400 straight fuzz configurations, 100 plan
     coarse; smoothing refuses one of them at two rows (`SmoothedSkinTwisted`)
     and none at one or three to six rows. Of 400 leaning configurations, 86
     plan coarse and smoothing refuses none at one to six rows. No smoothed
     skin crossed a tube wall.
   - A coarse face of an unexpected shape is refused as `UnrefinableFace`
     rather than refined with T-vertices; the coarse construction produces
     only bridge quads, bridge triangles and crotch quads.
   - Measured on a Y fork with twelve-sided branches, the median crease
     between a skin face and the tube face across a ring edge falls from
     16 to 8.5 degrees and the largest from 54 to 33, the largest at the
     tight crotch between the branches.
   - Plan statistics describe the coarse skin; new vertices are all
     `JunctionVertex::Skin`, crotch centers first.
   - The fairing energy is a least-squares system (ring umbrellas add rows),
     solved with conjugate gradients on its normal equations. The harmonic
     UV and layer systems are symmetric positive definite as given and use
     plain conjugate gradients, which keeps their conditioning.
8. **Caller layers follow the harmonic ring weights.** `add_junction`
   gives each new skin vertex the ring vertices' caller-defined values, and
   each of its corners the tube corners at those ring vertices, weighted by
   the discrete harmonic function over the skin that is one at the ring
   vertex (the same extension that carries UVs; uniform umbrella weights keep
   every weight non-negative). Each layer's own `Propagation` rule decides
   what a weighted capture means, as ADR-0003 sets for every operation. A
   skin corner at a ring vertex takes the tube corner across the ring edge
   the face shares with the tube, or across the ring edge leaving the vertex.
   Skin faces have no source face, so caller face layers start empty, as
   section caps do. `add_junction` takes no `PropagatePolicy`: edge tags are
   stored per edge, so ring edges keep the tubes' tags and new skin edges
   start clear; UVs and regions have their own parameters.

## Consequences

- `plan_junction` works on positions alone, so a constructive node can plan
  from sweep rings before any mesh exists.
- Without smoothing the skin is piecewise flat and quad-dominant; with it,
  the skin is a faired saddle with `rows` rows between rings, several times
  more faces, and rare extra refusals (one in 1116 smoothing runs over the
  fuzz configurations the coarse skin accepts, rows one to six).
- UVs are continuous across every ring for any tube chart; the remaining
  seams continue the tubes' own seams, and one sink face per junction
  swirls when the tubes' U periods do not balance.
