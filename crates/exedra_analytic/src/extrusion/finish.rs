// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exact finishing of full-length profile seams through shared corner geometry.

use super::{Boundary, Edge, EdgeId, EdgeRole, Extrusion, ExtrusionError, FaceId, pocket};
use alloc::vec::Vec;
use exedra_curves::profile::{
    CornerError, CornerSegment, CornerSource, CornerTreatment, LoopRef, ProfilePolicy, SegTag,
    SegmentRef,
};

/// One exact transition wall and the original selected edge that generated it.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct AxialTransition {
    /// Edge ID in the original extrusion.
    pub source_edge: EdgeId,
    /// Face ID in the returned extrusion.
    pub wall: FaceId,
}
/// Independent finished solid and complete profile-segment correspondence.
#[derive(Clone, Debug, PartialEq)]
pub struct AxialFinish {
    /// Retained native result. Select topology again on this solid after editing.
    pub solid: Extrusion,
    /// Every output profile segment's original interval or generating corner.
    pub profile_segments: Vec<CornerSegment>,
    /// Generated planar chamfer or cylindrical fillet walls, in profile order.
    pub transitions: Vec<AxialTransition>,
}
/// A selected native axial-edge finish could not be constructed.
#[derive(Clone, Debug, PartialEq)]
pub enum AxialFinishError {
    /// The ID is not present in the original solid.
    UnknownEdge {
        /// Requested edge.
        edge: EdgeId,
    },
    /// Only full-length profile seams are supported; cap rims and pocket seams
    /// require a different operation.
    UnsupportedEdge {
        /// Requested edge.
        edge: EdgeId,
        /// Native source role of that edge.
        role: EdgeRole,
    },
    /// One edge was selected more than once.
    RepeatedEdge {
        /// Duplicate selection.
        edge: EdgeId,
    },
    /// Shared profile corner treatment failed, retaining source corner witnesses.
    Profile(CornerError),
    /// Existing pockets at one end fail the changed profile's arrangement policy.
    PocketArrangement {
        /// End being checked.
        end: super::End,
        /// Original pocket indices in combined hole order, after profile holes.
        pockets: Vec<usize>,
        /// Combined arrangement diagnosis. Hole references index profile holes
        /// first, followed by this explicit pocket table.
        error: exedra_curves::profile::ProfileError,
    },
    /// Rebuilding or revalidating existing pockets failed.
    Extrusion(ExtrusionError),
}
impl core::fmt::Display for AxialFinishError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnknownEdge { edge } => write!(f, "axial finish edge {edge:?} does not exist"),
            Self::UnsupportedEdge { edge, role } => write!(
                f,
                "axial finish edge {edge:?} has unsupported role {role:?}"
            ),
            Self::RepeatedEdge { edge } => {
                write!(f, "axial finish edge {edge:?} was selected twice")
            }
            Self::Profile(error) => write!(f, "axial finish profile: {error}"),
            Self::PocketArrangement {
                end,
                pockets,
                error,
            } => write!(f, "axial finish pockets {pockets:?} at {end:?}: {error}"),
            Self::Extrusion(error) => write!(f, "axial finish extrusion: {error}"),
        }
    }
}
impl core::error::Error for AxialFinishError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Profile(e) => Some(e),
            Self::Extrusion(e) => Some(e),
            Self::PocketArrangement { error, .. } => Some(error),
            _ => None,
        }
    }
}
fn loop_index(r: LoopRef) -> usize {
    match r {
        LoopRef::Outer => 0,
        LoopRef::Hole(i) => i + 1,
    }
}

