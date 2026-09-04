// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Canonical selection representation helpers.

use alloc::vec::Vec;
use core::fmt;

use exedra::{FaceId, HalfEdgeId, VertexId};

/// Canonical face selection (`Vec<FaceId>` sorted and deduplicated).
///
/// Used by operator params such as
/// [`TagFaceRegionParams::faces`](crate::TagFaceRegionParams::faces).
pub type FaceSet = Vec<FaceId>;
/// Canonical edge selection (`Vec<HalfEdgeId>` sorted and deduplicated).
///
/// Used by edge-mark operators and UV/tagging selection APIs.
pub type EdgeSet = Vec<HalfEdgeId>;
/// Canonical vertex selection (`Vec<VertexId>` sorted and deduplicated).
pub type VertexSet = Vec<VertexId>;

/// Domain tag for [`Selection`].
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum SelectionKind {
    /// Face-domain selection.
    Faces,
    /// Edge-domain selection.
    Edges,
    /// Vertex-domain selection.
    Vertices,
}

impl fmt::Display for SelectionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Faces => "faces",
            Self::Edges => "edges",
            Self::Vertices => "vertices",
        };
        f.write_str(text)
    }
}

impl core::error::Error for SelectionKind {}

/// Error returned when a typed selection accessor receives the wrong domain.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct SelectionDomainError {
    /// Expected domain.
    pub expected: SelectionKind,
    /// Actual domain.
    pub actual: SelectionKind,
}

impl fmt::Display for SelectionDomainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "selection domain mismatch: expected {}, got {}",
            self.expected, self.actual
        )
    }
}

impl core::error::Error for SelectionDomainError {}

/// Generic selection bridge for composition APIs.
///
/// Operator params/outputs should still prefer typed sets (`FaceSet`,
/// `EdgeSet`, `VertexSet`) where possible. This enum is for domain-generic
/// composition layers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Selection {
    /// Face-domain selection.
    Faces(FaceSet),
    /// Edge-domain selection.
    Edges(EdgeSet),
    /// Vertex-domain selection.
    Vertices(VertexSet),
}

impl Selection {
    /// Returns the domain kind.
    #[must_use]
    pub fn kind(&self) -> SelectionKind {
        match self {
            Self::Faces(_) => SelectionKind::Faces,
            Self::Edges(_) => SelectionKind::Edges,
            Self::Vertices(_) => SelectionKind::Vertices,
        }
    }

    /// Canonicalizes contained IDs in-place.
    ///
    /// Returns `true` if the contained set changed.
    pub fn canonicalize(&mut self) -> bool {
        match self {
            Self::Faces(faces) => canonicalize_face_set(faces),
            Self::Edges(edges) => canonicalize_edge_set(edges),
            Self::Vertices(vertices) => canonicalize_vertex_set(vertices),
        }
    }

    /// Returns face-set view if this selection is face-domain.
    #[must_use]
    pub fn as_faces(&self) -> Option<&FaceSet> {
        match self {
            Self::Faces(faces) => Some(faces),
            _ => None,
        }
    }

    /// Returns edge-set view if this selection is edge-domain.
    #[must_use]
    pub fn as_edges(&self) -> Option<&EdgeSet> {
        match self {
            Self::Edges(edges) => Some(edges),
            _ => None,
        }
    }

    /// Returns vertex-set view if this selection is vertex-domain.
    #[must_use]
    pub fn as_vertices(&self) -> Option<&VertexSet> {
        match self {
            Self::Vertices(vertices) => Some(vertices),
            _ => None,
        }
    }

    /// Returns face-set view or a domain mismatch error.
    pub fn require_faces(&self) -> Result<&FaceSet, SelectionDomainError> {
        self.as_faces().ok_or(SelectionDomainError {
            expected: SelectionKind::Faces,
            actual: self.kind(),
        })
    }

    /// Returns edge-set view or a domain mismatch error.
    pub fn require_edges(&self) -> Result<&EdgeSet, SelectionDomainError> {
        self.as_edges().ok_or(SelectionDomainError {
            expected: SelectionKind::Edges,
            actual: self.kind(),
        })
    }

    /// Returns vertex-set view or a domain mismatch error.
    pub fn require_vertices(&self) -> Result<&VertexSet, SelectionDomainError> {
        self.as_vertices().ok_or(SelectionDomainError {
            expected: SelectionKind::Vertices,
            actual: self.kind(),
        })
    }

