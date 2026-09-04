# Brief: NumericPolicy and explicit tolerances (no hidden epsilons)

## Decision
All geometric comparisons, snapping/welding decisions, and boolean tolerances flow through an explicit **NumericPolicy** object. There are no hidden epsilons.

## Why
Hidden tolerances cause “retry storms” and unreproducible failures:

- different call sites quietly use different epsilons
- debugging becomes guesswork
- small refactors can change behavior

An explicit policy makes numeric behavior reviewable, testable, and tunable for different workloads.

## Alternatives considered
- **Scatter epsilons in code**: faster initially, but brittle and non-reviewable.
- **Global mutable tolerance**: easy to misuse and hostile to determinism and testing.

## Implications
- APIs that depend on tolerance accept (or access) NumericPolicy explicitly.
- Wind tunnels and golden tests record the policy used.
- Policy defaults must be documented and stable.

## Non-goals / deferrals
- This does not force “exact arithmetic”; it forces *explicit* approximate arithmetic.
