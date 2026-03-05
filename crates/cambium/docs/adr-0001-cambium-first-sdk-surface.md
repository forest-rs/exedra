# ADR-0001: Cambium-First SDK Surface

- Status: Accepted
- Date: 2026-03-05
- Owners: Cambium maintainers
- Ticket: `cam-inf3`

## Context

The workspace has two public crates:
- `exedra`: topology/attribute kernel and invariants.
- `cambium`: operators, planning lifecycle, reporting, and workflow UX.

Without an explicit boundary, workflow conveniences tend to leak into the
engine crate, increasing coupling and reducing long-term replaceability.

## Decision

Adopt an explicit API-tier policy:
- Cambium is the workflow-facing SDK surface.
- Exedra is the engine-tier kernel surface.

Cambium crate docs and manual are the primary entry point for workflows.
Exedra crate docs identify Exedra as engine tier and point workflow users to
Cambium.

Curated Cambium re-exports include common workflow-facing engine types
(`Mesh`, ID types, `BuildParams`, `ExtractParams`, `DeletePolicy`) so typical
workflow code can remain in `cambium::...` imports.

## Exedra Admission Checklist

New API in Exedra must satisfy all:
1. Required for kernel correctness, invariants, or performance-critical internals.
2. Deterministic and explicit (no hidden mutable global behavior).
3. Compatible with `no_std` and dependency policy.
4. Covered by kernel-level tests/validation.
5. Not workflow/operator convenience API.

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
- Some convenience APIs that are easy to add in Exedra are intentionally
  routed through Cambium, adding minor coordination overhead.