    /// Consumes the selection as face-set or returns mismatch error.
    pub fn into_faces(self) -> Result<FaceSet, SelectionDomainError> {
        match self {
            Self::Faces(faces) => Ok(faces),
            Self::Edges(_) => Err(SelectionDomainError {
                expected: SelectionKind::Faces,
                actual: SelectionKind::Edges,
            }),
            Self::Vertices(_) => Err(SelectionDomainError {
                expected: SelectionKind::Faces,
                actual: SelectionKind::Vertices,
            }),
        }
    }

    /// Consumes the selection as edge-set or returns mismatch error.
    pub fn into_edges(self) -> Result<EdgeSet, SelectionDomainError> {
        match self {
            Self::Edges(edges) => Ok(edges),
            Self::Faces(_) => Err(SelectionDomainError {
                expected: SelectionKind::Edges,
                actual: SelectionKind::Faces,
            }),
            Self::Vertices(_) => Err(SelectionDomainError {
                expected: SelectionKind::Edges,
                actual: SelectionKind::Vertices,
            }),
        }
    }

    /// Consumes the selection as vertex-set or returns mismatch error.
    pub fn into_vertices(self) -> Result<VertexSet, SelectionDomainError> {
        match self {
            Self::Vertices(vertices) => Ok(vertices),
            Self::Faces(_) => Err(SelectionDomainError {
                expected: SelectionKind::Vertices,
                actual: SelectionKind::Faces,
            }),
            Self::Edges(_) => Err(SelectionDomainError {
                expected: SelectionKind::Vertices,
                actual: SelectionKind::Edges,
            }),
        }
    }
}

impl From<FaceSet> for Selection {
    fn from(value: FaceSet) -> Self {
        Self::Faces(value)
    }
}

impl From<EdgeSet> for Selection {
    fn from(value: EdgeSet) -> Self {
        Self::Edges(value)
    }
}

impl From<VertexSet> for Selection {
    fn from(value: VertexSet) -> Self {
        Self::Vertices(value)
    }
}

/// Canonicalizes a face selection in-place.
///
/// Returns `true` if the input was modified.
pub fn canonicalize_face_set(faces: &mut FaceSet) -> bool {
    canonicalize_sorted_unique(faces)
}

/// Canonicalizes a half-edge selection in-place.
///
/// Returns `true` if the input was modified.
pub fn canonicalize_edge_set(edges: &mut EdgeSet) -> bool {
    canonicalize_sorted_unique(edges)
}

/// Canonicalizes a vertex selection in-place.
///
/// Returns `true` if the input was modified.
pub fn canonicalize_vertex_set(vertices: &mut VertexSet) -> bool {
    canonicalize_sorted_unique(vertices)
}

