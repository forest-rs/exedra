---
id: exe-mid7
title: NumericPolicy in Exedra core
status: open
deps: []
links: []
created: 2026-03-03T05:35:43Z
type: feature
priority: P1
assignee: Bruce Mitchener
tags: [v0.1]
---
# NumericPolicy in Exedra core

Implement NumericPolicy as a plain struct in Exedra core. All geometric comparisons and snapping/welding decisions flow through this policy — no hidden epsilons.

## Design

NumericPolicy { epsilon: f32, merge_tolerance: f32, coplanar_tolerance: f32, normal_epsilon: f32 }

- Passed explicitly into operations that depend on numeric thresholds
- Defaults must be documented and tested
- Lives in exedra core (not a separate crate)
- Copy + Clone + Debug
- Used by: mesh construction (welding), booleans (intersection), Cambium (UV projection tie-breaking)

This aligns with "Explicit Over Implicit" — no magic constants buried in code.

## Acceptance Criteria

- NumericPolicy struct exists with documented default values
- Copy + Clone + Debug
- Used by from_indexed_triangles (weld tolerance)
- No hidden epsilon constants elsewhere in the codebase
- Unit test validates default values are sensible


## Notes

**2026-03-03T06:17:41Z**

Design brief: crates/exedra/docs/briefs/04_numeric_policy_explicit_epsilons.md
