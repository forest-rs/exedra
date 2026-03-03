---
id: exe-lopy
status: open
deps: []
links: [cam-tzew]
created: 2026-03-03T05:52:56Z
type: feature
priority: 1
assignee: Bruce Mitchener
tags: [v0.1, infra]
---
# exedra_testkit crate

Create the exedra_testkit workspace crate. Provides test fixtures, mesh generators, golden output helpers, and debug dump utilities. Uses std (not no_std). Lives at crates/exedra_testkit/.

## Design

Mesh generators (programmatic, deterministic):
- Single triangle, quad, open strip
- Closed shapes: tetrahedron, cube, icosahedron
- Parameterized grid/plane with configurable resolution
- Meshes with UV seams and sharp edges for testing

Golden output helpers:
- Serialize TriMesh to a comparable format
- Compare TriMesh against golden snapshots
- Deterministic serialization (no floating-point formatting issues)

Debug dump:
- Dump mesh topology for human inspection
- Dump attribute layers
- OBJ export (simple, for visual inspection)

This crate can depend on std and on exedra.

## Acceptance Criteria

- exedra_testkit crate exists in workspace
- At least 3 mesh generators (triangle, quad, tetrahedron/cube)
- Golden snapshot comparison helper exists
- Can be used from exedra tests


## Notes

**2026-03-03T06:37:44Z**

Worked example: docs/worked_example_basilica.md — defines minimal dump set (program input, per-step reports, final mesh snapshot, final extraction) and golden test posture.

**2026-03-03 — expanded scope (exe-rxny)**

exe-rxny details the expanded testkit scope: RON/JSON golden dump format with ordered lists (not maps) for determinism, fixture builders wrapping exedra_primitives with canonical params, snapshot comparison utilities (dump_golden / assert_golden). testkit depends on exedra_primitives and std.
