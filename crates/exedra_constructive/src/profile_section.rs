// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Convert mesh boundaries into reusable constructive profiles.
//!
//! The authoritative pre-mesh profile model is
//! [`Profile2`]. This module converts *mesh*
//! loops — face-loop boundaries and edge loops picked interactively — into
//! `Profile2` polyline sections, so interactive loft/sweep tooling and the
//! constructive operators speak one vocabulary instead of maintaining
//! parallel profile models.
//!
//! For a whole plane section, use [`profiles_from_mesh_section`] or
//! [`PlaneSection::to_profiles`](crate::section::PlaneSection::to_profiles).
//! Each disconnected region becomes one profile, retaining its holes, frame,
//! and per-segment source correspondence. These conversions preserve the
//! section's existing order and coordinates; the projection contract below
//! applies only to [`section_from_loop`].
//!
//! ## Contract
//!
//! - **Ordering**: the section follows the mesh loop's own traversal
//!   order, re-rooted deterministically at the lowest vertex id, oriented
//!   so the projected polygon winds counter-clockwise in its projection plane
//!   (matching `Profile2`'s outer-loop requirement). Canonical rooting does
//!   not author correspondence between different sections for lofting.
//! - **Planarity**: vertices are projected onto the plane through their centroid
//!   with the Newell normal. The maximum deviation is reported; loops beyond
//!   `max_planar_deviation` are rejected. This is not a least-squares fit.
//! - **Provenance**: each polyline segment is tagged with its index so
//!   constructive source maps name mesh-derived features exactly like
//!   authored ones.

use alloc::vec::Vec;

use crate::profile::{Loop2, Profile2, ProfileError, Seg2, SegTag};
use exedra_mesh::{HalfEdgeId, Mesh, VertexId};

pub(crate) mod plane;
pub use plane::{SectionProfile, SectionProfileError, profiles_from_mesh_section};

/// Parameters for mesh-loop conversion.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct SectionParams {
    /// Maximum allowed out-of-plane deviation (model units) before the
    /// loop is rejected as non-planar. Must be finite and nonnegative.
    pub max_planar_deviation: f64,
}

impl Default for SectionParams {
    fn default() -> Self {
        Self {
            max_planar_deviation: 1.0e-3,
        }
    }
}

/// A converted section: the profile plus its 3D placement frame.
#[derive(Clone, Debug)]
pub struct MeshSection {
    /// The profile in its projected 2D frame, with this loop as its outer
    /// boundary. This conversion does not group separate loops into holes.
    pub profile: Profile2,
    /// Frame origin (the loop centroid) in mesh space.
    pub origin: [f64; 3],
    /// Frame tangent (2D x axis) in mesh space.
    pub tangent: [f64; 3],
    /// Frame bitangent (2D y axis) in mesh space.
    pub bitangent: [f64; 3],
    /// Newell plane normal.
    pub normal: [f64; 3],
    /// Largest out-of-plane deviation observed.
    pub planar_deviation: f64,
    /// The mesh vertices contributing each polyline point, in section
    /// order from the canonical root.
    pub vertices: Vec<VertexId>,
}

/// Typed conversion failure.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum SectionError {
    /// The loop has fewer than three distinct vertices.
    TooShort {
        /// Number of vertices found.
        count: usize,
    },
    /// Edges are stale or do not form a closed, ordered loop.
    BrokenLoop,
    /// The tolerance must be finite and nonnegative.
    InvalidTolerance,
    /// Positions or the loop normal are invalid or degenerate.
    InvalidGeometry,
    /// Vertices deviate from the Newell-normal plane through their centroid
    /// beyond the allowed tolerance.
    NonPlanar {
        /// Observed maximum deviation.
        deviation: f64,
        /// Allowed maximum.
        allowed: f64,
    },
    /// The loop is degenerate in its projection plane (zero area or invalid
    /// polygon).
    Degenerate(ProfileError),
}

