---
id: ea-8tpb
status: closed
deps: []
links: [exe-zqct]
type: bug
priority: 1
---
# Preserve constructive diagnostics during assembly compilation

## Problem

Assembly compilation can evaluate a recipe with zero bodies and an
`eval.unimplemented` diagnostic, then silently omit the instance during
flattening.

## Fence

`exedra_assembly` owns compilation-result visibility and deterministic part
diagnostics; it does not own constructive evaluation semantics or viewer policy.

## Acceptance

- Zero-body error or unimplemented evaluations cannot compile as silent
  success.
- Callers can inspect deterministic reports or receive a typed compile error.
- Warning retention and cache-hit report behavior are defined.
- A regression covers the omitted instance; public result changes include a
  migration note.

## Notes

**2026-08-24T18:24:09Z**

Implemented report-preserving assembly compilation. Successful recipe reports are cached with compiled geometry and exposed through `CompiledParts::report`; baked parts report `None`. A recipe that emits no bodies plus an Error diagnostic now returns `CompileError::NoGeometry` with the complete report, while partial geometry remains usable with diagnostics intact. The basilica now consumes retained reports instead of evaluating every recipe twice. This is a result-policy change in `exedra_assembly`, not a change to constructive evaluation or viewer behavior. Validated with `typos`, `taplo fmt --check`, `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace --all-features`, and `cargo doc --workspace --all-features --no-deps`; all passed.
