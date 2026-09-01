# `ec-14f5`: constructive stretch execution plan

## Goal

Make `NodeKind::Stretch` a deterministic insert/remove-material operation
with exact constructive rewrites where structure permits and a typed,
provenance-preserving closed-mesh fallback for imports and composed bodies.

## Non-goals

- External format parsing or domain-specific zone policy.
- Automatic repair of open, non-manifold, tangent, or incompatible sections.
- A simultaneous multi-zone IR variant.
- New production dependencies or changes to the core mesh scalar policy.

## Fence and invariants

`exedra_constructive` owns stretch semantics, normalization, composition,
rewrites, mesh realization, reporting, and feature attribution. The Exedra
mesh kernel continues to own topology and validation; frontends own how
single-plane stretches are nested.

- The normalized negative half-space is stationary; positive moves.
- Expansion inserts a band; contraction discards an input slab and never
  folds geometry through itself.
- Outer nodes consume the already-evaluated output of inner nodes.
- Identical input is bit-identical; traversal and loop ordering never depend
  on hash iteration.
- Every emitted face has a source feature and stable `FACE_REGION`.
- Unsupported topology is a diagnostic and envelope fallback, never repair.

## Steps

1. Add failing trap tests for orientation, exact box/extrude cases, imported
   meshes, contraction refusal, nested composition, CSG placement, AABBs,
   provenance, and deterministic output.
2. Add CT-4 to `constructive_wind_tunnel`, timing exact box/extrude rewrites
   separately from the imported-mesh path and checking output signatures.
3. Implement normalized-plane helpers and exact box/extrusion/profile
   rewrites without changing stored recipes.
4. Implement deterministic mesh partition, section-loop extraction, band
   emission, translation, and contraction compatibility checks.
5. Add `eval.stretch.*` diagnostics, stretch counters, seam provenance,
   region/crease/UV propagation, cache integration, and schema-version bump.
6. Run focused tests and wind-tunnel before/after measurements, then the full
   repository definition of done. Review Must/Should/Could findings, close
   the ticket with durable decisions and validation results, and commit the
   closure atomically with the implementation.

## Risks

- Section topology at vertices and coplanar faces is ambiguous. Version 1
  refuses these cases rather than introducing tolerance-dependent ownership.
- Profile curves need exact intersection/splitting. Unsupported curve/cut
  combinations must fall back to mesh evaluation or refuse, never flatten in
  the algebraic path while claiming `Exact`.
- Attribute propagation can be topologically correct while semantically
  wrong; tests must assert both forward and reverse provenance and regions.
- Mesh fallback cost can dominate imported furniture. CT-4 separates phases
  so later optimization has a stable wind tunnel.
