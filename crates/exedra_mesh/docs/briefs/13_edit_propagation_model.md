# Brief: Edit propagation model (topology-first, deterministic, policy-hooked)

## Decision
Exedra defines a unified, deterministic **edit propagation model** for how attributes and dirtiness propagate during topology edits. Edits are:

1. **Topology-first** (create/modify topology)
2. **Attribute propagation** (move/copy/derive authored data according to rules)
3. **Dirty marking** (record invalidation for derived data and caches)

Edits accept an optional `PropagatePolicy`; if absent, Exedra uses stable defaults.

## Why
Edit propagation is the crosscutting concern behind:
- stable shading (corner normals/UVs)
- predictable UV seams
- correct incremental extraction
- avoiding “spooky action at a distance” after edits

If propagation is ad-hoc per operator, results diverge and become untestable.

## Domains
- **Vertex-domain**: keyed by `VertexId` (positions, scalar fields)
- **Face-domain**: keyed by `FaceId` (materials/regions/tags)
- **Edge-domain**: keyed by canonical edge `(h, twin(h))` (sharpness/crease)
- **Corner-domain**: keyed by `CornerId == HalfEdgeId` (UV, normal override)

## General rules (locked)
1. **Deterministic tie-breaking**: if a choice must be made, prefer smallest stable ids.
2. **Missingness is explicit**: sparse/optional layers treat missing values deterministically.
3. **Derived stays derived**: derived normals/tangents are recomputed from dirtiness; only authored overrides are propagated.
4. **No hidden work**: edits return dirtiness via `ChangeSet.dirty`; no implicit recomputation.

## Policy shape (starter)
```rust
pub struct PropagatePolicy {
    pub position_split: PositionSplit,
    pub uv_split: UvSplit,
    pub normal_override_split: NormalOverrideSplit,
    pub face_attr_split: FaceAttrSplit,
    pub edge_attr_split: EdgeAttrSplit,
    pub split_face_diagonal_edge_attr: SplitFaceDiagonalEdgeSplit,
    pub missingness: MissingnessPolicy,
}

pub enum PositionSplit { Midpoint }
pub enum UvSplit { Midpoint, CopyFromSide }
pub enum NormalOverrideSplit { Clear, CopyFromSide, AverageRenorm }
pub enum FaceAttrSplit { Copy }
pub enum EdgeAttrSplit { Inherit, Clear }
pub enum SplitFaceDiagonalEdgeSplit { FromEdgePolicy, Smooth, Inherit, DecayOnSplit }

pub enum MissingnessPolicy {
    Strict,
    PreferExisting,
}
```

## Dirtiness rules (kernel contract)
Each edit conservatively marks:
- `dirty_faces`: faces whose triangulation/extraction is invalid
- `dirty_corners`: corners whose corner-derived data is invalid (derived normals, tangents, UV-dependent splits)
- `dirty_vertices`: vertices whose one-ring derived data is invalid (smoothing groups, adjacency caches)

Conservative marking is acceptable initially; precision may be improved later.

## Canonical edit behaviors (v0.1–v0.5)

### `split_edge(e)`
Propagation defaults:
- Vertex position: midpoint of endpoints.
- Other vertex attrs: numeric midpoint if present; otherwise follow `MissingnessPolicy`.
- Edge attrs (sharpness/crease): inherited to both child edges.
- Corner UVs: per face, midpoint of endpoint corner UVs (if present).
- Corner normal overrides: **clear** for newly created corners.
- Face attrs: unchanged.

Dirtiness:
- adjacent faces dirty
- endpoint stars + `v_new` star dirty
- affected corners dirty

### `split_face(face, a, b)` / `insert_diagonal`
Propagation defaults:
- Face attrs: copied to both new faces.
- Corner UVs: existing corners preserve; new diagonal corners copy-from-side deterministically if present, else missing.
- Corner normal overrides: clear for newly created corners.
- Edge attrs: new diagonal uses the split-face diagonal edge policy. The
  compatibility default (`FromEdgePolicy`) preserves v0.1 behavior: `Inherit`
  and `Clear` make a smooth diagonal, while `DecayOnSplit` derives from nearby
  authored sharpness. Explicit split-face modes can force smooth, inherit the
  maximum source sharpness, or apply subdivision-style decay.

Dirtiness:
- both new faces dirty
- incident vertices dirty
- new corners dirty

### `collapse_edge(e)` (v0.5+)
Propagation defaults:
- Survivor vertex chosen deterministically (smallest id) unless policy says midpoint.
- UV conflicts at merged corner: prefer survivor or clear on conflict (policy-controlled).
- Normal overrides: clear on affected corners by default.
- Edge attrs: sharpness OR; crease weight max (default).

Dirtiness:
- neighborhood faces/corners/vertices dirty.

## Non-goals / deferrals
- Optimal attribute interpolation for all types (categorical, quaternions, etc.).
- Automatic seam creation/merging; Exedra Ops may add higher-level seam tools.
