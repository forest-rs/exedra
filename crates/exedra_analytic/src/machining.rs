// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Transactional analytic through-cylinder subtraction on retained boundaries.
//!
//! This module edits topology, not meshes or constructor histories. It accepts
//! two parallel planar exit patches with polygonal or complete elliptic trims,
//! and proves separation from every remaining planar or cylindrical face.
//! Conservative bounds may refuse valid cuts. Tangency and intersecting bores
//! are refused. Input must already represent an embedded, outward-oriented
//! regular solid: incidence validation alone cannot establish that premise.

use crate::body::{
    Body, BodyError, BoundaryLoop, Coedge, Edge, EdgeId, Face, FaceId, LoopId, Sense,
    ValidationPolicy, VertexId,
};
use crate::body::{LoopTrims, TrimCurve};
use crate::geometry::intersection::{
    PlaneCylinderSection, SectionError, SectionPolicy, plane_cylinder,
};
use crate::geometry::{Curve, GeometryError, ParameterDomain, ParameterRange, Surface};
use alloc::vec::Vec;
use exedra_math::{Placement3, add, cross, dot, scale, sub};

/// Infinite circular cylinder used to remove a through passage.
#[derive(Clone, Debug, PartialEq)]
pub struct CylinderTool {
    surface: Surface,
}
impl CylinderTool {
    /// Constructs a cylinder in body-local coordinates; direction is normalized.
    /// # Errors
    /// Refuses nonfinite inputs, a zero direction or a nonpositive radius.
    pub fn new(point: [f64; 3], direction: [f64; 3], radius: f64) -> Result<Self, GeometryError> {
        let n = norm(direction);
        if !n.is_finite() {
            return Err(GeometryError::NonFinite);
        }
        if n == 0.0 {
            return Err(GeometryError::ZeroDirection);
        }
        let z = scale(direction, 1.0 / n);
        let seed = if z[0].abs() < 0.8 {
            [1.0, 0.0, 0.0]
        } else {
            [0.0, 1.0, 0.0]
        };
        let y = unit(cross(z, seed));
        let x = cross(y, z);
        Ok(Self {
            surface: Surface::cylinder(Placement3::from_axes(x, y, z, point), radius)?,
        })
    }
    /// Checked supporting cylinder in body-local coordinates.
    #[must_use]
    pub const fn surface(&self) -> &Surface {
        &self.surface
    }
}
/// Explicit numerical and material-clearance policy in body-local units.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct MachiningPolicy {
    /// Required strictly positive clearance from every untouched boundary.
    pub separation: f64,
    /// Maximum conservative support-agreement bound.
    pub geometry_gap: f64,
    /// Minimum absolute dot of tool axis and exit plane normal in `(0,1]`.
    pub min_axis_dot: f64,
}
impl Default for MachiningPolicy {
    fn default() -> Self {
        Self {
            separation: 1e-6,
            geometry_gap: 1e-9,
            min_axis_dot: 0.05,
        }
    }
}
/// How one source face participates in a successful operation.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FaceChangeKind {
    /// Geometry and trims were retained.
    Retained,
    /// A new inner trim was inserted.
    Trimmed,
}
/// Stable source-to-result face correspondence for one operation.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct FaceChange {
    /// Input body-local face identity.
    pub source: FaceId,
    /// Output body-local face identity (preserved by this operation).
    pub result: FaceId,
    /// Whether its trim changed.
    pub kind: FaceChangeKind,
}
/// Exact construction correspondence and conservative geometric evidence.
#[derive(Clone, Debug, PartialEq)]
pub struct MachiningReport {
    /// All input faces, in input table order.
    pub faces: Vec<FaceChange>,
    /// All retained input edges, preserving their indexes.
    pub retained_edges: Vec<EdgeId>,
    /// Exit faces ordered along the tool axis.
    pub trimmed_faces: [FaceId; 2],
    /// Newly created full elliptic rim edges.
    pub rims: [EdgeId; 2],
    /// Newly created cylindrical cavity face.
    pub wall: FaceId,
    /// Newly created periodic wall seam.
    pub seam: EdgeId,
    /// Certified clearance lower bound from untouched boundaries.
    pub min_clearance: f64,
    /// Largest conservative support-agreement bound in model units.
    pub max_gap: f64,
    /// Analytic removed volume, valid for the supported parallel exit planes.
    pub removed_volume: f64,
    /// Analytic change to surface area (new wall minus two openings).
    pub surface_area_delta: f64,
}
/// A complete successful edit; the input is never mutated.
#[derive(Clone, Debug, PartialEq)]
pub struct MachiningResult<F = (), E = ()> {
    /// Edited retained boundary, with source table identities preserved.
    pub body: Body<F, E>,
    /// Source correspondence, created topology and numerical evidence.
    pub report: MachiningReport,
}
/// Explicit refusal; no partial output or healed geometry is returned.
#[derive(Clone, Debug, PartialEq)]
pub enum MachiningError {
    /// Nonfinite or out-of-range policy values.
    InvalidPolicy,
    /// Input or constructed topology/supports fail validation.
    Boundary(BodyError),
    /// Input is not an oriented closed boundary.
    OpenBoundary,
    /// A support or trim is outside this operation's certified subset.
    UnsupportedFace {
        /// Offending input identity.
        face: FaceId,
    },
    /// A boundary cannot be proven separated from the tool.
    Clearance {
        /// Offending input identity.
        face: FaceId,
        /// Certified lower bound (may be negative).
        bound: f64,
        /// Required lower bound.
        required: f64,
    },
    /// Exactly two complete interior exit sections are required.
    ExitCount {
        /// Number found.
        found: usize,
    },
    /// Exit supports must be parallel for this version of the operation.
    NonparallelExits,
    /// Exact-form section construction was refused.
    Section(SectionError),
    /// Derived geometry was refused.
    Geometry(GeometryError),
    /// Finite arithmetic or conservative roundoff bound cannot meet policy.
    NumericLimit,
}
impl core::fmt::Display for MachiningError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "analytic machining: {self:?}")
    }
}
impl core::error::Error for MachiningError {}

