# ADR-0006: QEF Solver Scope

- Status: Accepted
- Date: 2026-03-24
- Revised: 2026-10-25, when the solver moved from the `exedra_qef` crate into
  this crate's `qef` module; dual contouring was its only consumer.

## Context

Dual contouring needs a small bounded QEF solver for cell-local vertex
placement. That solver is also useful outside the isosurface mesher for
feature-preserving remeshing, simplification, and fitting.

The workspace needs this capability without:

- pulling mesh or octree types into the solver,
- depending on a general-purpose linear algebra stack,
- giving up `no_std` compatibility in the reusable core.

## Decision

The `qef` module owns:

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
point so planar or edge-like neighborhoods do not drift arbitrarily. The
default solve path uses `bounds.center()`, while callers with better local
information may provide a custom anchor such as a Hermite mass point.

## Consequences

- Dual contouring uses the `qef` module without the solver knowing about
  fields, octrees or meshes.
- The solver remains small and replaceable.
- We keep the public API at the level the mesher actually needs: constraints in,
  bounded solution out.
