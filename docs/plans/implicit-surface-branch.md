# Implicit Surface Branch

Date: 2026-03-24
Owner: exedra workspace

## Goals

- Establish the implicit-surface branch as a reusable set of crates rather than
  a monolith hidden inside `exedra`.
- Land the dependency roots first: spatial indexing, scalar-field seam,
  Hermite bridge, and QEF solver.
- Reach a credible first extraction slice if the dependency roots settle
  cleanly.
- Extend the field layer enough that later adapters such as `exedra_fidget`
  target a composable field toolkit rather than a bare mesher seam.

## Non-goals

- A full canonical `exedra_implicit` head in this pass.
- Fidget integration before the generic field boundary is proven.
- Aggressive adaptive/manifold dual contouring in the first commit wave.
- A full implicit scene graph or generalized swept-surface model.

## Sequencing

1. `exe-0r1z` `exedra_spatial`
2. `exe-2r7w` `exedra_isosurface` field seam
3. `exe-5rwj` Hermite data in `exedra_isosurface`
4. `exe-5y1f` `exedra_qef`
5. `exe-a6p6` analytic reference fields
6. `exe-gosk` first dual-contouring path if the above slices stay calm
7. `ei-fq6w` transform wrappers for field composition
8. `ei-912r` `ScalarField2d` plus core profile primitives
9. `ei-7kzh` extrude/revolve lifting operators
10. `exe-vzlq` `exedra_fidget` once the generic field toolkit is stable

## Risks

- The `ScalarField` boundary may want to move into its own crate later.
- Provenance/tagging may expose gaps in `MeshBuilder` before the first extractor
  is complete.
- Full adaptive DC could expand into a larger slice than one turn reasonably
  supports.
- 2D profile lifting may expose naming or transform assumptions that want a
  small cleanup pass before `exedra_fidget`.

## Execution Rules

- Keep ticket closures atomic with their code commits.
- Keep `no_std` compatibility for the new reusable crates unless a ticket
  explicitly justifies otherwise.
- Prefer a working uniform-grid or bounded-cell extraction proof over a rushed
  adaptive DC implementation.
