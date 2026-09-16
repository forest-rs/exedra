# Smooth lofts with authored correspondence

## Boundary and decision

`exedra_constructive` owns loft correspondence, interpolation, bounded sampling,
and mesh realization. It does not infer semantic matches between unrelated
profiles or provide a general solid-validity certificate.

Both loft policies match loop indices and ordered source segments. Segment
boundaries are authored landmarks; `Loop2::with_seam` selects the first segment
without changing geometry, winding, or tags. Different segment structures fail.
No nearest-point alignment, hole permutation, or automatic seam rotation occurs.

`LoftPolicy::Smooth` uses uniform cubic Hermite interpolation in section-index
space. Interior tangents are half the difference between neighboring placed
points; endpoint tangents are one-sided secants. Corresponding sampled profile
points therefore follow C1 curves through every authored section. Two sections
reduce to a straight ruled band. Unequal physical section spacing does not
change this explicitly uniform parameterization.

Sampling bounds the chord deviation of these point trajectories in the placed
f64 construction domain, with separate finite band and vertex budgets. It does
not bound the original analytic profile between its samples or f32 quantization.
Source-band and parameter-interval evidence accompanies successful results.

Positive projection of every cubic derivative control onto its band's secant
provides a conservative local no-backtracking check. Both wall diagonals and
caps must retain positive orientation after f32 realization. Distant-surface
intersection is not certified; topology closure alone never establishes it.
Sampling and realization failures remain typed evaluation errors. A smooth
interpolant may overshoot its sections, so their bounding box cannot serve as
a conservative fallback envelope.

## Migration

`tessellate_loft` takes an explicit interpolation policy before its cap mode.
Evaluation policy and body evidence gain loft fields; direct struct literals
must supply those fields. Schema 31 invalidates old cached results and requires
regenerating schema-stamped IR. JSON loft nodes require an explicit policy.
