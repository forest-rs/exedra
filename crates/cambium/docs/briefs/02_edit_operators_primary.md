# Brief: EditOperators are the primary Cambium execution path

## Decision
Cambium operators are primarily **edit-based**: they apply changes through
Exedra edit scopes and can return an Exedra `ChangeSet` when the runner uses a
recording sink. “Pure operators” (mesh-in/mesh-out) are secondary for
offline/batch use.

## Why
Edit-based execution aligns with Exedra’s kernel contract:

- consistent dirtiness and cache invalidation via ChangeSet/DirtySet
- deterministic created/deleted id reporting
- supports incremental extraction and derived-data recompute
- keeps mutation explicit and debuggable

## Alternatives considered
- **All operators return new Mesh**: simpler in isolation, but complicates incremental workflows and encourages copying.
- **Operators mutate mesh directly without EditSession**: breaks the contract and makes invalidation brittle.

## Implications
- OperatorRunner finishes edit scopes and standardizes preview/apply behavior.
- Operators focus on “what edits to apply,” not on bookkeeping.
- Policies (propagation, budgets, validation) flow through the runner/context.

## Non-goals / deferrals
- A full operator DAG runtime in v0.1; simple direct invocation is sufficient.
