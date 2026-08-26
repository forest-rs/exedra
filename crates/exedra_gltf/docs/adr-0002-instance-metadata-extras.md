# ADR 0002: Preserve instance metadata under node extras

## Status

Accepted.

## Context

`exedra_assembly::Instance` carries opaque frontend metadata, but glTF export
previously discarded it. Exported assets therefore lost distinctions such as
source-authored versus policy-generated geometry even though the assembly had
retained that provenance.

## Decision

Export non-empty instance metadata as string properties under each glTF node's
`extras.metadata` object. Keep exporter-owned `instancePath`, `partKey`, and
`body` properties at the top level of `extras`.

The nested object prevents application keys from colliding with exporter
structure. Metadata follows the assembly's deterministic representation and
does not affect mesh sharing because it belongs to nodes, not meshes.

## Consequences

Consumers that ignore glTF `extras` remain unaffected. Consumers that inspect
assemblies can recover frontend provenance and roles without a side channel.
Empty metadata emits no `metadata` member.
