# Brief: Validation invariants and structured error reporting

## Decision
Exedra provides **strong validation** with a clear invariant set and structured, classifiable error reporting. Validation is available at two tiers:

- `validate_fast()` — cheap, local consistency checks suitable for frequent use
- `validate_deep()` — full graph walks and higher-cost checks, producing explainable reports

Validation results are **structured** (not just strings) and are suitable for tests/goldens, bug reports, and interactive debug tooling.

## Why
A topology kernel inevitably encounters invalid intermediate states (during development, during tool authoring, and sometimes in user content). Without strong validation:

- bugs are hard to localize
- boolean failures are opaque
- incremental caches can silently corrupt results

Structured validation is the cheapest way to keep the system “debuggable by default.”

## Invariant set (v0.1 baseline)

### ID/arena invariants
- Every public `Id` is `(index, generation)` and must either be **valid** for the arena or explicitly rejected.
- Arena slot order is stable; tombstones may exist.
- A valid handle must never reference a tombstoned slot with mismatched generation.

### Half-edge invariants (local)
For every live half-edge `h`:
- `twin(twin(h)) == h`
- `next(h)` is live
- `face(h)` is live
- `to(h)` is live
- `next(h)` belongs to the same face loop as `h`

### Face loop invariants
For every live face `f` (excluding `OUTSIDE` as applicable):
- `edge(f)` is live and `face(edge(f)) == f`
- Walking `next` from `edge(f)` terminates at `edge(f)` within a bounded step count
- The loop contains **no repeated half-edges** (simple cycle) unless explicitly permitted for intermediate repair operations
- `degree(f)` (if cached) matches the loop length

### Vertex star invariants (baseline)
For every live vertex `v`:
- `out(v)` is either a valid outgoing half-edge or a sentinel `INVALID` for isolated vertices
- If `out(v)` is valid, repeated `twin(next(..))` traversal around the vertex must terminate and must not reference invalid half-edges/faces
- Boundary behavior is well-defined via `OUTSIDE`

### Boundary model invariants
- Every topological edge has exactly two half-edges; boundary edges are represented via `face == OUTSIDE` on one side.
- Boundary half-edges form cycles attached to `OUTSIDE`.
- `OUTSIDE` representation is consistent with the chosen implementation (sentinel vs arena entry).

### Attribute invariants (baseline)
- Required layers (e.g., `VERTEX_POSITION`) are dense and sized to the vertex arena capacity.
- Optional layers are either dense-with-missingness (sized to arena capacity) or sparse with deterministic iteration.
- Domain correctness: a layer’s declared domain matches how it is indexed.
- Extraction splitting is consistent: render vertices split on corner discontinuities (UVs, normals, etc.) but topology remains stable.

## Deep checks (v0.1+)
`validate_deep()` may additionally check:
- Non-manifold conditions (optional v0.5+): edge/face incidence consistency via OUTSIDE
- Degenerate geometry checks: zero-area faces (policy-controlled), NaN/Inf positions
- Deterministic ordering checks: no externally visible list depends on hash iteration order

## Structured validation report format

```rust
pub enum ValidateLevel { Fast, Deep }

pub struct ValidateReport {
    pub level: ValidateLevel,
    pub errors: alloc::vec::Vec<ValidateIssue>,
    pub warnings: alloc::vec::Vec<ValidateIssue>,
    pub stats: ValidateStats,
}

pub struct ValidateStats {
    pub vertices: u64,
    pub half_edges: u64,
    pub faces: u64,
    pub boundary_loops: u64,
}

pub struct ValidateIssue {
    pub code: ValidateCode,
    pub message: alloc::string::String,
    pub spans: alloc::vec::Vec<ValidateSpan>,
}

pub enum ValidateSpan {
    Vertex(VertexId),
    HalfEdge(HalfEdgeId),
    Face(FaceId),
    Corner(CornerId),
}

pub enum ValidateCode {
    InvalidId,
    DanglingReference,
    TwinMismatch,
    NextMismatch,
    FaceLoopNonClosing,
    FaceLoopRepeatedHalfEdge,
    FaceEdgeWrongFace,
    DegreeMismatch,
    VertexStarNonTerminating,
    OutsideInconsistency,
    MissingRequiredLayer,
    LayerDomainMismatch,
    LayerSizeMismatch,
    NanOrInfPosition,
    // reserved for future:
    NonManifoldEdge,
    SelfIntersection,
}
```

### Determinism rules
- Issues are returned in a deterministic order:
  - primary: `code`
  - secondary: smallest referenced stable id in `spans`
  - tertiary: discovery order from deterministic traversal
- Hash-derived collections must be sorted before output.

## Relationship to other error types
- Validation issues are distinct from boolean failures.
- Boolean errors may include a `ValidateReport` snapshot (or a subset) when helpful.
- Cambium can present validation reports via its `DiagnosticsSink`, preserving codes/spans.

## Non-goals / deferrals
- Proving manifoldness or watertightness for all meshes in v0.1.
- Repairing meshes during validation; validation reports, it does not mutate.
