---
id: ec-dhp7
status: closed
deps: []
links: [ec-xkra]
created: 2026-09-05T02:28:19Z
type: bug
priority: 1
assignee: Bruce Mitchener
tags: [discretization, validation]
---
# Reject inverted discretization edge bounds

DiscretizePolicy accepts nonzero min_arc_edges greater than max_segment_edges. Arc discretization then calls clamp with an inverted range and panics instead of returning a typed policy error.

## Design

Validate min_arc_edges <= max_segment_edges once at the public discretization policy boundary. Preserve the existing InvalidEdgeBounds variant and extend its docs/display to cover inverted bounds. Add public loop/profile coverage across arc and line inputs, including zero, equal, and inverted bounds. This is policy validation only; no evaluation identity or serialized schema changes are needed because invalid policies never produce output.

## Acceptance Criteria

Inverted nonzero bounds return Err(DiscretizeError::InvalidEdgeBounds) without panic for valid arc and line loops; zero bounds retain typed errors; equal nonzero bounds are accepted; discretize_profile and public tessellation/evaluation consumers propagate the typed failure; focused tests and workspace quality gates pass.


## Notes

**2026-09-05T02:31:57Z**

Implemented min_arc_edges <= max_segment_edges validation at DiscretizePolicy::validate, retaining InvalidEdgeBounds and updating its public docs/display to describe zero and inverted bounds. Added regression coverage for zero minimum, zero maximum, inverted nonzero bounds, and equal bounds across line and arc loops plus discretize_profile. Inspected public tessellators: profile based planar, extrude, revolve, loft, and sweep all route through discretize_profile before curve work; primitive/grid paths do not discretize profiles. No API shape, cache fingerprint, evaluation identity, or schema change is needed because invalid policies produce no output. Relationship target: shared tolerance work ec-xkra (link after that ticket is visible in the integration checkout). Validation: pre-fix test reproduced panic at u32::clamp(min=8,max=4); afterward cargo fmt --all -- --check, typos, taplo fmt --check, cargo clippy -p exedra_constructive --all-targets --all-features -- -D warnings, cargo test -p exedra_constructive --all-features (186 passed, 2 ignored; 5 doctests passed), and cargo doc -p exedra_constructive --all-features --no-deps all passed.
