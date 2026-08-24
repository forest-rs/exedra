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

`joiner_timber` adopts `Length` for its rule dimensions in the first slice.
Setout's existing signed `Length` remains unchanged in that patch to avoid
combining a workspace-wide public migration with a timber-rule change. A
later setout migration should use `Offset` for coordinates and differences,
and `Length` where a quantity is intrinsically positive.
