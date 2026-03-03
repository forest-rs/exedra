# Brief: Preview vs commit is first-class in Cambium

## Decision
Cambium treats **preview** and **commit** as distinct, explicit execution modes. Preview may be budgeted/approximate and is discardable; commit is reproducible and defines the authoritative result.

## Why
Interactive modeling needs fast feedback without committing expensive or irreversible changes. Separating preview from commit:

- supports responsiveness and work budgeting
- keeps user intent explicit (no hidden commits)
- makes it easy to compare preview vs commit outputs
- maps cleanly to node-graph “viewport preview” vs “final evaluation”

## Alternatives considered
- **Always commit**: simplest, but too slow and risky for interactive iteration.
- **Implicit preview caches inside operators**: tends to create hidden state and nondeterministic behavior.

## Implications
- The runner owns preview orchestration (clone/COW/undo later) to keep operators simple.
- Budget exceed/cancel outcomes are not “bugs”; they are part of normal preview operation.
- Operators must produce deterministic reports even when truncated by budget.

## Non-goals / deferrals
- Perfect preview fidelity; correctness belongs to commit mode.