/// Subtracts a complete through cylinder, assigning default roles to new entities.
/// Source roles and indexes are preserved; use the report for operation history.
/// # Errors
/// Refuses unsupported intersections, insufficient clearance and invalid input.
pub fn subtract_through_cylinder<F: Clone + Default, E: Clone + Default>(
    body: &Body<F, E>,
    tool: &CylinderTool,
    policy: &MachiningPolicy,
) -> Result<MachiningResult<F, E>, MachiningError> {
    subtract_through_cylinder_with_roles(body, tool, policy, F::default(), E::default())
}
/// Subtracts a through cylinder, explicitly assigning generated face/edge roles.
///
/// All source IDs survive unchanged. A successful edit preserves a valid solid
/// provided the input is an embedded, outward-oriented regular solid. No global
/// self-intersection or shell-nesting certificate is inferred from incidence.
/// # Errors
/// Returns a structured refusal transactionally; see [`MachiningError`].
pub fn subtract_through_cylinder_with_roles<F: Clone, E: Clone>(
    body: &Body<F, E>,
    tool: &CylinderTool,
    policy: &MachiningPolicy,
    face_role: F,
    edge_role: E,
) -> Result<MachiningResult<F, E>, MachiningError> {
    if !policy.separation.is_finite()
        || policy.separation <= 0.0
        || !policy.geometry_gap.is_finite()
        || policy.geometry_gap < 0.0
        || !policy.min_axis_dot.is_finite()
        || !(0.0..=1.0).contains(&policy.min_axis_dot)
        || policy.min_axis_dot == 0.0
    {
        return Err(MachiningError::InvalidPolicy);
    }
    let validation = ValidationPolicy {
        max_gap: policy.geometry_gap,
    };
    body.validate(&validation)
        .map_err(MachiningError::Boundary)?;
    if !body.is_closed() {
        return Err(MachiningError::OpenBoundary);
    }
    let Surface::Cylinder(cylinder) = &tool.surface else {
        unreachable!()
    };
    let frame = cylinder.frame();
    let axis = col(frame, 2);
    let origin = col(frame, 3);
    let radius = cylinder.radius();
    let magnitude = body
        .vertices()
        .iter()
        .flatten()
        .copied()
        .chain(origin)
        .map(f64::abs)
        .fold(radius, f64::max);
    let roundoff = 512.0 * f64::EPSILON * magnitude;
    if !roundoff.is_finite() || roundoff > policy.geometry_gap {
        return Err(MachiningError::NumericLimit);
    }
    let mut exits: Vec<(FaceId, PlaneCylinderSection)> = Vec::new();
    let mut clearance = f64::INFINITY;
    for (index, face) in body.faces().iter().enumerate() {
        let id = FaceId(crate::usize_to_u32(index));
        let separated = match &face.surface {
            Surface::Plane(_) => planar_radial_clearance(body, id, origin, axis, radius)?,
            Surface::Cylinder(other) => {
                let of = other.frame();
                let oa = col(of, 2);
                let delta = sub(col(of, 3), origin);
                let c = cross(axis, oa);
                let cn = norm(c);
                if cn <= 64.0 * f64::EPSILON {
                    if cn != 0.0 {
                        return Err(MachiningError::UnsupportedFace { face: id });
                    }
                    (norm(sub(delta, scale(axis, dot(delta, axis)))) - other.radius()).abs()
                        - radius
                } else {
                    dot(delta, scale(c, 1.0 / cn)).abs() - radius - other.radius()
                }
            }
            _ => return Err(MachiningError::UnsupportedFace { face: id }),
        } - roundoff;
        if separated >= policy.separation {
            clearance = clearance.min(separated);
            continue;
        }
        let Surface::Plane(plane) = &face.surface else {
            return Err(MachiningError::Clearance {
                face: id,
                bound: separated,
                required: policy.separation,
            });
        };
        let section = plane_cylinder(
            plane,
            cylinder,
            &SectionPolicy {
                min_axis_dot: policy.min_axis_dot,
                max_gap: policy.geometry_gap,
            },
        )
        .map_err(MachiningError::Section)?;
        let bound = section_clearance(body, id, &section)? - roundoff - section.max_gap();
        if bound < policy.separation {
            return Err(MachiningError::Clearance {
                face: id,
                bound,
                required: policy.separation,
            });
        }
        clearance = clearance.min(bound);
        exits.push((id, section));
    }
    if exits.len() != 2 {
        return Err(MachiningError::ExitCount { found: exits.len() });
    }
    exits.sort_by(|a, b| a.1.axial().coefficients()[0].total_cmp(&b.1.axial().coefficients()[0]));
    let normals = exits
        .iter()
        .map(|(id, _)| {
            let Surface::Plane(p) = &body.faces()[id.index()].surface else {
                unreachable!()
            };
            col(p.frame(), 2)
        })
        .collect::<Vec<_>>();
    if norm(cross(normals[0], normals[1])) > 64.0 * f64::EPSILON {
        return Err(MachiningError::NonparallelExits);
    }
    // Parallel accepted frames must still yield the same canonical section phase.
    if exits[0].1.phase() != exits[1].1.phase() || exits[0].1.reversed() != exits[1].1.reversed() {
        return Err(MachiningError::NonparallelExits);
    }
    let height = exits[1].1.axial().coefficients()[0] - exits[0].1.axial().coefficients()[0];
    if !height.is_finite() || height <= policy.separation {
        return Err(MachiningError::NumericLimit);
    }
    let outward = |i: usize| scale(normals[i], body.faces()[exits[i].0.index()].sense.sign());
    if dot(outward(0), axis) >= 0.0 || dot(outward(1), axis) <= 0.0 {
        return Err(MachiningError::OpenBoundary);
    }
    let mut parts = body.clone().into_parts();
    let range =
        ParameterRange::new(0.0, core::f64::consts::TAU).map_err(MachiningError::Geometry)?;
    let rims = core::array::from_fn(|i| EdgeId(crate::usize_to_u32(parts.edges.len() + i)));
    let vertices = core::array::from_fn::<_, 2, _>(|i| {
        VertexId(crate::usize_to_u32(parts.vertices.len() + i))
    });
    let mut reversed = [false; 2];
    for i in 0..2 {
        let curve = exits[i].1.curve().clone();
        parts.vertices.push(
            curve
                .evaluate(0.0)
                .ok_or(MachiningError::NumericLimit)?
                .point,
        );
        parts.edges.push(Edge::new(
            curve,
            range,
            vertices[i],
            vertices[i],
            edge_role.clone(),
        ));
        reversed[i] = dot(col(exits[i].1.ellipse().frame(), 2), outward(i)) > 0.0;
        let loop_id = LoopId(crate::usize_to_u32(parts.loops.len()));
        parts.loops.push(BoundaryLoop {
            coedges: alloc::vec![Coedge {
                edge: rims[i],
                reversed: reversed[i]
            }],
        });
        parts.faces[exits[i].0.index()].loops.push(loop_id);
        crate::operation_trims::append_planar_loop(&mut parts, exits[i].0, loop_id)
            .map_err(MachiningError::Geometry)?;
    }
    if reversed[0] == reversed[1] {
        return Err(MachiningError::NonparallelExits);
    }
    let seam = EdgeId(crate::usize_to_u32(parts.edges.len()));
    let a = parts.vertices[vertices[0].index()];
    let b = parts.vertices[vertices[1].index()];
    parts.edges.push(Edge::new(
        Curve::line(a, sub(b, a)).map_err(MachiningError::Geometry)?,
        ParameterRange::UNIT,
        vertices[0],
        vertices[1],
        edge_role,
    ));
    let loop_id = LoopId(crate::usize_to_u32(parts.loops.len()));
    parts.loops.push(BoundaryLoop {
        coedges: alloc::vec![
            Coedge {
                edge: rims[0],
                reversed: !reversed[0]
            },
            Coedge {
                edge: seam,
                reversed: false
            },
            Coedge {
                edge: rims[1],
                reversed: !reversed[1]
            },
            Coedge {
                edge: seam,
                reversed: true
            }
        ],
    });
    let rim_trims = [
        section_trim(&exits[0].1, range),
        section_trim(&exits[1].1, range),
    ];
    let first_fraction = if !reversed[0] { 0.0 } else { 1.0 };
    let second_fraction = 1.0 - first_fraction;
    let seam_trim = |fraction| -> Result<TrimCurve, MachiningError> {
        Ok(TrimCurve::Line {
            start: rim_trims[0]
                .evaluate(fraction)
                .ok_or(MachiningError::NumericLimit)?,
            end: rim_trims[1]
                .evaluate(fraction)
                .ok_or(MachiningError::NumericLimit)?,
        })
    };
    parts.trims.push(LoopTrims {
        loop_id,
        curves: alloc::vec![
            rim_trims[0].clone(),
            seam_trim(first_fraction)?,
            rim_trims[1].clone(),
            seam_trim(second_fraction)?
        ],
    });
    let ua = rim_trims[0]
        .evaluate(0.0)
        .ok_or(MachiningError::NumericLimit)?[0];
    let ub = rim_trims[0]
        .evaluate(1.0)
        .ok_or(MachiningError::NumericLimit)?[0];
    let wall_u = ParameterRange::new(ua.min(ub), ua.max(ub)).map_err(MachiningError::Geometry)?;
    let wall = FaceId(crate::usize_to_u32(parts.faces.len()));
    let lo = exits[0].1.axial().bounds()[0];
    let hi = exits[1].1.axial().bounds()[1];
    parts.faces.push(Face {
        surface: tool.surface.clone(),
        domain: ParameterDomain {
            u: wall_u,
            v: ParameterRange::new(
                (lo - 4.0 * policy.geometry_gap).next_down(),
                (hi + 4.0 * policy.geometry_gap).next_up(),
            )
            .map_err(MachiningError::Geometry)?,
        },
        sense: Sense::Reverse,
        loops: alloc::vec![loop_id],
        role: face_role,
    });
    let output = Body { parts };
    if !output.is_closed() {
        return Err(MachiningError::OpenBoundary);
    }
    let checked = output
        .validate(&validation)
        .map_err(MachiningError::Boundary)?;
    let max_gap = exits
        .iter()
        .map(|e| e.1.max_gap())
        .fold(checked.max_gap, f64::max);
    let area = core::f64::consts::PI
        * exits[0].1.ellipse().semi_major()
        * exits[0].1.ellipse().semi_minor();
    let removed_volume = core::f64::consts::PI * radius * radius * height;
    let surface_area_delta = core::f64::consts::TAU * radius * height - 2.0 * area;
    if !removed_volume.is_finite() || !surface_area_delta.is_finite() {
        return Err(MachiningError::NumericLimit);
    }
    Ok(MachiningResult {
        body: output,
        report: MachiningReport {
            faces: body
                .faces()
                .iter()
                .enumerate()
                .map(|(i, _)| {
                    let id = FaceId(crate::usize_to_u32(i));
                    FaceChange {
                        source: id,
                        result: id,
                        kind: if exits.iter().any(|e| e.0 == id) {
                            FaceChangeKind::Trimmed
                        } else {
                            FaceChangeKind::Retained
                        },
                    }
                })
                .collect(),
            retained_edges: (0..body.edges().len())
                .map(|i| EdgeId(crate::usize_to_u32(i)))
                .collect(),
            trimmed_faces: [exits[0].0, exits[1].0],
            rims,
            wall,
            seam,
            min_clearance: clearance,
            max_gap,
            removed_volume,
            surface_area_delta,
        },
    })
}