impl core::fmt::Display for SectionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooShort { count } => {
                write!(f, "loop needs at least three vertices, found {count}")
            }
            Self::BrokenLoop => write!(f, "edges do not form a live, closed, ordered loop"),
            Self::InvalidTolerance => write!(f, "loop tolerance must be finite and nonnegative"),
            Self::InvalidGeometry => write!(f, "loop has invalid positions or a degenerate normal"),
            Self::NonPlanar { deviation, allowed } => write!(
                f,
                "loop deviates {deviation} from its Newell plane (allowed {allowed})"
            ),
            Self::Degenerate(e) => write!(f, "loop degenerates in its plane: {e}"),
        }
    }
}

impl core::error::Error for SectionError {}

/// Converts an ordered mesh half-edge loop into a profile section.
///
/// The half-edges must form one closed loop in order (as produced by
/// [`Mesh::boundary_loop`], [`Mesh::face_loop`], or an interactive edge
/// loop selection walked into order).
///
/// # Errors
///
/// Returns a typed [`SectionError`]; nothing is silently repaired.
pub fn section_from_loop(
    mesh: &Mesh,
    loop_edges: &[HalfEdgeId],
    params: &SectionParams,
) -> Result<MeshSection, SectionError> {
    use exedra_mesh_ops::planar::{LoopProjectionError as Error, project_loop};
    let projected = project_loop(mesh, loop_edges, params.max_planar_deviation).map_err(
        |error| match error {
            Error::TooShort { count } => SectionError::TooShort { count },
            Error::BrokenLoop => SectionError::BrokenLoop,
            Error::InvalidTolerance => SectionError::InvalidTolerance,
            Error::NonPlanar { deviation, allowed } => {
                SectionError::NonPlanar { deviation, allowed }
            }
            _ => SectionError::InvalidGeometry,
        },
    )?;
    let ordered_points = &projected.points;
    // Build the polyline loop with per-segment index tags. The segment
    // ending at point (i + 1) is segment i.
    let segs: Vec<Seg2> = (0..ordered_points.len())
        .map(|i| {
            let to = ordered_points[(i + 1) % ordered_points.len()];
            Seg2::line((to[0], to[1])).tagged(SegTag(u32::try_from(i).unwrap_or(u32::MAX)))
        })
        .collect();
    let outer = Loop2::new(segs).map_err(SectionError::Degenerate)?;
    let profile = Profile2::simple(outer).map_err(SectionError::Degenerate)?;

    Ok(MeshSection {
        profile,
        origin: projected.origin,
        tangent: projected.tangent,
        bitangent: projected.bitangent,
        normal: projected.normal,
        planar_deviation: projected.planar_deviation,
        vertices: projected.vertices,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::builders;
    use crate::evaluate::evaluate;
    use crate::ir::{CapMode, NodeKind, Placement3, RecipeBuilder};
    use crate::tessellate::EvalPolicy;

    fn open_prism() -> Mesh {
        // An uncapped extrusion has two boundary loops: the mesh-side
        // sections we convert.
        let mut b = RecipeBuilder::new();
        let p = b.add_profile(builders::rect_from_corner(2.0, 1.0).expect("rect"));
        let n = b
            .add(NodeKind::Extrude {
                profile: p,
                placement: Placement3::IDENTITY,
                height: 3.0,
                caps: CapMode::None,
            })
            .expect("valid");
        let recipe = b.finish(n).expect("valid recipe");
        let mut result = evaluate(&recipe, &EvalPolicy::default()).expect("evaluates");
        result.bodies.remove(0).body.mesh.clone()
    }

    #[test]
    fn boundary_loops_become_valid_profiles() {
        let mesh = open_prism();
        let loops = mesh.boundary_loops().expect("boundary loops");
        assert_eq!(loops.len(), 2, "open prism has two rims");
        for loop_edges in &loops {
            let section = section_from_loop(&mesh, loop_edges, &SectionParams::default())
                .expect("rim converts");
            // The rect rim projects to a 2 x 1 counter-clockwise polygon.
            let area = builders::profile_area(&section.profile);
            assert!((area - 2.0).abs() < 1e-6, "rim area {area}");
            assert_eq!(section.vertices.len(), 4);
            assert!(section.planar_deviation < 1e-6);
            // Canonical root: lowest vertex id leads.
            let min = section.vertices.iter().map(|v| v.index()).min().unwrap();
            assert_eq!(section.vertices[0].index(), min);
        }
    }

    #[test]
    fn conversion_is_deterministic() {
        let mesh = open_prism();
        let loops = mesh.boundary_loops().expect("boundary loops");
        let a = section_from_loop(&mesh, &loops[0], &SectionParams::default()).expect("a");
        let b = section_from_loop(&mesh, &loops[0], &SectionParams::default()).expect("b");
        assert_eq!(a.profile, b.profile);
        assert_eq!(a.vertices, b.vertices);
    }

    #[test]
    fn non_planar_loops_are_rejected_typed() {
        let mut mesh = open_prism();
        let loops = mesh.boundary_loops().expect("boundary loops");
        let victim = mesh.to_vertex(loops[0][0]).expect("vertex");
        {
            let mut session = mesh.edit();
            let p = *session.mesh().vertex_position(victim).expect("position");
            let _ = exedra_mesh::op::set_vertex_position(
                &mut session,
                victim,
                [p[0], p[1], p[2] + 0.5],
            );
            #[expect(unused_must_use, reason = "sink output unused")]
            {
                session.finish();
            }
        }
        let loops = mesh.boundary_loops().expect("boundary loops");
        let rejected = loops.iter().any(|l| {
            matches!(
                section_from_loop(&mesh, l, &SectionParams::default()),
                Err(SectionError::NonPlanar { .. })
            )
        });
        assert!(rejected, "the bent rim must reject typed");
    }
}

#[cfg(test)]
mod raw_input_tests {
    use super::*;

    fn quad() -> Mesh {
        Mesh::from_polygons(
            &[
                [0.0, 0.0, 0.0],
                [3.0, 0.0, 0.0],
                [2.0, 1.0, 0.0],
                [0.0, 2.0, 0.0],
            ],
            &[&[0, 1, 2, 3]],
        )
        .unwrap()
    }
    #[test]
    fn conversion_refuses_disconnected_edge_order() {
        let mesh = quad();
        let face = mesh.faces().next().unwrap();
        let disconnected: Vec<_> = mesh
            .face_loop(face)
            .map(|edge| mesh.twin(edge).unwrap())
            .collect();
        assert!(section_from_loop(&mesh, &disconnected, &SectionParams::default()).is_err());
    }
    #[test]
    fn conversion_refuses_invalid_tolerance() {
        let mesh = quad();
        let face = mesh.faces().next().unwrap();
        let edges: Vec<_> = mesh.face_loop(face).collect();
        assert!(
            section_from_loop(
                &mesh,
                &edges,
                &SectionParams {
                    max_planar_deviation: f64::NAN
                }
            )
            .is_err()
        );
    }
}

#[cfg(test)]
mod frame_parity_test {
    use super::*;
    #[test]
    fn tilted_asymmetric_loop_keeps_its_frame() {
        let mesh = Mesh::from_polygons(
            &[
                [10.0, 20.0, 30.0],
                [13.0, 26.0, 30.0],
                [12.0, 25.0, 32.0],
                [10.0, 22.0, 34.0],
            ],
            &[&[0, 1, 2, 3]],
        )
        .unwrap();
        let edges: Vec<_> = mesh.face_loop(mesh.faces().next().unwrap()).collect();
        let section = section_from_loop(&mesh, &edges, &SectionParams::default()).unwrap();
        assert_eq!(
            [
                section.origin,
                section.tangent,
                section.bitangent,
                section.normal
            ]
            .map(|v| v.map(f64::to_bits)),
            [
                [
                    4622522805030748160,
                    4627237510890651648,
                    4629559679448514560
                ],
                [
                    13819572158273122196,
                    4591696521790975892,
                    4606965345955040123
                ],
                [13825099940700876248, 13829603540328246744, 0],
                [
                    4606037347618495544,
                    13824905784845900856,
                    4597030148363754552
                ]
            ],
        );
    }
}
