---
id: et-c9eu
status: closed
deps: [et-jmpb]
links: [et-o66p, exe-ccxf, et-jmpb]
created: 2026-08-21T13:38:53Z
type: feature
priority: 2
assignee: Bruce Mitchener
---
# Add deterministic constrained-Delaunay legalization

Improve ear-clipped polygon quality without changing the input vertex set, using exact deterministic edge legality decisions.

## Design

Keep ownership in exedra_triangulate. Start from the existing valid ear-clipped triangulation, legalize only unconstrained interior edges, preserve polygon and hole boundaries, and use stable index tie-breaks for cocircular cases. Extend the existing wind tunnel to compare quality, signatures, work counts, and timing against EarClip.

## Acceptance Criteria

An opt-in strategy produces deterministic CCW cover-equivalent triangles; constrained boundary edges are preserved; exact cocircular ties are pinned; the quality corpus shows measured non-regression and improvement where diagonal choice is causal; no_std, zero production dependencies, safe Rust.


## Notes

**2026-09-04T15:36:25Z**

Landed TriStrategy::ConstrainedDelaunay and triangulate_with_stats. Legalization runs only on a cover that already passed boundary-incidence verification, so bridge edges flip and true boundaries cannot. The cocircular tie rule (diagonal containing the lowest index) is a symbolic perturbation of the paraboloid lift: termination follows from Lawson on a perturbed point set without cocircular quadruples, and the result is independent of the initial ear-clipped cover. Pinned by every-apex fan tests, an exactly cocircular integer octagon, a canonical golden for the holed square (2 flips), and torture-corpus checks for boundary preservation, zero illegal edges, and idempotence. Validation: cargo test -p exedra_triangulate (64 pass, also with --no-default-features), clippy -D warnings, wind tunnel --quick pins hold after rebase onto main.
