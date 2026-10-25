// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Feature labels resolved through geometry-local topology correspondence.

use super::{Boundary, BoundarySegment, Edge, EdgeId, EdgeRole, Extrusion, Face, FaceId, FaceRole};
use exedra_profile::SegTag;

impl Extrusion {
    /// Faces paired with their IDs in this solid, in deterministic table order.
    pub fn indexed_faces(&self) -> impl Iterator<Item = (FaceId, &Face)> + '_ {
        self.faces
            .iter()
            .enumerate()
            .map(|(i, face)| (FaceId(crate::usize_to_u32(i)), face))
    }

    /// Edges paired with their IDs in this solid, in deterministic table order.
    pub fn indexed_edges(&self) -> impl Iterator<Item = (EdgeId, &Edge)> + '_ {
        self.edges
            .iter()
            .enumerate()
            .map(|(i, edge)| (EdgeId(crate::usize_to_u32(i)), edge))
    }

    /// The face's caller feature label, if any.
    ///
    /// Pocket labels cover their floors and walls. An unlabelled pocket's
    /// walls, and the profile's walls, retain their segment labels. Caps have
    /// no feature label. Labels may repeat; they are not unique topology IDs.
    /// Returns `None` for an out-of-range face ID as well as an unlabelled face.
    #[must_use]
    pub fn face_tag(&self, face: FaceId) -> Option<SegTag> {
        match self.faces.get(face.index())?.role {
            FaceRole::Cap(_) => None,
            FaceRole::Floor { pocket } => self.pockets[pocket].tag,
            FaceRole::Wall { segment, .. } => self.boundary_tag(segment),
        }
    }

    /// The edge's caller feature label, if any.
    ///
    /// A pocket label covers both rim rings and its vertical seams. Otherwise
    /// a rim retains its segment label; a seam is labelled only when its two
    /// neighbouring segments have the same label. An edge ID is local to this
    /// solid; an out-of-range ID returns `None`.
    #[must_use]
    pub fn edge_tag(&self, edge: EdgeId) -> Option<SegTag> {
        match self.edges.get(edge.index())?.role {
            EdgeRole::Boundary { segment, .. } => self.boundary_tag(segment),
            EdgeRole::Seam { segment } => {
                if let Boundary::Pocket(pocket) = segment.boundary
                    && let Some(tag) = self.pockets[pocket].tag
                {
                    return Some(tag);
                }
                let segments = self.boundary_loop(segment.boundary).segments();
                let first = segments[segment.segment].tag;
                let next = segments[(segment.segment + 1) % segments.len()].tag;
                (first == next).then_some(first).flatten()
            }
        }
    }

    /// Every face carrying `tag`, including a tagged pocket's floor and walls.
    /// Repeated labels deliberately return all matching features.
    pub fn faces_tagged(&self, tag: SegTag) -> impl Iterator<Item = (FaceId, &Face)> + '_ {
        self.indexed_faces()
            .filter(move |(id, _)| self.face_tag(*id) == Some(tag))
    }

    /// Every edge carrying `tag`, including both rims of a tagged pocket.
    /// Mesh correspondence can map these native edges to emitted vertex chains.
    pub fn edges_tagged(&self, tag: SegTag) -> impl Iterator<Item = (EdgeId, &Edge)> + '_ {
        self.indexed_edges()
            .filter(move |(id, _)| self.edge_tag(*id) == Some(tag))
    }

    fn boundary_tag(&self, segment: BoundarySegment) -> Option<SegTag> {
        let feature = match segment.boundary {
            Boundary::Pocket(pocket) => self.pockets[pocket].tag,
            Boundary::Profile(_) => None,
        };
        feature.or(self.boundary_loop(segment.boundary).segments()[segment.segment].tag)
    }
}
