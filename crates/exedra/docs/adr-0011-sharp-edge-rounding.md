# ADR 0011: Sharp-Edge Rounding (Fillet/Chamfer) Pass

## Status

Accepted.

## Context

Producers tag feature edges with `EDGE_SHARPNESS`: constructive
tessellation marks crease edges, the boolean pipeline marks seam rings.
Downstream consumers want those features rounded — the single most
requested finishing operation for panel-class geometry — without a
surface kernel. The kernel already has the required surgery primitives
(`delete_faces`/`add_face`/`delete_vertices` with attribute discipline),
so rounding can be a deterministic mesh pass above them.

## Decision

`exedra::round::round_sharp_edges` plans read-only and applies in one
edit session:

- **Selection and chains.** Canonical edges with sharpness at or above a
  policy threshold form a graph; maximal chains through valence-2
  vertices become fillet strips, traced and processed in ascending stable
  id order. Valence 1 is an open end, 3 a corner candidate, 4+ a typed
  refusal.
- **Rolling-ball construction.** Per chain vertex, averaged tangent and
  flank normals give the tangency offsets `t = r·tan(sweep/2)` (fillet)
  or the flat setback (chamfer); fillet cross-sections are arcs around
  the per-vertex ball center. Band counts derive from an integer ceiling
  over the chord tolerance; chamfers always use one band. Backend `f32`
  math follows the primitives trig policy (`std` by default, `libm`
  under `no_std`; integer-derived topology; no cross-backend bit
  contract). All construction is f64 with one narrowing per new vertex.
- **Face rewriting by substitution.** Every face touching a chain vertex
  is rewritten through a per-`(face, vertex)` substitution table: flanks
  shrink to tangency lines, open-end faces splice the whole end section
  (oriented by the incoming edge's twin — a topological rule, no
  geometry), vertex-only faces classify to a side by normal agreement.
  Conflicting substitutions are typed refusals.
- **Corners.** Exactly three rounded chains meeting at a convex
  trihedral corner (its faces exactly the three flanks) get a ball
  center solved from the three plane offsets; each face's tangency point
  is shared by both adjoining strips, and the patch ring is recovered by
  walking the boundary the strips leave open — orientation is derived,
  never assumed. Chamfer corners are single triangles.
- **Seam-duplicate merging.** Boolean seams carry distinct vertices at
  identical narrowed positions; their zero-length chain edges merge into
  one cross-section with alias substitution, and faces that collapse to
  fewer than three distinct points (exactly-degenerate slivers) vanish.
- **Sliver plane borrowing.** A face whose Newell norm is below
  `1e-4 · perimeter²` has a noise normal; it borrows a verified coplanar
  neighbor's plane (breadth-first, never crossing a sharp edge, with a
  containment check). This is what makes seam-adjacent sliver flanks
  round correctly.
- **Plan-validate-apply.** Planning pre-validates everything application
  needs: simple loops, orientation survival, and a manifold pre-check
  (every planned directed edge unique; every undirected edge paired
  internally or matched by a surviving outside twin). All refusals leave
  the mesh byte-identical; an application failure after a clean plan is
  the typed `Internal` bug signal.
- **Attributes.** Rewritten faces keep their `FACE_REGION`; strips and
  patches take the policy region. Surviving sharpness/seam tags re-key
  onto the rewritten edges; the consumed chain edges' sharpness does not
  transfer — removing it is the point of the pass.

## Consequences

The v1 envelope covers open chains onto a transversal end face, convex
trihedral corners, closed seam rings (the drilled rim), gently bent
chains, and per-edge varying flanks. Deliberate v1 limits, all typed
refusals or documented quality caveats: concave edges, higher-valence
junctions, sharp chain turns (`max_tangent_turn`), slightly non-planar
rewritten facet walls under averaged frames, and undetected strip
self-overlap on chains curved tighter than the offset. Extending corners
beyond trihedral and adding concave (additive) fillets are follow-up
work above the same substitution machinery.
