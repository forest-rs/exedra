# Explicit profile origins, checked frames and geometric diagnostics

Profile authoring must expose coordinate choices and locate invalid geometry
without requiring a render or reconstruction of private evaluator state.

Rectangles and rounded rectangles name their origin in the builder. Corner
variants retain the existing geometry, winding and segment tags; centered
variants translate the same boundary. Circles and rings remain centered.

`exedra_math::Placement3::try_from_orthonormal_axes` checks authored frames with
an explicit dimensionless tolerance. It preserves supplied coordinates and
reports bad axis lengths, orthogonality and handedness. General affine
construction still supports scale, shear and reflection. A checked construction
does not make subsequent edits to public matrix fields checked.

The triangulator owns exact predicates and diagnoses original polygon edges
after its existing triangulation attempts fail. A contact witness identifies
the two original cyclic edges and crossing versus touching/overlap. It scans at
most N(N-1)/2 edge pairs, without allocations, only on the failure path; adjacent
edges of one loop are excluded. Absence of a found witness does not certify the
input: the original failure remains. Successful triangulation is unchanged.
Constructive owns mapping these sampled edges to authored segments and tags.
For curves, the witness describes the discretization used by the operation.
The body cache stores successful geometry, so this failure-only enrichment
does not require an evaluation schema or fingerprint change. The analytic
adapter preserves opening identity when a contact witness identifies a hole.

Signed-volume arithmetic belongs to shared mesh operations. A compiled-body
measurement describes part-local triangles; occurrence placement is a separate
input. Signed sums do not establish closure, absence of self-intersections or
the orientation of every connected component. Intentional reflections must not
be conflated with invalid body geometry.
Assembly depends directly on the existing `exedra_mesh_ops` workspace crate
for this arithmetic; the adapter adds no third-party dependencies.

## Migration

Replace `builders::rect` with `builders::rect_from_corner` and
`builders::rounded_rect` with `builders::rounded_rect_from_corner` for identical
geometry. Use the `*_centered` variants to align concentric profiles with circles.
No compatibility aliases are retained. Existing recipe geometry and serialization
are unchanged by the source rename.

Frame construction is additive. Handle the new detailed boundary-contact error
variants when inspecting geometry failures. Do not interpret their edge indices
as persistent selections or as analytic curve-intersection certificates.
