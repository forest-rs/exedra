# Brief: Attribute storage strategy (hybrid dense + sparse)

## Decision
Use a **hybrid attribute storage** model:

- **Required layers** are dense arrays (`Vec<T>`) sized to arena capacity (e.g., vertex positions).
- **Common optional layers** may be dense-with-missingness (dense `Vec<Option<T>>` or equivalent) when presence is high.
- **Rare override layers** may be stored sparsely (e.g., hash map keyed by stable id) when presence is low.

All layers are typed and bound to an explicit domain (Vertex/Face/Corner/HalfEdge).

## Why
Attributes vary widely in presence and size:

- positions are always present and hot → dense
- UVs/normals are common but not guaranteed → often dense optional
- authored overrides may be rare → sparse saves memory

Hybrid storage preserves cache locality for hot layers while avoiding the memory blow-up of “everything dense always.”

## Alternatives considered
- **All dense always.** Simple, fast, but wastes memory and makes rare layers expensive.
- **All sparse always.** Memory-efficient in some cases, but slower and adds nondeterminism risks (hash iteration) unless carefully controlled.
- **Per-element heap allocations.** Not acceptable for performance or memory fragmentation.

## Implications
- Attribute APIs must support:
  - presence checks
  - deterministic iteration (domain order, not hash order)
  - remapping on compaction (when enabled)
- Extraction splitting uses corner-domain layers and must treat missingness deterministically (policy-defined).
- Edit propagation rules must define how missing values propagate.

## Non-goals / deferrals
- A full dynamic schema system in v0.1.
- Automatic promotion/demotion between dense and sparse without an explicit operation; start with explicit choices and measure.
