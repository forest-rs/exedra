// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Face subdivision into triangle fans, with explicit attribute propagation.

use alloc::vec::Vec;
use exedra_mesh::{
    ChangeSink, DeletePolicy, EditSession, FaceId, Mesh, MeshRevision, PropagatePolicy, VertexId,
    op,
};

use crate::patch::attrs::{
    SourceEdgeAttrs, propagate_edge_attrs_for_vertices, propagate_face_corner_uvs,
};
use crate::patch::source::{CaptureError, FaceInput, capture_inputs};

impl From<CaptureError> for PokeError {
    fn from(error: CaptureError) -> Self {
        match error {
            CaptureError::Selection(error) => Self::Selection(error),
            CaptureError::NumericLimit { face } => Self::NumericLimit { face },
        }
    }
}

/// Faces to subdivide. Input order and duplicate IDs are canonicalized.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PokeFacesParams {
    /// Source face IDs.
    pub faces: Vec<FaceId>,
}

/// Created topology, ordered by canonical source face and its corner order.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PokeFacesOutput {
    /// Canonical source faces replaced by triangle fans.
    pub source_faces: Vec<FaceId>,
    /// One center vertex per source face.
    pub center_vertices: Vec<VertexId>,
    /// Created triangle-fan faces.
    pub fan_faces: Vec<FaceId>,
}

/// Planned center of one triangle fan.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PokeCenter {
    /// Source face replaced by this fan.
    pub face: FaceId,
    /// Arithmetic mean of source vertex positions.
    pub position: [f32; 3],
    /// Mean corner UV, present only when every source corner has a UV.
    pub uv: Option<[f32; 2]>,
}

/// Typed refusal from face subdivision.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PokeError {
    /// Invalid or stale source face selection.
    Selection(exedra_mesh::SelectedFacePatchError),
    /// Missing or nonfinite source positions, or unrepresentable center/UV.
    NumericLimit {
        /// Source face that cannot be subdivided.
        face: FaceId,
    },
    /// The source revision or captured topology/geometry/attributes changed.
    StalePreparation,
    /// Source-face deletion was refused by the kernel.
    Delete(op::DeleteFacesError),
    /// Triangle insertion was refused after editing began.
    AddFace(op::AddFaceError),
    /// A newly created face could not receive its source region.
    SetRegion(op::SetFaceRegionError),
}

impl core::fmt::Display for PokeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Selection(error) => write!(f, "poke selection: {error}"),
            Self::NumericLimit { face } => write!(
                f,
                "poke center exceeds numeric limits on face {}",
                face.index()
            ),
            Self::StalePreparation => f.write_str("poke preparation no longer matches its source"),
            Self::Delete(error) => write!(f, "poke deletion: {error}"),
            Self::AddFace(error) => write!(f, "poke triangle insertion: {error}"),
            Self::SetRegion(error) => write!(f, "poke region transfer: {error}"),
        }
    }
}
impl core::error::Error for PokeError {}

/// Prepared fan geometry, checked independently of any operation runner.
///
/// Preparation retains the selected topology, positions and consumed attributes.
/// Applying to an equivalent clone is valid. A changed revision or changed input
/// dependency is refused before mutation, including edits in an unfinished scope.
#[derive(Clone, Debug)]
pub struct PokeFacesPlan {
    faces: Vec<FaceId>,
    inputs: Vec<FaceInput>,
    centers: Vec<PokeCenter>,
    revision: MeshRevision,
    canonicalized: bool,
}

impl PokeFacesPlan {
    /// Validates and prepares a triangle fan for each selected face.
    pub fn prepare(mesh: &Mesh, params: &PokeFacesParams) -> Result<Self, PokeError> {
        let mut faces = params.faces.clone();
        faces.sort_unstable();
        faces.dedup();
        let canonicalized = faces != params.faces;
        let inputs = capture_inputs(mesh, &faces)?;
        let mut centers = Vec::with_capacity(inputs.len());
        for input in &inputs {
            let mut position = [0.0; 3];
            let mut uv = Some([0.0; 2]);
            for corner in &input.corners {
                for (sum, value) in position.iter_mut().zip(corner.position) {
                    *sum += value;
                }
                uv = uv
                    .zip(corner.uv)
                    .map(|(sum, value)| [sum[0] + value[0], sum[1] + value[1]]);
            }
            let inverse = 1.0 / input.corners.len() as f32;
            position = position.map(|v| v * inverse);
            uv = uv.map(|v| v.map(|v| v * inverse));
            if !exedra_math::finite(position)
                || uv.is_some_and(|v| !v.iter().all(|v| v.is_finite()))
            {
                return Err(PokeError::NumericLimit { face: input.face });
            }
            centers.push(PokeCenter {
                face: input.face,
                position,
                uv,
            });
        }
        Ok(Self {
            faces,
            inputs,
            centers,
            revision: mesh.revision(),
            canonicalized,
        })
    }

