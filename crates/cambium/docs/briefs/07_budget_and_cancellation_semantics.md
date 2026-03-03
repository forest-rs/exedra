# Brief: Budget and cancellation semantics (deterministic preview truncation)

## Decision
Preview budgeting is primarily deterministic via `max_faces`/`max_corners` limits checked at deterministic checkpoints. Time budgets are advisory. Exceeding budget or cancellation yields structured outcomes (`BudgetExceeded`, `Cancelled`) rather than being treated as bugs.

## Why
Interactive preview must be predictable. Deterministic truncation prevents “sometimes it finishes, sometimes it doesn’t” behavior and keeps goldens meaningful.

## Alternatives considered
- **Time-only budgets**: inherently nondeterministic across machines and loads.
- **Implicit partial commits**: hidden state and confusing behavior.

## Implications
- Operators must check deterministic budgets in fixed-order loops (per-face, per-N corners).
- Partial artifacts/diagnostics are allowed but bounded and deterministically ordered.

## Non-goals / deferrals
- Sophisticated adaptive budgeting algorithms; start with simple limits.
