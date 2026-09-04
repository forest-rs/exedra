# Brief: Clock and timing buckets (profiling without nondeterminism)

## Decision
Exedra Ops records timings in named buckets using an RAII guard. Timing values are best-effort and not deterministic; bucket naming and presence are stable, and repeated measurements accumulate.

## Why
We need performance visibility (wind tunnels, profiling) without leaking nondeterminism into correctness-sensitive outputs. Bucketed timing gives:

- structured profiling that compares across runs
- low overhead
- stable naming for dashboards and logs

## Alternatives considered
- **No timing**: impossible to measure regressions well.
- **High-cardinality per-call timing**: too much overhead and noisy results.

## Implications
- Bucket names are stable `'static` strings.
- `Timings` caps the number of distinct buckets; overflow drops deterministically.
- `no_std` builds may record zeros (still deterministic).

## Non-goals / deferrals
- Subtracting nested bucket time to form exclusive timing; additive attribution is simpler and sufficient.
