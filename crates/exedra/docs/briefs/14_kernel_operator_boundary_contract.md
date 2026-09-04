# Brief: Kernel/operator boundary contract (kernel promises and operator responsibilities)

## Decision
Exedra and Exedra Ops have an explicit boundary contract:

- **Exedra** is the calm kernel: topology + attributes + deterministic extraction + validation + (later) booleans.
- **Exedra Ops** supplies the deterministic mesh-operator lifecycle: workflows,
  preview/commit orchestration, diagnostics, and thin explicit adapters.

Exedra Ops never depends on Exedra internals; Exedra never depends on Exedra Ops concepts.

## Why
Without a boundary contract:
- Exedra Ops reaches into kernel data structures
- Exedra gets polluted with tool semantics
- incremental workflows become brittle (duplicated invalidation logic)
- determinism breaks when ordering rules diverge

A crisp contract preserves modularity and replaceability.

## What Exedra promises (kernel guarantees)
1. **Stable handles**: public IDs are `(index, generation)` and validated.
2. **Deterministic behavior**:
   - deterministic traversal order for faces/corners
   - deterministic extraction buffer ordering
   - no hash iteration leakage into public outputs
3. **Explicit mutation**:
   - mutations occur via `EditSession`
   - finishing a recorded edit scope returns `ChangeSet` with `DirtySet`
4. **Attribute domains and built-ins**:
   - vertex/face/edge/corner domains are stable concepts
   - built-in keys exist for core layers:
     - `attr::VERTEX_POSITION`
     - `attr::CORNER_UV`
     - (later) `attr::CORNER_NORMAL_OVERRIDE`
5. **Validation**:
   - `validate_fast` and `validate_deep` produce structured reports
6. **Compaction is explicit**:
   - if supported, compaction returns a remap and invalidates old IDs for the new mesh
7. **Boolean pipeline (later)**:
   - staged execution with artifacts and structured failure taxonomy

## What Exedra Ops may do (allowed responsibilities)
- Execute generic mesh workflows; applications and domain crates define their
  own semantic vocabulary.
- Compose Exedra edits inside eager edit scopes; supply propagation policies intentionally.
- Orchestrate preview/commit (clone/COW/undo later) without mutating the committed base mesh in preview.
- Maintain operator-local caches keyed by (mesh version/change set, params hash).
- Use `invalidation` (formerly `understory_dirty`) for Exedra Ops runtime cache invalidation (not for kernel dirtiness).
- Own UV generation utilities and seam tooling.

## What Exedra Ops must NOT do (boundary violations)
- Mutate Exedra topology/attributes without a `EditSession`.
- Infer kernel dirtiness by inspection; must consume `ChangeSet.dirty`.
- Depend on internal arena layout beyond stable APIs.
- Invent identities for Exedra built-in attribute layers (no ad-hoc string keys).
- Leak nondeterministic ordering into externally visible results.

## Boundary objects (the seam)
Objects crossing the boundary and expected to remain stable:
- `EditSession`, `ChangeSet`, `DirtySet`
- `ValidateReport`
- extraction outputs (`TriMesh`, `ExtractStats`)
- boolean errors/artifacts (later)
- `Remap` (if compaction used)

## Implications
- Exedra APIs are intentionally boring and semver-stable.
- Exedra Ops can add mesh operators without changing Exedra.
- Debuggability is shared: Exedra provides core reports/artifacts; Exedra Ops provides operator reports/artifacts.

## Non-goals / deferrals
- A monolithic scene graph that owns both kernel and operators.
- Allowing Exedra Ops to bypass the edit-scope/change-set contract for performance.
