# Brief: Determinism as a first-class feature

## Decision
Exedra treats determinism as a feature: given identical inputs (mesh + attributes), parameters, and supported toolchain range, Exedra produces **identical outputs** (topology edits, extraction buffer ordering, boolean artifacts ordering).

## Why
Determinism is architectural leverage:

- makes caching safe and debuggable
- enables golden tests that actually mean something
- makes failures reproducible (especially for booleans and numeric edge cases)
- supports “wind tunnel” performance regression tracking without noise from ordering variation

## Alternatives considered
- **“Mostly deterministic” best-effort**: leads to flaky tests and hard-to-debug cache invalidation.
- **Hide nondeterminism behind hash maps**: faster to write, but leaks ordering differences into public outputs.

## Implications
- Stable iteration order over arenas is required.
- Hash-based internal structures must never leak iteration order into externally visible lists; sort deterministically.
- Tie-breaking rules must be explicit (prefer smallest stable IDs, axis precedence, etc.).
- Output ordering for extraction and artifacts must be specified and tested.

## Non-goals / deferrals
- Determinism does not imply bit-identical floating point results across every CPU/feature set; the goal is stable ordering and stable algorithmic decisions for the supported target set.
