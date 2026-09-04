# Brief: Attribute access via Exedra built-in keys

## Decision
Exedra Ops relies on Exedra-provided **built-in attribute keys** for core layers (vertex positions, corner UVs, and later corner normal overrides). Exedra Ops does not invent identity for these layers.

## Why
Shared attribute identity is a kernel boundary concern. Built-in keys ensure:

- consistent layer identity across crates
- deterministic behavior (no string-key drift)
- easier interop and tooling

## Alternatives considered
- **Exedra Ops keys**: easy to start but leads to drift and ambiguity.
- **Per-operator local layers only**: prevents reuse and interop.

## Implications
- Exedra must expose stable built-in keys (e.g., `exedra::attr::CORNER_UV`).
- Additional layers can be introduced later via explicit registration APIs.

## Non-goals / deferrals
- A full dynamic attribute registry in v0.1; built-ins are enough.
