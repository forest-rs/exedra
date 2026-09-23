# ADR-0005: Skeletal Fields and N-ary Smooth Union

- Status: Accepted
- Date: 2026-09-23
- Owners: Exedra implicit-surface maintainers

## Context

Branching shapes, such as tree forks or roots meeting a flare, are
naturally described as segments with radii blended where they meet. The
crate offered a finite cylinder with a sampled interval and a binary
polynomial `SmoothUnion`. Folding the binary union over many children is
order-dependent and makes junctions swell by `k/4` per fold, and every child
is evaluated at every point even when most are far away.

## Decision

`exedra_isosurface::skeletal` adds three fields.

1. **`CapsuleField` and `RoundConeField` are exact signed distances.** A
   round cone is the convex hull of two spheres. It uses Quilez's region
   classification, and degenerates to the larger sphere when one sphere
   contains the other. Gradients are the unit direction from the nearest
   feature. They are NaN only on the axis inside the shape, which the
   `ScalarField` contract permits.
2. **Evaluation is `f64`, rounded once to `f32`.** Values do not depend on
   batching. Intervals are computed in `f64` and widened outward by
   `64 * f64::EPSILON` times the largest magnitude involved, then rounded
   outward to `f32`. That keeps them sound for the `f32` values the same
   fields return. A round cone's side region computes
   `cos = sqrt(l^2 - (ra - rb)^2) / l`, which loses relative accuracy by
   `l^2 / (l^2 - (ra - rb)^2)` as the cone approaches containment, so the
   margin is multiplied by that factor.
3. **Intervals come from convexity and the Lipschitz bound.** Both fields
   are convex, 1-Lipschitz distances:
   - the maximum over a box is at a corner, which is exact;
   - the minimum is at least the center value less the half diagonal, or
     the distance to the segment's box less the larger radius, whichever is
     greater.
4. **`SmoothUnionN` is defined implicitly.** With child values `v_i`, the
   value `F` solves

   ```text
   sum_i w((v_i - F) / k) = 1,   w(x) = (1 - x/2)^2 on [0, 2), else 0.
   ```

   `w` is continuously differentiable with compact support, so by the
   implicit function theorem `F` is continuously differentiable wherever
   the children are, with gradient `sum c_i grad v_i / sum c_i`,
   `c_i = -w'((v_i - F) / k)`. That holds across ties of any number of
   children and where a child enters or leaves the band. An earlier closed
   form, `F = m - k ln sum w((v_i - m) / k)`, was continuous only at pure
   ties: with a third child in the band it creased along every two-way tie,
   and dual contouring turned those creases into sharp features.
   - With `m` the smallest child and `n` the children within `2k` of it,
     `m - 2k (1 - 1/sqrt(n)) <= F <= m`. One child in the band is returned
     unchanged.
   - Children beyond `2k` contribute exactly zero.
   - The equation is piecewise quadratic in `F`, so it is solved in closed
     form: scan the band members by descending height to the bracket that
     holds the root, then take the quadratic's smaller root in its
     cancellation-free form. No iteration, no tolerance loop.
   - Band members are sorted by value, then gradient, before any sum, so
     value and gradient bits do not depend on child order. The
     hard-minimum case breaks ties the same way.
   - NaN child values are ignored, and a `-inf` minimum is a hard minimum.
   - For two children it differs from the binary `SmoothUnion`, which
     stays as it is.
   - Intervals use monotonicity: `F` is nondecreasing in every child, so
     over a cell it lies between the union of the children's lower bounds
     and the union of their upper bounds, widened by `1e-9` of the value's
     scale for the closed-form solve.

5. **Culling is exact and opt-in.** `SmoothUnionN::with_distance_culling`
   requires children to implement `BoundedField`, stating
   `value(p) >= distance(p, core) - radius`. A child is skipped where its
   bound exceeds a reachable minimum by more than the band, plus
   `2^-10` of the band, `f32` rounding slack at that value, and the smallest
   normal `f32`, so a skipped child is strictly above the minimum even for a
   hard minimum at zero.
   - A skipped child can neither be the minimum, tie with it, nor carry
     weight, so culled and unculled unions return identical value and
     gradient bits. A test checks this.
   - Interval queries skip children the same way. For `CapsuleField` and
     `RoundConeField` the interval is identical too, since their interval
     lower bounds already include the distance bound. A child whose
     interval is looser than its `DistanceBound` can get a tighter, still
     sound, interval with culling than without.
   - `SmoothUnionStats` counts child evaluations and culls.

## Consequences

- Tree forks and similar skeletal shapes extract as closed meshes through
  the existing dual contouring. A three-branch fork is tested.
- **Measured culling benefit.** The `isosurface_wind_tunnel --fork`
  scenario extracts a 1.2 m box around a fork among 36 scattered twigs,
  39 children in all:
  - culling cuts point-level child evaluations 21–23× and interval
    child queries 17–19× at depths 5–7;
  - wall time improves 1.20–1.30× (274 ms against 346 ms at depth 7), and
    the remainder is extraction work, not field evaluation;
  - the extracted mesh is bit-identical with and without culling.
- `SmoothUnionN` counts work through a `Cell`, so it is not `Sync`.
- A culled child's value is never computed. That matches the unculled
  union, which ignores NaN child values.