fn canonicalize_sorted_unique<T: Ord>(values: &mut Vec<T>) -> bool {
    if values.windows(2).all(|pair| pair[0] < pair[1]) {
        return false;
    }
    values.sort_unstable();
    values.dedup();
    true
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use core::num::NonZeroU32;

    use exedra::{FaceId, HalfEdgeId, Id, VertexId};

    use super::{
        EdgeSet, FaceSet, Selection, SelectionDomainError, SelectionKind, VertexSet,
        canonicalize_edge_set, canonicalize_face_set, canonicalize_vertex_set,
    };

    #[test]
    fn canonicalize_face_set_sorts_and_dedups() {
        let f0 = FaceId::from(Id::new(0, NonZeroU32::MIN));
        let f1 = FaceId::from(Id::new(1, NonZeroU32::MIN));
        let f2 = FaceId::from(Id::new(2, NonZeroU32::MIN));
        let mut faces: FaceSet = vec![f2, f1, f2, f0];

        let changed = canonicalize_face_set(&mut faces);
        assert!(changed);
        assert_eq!(faces, vec![f0, f1, f2]);
    }

    #[test]
    fn canonicalize_face_set_reports_no_change_for_canonical_input() {
        let f0 = FaceId::from(Id::new(0, NonZeroU32::MIN));
        let f1 = FaceId::from(Id::new(1, NonZeroU32::MIN));
        let mut faces: FaceSet = vec![f0, f1];
        let changed = canonicalize_face_set(&mut faces);
        assert!(!changed);
    }

    #[test]
    fn canonicalize_face_set_empty_input_is_stable() {
        let mut faces: FaceSet = vec![];
        let changed = canonicalize_face_set(&mut faces);
        assert!(!changed);
        assert!(faces.is_empty());
    }

    #[test]
    fn canonicalize_face_set_reports_change_for_reordered_unique_input() {
        let f0 = FaceId::from(Id::new(0, NonZeroU32::MIN));
        let f1 = FaceId::from(Id::new(1, NonZeroU32::MIN));
        let mut faces: FaceSet = vec![f1, f0];

        let changed = canonicalize_face_set(&mut faces);
        assert!(changed);
        assert_eq!(faces, vec![f0, f1]);
    }

    #[test]
    fn canonicalize_edge_set_sorts_and_dedups() {
        let e0 = HalfEdgeId::from(Id::new(0, NonZeroU32::MIN));
        let e1 = HalfEdgeId::from(Id::new(1, NonZeroU32::MIN));
        let e2 = HalfEdgeId::from(Id::new(2, NonZeroU32::MIN));
        let mut edges: EdgeSet = vec![e2, e1, e2, e0];

        let changed = canonicalize_edge_set(&mut edges);
        assert!(changed);
        assert_eq!(edges, vec![e0, e1, e2]);
    }

    #[test]
    fn canonicalize_edge_set_reports_no_change_for_canonical_input() {
        let e0 = HalfEdgeId::from(Id::new(0, NonZeroU32::MIN));
        let e1 = HalfEdgeId::from(Id::new(1, NonZeroU32::MIN));
        let mut edges: EdgeSet = vec![e0, e1];
        let changed = canonicalize_edge_set(&mut edges);
        assert!(!changed);
    }

    #[test]
    fn canonicalize_vertex_set_sorts_and_dedups() {
        let v0 = VertexId::from(Id::new(0, NonZeroU32::MIN));
        let v1 = VertexId::from(Id::new(1, NonZeroU32::MIN));
        let v2 = VertexId::from(Id::new(2, NonZeroU32::MIN));
        let mut vertices: VertexSet = vec![v2, v1, v2, v0];

        let changed = canonicalize_vertex_set(&mut vertices);
        assert!(changed);
        assert_eq!(vertices, vec![v0, v1, v2]);
    }

    #[test]
    fn selection_kind_and_views_match_variant() {
        let f0 = FaceId::from(Id::new(0, NonZeroU32::MIN));
        let sel = Selection::from(vec![f0]);
        assert_eq!(sel.kind(), SelectionKind::Faces);
        assert!(sel.as_faces().is_some());
        assert!(sel.as_edges().is_none());
        assert!(sel.as_vertices().is_none());
    }

    #[test]
    fn selection_canonicalize_is_deterministic() {
        let f0 = FaceId::from(Id::new(0, NonZeroU32::MIN));
        let f1 = FaceId::from(Id::new(1, NonZeroU32::MIN));
        let mut sel = Selection::Faces(vec![f1, f0, f1]);
        assert!(sel.canonicalize());
        assert_eq!(sel.require_faces().expect("faces"), &vec![f0, f1]);
        assert!(!sel.canonicalize());
    }

    #[test]
    fn selection_require_mismatch_reports_expected_and_actual_domains() {
        let e0 = HalfEdgeId::from(Id::new(0, NonZeroU32::MIN));
        let sel = Selection::Edges(vec![e0]);
        let err = sel.require_faces().expect_err("mismatch expected");
        assert_eq!(
            err,
            SelectionDomainError {
                expected: SelectionKind::Faces,
                actual: SelectionKind::Edges,
            }
        );
    }

    #[test]
    fn selection_into_domain_conversions_work_and_mismatch() {
        let v0 = VertexId::from(Id::new(0, NonZeroU32::MIN));
        let sel = Selection::Vertices(vec![v0]);
        let vertices = sel.clone().into_vertices().expect("vertices expected");
        assert_eq!(vertices, vec![v0]);
        let err = sel.into_faces().expect_err("faces mismatch");
        assert_eq!(err.expected, SelectionKind::Faces);
        assert_eq!(err.actual, SelectionKind::Vertices);
    }
}