    /// Canonical source faces, in output order.
    #[must_use]
    pub fn faces(&self) -> &[FaceId] {
        &self.faces
    }

    /// Prepared centers, in source-face order.
    #[must_use]
    pub fn centers(&self) -> &[PokeCenter] {
        &self.centers
    }

    /// Whether preparation changed input selection order or removed duplicates.
    #[must_use]
    pub const fn selections_canonicalized(&self) -> bool {
        self.canonicalized
    }

    /// Applies the prepared fans to an eager edit session.
    ///
    /// Regions and authored UVs follow the source face; outer edge tags follow
    /// `propagate.edge_attr`, and new radial edges start clear. Source corner
    /// normal overrides and custom layers are not transferred to new topology.
    /// Stale preparation is refused before editing. An insertion or attribute
    /// failure can leave partial edits in the caller's session; there is no
    /// rollback. Finish the caller's change sink even after such a failure.
    pub fn apply<S: ChangeSink>(
        &self,
        txn: &mut EditSession<'_, S>,
        propagate: &PropagatePolicy,
    ) -> Result<PokeFacesOutput, PokeError> {
        if txn.mesh().revision() != self.revision
            || capture_inputs(txn.mesh(), &self.faces).ok().as_ref() != Some(&self.inputs)
        {
            return Err(PokeError::StalePreparation);
        }
        op::delete_faces(txn, &self.faces, DeletePolicy::KeepIsolated)
            .map_err(PokeError::Delete)?;
        let mut center_vertices = Vec::with_capacity(self.inputs.len());
        let mut fan_faces = Vec::new();
        for (input, planned) in self.inputs.iter().zip(&self.centers) {
            let center = op::add_vertex(txn, planned.position);
            center_vertices.push(center);
            for i in 0..input.corners.len() {
                let current = &input.corners[i];
                let next = &input.corners[(i + 1) % input.corners.len()];
                let triangle = op::add_face(txn, &[current.vertex, next.vertex, center])
                    .map_err(PokeError::AddFace)?;
                op::set_face_region(txn, triangle, input.region).map_err(PokeError::SetRegion)?;
                // Edge tags belong to the source edge ending at the next corner.
                let source = SourceEdgeAttrs {
                    seam: next.seam,
                    sharpness: next.sharpness,
                };
                propagate_edge_attrs_for_vertices(
                    txn,
                    triangle,
                    current.vertex,
                    next.vertex,
                    source,
                    propagate,
                );
                propagate_edge_attrs_for_vertices(
                    txn,
                    triangle,
                    next.vertex,
                    center,
                    SourceEdgeAttrs::default(),
                    propagate,
                );
                propagate_edge_attrs_for_vertices(
                    txn,
                    triangle,
                    center,
                    current.vertex,
                    SourceEdgeAttrs::default(),
                    propagate,
                );
                propagate_face_corner_uvs(
                    txn,
                    triangle,
                    &[
                        (current.vertex, current.uv),
                        (next.vertex, next.uv),
                        (center, planned.uv),
                    ],
                );
                fan_faces.push(triangle);
            }
        }
        Ok(PokeFacesOutput {
            source_faces: self.faces.clone(),
            center_vertices,
            fan_faces,
        })
    }
}

