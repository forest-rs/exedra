---
id: exe-17rj
status: open
deps: [exe-nca7, exe-dc9l]
links: []
created: 2026-03-03T05:27:36Z
type: feature
priority: 0
assignee: Bruce Mitchener
tags: [v0.1, foundation]
---
# Attribute system core

Implement the typed attribute layer system. Attributes are typed data layers keyed by domain (Vertex, Face, HalfEdge/Corner). This is the extensible data model that carries positions, UVs, sharpness, and custom data.

## Design

Domains: Vertex, Face, HalfEdge (Corner == HalfEdge)

Storage strategy (locked architectural decision: hybrid dense + sparse):
- Required layers (e.g. positions): dense Vec<T>
- Frequently-present optional layers (e.g. corner UVs): dense Option or dense with default
- Rare override layers (e.g. custom normals): sparse

AttrKey<T> is a strongly-typed key:
- domain: Domain
- name: &static str (or a static identifier)
- PhantomData<T> for type safety

Built-in attribute keys (Exedra provides these; Cambium uses them):
- exedra::attr::VERTEX_POSITION
- exedra::attr::CORNER_UV
- (later) exedra::attr::CORNER_NORMAL_OVERRIDE

Layers must support:
- get(id) -> Option<&T> or &T for required layers
- set(id, value)
- Remapping if arenas compact (but compaction is explicit, not implicit)
- Matching arena capacities (dense) or explicit sparseness

v0.1 scope: implement the registry/storage infrastructure and the required position layer. Corner UVs and edge sharpness are separate tickets.

## Acceptance Criteria

- Domain enum exists (Vertex, Face, HalfEdge)
- AttrKey<T> provides typed, domain-scoped attribute identity
- Dense and sparse layer storage exists
- Attributes struct can hold multiple typed layers
- Mesh exposes attrs() and attrs_mut()
- Built-in key constants defined (at least VERTEX_POSITION)
- Layer capacity tracks arena capacity for dense layers
- Unit tests for layer creation, get/set, capacity tracking

