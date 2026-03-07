// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::vec::Vec;

use crate::session::sort_dedup;
use crate::{ChangeSink, DeletePolicy, EditSession, FaceId, HalfEdgeId};

use super::DeleteEdgesError;

/// Deletes a canonical set of undirected edges.
pub fn delete_edges<S: ChangeSink>(
    session: &mut EditSession<'_, S>,
    edges: &[HalfEdgeId],
    policy: DeletePolicy,
) -> Result<(), DeleteEdgesError> {
    let mut faces = Vec::<FaceId>::new();
    let mut previous = None;

    for &half_edge in edges {
        if let Some(prev) = previous
            && prev >= half_edge
        {
            return Err(DeleteEdgesError::NonCanonicalEdgeSet);
        }
        previous = Some(half_edge);

        let twin = session
            .mesh()
            .twin(half_edge)
            .ok_or(DeleteEdgesError::HalfEdgeNotLive {
                half_edge: half_edge.index(),
            })?;
        if core::cmp::min(half_edge, twin) != half_edge {
            return Err(DeleteEdgesError::NonCanonicalEdgeSet);
        }

        let face = session
            .mesh()
            .face(half_edge)
            .ok_or(DeleteEdgesError::HalfEdgeNotLive {
                half_edge: half_edge.index(),
            })?;
        let twin_face = session
            .mesh()
            .face(twin)
            .ok_or(DeleteEdgesError::HalfEdgeNotLive {
                half_edge: half_edge.index(),
            })?;
        if face != FaceId::OUTSIDE {
            faces.push(face);
        }
        if twin_face != FaceId::OUTSIDE {
            faces.push(twin_face);
        }
        if face == FaceId::OUTSIDE && twin_face == FaceId::OUTSIDE {
            return Err(DeleteEdgesError::EdgeHasNoInteriorFace {
                half_edge: half_edge.index(),
            });
        }
    }

    sort_dedup(&mut faces);
    super::delete_faces(session, &faces, policy).map_err(DeleteEdgesError::FaceDeleteFailed)
}