/// Prepares and immediately subdivides selected faces in the caller's session.
///
/// See [`PokeFacesPlan::apply`] for propagation and eager failure semantics.
pub fn poke_faces<S: ChangeSink>(
    txn: &mut EditSession<'_, S>,
    params: &PokeFacesParams,
    propagate: &PropagatePolicy,
) -> Result<PokeFacesOutput, PokeError> {
    PokeFacesPlan::prepare(txn.mesh(), params)?.apply(txn, propagate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use exedra_mesh::{ChangeSetBuilder, EdgeAttrPropagation, attr};

    fn quad() -> (Mesh, FaceId) {
        let mesh = Mesh::from_polygons(
            &[
                [0.0, 0.0, 0.0],
                [2.0, 0.0, 0.0],
                [2.0, 2.0, 0.0],
                [0.0, 2.0, 0.0],
            ],
            &[&[0, 1, 2, 3]],
        )
        .unwrap();
        let face = mesh.faces().next().unwrap();
        (mesh, face)
    }

    #[test]
    fn direct_fans_preserve_regions_uvs_and_boundary_tags() {
        let (mut mesh, face) = quad();
        let corners: Vec<_> = mesh.face_loop(face).collect();
        {
            let mut edit = mesh.edit();
            op::set_face_region(&mut edit, face, 17).unwrap();
            for &corner in &corners {
                let vertex = edit.mesh().to_vertex(corner).unwrap();
                let p = *edit.mesh().vertex_position(vertex).unwrap();
                op::set_corner_uv(&mut edit, corner, [p[0], p[1]]).unwrap();
                op::set_edge_seam(&mut edit, corner, true).unwrap();
                op::set_edge_sharpness(&mut edit, corner, 2.0).unwrap();
            }
        }
        let params = PokeFacesParams {
            faces: vec![face, face],
        };
        let plan = PokeFacesPlan::prepare(&mesh, &params).unwrap();
        assert!(plan.selections_canonicalized());
        assert_eq!(plan.centers()[0].position, [1.0, 1.0, 0.0]);
        assert_eq!(plan.centers()[0].uv, Some([1.0, 1.0]));
        // Applying to an equivalent clone is explicitly supported.
        let mut clone = mesh.clone();
        let mut edit = clone.edit_with(ChangeSetBuilder::new());
        let propagate = PropagatePolicy {
            edge_attr: EdgeAttrPropagation::Inherit,
            ..Default::default()
        };
        let result = plan.apply(&mut edit, &propagate).unwrap();
        let changes = edit.finish();
        assert!(changes.dirty.has_dirty_faces());
        assert_eq!(result.fan_faces.len(), 4);
        assert!(clone.validate_deep().is_empty());
        let center = result.center_vertices[0];
        for face in result.fan_faces {
            assert_eq!(
                clone
                    .attrs()
                    .dense(attr::FACE_REGION)
                    .unwrap()
                    .get(face.as_id()),
                Some(&17)
            );
            for corner in clone.face_loop(face) {
                let from = clone.from_vertex(corner).unwrap();
                let to = clone.to_vertex(corner).unwrap();
                let p = *clone.vertex_position(to).unwrap();
                assert_eq!(
                    clone
                        .attrs()
                        .sparse(attr::CORNER_UV)
                        .unwrap()
                        .get(corner.as_id()),
                    Some(&[p[0], p[1]])
                );
                let radial = from == center || to == center;
                assert_eq!(clone.edge_seam(corner), Some(!radial));
                assert_eq!(
                    clone.edge_sharpness(corner),
                    Some(if radial { 0.0 } else { 2.0 })
                );
            }
        }
        assert_eq!(mesh.faces().count(), 1, "source clone was not edited");
    }

    #[test]
    fn preparation_rejects_unfinished_geometry_and_attribute_edits() {
        for edit_uv in [false, true] {
            let (mut mesh, face) = quad();
            let corner = mesh.face_loop(face).next().unwrap();
            let vertex = mesh.to_vertex(corner).unwrap();
            let plan =
                PokeFacesPlan::prepare(&mesh, &PokeFacesParams { faces: vec![face] }).unwrap();
            let revision = mesh.revision();
            let mut edit = mesh.edit();
            if edit_uv {
                op::set_corner_uv(&mut edit, corner, [9.0, 4.0]).unwrap();
            } else {
                op::set_vertex_position(&mut edit, vertex, [3.0, 2.0, 0.0]).unwrap();
            }
            assert_eq!(edit.mesh().revision(), revision);
            assert_eq!(
                plan.apply(&mut edit, &PropagatePolicy::default()),
                Err(PokeError::StalePreparation)
            );
            assert_eq!(
                edit.mesh().faces().collect::<Vec<_>>(),
                vec![face],
                "stale refusal precedes mutation"
            );
        }
    }

    #[test]
    fn raw_calls_refuse_unrepresentable_centers_before_mutation() {
        let (mut mesh, face) = quad();
        let vertices: Vec<_> = mesh.vertices().collect();
        let mut edit = mesh.edit();
        for vertex in vertices {
            op::set_vertex_position(&mut edit, vertex, [f32::MAX, 0.0, 0.0]).unwrap();
        }
        assert_eq!(
            poke_faces(
                &mut edit,
                &PokeFacesParams { faces: vec![face] },
                &PropagatePolicy::default()
            ),
            Err(PokeError::NumericLimit { face })
        );
        assert_eq!(edit.mesh().faces().collect::<Vec<_>>(), vec![face]);
    }
}