fn section_trim(section: &PlaneCylinderSection, range: ParameterRange) -> TrimCurve {
    let [offset, cosine, sine] = section.axial().coefficients();
    let phase = section.phase();
    let sign = if section.reversed() { -1.0 } else { 1.0 };
    let (s, c) = (libm::sin(phase), libm::cos(phase));
    TrimCurve::Harmonic {
        origin: [phase, offset],
        linear: [sign, 0.0],
        cosine: [0.0, cosine * c + sine * s],
        sine: [0.0, sign * (-cosine * s + sine * c)],
        range,
    }
}
fn col(frame: Placement3, i: usize) -> [f64; 3] {
    frame.rows.map(|r| r[i])
}
fn norm(v: [f64; 3]) -> f64 {
    libm::hypot(libm::hypot(v[0], v[1]), v[2])
}
fn unit(v: [f64; 3]) -> [f64; 3] {
    scale(v, 1.0 / norm(v))
}
fn curve_projection(
    curve: &Curve,
    range: ParameterRange,
    n: [f64; 3],
    origin: [f64; 3],
) -> Option<f64> {
    Some(match curve {
        Curve::Line(_) => dot(sub(curve.evaluate(range.bounds()[0])?.point, origin), n).min(dot(
            sub(curve.evaluate(range.bounds()[1])?.point, origin),
            n,
        )),
        Curve::Circle(c) => {
            dot(sub(col(c.frame(), 3), origin), n)
                - c.radius() * norm([dot(col(c.frame(), 0), n), dot(col(c.frame(), 1), n), 0.0])
        }
        Curve::Ellipse(e) => {
            dot(sub(col(e.frame(), 3), origin), n)
                - libm::hypot(
                    e.semi_major() * dot(col(e.frame(), 0), n),
                    e.semi_minor() * dot(col(e.frame(), 1), n),
                )
        }
    })
}
fn planar_radial_clearance<F, E>(
    body: &Body<F, E>,
    face: FaceId,
    origin: [f64; 3],
    axis: [f64; 3],
    radius: f64,
) -> Result<f64, MachiningError> {
    let f = &body.faces()[face.index()];
    let mut center = [0.0; 3];
    let mut count = 0.0;
    for l in &f.loops {
        for co in &body.loops()[l.index()].coedges {
            center = add(
                center,
                body.vertices()[body
                    .coedge_vertices(*co)
                    .ok_or(MachiningError::NumericLimit)?[0]
                    .index()],
            );
            count += 1.0;
        }
    }
    if count == 0.0 {
        return Err(MachiningError::UnsupportedFace { face });
    }
    center = scale(center, 1.0 / count);
    let d = sub(center, origin);
    let radial = sub(d, scale(axis, dot(d, axis)));
    let Surface::Plane(plane) = &f.surface else {
        unreachable!()
    };
    let normal = col(plane.frame(), 2);
    let projected = sub(normal, scale(axis, dot(normal, axis)));
    let mut best = -radius;
    for candidate in [radial, projected, scale(projected, -1.0)] {
        let length = norm(candidate);
        if length == 0.0 {
            continue;
        }
        let n = scale(candidate, 1.0 / length);
        let mut min = f64::INFINITY;
        for l in &f.loops {
            for co in &body.loops()[l.index()].coedges {
                let Some((curve, range)) = body.edges()[co.edge.index()].regular() else {
                    return Err(MachiningError::UnsupportedFace { face });
                };
                min = min.min(
                    curve_projection(curve, range, n, origin)
                        .ok_or(MachiningError::NumericLimit)?,
                );
            }
        }
        best = best.max(min - radius);
    }
    Ok(best)
}
fn section_clearance<F, E>(
    body: &Body<F, E>,
    face: FaceId,
    section: &PlaneCylinderSection,
) -> Result<f64, MachiningError> {
    let ellipse = section.ellipse();
    let center = col(ellipse.frame(), 3);
    let radius = ellipse.semi_major();
    let f = &body.faces()[face.index()];
    let Surface::Plane(plane) = &f.surface else {
        unreachable!()
    };
    let frame = plane.frame();
    let xy = |p: [f64; 3]| {
        let d = sub(p, col(frame, 3));
        [dot(d, col(frame, 0)), dot(d, col(frame, 1))]
    };
    let p = xy(center);
    let mut clearance = f64::INFINITY;
    for (loop_index, l) in f.loops.iter().enumerate() {
        let coedges = &body.loops()[l.index()].coedges;
        // A circle may be authored as several arcs. Certify one complete turn
        // on exactly the same support; never infer it from sampled points.
        if coedges.len() > 1
            && let Some((Curve::Circle(first), _)) = body.edges()[coedges[0].edge.index()].regular()
        {
            let mut turn = 0.0_f64;
            let mut complete = true;
            for co in coedges {
                let Some((Curve::Circle(c), range)) = body.edges()[co.edge.index()].regular()
                else {
                    complete = false;
                    break;
                };
                if col(c.frame(), 3) != col(first.frame(), 3)
                    || c.radius() != first.radius()
                    || norm(cross(col(c.frame(), 2), col(first.frame(), 2))) != 0.0
                {
                    complete = false;
                    break;
                }
                let sign = if co.reversed { -1.0 } else { 1.0 }
                    * dot(col(c.frame(), 2), col(first.frame(), 2)).signum();
                if turn != 0.0 && turn.signum() != sign {
                    complete = false;
                    break;
                }
                turn += sign * range.span();
            }
            if complete && (turn.abs() - core::f64::consts::TAU).abs() <= 32.0 * f64::EPSILON {
                let distance = norm(sub(col(first.frame(), 3), center));
                let bound = if loop_index == 0 {
                    first.radius() - distance - radius
                } else {
                    distance - first.radius() - radius
                };
                clearance = clearance.min(bound);
                continue;
            }
        }
        if coedges.len() == 1 {
            let Some((curve, range)) = body.edges()[coedges[0].edge.index()].regular() else {
                return Err(MachiningError::UnsupportedFace { face });
            };
            if range.span() != core::f64::consts::TAU {
                return Err(MachiningError::UnsupportedFace { face });
            }
            let (c, inner, outer) = match curve {
                Curve::Circle(c) => (col(c.frame(), 3), c.radius(), c.radius()),
                Curve::Ellipse(e) => (col(e.frame(), 3), e.semi_minor(), e.semi_major()),
                _ => return Err(MachiningError::UnsupportedFace { face }),
            };
            let distance = norm(sub(c, center));
            let bound = if loop_index == 0 {
                inner - distance - radius
            } else {
                distance - outer - radius
            };
            clearance = clearance.min(bound);
            continue;
        }
        let mut winding = false;
        let mut min = f64::INFINITY;
        for co in coedges {
            let edge = &body.edges()[co.edge.index()];
            let Some((Curve::Line(_), _)) = edge.regular() else {
                return Err(MachiningError::UnsupportedFace { face });
            };
            let [v, w] = body
                .coedge_vertices(*co)
                .ok_or(MachiningError::NumericLimit)?;
            let a = xy(body.vertices()[v.index()]);
            let b = xy(body.vertices()[w.index()]);
            let d = [b[0] - a[0], b[1] - a[1]];
            let squared = d[0] * d[0] + d[1] * d[1];
            if squared == 0.0 {
                return Err(MachiningError::UnsupportedFace { face });
            }
            let t = (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / squared).clamp(0.0, 1.0);
            min = min.min(libm::hypot(p[0] - a[0] - t * d[0], p[1] - a[1] - t * d[1]));
            if (a[1] > p[1]) != (b[1] > p[1])
                && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
            {
                winding = !winding;
            }
        }
        let bound = if winding == (loop_index == 0) {
            min - radius
        } else {
            -min - radius
        };
        clearance = clearance.min(bound);
    }
    if clearance.is_finite() {
        Ok(clearance)
    } else {
        Err(MachiningError::UnsupportedFace { face })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extrusion::Extrusion;
    use exedra_profile::{Loop, Profile, ProfilePolicy, Segment};
    fn stock() -> Body {
        let outline = Loop::new(alloc::vec![
            Segment::line([30.0, -20.0]),
            Segment::line([30.0, 20.0]),
            Segment::line([-30.0, 20.0]),
            Segment::line([-30.0, -20.0])
        ])
        .unwrap();
        from_profile(Profile::new(outline, alloc::vec![], &ProfilePolicy::new(1e-9)).unwrap())
    }
    fn from_profile(profile: Profile) -> Body {
        let extrusion = Extrusion::new(profile, Placement3::IDENTITY, 10.0).unwrap();
        let source = extrusion.body().clone().into_parts();
        let parts = crate::body::BodyParts {
            trims: source.trims,
            vertices: source.vertices,
            loops: source.loops,
            edges: source
                .edges
                .into_iter()
                .map(|e| Edge {
                    geometry: e.geometry,
                    start: e.start,
                    end: e.end,
                    role: (),
                })
                .collect(),
            faces: source
                .faces
                .into_iter()
                .map(|f| Face {
                    surface: f.surface,
                    domain: f.domain,
                    sense: f.sense,
                    loops: f.loops,
                    role: (),
                })
                .collect(),
        };
        Body::try_from_parts(parts, &ValidationPolicy { max_gap: 1e-9 }).unwrap()
    }
    #[test]
    fn annular_and_nonconvex_stock_are_edited_without_constructor_dispatch() {
        let circle = |r: f64| {
            Loop::new(alloc::vec![
                Segment::arc([-r, 0.0], 1.0),
                Segment::arc([r, 0.0], 1.0)
            ])
            .unwrap()
        };
        let annulus = from_profile(
            Profile::new(
                circle(30.0),
                alloc::vec![circle(10.0).reversed()],
                &ProfilePolicy::new(1e-9),
            )
            .unwrap(),
        );
        let result = subtract_through_cylinder(
            &annulus,
            &CylinderTool::new([20.0, 0.0, 0.0], [0.0, 0.0, 1.0], 2.0).unwrap(),
            &MachiningPolicy::default(),
        )
        .unwrap();
        assert!(result.body.is_closed());
        assert!(
            subtract_through_cylinder(
                &annulus,
                &CylinderTool::new([0.0; 3], [0.0, 0.0, 1.0], 2.0).unwrap(),
                &MachiningPolicy::default()
            )
            .is_err()
        );
        let outline = Loop::new(alloc::vec![
            Segment::line([30.0, -20.0]),
            Segment::line([30.0, 0.0]),
            Segment::line([0.0, 0.0]),
            Segment::line([0.0, 20.0]),
            Segment::line([-30.0, 20.0]),
            Segment::line([-30.0, -20.0])
        ])
        .unwrap();
        let concave =
            from_profile(Profile::new(outline, alloc::vec![], &ProfilePolicy::new(1e-9)).unwrap());
        let result = subtract_through_cylinder(
            &concave,
            &CylinderTool::new([-15.0, 5.0, 0.0], [0.0, 0.0, 1.0], 2.0).unwrap(),
            &MachiningPolicy::default(),
        )
        .unwrap();
        assert!(result.body.is_closed());
    }
    #[test]
    fn flanged_housing_accepts_mount_pattern_and_oblique_main_passage() {
        let outline = Loop::new(alloc::vec![
            Segment::line([52.0, -45.0]),
            Segment::line([52.0, -21.0]),
            Segment::line([32.0, -21.0]),
            Segment::line([32.0, 45.0]),
            Segment::line([-32.0, 45.0]),
            Segment::line([-32.0, -21.0]),
            Segment::line([-52.0, -21.0]),
            Segment::line([-52.0, -45.0])
        ])
        .unwrap();
        let profile = Profile::new(outline, alloc::vec![], &ProfilePolicy::new(1e-9)).unwrap();
        let source = Extrusion::new(profile, Placement3::IDENTITY, 18.0).unwrap();
        let mut body = source.body().clone();
        for (point, direction, radius) in [
            ([-38.0, -33.0, 0.0], [0.0, 0.0, 1.0], 5.0),
            ([38.0, -33.0, 0.0], [0.0, 0.0, 1.0], 5.0),
            ([0.0, 15.0, 0.0], [0.364, 0.0, 1.0], 12.0),
            ([0.0, -10.0, 0.0], [0.0, 0.0, 1.0], 5.0),
        ] {
            body = subtract_through_cylinder_with_roles(
                &body,
                &CylinderTool::new(point, direction, radius).unwrap(),
                &MachiningPolicy::default(),
                source.body().faces()[0].role,
                source.body().edges()[0].role,
            )
            .unwrap()
            .body;
        }
        assert!(body.is_closed());
        let realized = body
            .realize(
                Placement3::IDENTITY,
                &crate::realize::RealizationPolicy {
                    tolerance: 0.03,
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(realized.mesh.validate_deep().is_empty());
        assert!(realized.mesh.boundary_loops().unwrap().is_empty());
        assert!(realized.report.within_tolerance());
    }
    #[test]
    fn repeated_cuts_preserve_ids_and_exact_measures() {
        let original = stock();
        let mut body = original.clone();
        for (index, x) in [-15.0, 0.0, 15.0].into_iter().enumerate() {
            let before = body.clone();
            let result = subtract_through_cylinder(
                &body,
                &CylinderTool::new([x, 0.0, 0.0], [0.0, 0.0, 1.0], 2.0).unwrap(),
                &MachiningPolicy::default(),
            )
            .unwrap();
            assert_eq!(body, before);
            assert!(result.body.is_closed());
            assert!(result.body.has_explicit_trims(result.report.wall));
            assert_eq!(&result.body.trims()[..body.trims().len()], body.trims());
            for face in result.report.trimmed_faces {
                let loop_id = *result.body.faces()[face.index()].loops.last().unwrap();
                assert!(matches!(
                    result.body.trim_curve(loop_id, 0),
                    Some(TrimCurve::Harmonic { .. })
                ));
            }
            assert_eq!(result.report.wall.index(), 6 + index);
            assert_eq!(result.report.retained_edges.len(), body.edges().len());
            assert!((result.report.removed_volume - core::f64::consts::PI * 40.0).abs() < 1e-10);
            assert_eq!(
                result
                    .report
                    .faces
                    .iter()
                    .filter(|f| f.kind == FaceChangeKind::Trimmed)
                    .count(),
                2
            );
            result
                .body
                .cylindrical_band(result.report.wall, &ValidationPolicy { max_gap: 1e-9 })
                .unwrap();
            let mesh = result
                .body
                .realize(
                    Placement3::IDENTITY,
                    &crate::realize::RealizationPolicy {
                        tolerance: 0.03,
                        geometry_gap: 1e-9,
                        ..Default::default()
                    },
                )
                .unwrap();
            assert!(mesh.mesh.validate_deep().is_empty());
            assert!(mesh.mesh.boundary_loops().unwrap().is_empty());
            assert!(mesh.report.within_tolerance());
            body = result.body;
        }
        assert_eq!(original.faces().len(), 6);
    }
    #[test]
    fn oblique_cut_axis_reversal_and_numerical_refusals() {
        let body = stock();
        for sign in [-1.0, 1.0] {
            let result = subtract_through_cylinder(
                &body,
                &CylinderTool::new([0.0; 3], [0.3 * sign, 0.1 * sign, sign], 2.0).unwrap(),
                &MachiningPolicy::default(),
            )
            .unwrap();
            assert!(result.body.is_closed());
            result
                .body
                .cylindrical_band(result.report.wall, &ValidationPolicy { max_gap: 1e-9 })
                .unwrap();
            let expected = core::f64::consts::PI * 40.0 * libm::sqrt(1.1);
            assert!((result.report.removed_volume - expected).abs() < 1e-10);
        }
        assert!(CylinderTool::new([0.0; 3], [0.0; 3], 2.0).is_err());
        assert!(CylinderTool::new([0.0; 3], [0.0, 0.0, 1.0], 0.0).is_err());
        assert!(matches!(
            subtract_through_cylinder(
                &body,
                &CylinderTool::new([0.0; 3], [0.0, 0.0, 1.0], 2.0).unwrap(),
                &MachiningPolicy {
                    separation: 0.0,
                    ..Default::default()
                }
            ),
            Err(MachiningError::InvalidPolicy)
        ));
    }
    #[test]
    fn tangent_overlapping_and_outside_tools_refuse_transactionally() {
        let input = stock();
        let first = subtract_through_cylinder(
            &input,
            &CylinderTool::new([0.0; 3], [0.0, 0.0, 1.0], 2.0).unwrap(),
            &MachiningPolicy::default(),
        )
        .unwrap();
        for x in [0.0, 3.0, 4.0, 28.0, 29.0] {
            assert!(
                subtract_through_cylinder(
                    &first.body,
                    &CylinderTool::new([x, 0.0, 0.0], [0.0, 0.0, 1.0], 2.0).unwrap(),
                    &MachiningPolicy::default()
                )
                .is_err(),
                "x={x}"
            );
        }
        assert!(matches!(
            subtract_through_cylinder(
                &input,
                &CylinderTool::new([100.0, 0.0, 0.0], [0.0, 0.0, 1.0], 2.0).unwrap(),
                &MachiningPolicy::default()
            ),
            Err(MachiningError::ExitCount { found: 0 })
        ));
        assert_eq!(input, stock());
    }
}
