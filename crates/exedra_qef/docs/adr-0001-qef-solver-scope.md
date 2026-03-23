# ADR-0001: QEF Solver Scope

- Status: Accepted
- Date: 2026-03-24

## Context

Dual contouring needs a small bounded QEF solver for cell-local vertex
placement. That solver is also useful outside the isosurface mesher for
feature-preserving remeshing, simplification, and fitting.

The workspace needs this capability without:

- pulling mesh or octree types into the solver crate,
- depending on a general-purpose linear algebra stack,
- giving up `no_std` compatibility in the reusable core.

## Decision

`exedra_qef` owns:

- raw plane-constraint accumulation,
- bounded QEF solve for 3x3 symmetric systems,
- rank-based sharpness classification,
- residual-error reporting.

It does not own:

- Hermite sampling,
- scalar-field evaluation,
- octree traversal,
- mesh output or attribute tagging.

The first implementation uses an inline Jacobi eigensolver for the 3x3
symmetric normal-equation matrix. Rank selection uses a relative eigenvalue
cutoff, and low-rank solves pin null-space dimensions to an explicit anchor
point so planar or edge-like neighborhoods do not drift arbitrarily.

## Consequences

- `exedra_isosurface` can depend on `exedra_qef` without creating ownership
  confusion.
- The solver remains small and replaceable.
- We keep the public API at the level the mesher actually needs: constraints in,
  bounded solution out.

