# ADR 0001: Exact linear and angular measurements

- Status: accepted
- Date: 2026-08-30

## Context

Several Exedra subsystems need exact dimensions before they construct
floating-point geometry. A plain `f64` admits zero, negative, NaN, and
infinite values where a physical size must be positive. Setout also has a
signed type named `Length`, although signed coordinates and differences are
displacements rather than sizes.

## Decision

`exedra_measurements` owns exact physical measurement values; it explicitly
does not own parsing, formatting, user-input quantization policy, dimensional
analysis, or geometry.

- `Length` is a strictly positive count of joto iotas.
- `Offset` is a signed count of joto iotas and includes zero.
- `Angle` is a nonnegative count of microarcseconds.
- `AngularOffset` is the signed angular counterpart.
- Angular values do not implicitly normalize or wrap; an accumulated two-turn
  value remains distinct from one turn.
- Conversion to meters, degrees, or radians is explicit and belongs at the
  floating-point geometry boundary.
- Arithmetic that can leave a type's domain or overflow is checked.

The angular basis represents whole degrees, arcminutes, and arcseconds exactly.
`joto_constants` 0.2.0 does not provide an angular family, so the basis is
defined here rather than approximating authored angles as radians. The crate
is `no_std` and depends only on `joto_constants`.

## Consequences

Rule APIs can make invalid physical dimensions unrepresentable instead of
rechecking floating-point values during evaluation. Coordinates and signed
differences can state their distinct semantics with `Offset`.
Authored angles can likewise remain exact until a trigonometric or other
floating-point geometry operation needs radians.

`joiner_timber` adopted `Length` for its rule dimensions in the first slice.
The follow-up migration makes the same distinction below the rule library:

- `setout` uses this crate's `Length` for positive dimensions and `Offset` for
  datums, coordinates, and signed differences. Its `Point3` therefore contains
  three `Offset` values. Setout continues to own parsing, floating-root
  quantization, rational propagation, and exactness traces.
- `setout_joiner` is the sole exact-to-floating geometry adapter for resolved
  setout points and dimensions.
- `joiner` accepts exact authored overlap and rejection limits, while retaining
  `f64` for analytic extents, geometry-derived measurements, and numerical
  tolerances. Geometry-derived overlap enters through an explicitly named
  `with_minimum_overlap_meters` seam.

This migration changes setout's public value domains and canonical encoding;
setout fingerprint schema version 2 prevents version-1 identities from being
silently reinterpreted.
