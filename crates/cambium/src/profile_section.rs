// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Mesh-derived profile sections: the bridge from interactive mesh
//! selections to the constructive head's profile vocabulary.
//!
//! The authoritative pre-mesh profile model is
//! `exedra_constructive::profile::Profile2`. This module converts *mesh*
//! loops — face-loop boundaries and edge loops picked interactively — into
//! `Profile2` polyline sections, so interactive loft/sweep tooling and the
//! constructive operators speak one vocabulary instead of maintaining
//! parallel profile models.
//!
//! ## Contract
//!
//! - **Ordering**: the section follows the mesh loop's own traversal
//!   order, re-rooted deterministically at the lowest vertex id, oriented
//!   so the projected polygon winds counter-clockwise in its fitted plane
//!   (matching `Profile2`'s outer-loop requirement). Corresponding
//!   sections for lofting therefore correspond by index from their
//!   canonical roots.
//! - **Planarity**: vertices are projected onto their best-fit plane
//!   (Newell normal); the maximum out-of-plane deviation is measured and
//!   reported, and sections beyond `max_planar_deviation` are rejected
//!   typed — never silently flattened.
//! - **Provenance**: each polyline segment is tagged with its index so
//!   constructive source maps name mesh-derived features exactly like
//!   authored ones.

use alloc::vec::Vec;

use crate::math::FloatExt as _;
use exedra::{HalfEdgeId, Mesh, VertexId};
use exedra_constructive::profile::{Loop2, Profile2, ProfileError, Seg2, SegTag};

/// Parameters for mesh-loop conversion.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct SectionParams {
    /// Maximum allowed out-of-plane deviation (model units) before the
    /// loop is rejected as non-planar.
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
    /// The profile in its fitted 2D frame (outer loop only; mesh loops
    /// have no holes).
    pub profile: Profile2,
    /// Frame origin (the loop centroid) in mesh space.
    pub origin: [f64; 3],
    /// Frame tangent (2D x axis) in mesh space.
    pub tangent: [f64; 3],
    /// Frame bitangent (2D y axis) in mesh space.
    pub bitangent: [f64; 3],
    /// Fitted plane normal.
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
    /// A half-edge did not resolve to a live vertex.
    BrokenLoop,
    /// The loop's vertices deviate from their best-fit plane beyond the
    /// allowed tolerance.
    NonPlanar {
        /// Observed maximum deviation.
        deviation: f64,
        /// Allowed maximum.
        allowed: f64,
    },
    /// The loop is degenerate in its fitted plane (zero area or invalid
    /// polygon).
    Degenerate(ProfileError),
}

