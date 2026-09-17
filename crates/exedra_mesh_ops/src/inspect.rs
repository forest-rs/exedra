// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Read-only mesh bounds and selection inspection.

use crate::math::FloatExt;
use crate::selection::{FaceSet, Selection, SelectionKind, canonicalize_face_set};
use alloc::vec::Vec;
use exedra_mesh::{CornerId, FaceId, Mesh, VertexId};

/// Face selection scope for [`bounds`].
#[derive(Clone, Debug, PartialEq)]
pub enum BoundsScope {
    /// Use all mesh vertices.
    WholeMesh,
    /// Use vertices referenced by selected faces.
    FaceSet(FaceSet),
}

/// Parameters for [`bounds`].
#[derive(Clone, Debug, PartialEq)]
pub struct BoundsParams {
    /// Selection scope.
    pub scope: BoundsScope,
}

impl Default for BoundsParams {
    fn default() -> Self {
        Self {
            scope: BoundsScope::WholeMesh,
        }
    }
}

/// Axis-aligned bounds summary.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BoundsSummary {
    /// Minimum corner.
    pub min: [f32; 3],
    /// Maximum corner.
    pub max: [f32; 3],
    /// Arithmetic center of included points.
    pub centroid: [f32; 3],
    /// Length of diagonal (`|max - min|`).
    pub diagonal: f32,
}

/// Typed output from [`bounds`].
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct BoundsOutput {
    /// Computed bounds, `None` when no vertices are selected.
    pub bounds: Option<BoundsSummary>,
    /// Number of vertices included in the computation.
    pub vertex_count: u64,
    /// Number of faces processed by the selected scope.
    pub face_count: u64,
    /// The input face selection needed sorting or duplicate removal.
    pub selections_canonicalized: bool,
}

/// Summary detail level for [`selection_summary`].
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub enum SelectionSummaryDetail {
    /// Count only liveness and domain totals.
    #[default]
    Basic,
    /// Include extra lightweight topology checks (for example edge canonicality).
    Topology,
}

/// Parameters for [`selection_summary`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectionSummaryParams {
    /// Input selection to summarize.
    pub selection: Selection,
    /// Summary detail level.
    pub detail: SelectionSummaryDetail,
}

impl Default for SelectionSummaryParams {
    fn default() -> Self {
        Self {
            selection: Selection::from(FaceSet::new()),
            detail: SelectionSummaryDetail::Basic,
        }
    }
}

/// Typed output from [`selection_summary`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct SelectionSummaryOutput {
    /// Selection domain.
    pub kind: SelectionKind,
    /// Canonicalized item count.
    pub item_count: u64,
    /// Number of live IDs in the mesh.
    pub live_count: u64,
    /// Number of stale IDs in the selection.
    pub stale_count: u64,
    /// Number of `FaceId::OUTSIDE` entries (face-domain only).
    pub outside_count: u64,
    /// Number of non-canonical undirected edge IDs (edge-domain + topology detail).
    pub non_canonical_edge_count: u64,
    /// True when no stale IDs were found.
    pub all_live: bool,
}

impl Default for SelectionSummaryOutput {
    fn default() -> Self {
        Self {
            kind: SelectionKind::Faces,
            item_count: 0,
            live_count: 0,
            stale_count: 0,
            outside_count: 0,
            non_canonical_edge_count: 0,
            all_live: true,
        }
    }
}

/// Failure from mesh bounds inspection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BoundsError {
    /// An explicitly selected face is outside or stale.
    InvalidFace {
        /// Rejected face.
        face: FaceId,
    },
    /// A traversed corner has no destination vertex.
    InvalidCorner {
        /// Rejected corner.
        corner: CornerId,
    },
    /// A live vertex has no position.
    MissingPosition {
        /// Rejected vertex.
        vertex: VertexId,
    },
    /// Positions or the computed bounds exceed finite f32 representation.
    NumericLimit,
}
impl core::fmt::Display for BoundsError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidFace { face } => write!(f, "invalid bounds face {face:?}"),
            Self::InvalidCorner { corner } => write!(f, "invalid bounds corner {corner:?}"),
            Self::MissingPosition { vertex } => write!(f, "missing bounds position {vertex:?}"),
            Self::NumericLimit => f.write_str("bounds exceed numeric limits"),
        }
    }
}
impl core::error::Error for BoundsError {}

