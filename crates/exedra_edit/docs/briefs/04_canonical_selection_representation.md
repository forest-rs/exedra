# Brief: Canonical selection representation (sorted Vec<FaceId>)

## Decision
v0.1 defines a canonical face selection as a `Vec<FaceId>` that is **sorted** by stable id order and **deduplicated**. Operators require canonical selections or canonicalize internally.

## Why
Selections are ubiquitous in modeling ops. A sorted Vec:

- is deterministic by construction
- has predictable memory overhead
- is easy to serialize and test
- works well for small/medium selections and is a stable baseline

## Alternatives considered
- **HashSet**: nondeterministic iteration order and higher overhead.
- **Bitsets**: efficient for whole-mesh selections but heavier to serialize and requires stable indexing semantics.
- **Compressed ranges**: good later, but adds complexity early.

## Implications
- All selection artifacts and params must maintain the canonical invariant.
- Later compressed representations must preserve deterministic iteration/order.

## Non-goals / deferrals
- High-performance selection storage for huge selections; revisit after real workloads exist.