impl core::fmt::Display for SectionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooShort { count } => {
                write!(f, "loop needs at least three vertices, found {count}")
            }
            Self::BrokenLoop => write!(f, "loop traversal hit a dead half-edge"),
            Self::NonPlanar { deviation, allowed } => write!(
                f,
                "loop deviates {deviation} from its best-fit plane (allowed {allowed})"
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
    // Collect the loop's vertices in traversal order.
    let mut vertices: Vec<VertexId> = Vec::with_capacity(loop_edges.len());
    for &he in loop_edges {
        vertices.push(mesh.to_vertex(he).ok_or(SectionError::BrokenLoop)?);
    }
    vertices.dedup();
    if vertices.len() >= 2 && vertices.first() == vertices.last() {
        vertices.pop();
    }
    if vertices.len() < 3 {
        return Err(SectionError::TooShort {
            count: vertices.len(),
        });
    }

    // Canonical root: rotate so the lowest vertex id leads.
    let root = vertices
        .iter()
        .enumerate()
        .min_by_key(|(_, v)| v.index())
        .map(|(i, _)| i)
        .unwrap_or(0);
    vertices.rotate_left(root);

    let positions: Vec<[f64; 3]> = vertices
        .iter()
        .map(|&v| {
            let p = mesh.vertex_position(v).copied().unwrap_or([0.0, 0.0, 0.0]);
            [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])]
        })
        .collect();

    // Newell normal + centroid origin.
    let mut normal = [0.0_f64; 3];
    let mut origin = [0.0_f64; 3];
    let n = positions.len();
    for i in 0..n {
        let a = positions[i];
        let b = positions[(i + 1) % n];
        normal[0] += (a[1] - b[1]) * (a[2] + b[2]);
        normal[1] += (a[2] - b[2]) * (a[0] + b[0]);
        normal[2] += (a[0] - b[0]) * (a[1] + b[1]);
        for axis in 0..3 {
            origin[axis] += a[axis] / n as f64;
        }
    }
    let len = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt_ext();
    if len <= 0.0 {
        return Err(SectionError::Degenerate(ProfileError::TooFewSegments {
            count: n,
        }));
    }
    for c in &mut normal {
        *c /= len;
    }

    // Deterministic in-plane frame: Gram-Schmidt the world axis least
    // aligned with the normal (ties x before y before z).
    fn fabs(v: f64) -> f64 {
        if v < 0.0 { -v } else { v }
    }
    let abs = [fabs(normal[0]), fabs(normal[1]), fabs(normal[2])];
    let axis_index = if abs[0] <= abs[1] && abs[0] <= abs[2] {
        0
    } else if abs[1] <= abs[2] {
        1
    } else {
        2
    };
    let mut axis = [0.0; 3];
    axis[axis_index] = 1.0;
    let dot = axis[0] * normal[0] + axis[1] * normal[1] + axis[2] * normal[2];
    let mut tangent = [
        axis[0] - dot * normal[0],
        axis[1] - dot * normal[1],
        axis[2] - dot * normal[2],
    ];
    let tangent_len =
        (tangent[0] * tangent[0] + tangent[1] * tangent[1] + tangent[2] * tangent[2]).sqrt_ext();
    for c in &mut tangent {
        *c /= tangent_len;
    }
    let bitangent = [
        normal[1] * tangent[2] - normal[2] * tangent[1],
        normal[2] * tangent[0] - normal[0] * tangent[2],
        normal[0] * tangent[1] - normal[1] * tangent[0],
    ];

    // Project; measure planarity.
    let mut deviation = 0.0_f64;
    let projected: Vec<[f64; 2]> = positions
        .iter()
        .map(|p| {
            let d = [p[0] - origin[0], p[1] - origin[1], p[2] - origin[2]];
            let out_of_plane = d[0] * normal[0] + d[1] * normal[1] + d[2] * normal[2];
            deviation = deviation.max(fabs(out_of_plane));
            [
                d[0] * tangent[0] + d[1] * tangent[1] + d[2] * tangent[2],
                d[0] * bitangent[0] + d[1] * bitangent[1] + d[2] * bitangent[2],
            ]
        })
        .collect();
    if deviation > params.max_planar_deviation {
        return Err(SectionError::NonPlanar {
            deviation,
            allowed: params.max_planar_deviation,
        });
    }

    // Orientation: Profile2 requires counter-clockwise; flip by reversal
    // (keeping the canonical root leading) when the projection winds
    // clockwise.
    let mut area = 0.0;
    for i in 0..projected.len() {
        let a = projected[i];
        let b = projected[(i + 1) % projected.len()];
        area += a[0] * b[1] - b[0] * a[1];
    }
    let (ordered_points, ordered_vertices): (Vec<[f64; 2]>, Vec<VertexId>) = if area < 0.0 {
        let mut points = projected;
        let mut verts = vertices;
        points[1..].reverse();
        verts[1..].reverse();
        (points, verts)
    } else {
        (projected, vertices)
    };

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
        origin,
        tangent,
        bitangent,
        normal,
        planar_deviation: deviation,
        vertices: ordered_vertices,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use exedra_constructive::builders;
    use exedra_constructive::evaluate::evaluate;
    use exedra_constructive::ir::{CapMode, NodeKind, Placement3, RecipeBuilder};
    use exedra_constructive::tessellate::EvalPolicy;

    fn open_prism() -> Mesh {
        // An uncapped extrusion has two boundary loops: the mesh-side
        // sections we convert.
        let mut b = RecipeBuilder::new();
        let p = b.add_profile(builders::rect(2.0, 1.0).expect("rect"));
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
            let _ = exedra::op::set_vertex_position(&mut session, victim, [p[0], p[1], p[2] + 0.5]);
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
