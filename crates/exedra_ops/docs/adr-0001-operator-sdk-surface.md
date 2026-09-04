# ADR-0001: Exedra Ops SDK Surface

- Status: Accepted
- Date: 2026-03-05
- Owners: Exedra Ops maintainers

## Context

The Exedra family has a topology/attribute kernel (`exedra_mesh`) and a separate
mesh-operations crate (`exedra_ops`). The kernel owns polygon state and its
invariants; the operations crate owns deterministic mesh workflows, reports,
policies, and selections. Keeping those responsibilities separate prevents
workflow conveniences from leaking into the kernel and preserves
replaceability.

Sibling domain crates own their native values and lifecycles. `exedra_ops` has
typed adapters for explicit crossings, but it is not a heterogeneous runtime
or a cross-domain scheduler. ADR-0005 records the current limits and the
future graph boundary.

## Decision

Adopt an explicit API-tier and ownership policy:
- Exedra is the engine-tier polygon kernel surface.
- Exedra Ops is the workflow-facing mesh-operations surface.
- Exedra Ops may expose a typed adapter when a caller explicitly crosses into
  a sibling domain; the adapter does not transfer ownership of that domain.
- `EditOperator` and `OperatorRunner` remain mesh-specific. They consume and
  edit `Mesh` values and do not dispatch heterogeneous domain values.

Exedra Ops crate docs and manual are the primary entry point for mesh
workflows. Exedra crate docs identify Exedra as engine tier and point workflow
users to Exedra Ops. Cross-domain limits and the future typed-graph seam are
documented in ADR-0005.

Curated Exedra Ops re-exports include common workflow-facing engine types
(`Mesh`, ID types, `BuildParams`, `ExtractParams`, `DeletePolicy`) so typical
workflow code can remain in `exedra_ops::...` imports.

## Exedra Admission Checklist

New API in Exedra must satisfy all:
1. Required for kernel correctness, invariants, or performance-critical internals.
2. Deterministic and explicit (no hidden mutable global behavior).
3. Compatible with `no_std` and dependency policy.
4. Covered by kernel-level tests/validation.
5. Not a mesh-workflow/operator convenience API.

## Exception Process

When a change crosses API tiers, require a ticket tagged `api-exception`
documenting:
- rationale,
- alternatives considered,
- maintainer approval note before merge.

## Consequences

Positive:
- Clear discoverability and calmer public workflow surface.
- Better long-term modularity and replaceability.
- Exedra remains focused on invariants and kernel performance.

Tradeoffs:
- Some mesh-workflow conveniences that are easy to add in Exedra are
  intentionally routed through Exedra Ops, adding minor coordination overhead.
