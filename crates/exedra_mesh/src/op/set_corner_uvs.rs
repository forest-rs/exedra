// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::uv_mapping::UvSet;
use crate::{ChangeSink, CornerId, EditSession};

/// Refusal from a sorted UV batch. All input checks precede mutation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetCornerUvsError {
    /// Corner slots must be strictly ascending and unique.
    UnsortedOrDuplicate,
    /// A corner handle is stale or missing.
    CornerNotLive {
        /// Rejected corner.
        corner: CornerId,
    },
    /// The channel name has incompatible storage or value type.
    IncompatibleLayer,
}
impl core::fmt::Display for SetCornerUvsError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnsortedOrDuplicate => {
                f.write_str("UV batch corner slots must be sorted and unique")
            }
            Self::CornerNotLive { corner } => write!(f, "UV batch corner is not live: {corner:?}"),
            Self::IncompatibleLayer => {
                f.write_str("UV channel has incompatible storage or value type")
            }
        }
    }
}
impl core::error::Error for SetCornerUvsError {}

/// Writes a batch ordered by strictly increasing corner slot index.
///
/// Validates every handle and the target layer before mutation. Sparse values
/// merge in linear time with one capacity growth at most; dense secondary UVs
/// write directly. Unmentioned values survive, and every supplied corner is
/// marked dirty. Values are stored verbatim, like `set_corner_uv`.
/// A newly created secondary channel declares interpolating propagation;
/// existing channel rules are preserved. Empty batches create no layer.
pub fn set_corner_uvs<S: ChangeSink>(
    session: &mut EditSession<'_, S>,
    set: UvSet,
    values: &[(CornerId, [f32; 2])],
) -> Result<(), SetCornerUvsError> {
    if values
        .windows(2)
        .any(|pair| pair[0].0.index() >= pair[1].0.index())
    {
        return Err(SetCornerUvsError::UnsortedOrDuplicate);
    }
    for &(corner, _) in values {
        if session.mesh().to_vertex(corner).is_none() {
            return Err(SetCornerUvsError::CornerNotLive { corner });
        }
    }
    let attrs = session.mesh().attrs();
    if attrs
        .layer_kind(set.key().domain(), set.key().name())
        .is_some()
        && attrs.sparse(set.key()).is_none()
        && (set == UvSet::Primary || attrs.dense(set.key()).is_none())
    {
        return Err(SetCornerUvsError::IncompatibleLayer);
    }
    session.set_corner_uvs_impl(set, values);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attributes::Propagation;
    use crate::{ChangeSetBuilder, Id, attr, op};
    use core::num::NonZeroU32;
    fn quad_mesh() -> crate::Mesh {
        crate::Mesh::from_polygons(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            &[&[0, 1, 2, 3]],
        )
        .unwrap()
    }

    #[test]
    fn batch_merge_retains_gaps_updates_values_and_declares_secondary_interpolation() {
        let mut mesh = quad_mesh();
        let mut corners: alloc::vec::Vec<_> =
            mesh.face_loop(mesh.faces().next().unwrap()).collect();
        corners.sort_unstable_by_key(|corner| corner.index());
        let mut edit = mesh.edit();
        op::set_corner_uv(&mut edit, corners[0], [1.0; 2]).unwrap();
        op::set_corner_uv(&mut edit, corners[2], [3.0; 2]).unwrap();
        set_corner_uvs(
            &mut edit,
            UvSet::Primary,
            &[(corners[1], [2.0; 2]), (corners[2], [4.0; 2])],
        )
        .unwrap();
        assert_eq!(edit.corner_uv(corners[0]), Some([1.0; 2]));
        assert_eq!(edit.corner_uv(corners[1]), Some([2.0; 2]));
        assert_eq!(edit.corner_uv(corners[2]), Some([4.0; 2]));
        assert_eq!(edit.corner_uv(corners[3]), None);
        set_corner_uvs(&mut edit, UvSet::Secondary, &[(corners[1], [2.0; 2])]).unwrap();
        let _: () = edit.finish();
        assert_eq!(
            mesh.attrs()
                .propagation(attr::CORNER_UV1.domain(), attr::CORNER_UV1.name()),
            Some(Propagation::Interpolate)
        );
        mesh.set_layer_propagation(attr::CORNER_UV1, Propagation::Copy)
            .unwrap();
        set_corner_uvs(
            &mut mesh.edit(),
            UvSet::Secondary,
            &[(corners[2], [5.0; 2])],
        )
        .unwrap();
        assert_eq!(
            mesh.attrs()
                .propagation(attr::CORNER_UV1.domain(), attr::CORNER_UV1.name()),
            Some(Propagation::Copy)
        );
    }

    #[test]
    fn invalid_batches_leave_mesh_and_sink_untouched() {
        let mut mesh = quad_mesh();
        let corner = mesh.face_loop(mesh.faces().next().unwrap()).next().unwrap();
        let stale = CornerId::from(Id::new(999, NonZeroU32::MIN));
        for values in [
            alloc::vec![(corner, [0.0; 2]), (corner, [1.0; 2])],
            alloc::vec![(corner, [0.0; 2]), (stale, [1.0; 2])],
        ] {
            let mut edit = mesh.edit_with(ChangeSetBuilder::new());
            assert!(set_corner_uvs(&mut edit, UvSet::Primary, &values).is_err());
            assert_eq!(edit.corner_uv(corner), None);
            assert!(!edit.finish().dirty.has_dirty_corners());
        }
    }
}
