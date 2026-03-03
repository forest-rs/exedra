---
id: cam-wu7q
status: open
deps: [cam-vt4j, cam-xis1, cam-0w9l]
links: []
created: 2026-03-03T05:57:52Z
type: feature
priority: 0
assignee: Bruce Mitchener
tags: [v0.1, foundation]
---
# EditOperator trait

Define the EditOperator trait — the primary operator execution path in Cambium. Operators apply edits via an Exedra transaction.

## Design

trait EditOperator {
    type Params;
    fn name(&self) -> &static str;
    fn apply(&self, txn: &mut exedra::Txn, params: &Self::Params, ctx: &mut OpContext) -> Result<OpReport, OpError>;
}

Operator identity:
- name() returns a stable dot-separated namespace identifier (e.g. "uv.planar")
- Names are part of the determinism/debug contract
- Names are machine-friendly, not user-facing prose

The Txn is committed by OperatorRunner, not by the operator itself.
OpReport returned from apply() ensures reports exist even if commit later fails.

No global operator registry in v0.1 — static dispatch is fine.

## Acceptance Criteria

- EditOperator trait defined with Params associated type
- name() returns stable identifier
- apply() takes Txn, Params, OpContext
- Trait is object-safe if practical (or document why not)
- At least one implementation exists (uv_planar)