impl Extrusion {
    /// Full-length native profile seams, excluding cap rims and pocket edges.
    /// Visits only the profile bands, independently of the number of pockets.
    pub fn profile_axial_edges(&self) -> impl Iterator<Item = (EdgeId, &Edge)> + '_ {
        self.bands[..self.profile.holes().len() + 1]
            .iter()
            .flat_map(move |band| {
                (0..self.boundary_loop(band.boundary).segments().len()).map(move |i| {
                    let id = EdgeId(crate::usize_to_u32(band.seams + i));
                    (id, &self.edges()[id.index()])
                })
            })
    }
    /// Full-length profile seams whose two neighboring pieces share `tag`.
    pub fn profile_axial_edges_tagged(
        &self,
        tag: SegTag,
    ) -> impl Iterator<Item = (EdgeId, &Edge)> + '_ {
        self.profile_axial_edges()
            .filter(move |(id, _)| self.edge_tag(*id) == Some(tag))
    }
    /// Adds exact fillets or chamfers along selected full-length profile seams.
    ///
    /// Each seam must meet two straight profile walls. Shared profile corner
    /// treatment owns the exact line trims, tangent arc/chamfer and material
    /// sense, including holes. Unselected arcs stay exact. `treatment.tag`
    /// labels generated walls, and the report maps them to the original edges.
    ///
    /// Existing pockets are revalidated against the changed profile under
    /// `policy`; native tables are built once after all checks pass. The original
    /// solid stays unchanged. Frame, length and pocket order/labels are retained.
    /// Returned IDs belong to the new solid; reselect after another edit.
    ///
    /// ```
    /// use exedra_analytic::extrusion::Extrusion;
    /// use exedra_math::Placement3;
    /// use exedra_curves::profile::{CornerTreatment, Loop, Profile, ProfilePolicy, SegTag, Segment};
    /// let policy = ProfilePolicy::new(1e-9);
    /// let outline = Loop::new([
    ///     Segment::line([2.0, -2.0]), Segment::line([2.0, 2.0]),
    ///     Segment::line([-2.0, 2.0]), Segment::line([-2.0, -2.0]),
    /// ].map(|s| s.tagged(SegTag(1))).to_vec())?;
    /// let solid = Extrusion::new(Profile::new(outline, vec![], &policy)?, Placement3::IDENTITY, 3.0)?;
    /// let finished = solid.with_axial_corners(
    ///     solid.profile_axial_edges_tagged(SegTag(1)).map(|(id, _)| id),
    ///     CornerTreatment::fillet(0.5).tagged(SegTag(2)), &policy,
    /// )?;
    /// assert_eq!(finished.solid.faces_tagged(SegTag(2)).count(), 4);
    /// # Ok::<(), Box<dyn core::error::Error>>(())
    /// ```
    ///
    /// # Errors
    /// Refuses unknown, repeated, rim or pocket-edge selections; preserves the
    /// shared profile treatment's dimension, junction, trim and clearance errors,
    /// and any existing pocket's revalidation diagnosis.
    pub fn with_axial_corners(
        &self,
        selected: impl IntoIterator<Item = EdgeId>,
        treatment: CornerTreatment,
        policy: &ProfilePolicy,
    ) -> Result<AxialFinish, AxialFinishError> {
        let maximum = self.profile_axial_edges().count();
        let mut selection = Vec::new();
        for edge in selected {
            let record = self
                .body
                .edge(edge)
                .ok_or(AxialFinishError::UnknownEdge { edge })?;
            let EdgeRole::Seam {
                segment:
                    super::BoundarySegment {
                        boundary: Boundary::Profile(loop_ref),
                        segment,
                    },
            } = record.role
            else {
                return Err(AxialFinishError::UnsupportedEdge {
                    edge,
                    role: record.role,
                });
            };
            selection.push((edge, SegmentRef { loop_ref, segment }));
            // At most N distinct full-length seams exist. N+1 valid IDs must
            // repeat, so even an unbounded iterator gets a bounded refusal.
            if selection.len() > maximum {
                break;
            }
        }
        selection.sort_unstable_by_key(|s| s.0);
        if let Some(pair) = selection.windows(2).find(|w| w[0].0 == w[1].0) {
            return Err(AxialFinishError::RepeatedEdge { edge: pair[0].0 });
        }
        let treated = self
            .profile
            .with_corners(selection.iter().map(|s| s.1), treatment, policy)
            .map_err(AxialFinishError::Profile)?;
        let mut webs = Vec::new();
        for (index, p) in self.pockets.iter().enumerate() {
            pocket::checked_depth(self.length, p, index, policy)
                .map_err(AxialFinishError::Extrusion)?;
        }
        // Check each complete end once. Revalidating every growing prefix
        // repeats whole arrangements and makes a local finish scale cubically
        // with an unchanged pocket set.
        for end in [super::End::Start, super::End::End] {
            let indices: Vec<_> = self
                .pockets
                .iter()
                .enumerate()
                .filter(|(_, p)| p.end == end)
                .map(|(i, _)| i)
                .collect();
            if indices.is_empty() {
                continue;
            }
            let mut holes = treated.profile.holes().to_vec();
            holes.extend(indices.iter().map(|&i| self.pockets[i].outline.clone()));
            exedra_curves::profile::Profile::new(treated.profile.outer().clone(), holes, policy)
                .map_err(|error| AxialFinishError::PocketArrangement {
                    end,
                    pockets: indices,
                    error,
                })?;
        }
        for (index, p) in self.pockets.iter().enumerate() {
            for (other, existing) in self.pockets[..index].iter().enumerate() {
                webs.extend(
                    pocket::checked_pair(
                        &treated.profile,
                        self.length,
                        [p, existing],
                        [index, other],
                        policy,
                    )
                    .map_err(AxialFinishError::Extrusion)?,
                );
            }
        }
        let solid = Self::build(
            treated.profile,
            self.frame,
            self.length,
            self.pockets.clone(),
            webs,
        )
        .map_err(AxialFinishError::Extrusion)?;
        let mut transitions = Vec::with_capacity(selection.len());
        for output in &treated.segments {
            if let CornerSource::Transition { corner } = output.source {
                let source_edge = EdgeId(crate::usize_to_u32(
                    self.bands[loop_index(corner.loop_ref)].seams + corner.segment,
                ));
                let wall = FaceId(crate::usize_to_u32(
                    solid.bands[loop_index(output.segment.loop_ref)].walls + output.segment.segment,
                ));
                transitions.push(AxialTransition { source_edge, wall });
            }
        }
        Ok(AxialFinish {
            solid,
            profile_segments: treated.segments,
            transitions,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        End,
        extrusion::{Pocket, TessellationPolicy},
        geometry::Surface,
    };
    use alloc::vec;
    use exedra_curves::profile::{Loop, Profile, Segment};
    use exedra_math::Placement3;
    const POLICY: ProfilePolicy = ProfilePolicy::new(1e-9);
    const BODY: SegTag = SegTag(1);
    const FINISH: SegTag = SegTag(2);
    fn rectangle(x: f64, y: f64, tag: SegTag) -> Loop {
        Loop::new(vec![
            Segment::line([x, -y]).tagged(tag),
            Segment::line([x, y]).tagged(tag),
            Segment::line([-x, y]).tagged(tag),
            Segment::line([-x, -y]).tagged(tag),
        ])
        .unwrap()
    }
    fn block() -> Extrusion {
        Extrusion::new(
            Profile::new(rectangle(10.0, 8.0, BODY), vec![], &POLICY).unwrap(),
            Placement3::IDENTITY,
            10.0,
        )
        .unwrap()
    }
    fn disk(x: f64, y: f64, r: f64) -> Loop {
        Loop::new(vec![
            Segment::arc([x - r, y], 1.0),
            Segment::arc([x + r, y], 1.0),
        ])
        .unwrap()
    }
    #[test]
    fn selected_native_edges_generate_exact_tagged_chamfer_and_cylinder_walls() {
        let original = block();
        for treatment in [CornerTreatment::chamfer(2.0), CornerTreatment::fillet(2.0)] {
            let output = original
                .with_axial_corners(
                    original.profile_axial_edges_tagged(BODY).map(|(id, _)| id),
                    treatment.tagged(FINISH),
                    &POLICY,
                )
                .unwrap();
            assert_eq!(output.transitions.len(), 4);
            assert_eq!(output.profile_segments.len(), 8);
            assert_eq!(output.solid.faces_tagged(FINISH).count(), 4);
            let area = match treatment.shape {
                exedra_curves::profile::CornerShape::Chamfer { .. } => 312.0,
                exedra_curves::profile::CornerShape::Fillet { .. } => {
                    320.0 - 16.0 + 4.0 * core::f64::consts::PI
                }
            };
            assert!((output.solid.volume() - 10.0 * area).abs() < 1e-10);
            for transition in &output.transitions {
                assert!(matches!(
                    original.edges()[transition.source_edge.index()].role,
                    EdgeRole::Seam { .. }
                ));
                assert_eq!(output.solid.face_tag(transition.wall), Some(FINISH));
                match treatment.shape {
                    exedra_curves::profile::CornerShape::Chamfer { .. } => assert!(matches!(
                        output.solid.faces()[transition.wall.index()].surface,
                        Surface::Plane(_)
                    )),
                    exedra_curves::profile::CornerShape::Fillet { .. } => assert!(matches!(
                        output.solid.faces()[transition.wall.index()].surface,
                        Surface::Cylinder(_)
                    )),
                }
            }
            let mesh = output
                .solid
                .tessellate(&TessellationPolicy::default())
                .unwrap();
            assert!(mesh.report.within_tolerance());
            assert!(mesh.mesh.validate_deep().is_empty());
            assert!(mesh.mesh.boundary_loops().unwrap().is_empty());
        }
        assert_eq!(original.volume(), 3200.0);
        assert_eq!(original.faces().len(), 6);
    }
    #[test]
    fn hole_corners_preserve_material_sense_and_original_edge_correspondence() {
        const HOLE: SegTag = SegTag(3);
        let original = Extrusion::new(
            Profile::new(
                rectangle(10.0, 8.0, BODY),
                vec![rectangle(2.0, 2.0, HOLE).reversed()],
                &POLICY,
            )
            .unwrap(),
            Placement3::IDENTITY,
            10.0,
        )
        .unwrap();
        let output = original
            .with_axial_corners(
                original.profile_axial_edges_tagged(HOLE).map(|(id, _)| id),
                CornerTreatment::chamfer(0.5).tagged(FINISH),
                &POLICY,
            )
            .unwrap();
        assert_eq!(output.transitions.len(), 4);
        assert!((output.solid.volume() - original.volume() - 5.0).abs() < 1e-10);
        for t in &output.transitions {
            assert!(matches!(
                original.edges()[t.source_edge.index()].role,
                EdgeRole::Seam {
                    segment: super::super::BoundarySegment {
                        boundary: Boundary::Profile(LoopRef::Hole(0)),
                        ..
                    }
                }
            ));
        }
        assert!(output.solid.is_closed());
    }
    #[test]
    fn existing_pockets_are_revalidated_before_building_one_finished_boundary() {
        let original = block()
            .with_pocket(
                Pocket::from_region(&disk(0.0, 0.0, 2.0), End::End, 2.0).tagged(SegTag(8)),
                &POLICY,
            )
            .unwrap()
            .with_pocket(
                Pocket::from_region(&disk(0.0, 0.0, 2.0), End::Start, 1.0).tagged(SegTag(9)),
                &POLICY,
            )
            .unwrap();
        let output = original
            .with_axial_corners(
                original.profile_axial_edges().map(|(id, _)| id),
                CornerTreatment::fillet(1.0),
                &POLICY,
            )
            .unwrap();
        assert_eq!(output.solid.pockets(), original.pockets());
        assert_eq!(output.solid.webs, original.webs);
        assert_eq!(output.solid.frame(), original.frame());
        let corner_pocket = block()
            .with_pocket(
                Pocket::from_region(&disk(8.5, 6.5, 0.5), End::End, 2.0),
                &POLICY,
            )
            .unwrap()
            .with_pocket(
                Pocket::from_region(&disk(-5.0, 0.0, 0.5), End::Start, 8.0),
                &POLICY,
            )
            .unwrap();
        let before = corner_pocket.clone();
        assert!(matches!(
            corner_pocket.with_axial_corners(
                corner_pocket.profile_axial_edges().map(|(id, _)| id),
                CornerTreatment::chamfer(3.0),
                &POLICY
            ),
            Err(AxialFinishError::PocketArrangement { .. })
        ));
        assert_eq!(corner_pocket, before);
    }
    #[test]
    fn unsupported_edges_repetitions_and_overlapping_trims_are_explicit_refusals() {
        let solid = block();
        let edge = solid.profile_axial_edges().next().unwrap().0;
        assert!(matches!(
            solid.with_axial_corners(
                core::iter::repeat(edge),
                CornerTreatment::fillet(1.0),
                &POLICY
            ),
            Err(AxialFinishError::RepeatedEdge { .. })
        ));
        assert!(matches!(
            solid.with_axial_corners([EdgeId(u32::MAX)], CornerTreatment::fillet(1.0), &POLICY),
            Err(AxialFinishError::UnknownEdge { .. })
        ));
        assert!(matches!(
            solid.with_axial_corners([EdgeId(0)], CornerTreatment::fillet(1.0), &POLICY),
            Err(AxialFinishError::UnsupportedEdge { .. })
        ));
        assert!(matches!(
            solid.with_axial_corners(
                solid.profile_axial_edges().map(|(id, _)| id),
                CornerTreatment::chamfer(20.0),
                &POLICY
            ),
            Err(AxialFinishError::Profile(
                CornerError::OverlappingTrims { .. }
            ))
        ));
        let pocketed = solid
            .with_pocket(
                Pocket::from_region(&disk(0.0, 0.0, 2.0), End::End, 1.0),
                &POLICY,
            )
            .unwrap();
        let pocket_edge = pocketed
            .indexed_edges()
            .find(|(_, e)| {
                matches!(
                    e.role,
                    EdgeRole::Seam {
                        segment: super::super::BoundarySegment {
                            boundary: Boundary::Pocket(_),
                            ..
                        }
                    }
                )
            })
            .unwrap()
            .0;
        assert!(matches!(
            pocketed.with_axial_corners([pocket_edge], CornerTreatment::fillet(0.1), &POLICY),
            Err(AxialFinishError::UnsupportedEdge { .. })
        ));
        let outline = Loop::new(vec![
            Segment::arc([0.0, 1.0], libm::tan(core::f64::consts::FRAC_PI_8)),
            Segment::line([0.0, 0.0]),
            Segment::line([1.0, 0.0]),
        ])
        .unwrap();
        let curved = Extrusion::new(
            Profile::new(outline, vec![], &POLICY).unwrap(),
            Placement3::IDENTITY,
            1.0,
        )
        .unwrap();
        let edge = curved.profile_axial_edges().nth(2).unwrap().0;
        assert!(matches!(
            curved.with_axial_corners([edge], CornerTreatment::fillet(0.1), &POLICY),
            Err(AxialFinishError::Profile(
                CornerError::UnsupportedCorner { .. }
            ))
        ));
    }
    #[test]
    fn a_stronger_finish_policy_rechecks_pocket_clearance_and_opposite_webs() {
        let policy = ProfilePolicy::new(1.0);
        let same_end = block()
            .with_pocket(
                Pocket::from_region(&disk(-1.1, 0.0, 1.0), End::End, 2.0),
                &POLICY,
            )
            .unwrap()
            .with_pocket(
                Pocket::from_region(&disk(1.1, 0.0, 1.0), End::End, 2.0),
                &POLICY,
            )
            .unwrap();
        let error = same_end
            .with_axial_corners(
                same_end.profile_axial_edges().map(|(id, _)| id),
                CornerTreatment::chamfer(1.0),
                &policy,
            )
            .unwrap_err();
        assert!(
            matches!(error,AxialFinishError::PocketArrangement {end:End::End,pockets,error:exedra_curves::profile::ProfileError::Contact {..}} if pockets==vec![0,1])
        );
        let opposite = block()
            .with_pocket(
                Pocket::from_region(&disk(0.0, 0.0, 2.0), End::End, 5.0),
                &POLICY,
            )
            .unwrap()
            .with_pocket(
                Pocket::from_region(&disk(0.0, 0.0, 2.0), End::Start, 4.5),
                &POLICY,
            )
            .unwrap();
        assert!(matches!(
            opposite.with_axial_corners(
                opposite.profile_axial_edges().map(|(id, _)| id),
                CornerTreatment::chamfer(1.0),
                &policy
            ),
            Err(AxialFinishError::Extrusion(ExtrusionError::Pocket {
                pocket: 1,
                error: super::super::PocketError::BreaksThrough { other: 0, .. }
            }))
        ));
    }
}
