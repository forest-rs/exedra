# Brief: Operator reports and bounded artifacts are mandatory

## Decision
Every Cambium operator produces an `OpReport` with deterministic stats/counters and optional bounded artifacts. Artifacts/diagnostics are bounded and deterministically ordered.

## Why
Procedural systems become opaque without introspection. Reports/artifacts make operators:

- measurable (wind tunnels, profiling)
- debuggable (why did it do that?)
- testable (goldens, reproducible failure artifacts)
- safe to evolve (you can detect regressions early)

Bounding prevents runaway memory usage, especially on large meshes.

## Alternatives considered
- **No reports**: fastest to write, slowest to debug.
- **Unbounded artifacts**: turns diagnostics into a memory bomb.

## Implications
- Counters must not depend on nondeterministic ordering.
- Diagnostics overflow uses a deterministic severity-aware retention policy.
- Artifact output is truncated deterministically.

## Non-goals / deferrals
- Rich UI presentation of artifacts; v0.1 focuses on structured data and serialization.
