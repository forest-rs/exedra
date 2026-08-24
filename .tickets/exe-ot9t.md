---
id: exe-ot9t
status: closed
deps: []
links: []
type: task
priority: 1
---
# Canonicalize coincident vertices in Boolean seams

## Problem

Stitched Boolean output can contain distinct vertices at identical narrowed
`f32` positions along seam rings, producing zero-length rim edges.

## Fence

Exedra owns canonical mesh identity and attribute-preserving seam cleanup; it
does not own higher-level fillet policy.

## Acceptance

- Seam rings contain no exact-position duplicate vertices.
- Survivor and attribute/provenance merge rules are deterministic and
  documented.
- Deep validation passes and cleanup or rounding no longer needs alias
  workarounds.
- The Boolean oracle remains deterministic for existing valid outputs.

## Notes

**2026-08-24T02:47:29Z**

Implemented at the Boolean stitch boundary.

- Root cause: provenance-distinct `f64` intersection-graph constructions can
  narrow to one numerical `f32` mesh position inside a connected seam.
- Stored-position identity is scoped by graph connectivity, canonicalizes
  signed zero, and selects the lowest graph index as survivor. Disconnected
  coincident seams and point-contact shells remain distinct.
- If the identity merge pinches a selected source-face walk, stitching
  decomposes it into simple cycles without adding diagonals. Non-degenerate
  cycles emit in encounter order, repeat the source provenance, and retain
  attributes only on surviving edges.
- Rounding no longer welds aliases by coordinate or deletes source faces to
  make a rewrite fit. A selected zero-length edge returns `DegenerateEdge`.
- Migration: `RoundStats::vanished_faces` was removed. Delete field reads and
  struct-literal entries; there is no replacement counter because the
  behavior it counted no longer exists.
- The Boolean oracle now fails on deep-validation errors and exact-position
  aliases in connected marked seams, as well as mesh/field disagreements.

Validation passed: `typos`; `cargo fmt --all -- --check`; `taplo fmt --check`;
workspace clippy with warnings denied; all-feature workspace tests and docs;
and the `exedra` `libm`/`no_std` check. Release-oracle coverage comprised
4,900 configured generated cases across seeds 1, 2, 3, 42, 999, and 20260824,
plus a 1,400-case post-tightening rerun. There were zero mesh disagreements,
topology findings, seam-identity conflicts, or field disagreements. The
curved-wall seed 1 reproducer also passed 4,000 sampled points.