/// Measures all vertices, or unique vertices belonging to selected faces.
///
/// The centroid is the arithmetic mean of included vertices, not the AABB
/// center or an area/volume centroid. Whole-mesh scope includes isolated
/// vertices. Empty scope returns `None`; stale selections and nonfinite or
/// overflowing f32 calculations return typed errors. Inspection does not edit.
pub fn bounds(mesh: &Mesh, params: &BoundsParams) -> Result<BoundsOutput, BoundsError> {
    let mut acc = BoundsAccumulator::default();
    let mut output = BoundsOutput::default();
    let push = |acc: &mut BoundsAccumulator, vertex| {
        let position = *mesh
            .vertex_position(vertex)
            .ok_or(BoundsError::MissingPosition { vertex })?;
        if !exedra_math::finite(position) {
            return Err(BoundsError::NumericLimit);
        }
        acc.push(position);
        Ok(())
    };
    match &params.scope {
        BoundsScope::WholeMesh => {
            output.face_count = mesh.faces().count() as u64;
            for vertex in mesh.vertices() {
                push(&mut acc, vertex)?;
            }
        }
        BoundsScope::FaceSet(input) => {
            let mut faces = input.clone();
            output.selections_canonicalized = canonicalize_face_set(&mut faces);
            let mut vertices = Vec::new();
            for face in faces {
                if face == FaceId::OUTSIDE || mesh.face_edge(face).is_none() {
                    return Err(BoundsError::InvalidFace { face });
                }
                output.face_count += 1;
                for corner in mesh.face_loop(face) {
                    vertices.push(
                        mesh.to_vertex(corner)
                            .ok_or(BoundsError::InvalidCorner { corner })?,
                    );
                }
            }
            vertices.sort_unstable();
            vertices.dedup();
            for vertex in vertices {
                push(&mut acc, vertex)?;
            }
        }
    }
    output.vertex_count = acc.count;
    output.bounds = acc.bounds();
    if output
        .bounds
        .is_some_and(|b| !exedra_math::finite(b.centroid) || !b.diagonal.is_finite())
    {
        return Err(BoundsError::NumericLimit);
    }
    Ok(output)
}

#[derive(Copy, Clone, Debug, Default)]
struct BoundsAccumulator {
    min: Option<[f32; 3]>,
    max: [f32; 3],
    sum: [f32; 3],
    count: u64,
}

impl BoundsAccumulator {
    fn push(&mut self, point: [f32; 3]) {
        if let Some(min) = self.min.as_mut() {
            min[0] = min[0].min(point[0]);
            min[1] = min[1].min(point[1]);
            min[2] = min[2].min(point[2]);
            self.max[0] = self.max[0].max(point[0]);
            self.max[1] = self.max[1].max(point[1]);
            self.max[2] = self.max[2].max(point[2]);
        } else {
            self.min = Some(point);
            self.max = point;
        }
        self.sum[0] += point[0];
        self.sum[1] += point[1];
        self.sum[2] += point[2];
        self.count = self.count.saturating_add(1);
    }

    fn bounds(self) -> Option<BoundsSummary> {
        let min = self.min?;
        let inv = 1.0 / (self.count as f32);
        let centroid = [self.sum[0] * inv, self.sum[1] * inv, self.sum[2] * inv];
        let dx = self.max[0] - min[0];
        let dy = self.max[1] - min[1];
        let dz = self.max[2] - min[2];
        Some(BoundsSummary {
            min,
            max: self.max,
            centroid,
            diagonal: (dx * dx + dy * dy + dz * dz).sqrt_ext(),
        })
    }
}

/// Canonical selection and its liveness/topology summary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectionInspection {
    /// Sorted, deduplicated selection. Stale IDs remain available for diagnosis.
    pub selection: Selection,
    /// Counts for the canonical selection.
    pub summary: SelectionSummaryOutput,
    /// Sorting or duplicate removal changed the supplied selection.
    pub selections_canonicalized: bool,
}

/// Inspects selection liveness without changing the mesh or supplied selection.
///
/// `FaceId::OUTSIDE` is counted separately from stale IDs. `all_live` means
/// no stale IDs; it does not imply the selection is valid for every operation.
#[must_use]
pub fn selection_summary(mesh: &Mesh, params: &SelectionSummaryParams) -> SelectionInspection {
    let mut selection = params.selection.clone();
    let selections_canonicalized = selection.canonicalize();
    let mut output = SelectionSummaryOutput {
        kind: selection.kind(),
        ..SelectionSummaryOutput::default()
    };

    match &selection {
        Selection::Faces(faces) => {
            output.item_count = u64::try_from(faces.len()).expect("face count should fit u64");
            for &face in faces {
                if face == FaceId::OUTSIDE {
                    output.outside_count = output.outside_count.saturating_add(1);
                    continue;
                }
                if mesh.face_edge(face).is_some() {
                    output.live_count = output.live_count.saturating_add(1);
                } else {
                    output.stale_count = output.stale_count.saturating_add(1);
                }
            }
        }
        Selection::Edges(edges) => {
            output.item_count = u64::try_from(edges.len()).expect("edge count should fit u64");
            for &edge in edges {
                if mesh.twin(edge).is_some() {
                    output.live_count = output.live_count.saturating_add(1);
                    if params.detail == SelectionSummaryDetail::Topology
                        && mesh
                            .canonical_edge(edge)
                            .is_some_and(|canonical| canonical != edge)
                    {
                        output.non_canonical_edge_count =
                            output.non_canonical_edge_count.saturating_add(1);
                    }
                } else {
                    output.stale_count = output.stale_count.saturating_add(1);
                }
            }
        }
        Selection::Vertices(vertices) => {
            output.item_count = u64::try_from(vertices.len()).expect("vertex count should fit u64");
            for &vertex in vertices {
                if mesh.vertex_position(vertex).is_some() {
                    output.live_count = output.live_count.saturating_add(1);
                } else {
                    output.stale_count = output.stale_count.saturating_add(1);
                }
            }
        }
    }

    output.all_live = output.stale_count == 0;
    SelectionInspection {
        selection,
        summary: output,
        selections_canonicalized,
    }
}

#[cfg(test)]
mod tests;
