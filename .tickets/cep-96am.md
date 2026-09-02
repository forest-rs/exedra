---
id: cep-96am
status: closed
deps: []
links: []
created: 2026-09-02T13:19:05Z
type: task
priority: 1
assignee: Bruce Mitchener
---
# Name Euler rotation conventions explicitly

Correct the public Placement3 contract so fixed-axis and body-axis XYZ rotations are named and documented without ambiguity.

## Design

Preserve the existing matrix and compatibility helper, expose explicit extrinsic and intrinsic constructors, and pin multiplication order with mixed-angle basis-vector tests.

## Acceptance Criteria

Public rustdoc states axis space, application order, matrix order, and column-vector convention; both constructors produce their documented mixed-angle transforms; the existing helper remains behaviorally stable; internal call sites use an explicit convention; workspace gates pass.

## Notes

The existing matrix was correct for fixed-axis X, Y, Z rotations, but its
documentation described the opposite moving-axis order. Added explicit
`euler_extrinsic_xyz_then_translate` and
`euler_intrinsic_xyz_then_translate` constructors, retained the old matrix as
a deprecated compatibility alias, and migrated the workspace call site to the
explicit extrinsic spelling.

The owning ADR now records the column-vector and matrix-order conventions plus
the migration path. This is an API clarification, not an evaluator semantic
change, so recipe fingerprints and `EVAL_SCHEMA_VERSION` remain unchanged.
Regression tests pin a noncommuting quarter-turn example, an all-nonzero
sequential-rotation oracle, translation placement, and bit-identical behavior
through the compatibility alias.

Validation passed:

```sh
typos
cargo fmt --all -- --check
taplo fmt --check
cargo check -p exedra_constructive --no-default-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo doc --workspace --all-features --no-deps
```
