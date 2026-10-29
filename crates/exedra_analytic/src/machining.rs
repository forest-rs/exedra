// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Transactional analytic through-cylinder subtraction on retained boundaries.
//!
//! This module edits topology, not meshes or constructor histories. It accepts
//! two planar exit patches with polygonal or complete elliptic trims,
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

mod parallel;
pub use parallel::{
    PassageError, PassageReport, PassageResult, subtract_parallel_passages,
    subtract_parallel_passages_with_roles,
};
mod stepped;
pub use stepped::{
    BoreDepth, BoreError, BoreReport, BoreResult, BoreStep, BoreTool, subtract_bore,
    subtract_bore_with_roles,
};

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
    /// Analytic removed volume between the two supported planar exits.
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
/// One transactional cylinder pattern with intermediate per-tool correspondence.
#[derive(Clone, Debug, PartialEq)]
pub struct MachiningBatchResult<F = (), E = ()> {
    /// Final retained native boundary; original input is unchanged.
    pub body: Body<F, E>,
    /// One report per tool, in request order. Source IDs in each report refer to
    /// the preceding intermediate body; table identities survive every step.
    pub operations: Vec<MachiningReport>,
}
/// An indexed pattern refusal; no partial output is published.
#[derive(Clone, Debug, PartialEq)]
pub struct MachiningBatchError {
    /// Failing tool index, or `None` for global policy/empty-pattern validation.
    pub tool: Option<usize>,
    /// The same structured cause used by a single-cylinder edit.
    pub error: MachiningError,
}
impl core::fmt::Display for MachiningBatchError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "analytic cylinder pattern at {:?}: {}",
            self.tool, self.error
        )
    }
}
impl core::error::Error for MachiningBatchError {}

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
    check_policy(policy)?;
    subtract_owned(body.clone(), tool, policy, face_role, edge_role)
}

/// Subtracts an ordered cylinder pattern with default generated metadata.
///
/// Copies the source boundary once, then transfers owned intermediate tables
/// between the same certified single-cut operations. Empty patterns validate and
/// return an unchanged copy. All tools succeed or the original input survives
/// unchanged with an indexed error; no partial pattern is returned.
/// # Errors
/// Returns the global policy/input refusal or the first failing tool and cause.
pub fn subtract_through_cylinders<F: Clone + Default, E: Clone + Default>(
    body: &Body<F, E>,
    tools: &[CylinderTool],
    policy: &MachiningPolicy,
) -> Result<MachiningBatchResult<F, E>, MachiningBatchError> {
    subtract_through_cylinders_with_roles(body, tools, policy, F::default(), E::default())
}
/// Subtracts a transactional cylinder pattern with common generated roles.
///
/// Uses the identical geometry, input/output validation and per-tool reports as
/// repeated [`subtract_through_cylinder_with_roles`] calls, while cloning source
/// geometry only once. Each pattern can carry one feature role; use individual
/// calls when generated roles differ by tool. Failed patterns leave the borrowed
/// input untouched and discard every intermediate result.
/// # Errors
/// Returns the first failing tool index and [`MachiningError`], or a global
/// policy/empty-pattern input error with no tool index.
pub fn subtract_through_cylinders_with_roles<F: Clone, E: Clone>(
    body: &Body<F, E>,
    tools: &[CylinderTool],
    policy: &MachiningPolicy,
    face_role: F,
    edge_role: E,
) -> Result<MachiningBatchResult<F, E>, MachiningBatchError> {
    let global = |error| MachiningBatchError { tool: None, error };
    check_policy(policy).map_err(global)?;
    if tools.is_empty() {
        body.validate(&ValidationPolicy {
            max_gap: policy.geometry_gap,
        })
        .map_err(|e| global(MachiningError::Boundary(e)))?;
        if !body.is_closed() {
            return Err(global(MachiningError::OpenBoundary));
        }
    }
    let mut owned = body.clone();
    let mut operations = Vec::with_capacity(tools.len());
    for (index, tool) in tools.iter().enumerate() {
        let result = subtract_owned(owned, tool, policy, face_role.clone(), edge_role.clone())
            .map_err(|error| MachiningBatchError {
                tool: Some(index),
                error,
            })?;
        operations.push(result.report);
        owned = result.body;
    }
    Ok(MachiningBatchResult {
        body: owned,
        operations,
    })
}
fn check_policy(policy: &MachiningPolicy) -> Result<(), MachiningError> {
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
    Ok(())
}
struct CylinderPlan {
    exits: Vec<(FaceId, PlaneCylinderSection)>,
    clearance: f64,
    outward: [[f64; 3]; 2],
}
fn plan_cylinder<F, E>(
    body: &Body<F, E>,
    tool: &CylinderTool,
    policy: &MachiningPolicy,
) -> Result<CylinderPlan, MachiningError> {
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
    let lower = exits[0].1.axial().coefficients();
    let upper = exits[1].1.axial().coefficients();
    let height = upper[0] - lower[0];
    // Certify the complete difference, not merely the two center heights.
    let minimum = height - (upper[1] - lower[1]).abs() - (upper[2] - lower[2]).abs();
    if !minimum.is_finite() || minimum <= policy.separation + 2.0 * roundoff {
        return Err(MachiningError::NumericLimit);
    }
    let outward = core::array::from_fn::<_, 2, _>(|i| {
        scale(normals[i], body.faces()[exits[i].0.index()].sense.sign())
    });
    if dot(outward[0], axis) >= 0.0 || dot(outward[1], axis) <= 0.0 {
        return Err(MachiningError::OpenBoundary);
    }
    Ok(CylinderPlan {
        exits,
        clearance,
        outward,
    })
}
fn subtract_owned<F: Clone, E: Clone>(
    body: Body<F, E>,
    tool: &CylinderTool,
    policy: &MachiningPolicy,
    face_role: F,
    edge_role: E,
) -> Result<MachiningResult<F, E>, MachiningError> {
    check_policy(policy)?;
    let CylinderPlan {
        exits,
        clearance,
        outward,
        ..
    } = plan_cylinder(&body, tool, policy)?;
    let validation = ValidationPolicy {
        max_gap: policy.geometry_gap,
    };
    let Surface::Cylinder(cylinder) = &tool.surface else {
        unreachable!()
    };
    let radius = cylinder.radius();
    let height = exits[1].1.axial().coefficients()[0] - exits[0].1.axial().coefficients()[0];
    // A common native azimuth keeps both copies of the axial seam aligned even
    // when the two ellipses have different principal axes or opposite senses.
    let shared_parameterization =
        exits[0].1.phase() == exits[1].1.phase() && exits[0].1.reversed() == exits[1].1.reversed();
    let source_face_count = body.faces().len();
    let source_edge_count = body.edges().len();
    for (table, count, extra) in [
        ("vertices", body.vertices().len(), 2),
        ("edges", source_edge_count, 3),
        ("loops", body.loops().len(), 3),
        ("faces", source_face_count, 1),
    ] {
        let count = count
            .checked_add(extra)
            .ok_or(MachiningError::NumericLimit)?;
        if u32::try_from(count).is_err() {
            return Err(MachiningError::Boundary(BodyError::TooLarge {
                table,
                count,
            }));
        }
    }
    let mut parts = body.into_parts();
    let ranges = core::array::from_fn::<_, 2, _>(|i| {
        let start = if shared_parameterization {
            0.0
        } else {
            -if exits[i].1.reversed() { -1.0 } else { 1.0 } * exits[i].1.phase()
        };
        ParameterRange::new(start, start + core::f64::consts::TAU).expect("bounded section phase")
    });
    let rims = core::array::from_fn(|i| EdgeId(crate::usize_to_u32(parts.edges.len() + i)));
    let vertices = core::array::from_fn::<_, 2, _>(|i| {
        VertexId(crate::usize_to_u32(parts.vertices.len() + i))
    });
    let mut reversed = [false; 2];
    for i in 0..2 {
        let curve = exits[i].1.curve().clone();
        parts.vertices.push(
            curve
                .evaluate(ranges[i].bounds()[0])
                .ok_or(MachiningError::NumericLimit)?
                .point,
        );
        parts.edges.push(Edge::new(
            curve,
            ranges[i],
            vertices[i],
            vertices[i],
            edge_role.clone(),
        ));
        reversed[i] = dot(col(exits[i].1.ellipse().frame(), 2), outward[i]) > 0.0;
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
    let rim_trims = core::array::from_fn::<_, 2, _>(|i| {
        let mut trim = section_trim(&exits[i].1, ranges[i]);
        if !shared_parameterization
            && exits[i].1.reversed()
            && let TrimCurve::Harmonic { origin, .. } = &mut trim
        {
            origin[0] += core::f64::consts::TAU;
        }
        trim
    });
    let lower_fraction = if !reversed[0] { 0.0 } else { 1.0 };
    let upper_fraction = if !reversed[1] { 1.0 } else { 0.0 };
    let seam_trim = |opposite: bool| -> Result<TrimCurve, MachiningError> {
        Ok(TrimCurve::Line {
            start: rim_trims[0]
                .evaluate(if opposite {
                    1.0 - lower_fraction
                } else {
                    lower_fraction
                })
                .ok_or(MachiningError::NumericLimit)?,
            end: rim_trims[1]
                .evaluate(if opposite {
                    1.0 - upper_fraction
                } else {
                    upper_fraction
                })
                .ok_or(MachiningError::NumericLimit)?,
        })
    };
    parts.trims.push(LoopTrims {
        loop_id,
        curves: alloc::vec![
            rim_trims[0].clone(),
            seam_trim(false)?,
            rim_trims[1].clone(),
            seam_trim(true)?
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
    let area = exits
        .iter()
        .map(|(_, section)| {
            core::f64::consts::PI * section.ellipse().semi_major() * section.ellipse().semi_minor()
        })
        .sum::<f64>();
    let removed_volume = core::f64::consts::PI * radius * radius * height;
    let surface_area_delta = core::f64::consts::TAU * radius * height - area;
    if !removed_volume.is_finite() || !surface_area_delta.is_finite() {
        return Err(MachiningError::NumericLimit);
    }
    Ok(MachiningResult {
        body: output,
        report: MachiningReport {
            faces: (0..source_face_count)
                .map(|i| {
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
            retained_edges: (0..source_edge_count)
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
        if coedges.iter().any(|co| {
            matches!(
                body.edges()[co.edge.index()].regular(),
                Some((Curve::Circle(_), _))
            )
        }) {
            let (profile, gap) = body
                .planar_profile_loop(face, *l)
                .ok_or(MachiningError::UnsupportedFace { face })?;
            let point = f
                .surface
                .parameters(center)
                .ok_or(MachiningError::NumericLimit)?;
            let query = profile
                .query_point(point)
                .map_err(|_| MachiningError::NumericLimit)?;
            let bound = query.boundary_distance - query.numeric_allowance - gap - radius;
            let correct_side = query.winding.is_some_and(|winding| {
                if loop_index == 0 {
                    winding.abs() == 1
                } else {
                    winding == 0
                }
            });
            if !correct_side || bound <= 0.0 {
                return Err(MachiningError::Clearance {
                    face,
                    bound: bound.min(0.0),
                    required: radius,
                });
            }
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
    fn overlapping_parallel_passages_split_shared_native_walls_and_count_overlap_once() {
        let source = stock();
        let before = source.clone();
        let tools = [
            CylinderTool::new([-2.0, 0.0, 0.0], [0.0, 0.0, 1.0], 3.0).unwrap(),
            CylinderTool::new([2.0, 0.0, 7.0], [0.0, 0.0, -1.0], 3.0).unwrap(),
        ];
        let result =
            subtract_parallel_passages(&source, &tools, &MachiningPolicy::default()).unwrap();
        assert_eq!(source, before);
        assert!(result.body.is_closed());
        assert_eq!(result.report.walls.len(), 2);
        assert_eq!(result.report.intersection_vertices, 2);
        assert_eq!(result.report.opening_components, 1);
        let overlap = 18.0 * libm::acos(2.0 / 3.0) - 2.0 * libm::sqrt(20.0);
        let area = 18.0 * core::f64::consts::PI - overlap;
        assert!((result.report.removed_section_area - area).abs() < 1e-10);
        assert!((result.report.removed_volume - 10.0 * area).abs() < 1e-10);
        assert!(
            (result.report.surface_area_delta
                - (12.0 * (core::f64::consts::PI - libm::acos(2.0 / 3.0)) * 10.0 - 2.0 * area))
                .abs()
                < 1e-9
        );
        let next = BoreTool::new(
            [12.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            &[BoreStep {
                radius: 1.0,
                end: BoreDepth::ToDepth(3.0),
            }],
        )
        .unwrap();
        let continued = subtract_bore(&result.body, &next, &MachiningPolicy::default()).unwrap();
        assert!(continued.body.is_closed());
        assert_eq!(
            &continued.body.edges()[..result.body.edges().len()],
            result.body.edges()
        );
        for tolerance in [0.03, 0.005] {
            let mesh = result
                .body
                .realize(
                    Placement3::IDENTITY,
                    &crate::realize::RealizationPolicy {
                        tolerance,
                        ..Default::default()
                    },
                )
                .unwrap();
            assert!(mesh.mesh.validate_deep().is_empty());
            assert!(mesh.mesh.boundary_loops().unwrap().is_empty());
            assert!(mesh.report.within_tolerance(), "{:?}", mesh.report);
            let error =
                (realized_volume(&mesh.mesh) - (24000.0 - result.report.removed_volume)).abs();
            assert!(error < if tolerance > 0.01 { 8.0 } else { 2.0 }, "{error}");
        }
    }
    #[test]
    fn parallel_union_handles_chain_covered_arcs_disjoint_and_contained_tools() {
        for requests in [
            alloc::vec![([-5.0, 0.0], 4.0), ([0.0, 0.0], 4.0), ([5.0, 0.0], 4.0)],
            alloc::vec![([-3.0, 0.0], 4.0), ([3.0, 0.0], 4.0), ([0.0, 5.0], 4.0)],
            alloc::vec![([-10.0, 0.0], 2.0), ([10.0, 0.0], 2.0)],
            alloc::vec![([0.0, 0.0], 2.0), ([0.0, 0.0], 4.0)],
        ] {
            let tools: Vec<_> = requests
                .iter()
                .map(|&(p, r)| CylinderTool::new([p[0], p[1], 0.0], [0.0, 0.0, 1.0], r).unwrap())
                .collect();
            let result =
                subtract_parallel_passages(&stock(), &tools, &MachiningPolicy::default()).unwrap();
            let mesh = result
                .body
                .realize(
                    Placement3::IDENTITY,
                    &crate::realize::RealizationPolicy {
                        tolerance: 0.02,
                        ..Default::default()
                    },
                )
                .unwrap();
            assert!(mesh.mesh.validate_deep().is_empty());
            assert!(mesh.mesh.boundary_loops().unwrap().is_empty());
            assert!(mesh.report.within_tolerance());
            assert!(result.report.removed_section_area > 0.0);
        }
    }
    #[test]
    fn parallel_union_refuses_tangencies_skew_axes_and_enclosed_islands() {
        let source = stock();
        let original = source.clone();
        for distance in [0.0, 4.0, 4.0 - 1e-8, 4.0 + 1e-8] {
            let tools = [
                CylinderTool::new([0.0; 3], [0.0, 0.0, 1.0], 2.0).unwrap(),
                CylinderTool::new([distance, 0.0, 0.0], [0.0, 0.0, 1.0], 2.0).unwrap(),
            ];
            assert!(matches!(
                subtract_parallel_passages(&source, &tools, &MachiningPolicy::default()),
                Err(PassageError::Contact { .. })
            ));
        }
        let tools = [
            CylinderTool::new([-5.0, 0.0, 0.0], [0.0, 0.0, 1.0], 2.0).unwrap(),
            CylinderTool::new([5.0, 0.0, 0.0], [0.01, 0.0, 1.0], 2.0).unwrap(),
        ];
        assert!(matches!(
            subtract_parallel_passages(&source, &tools, &MachiningPolicy::default()),
            Err(PassageError::Axis { tool: 1 })
        ));
        // Three disks with pairwise overlaps but a central uncovered island.
        let tools: Vec<_> = (0..3)
            .map(|i| {
                let a = f64::from(i) * core::f64::consts::TAU / 3.0;
                CylinderTool::new(
                    [5.0 * libm::cos(a), 5.0 * libm::sin(a), 0.0],
                    [0.0, 0.0, 1.0],
                    4.5,
                )
                .unwrap()
            })
            .collect();
        assert!(matches!(
            subtract_parallel_passages(&source, &tools, &MachiningPolicy::default()),
            Err(PassageError::EnclosedIsland)
        ));
        assert_eq!(source, original);
    }
    fn realized_volume(mesh: &exedra_mesh::Mesh) -> f64 {
        mesh.faces()
            .map(|face| {
                let p: Vec<_> = mesh
                    .face_loop(face)
                    .map(|c| {
                        mesh.vertex_position(mesh.from_vertex(c).unwrap())
                            .unwrap()
                            .map(f64::from)
                    })
                    .collect();
                dot(p[0], cross(p[1], p[2])) / 6.0
            })
            .sum()
    }
    #[test]
    fn blind_and_stepped_bores_share_native_shoulders_and_keep_exact_integrals() {
        for reverse in [false, true] {
            let entry = if reverse { [0.0, 0.0, 10.0] } else { [0.0; 3] };
            let direction = [0.0, 0.0, if reverse { -1.0 } else { 1.0 }];
            for through in [false, true] {
                let steps = [
                    BoreStep {
                        radius: 5.0,
                        end: BoreDepth::ToDepth(2.0),
                    },
                    BoreStep {
                        radius: 3.0,
                        end: if through {
                            BoreDepth::Through
                        } else {
                            BoreDepth::ToDepth(7.0)
                        },
                    },
                ];
                let tool = BoreTool::new(entry, direction, &steps).unwrap();
                let source = stock();
                let before = source.clone();
                let result = subtract_bore(&source, &tool, &MachiningPolicy::default()).unwrap();
                assert_eq!(source, before);
                assert!(result.body.is_closed());
                assert_eq!(result.report.walls.len(), 2);
                assert_eq!(result.report.shoulders.len(), 1);
                assert_eq!(result.report.rims.len(), 4);
                assert_eq!(result.report.floor.is_some(), !through);
                assert_eq!(result.report.exit.is_some(), through);
                let removed =
                    core::f64::consts::PI * (50.0 + 9.0 * if through { 8.0 } else { 5.0 });
                assert!((result.report.removed_volume - removed).abs() < 1e-10);
                let walls = core::f64::consts::TAU * (10.0 + 3.0 * if through { 8.0 } else { 5.0 });
                let area = walls
                    + core::f64::consts::PI * (16.0 + if through { -9.0 } else { 9.0 } - 25.0);
                assert!((result.report.surface_area_delta - area).abs() < 1e-10);
                for &face in result
                    .report
                    .walls
                    .iter()
                    .chain(&result.report.shoulders)
                    .chain(result.report.floor.iter())
                {
                    assert!(result.body.has_explicit_trims(face));
                }
                for tolerance in [0.03, 0.005] {
                    let realized = result
                        .body
                        .realize(
                            Placement3::IDENTITY,
                            &crate::realize::RealizationPolicy {
                                tolerance,
                                ..Default::default()
                            },
                        )
                        .unwrap();
                    assert!(realized.mesh.validate_deep().is_empty());
                    assert!(realized.mesh.boundary_loops().unwrap().is_empty());
                    assert!(realized.report.within_tolerance(), "{:?}", realized.report);
                    let difference = (realized_volume(&realized.mesh) - (24000.0 - removed)).abs();
                    assert!(difference < 5.0, "{difference}");
                }
            }
        }
    }
    #[test]
    fn stepped_bores_accept_oblique_entries_nonparallel_exits_and_later_disjoint_cuts() {
        let wedge = crate::halfspace::intersect_half_space(
            &stock(),
            &crate::halfspace::HalfSpace::new([0.0, 0.0, 7.0], [-0.15, 0.1, 1.0]).unwrap(),
            &crate::halfspace::ClipPolicy::default(),
        )
        .unwrap()
        .body;
        for sign in [-1.0, 1.0] {
            let entry = if sign > 0.0 {
                [0.0; 3]
            } else {
                [0.0, 0.0, 7.0]
            };
            for end in [BoreDepth::ToDepth(5.0), BoreDepth::Through] {
                let tool = BoreTool::new(
                    entry,
                    [sign * 0.2, sign * 0.1, sign],
                    &[
                        BoreStep {
                            radius: 3.0,
                            end: BoreDepth::ToDepth(1.5),
                        },
                        BoreStep { radius: 1.5, end },
                    ],
                )
                .unwrap();
                let result = subtract_bore(&wedge, &tool, &MachiningPolicy::default()).unwrap();
                let realized = result
                    .body
                    .realize(
                        Placement3::IDENTITY,
                        &crate::realize::RealizationPolicy {
                            tolerance: 0.02,
                            ..Default::default()
                        },
                    )
                    .unwrap();
                assert!(realized.mesh.validate_deep().is_empty());
                assert!(realized.mesh.boundary_loops().unwrap().is_empty());
                assert!(realized.report.within_tolerance());
                let next = BoreTool::new(
                    [-15.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0],
                    &[BoreStep {
                        radius: 1.0,
                        end: BoreDepth::ToDepth(2.0),
                    }],
                )
                .unwrap();
                let again =
                    subtract_bore(&result.body, &next, &MachiningPolicy::default()).unwrap();
                assert_eq!(
                    &again.body.edges()[..result.body.edges().len()],
                    result.body.edges()
                );
                assert!(again.body.is_closed());
            }
        }
    }
    #[test]
    fn blind_bore_refusals_are_indexed_and_leave_the_source_unchanged() {
        let source = stock();
        let before = source.clone();
        for (entry, depth) in [
            ([0.0; 3], 10.0),
            ([0.0; 3], 10.0 - 1e-9),
            ([0.0, 0.0, -1.0], 4.0),
        ] {
            let tool = BoreTool::new(
                entry,
                [0.0, 0.0, 1.0],
                &[BoreStep {
                    radius: 2.0,
                    end: BoreDepth::ToDepth(depth),
                }],
            )
            .unwrap();
            let error = subtract_bore(&source, &tool, &MachiningPolicy::default()).unwrap_err();
            match &error {
                BoreError::DepthClearance { step, .. } => {
                    assert_eq!(*step, 0);
                    let message = alloc::format!("{error}");
                    assert!(message.contains("bore step 1"));
                    assert!(message.contains("model units"));
                }
                BoreError::Entry { face, .. } => assert!(face.index() < source.faces().len()),
                _ => panic!("unexpected bore refusal: {error}"),
            }
        }
        let tool = BoreTool::new(
            [0.0; 3],
            [0.0, 0.0, 1.0],
            &[
                BoreStep {
                    radius: 2.0,
                    end: BoreDepth::ToDepth(4.0),
                },
                BoreStep {
                    radius: 2.0,
                    end: BoreDepth::Through,
                },
            ],
        );
        assert!(matches!(tool, Err(BoreError::InvalidStep { step: 1 })));
        assert_eq!(source, before);
    }
    #[test]
    fn nonparallel_exits_keep_complete_trim_correspondence_and_exact_integrals() {
        let wedge = crate::halfspace::intersect_half_space(
            &stock(),
            &crate::halfspace::HalfSpace::new([0.0, 0.0, 7.0], [-0.15, 0.1, 1.0]).unwrap(),
            &crate::halfspace::ClipPolicy::default(),
        )
        .unwrap()
        .body;
        for sign in [-1.0, 1.0] {
            let result = subtract_through_cylinder(
                &wedge,
                &CylinderTool::new([0.0; 3], [0.0, 0.0, sign], 2.0).unwrap(),
                &MachiningPolicy::default(),
            )
            .unwrap();
            assert!(result.body.is_closed());
            assert!((result.report.removed_volume - core::f64::consts::PI * 28.0).abs() < 1e-10);
            let expected_area = core::f64::consts::TAU * 14.0
                - core::f64::consts::PI * 4.0 * (1.0 + libm::sqrt(1.0325));
            assert!((result.report.surface_area_delta - expected_area).abs() < 1e-10);
            let realized = result
                .body
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
            assert!(realized.report.within_tolerance(), "{:?}", realized.report);
            assert_eq!(realized.report.work.trim_normal_corners, 0);
            assert!(realized.report.work.completed_triangles < 1000);
        }
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
    fn owned_pattern_matches_individual_geometry_metadata_and_history() {
        let source = stock().map_roles(|_| alloc::vec![7_u32], |_| alloc::vec![9_u32]);
        let original = source.clone();
        let tools = [-15.0, 0.0, 15.0]
            .map(|x| CylinderTool::new([x, 0.0, 0.0], [0.1, 0.0, 1.0], 2.0).unwrap());
        let mut expected = source.clone();
        let mut history = Vec::new();
        for tool in &tools {
            let result = subtract_through_cylinder_with_roles(
                &expected,
                tool,
                &MachiningPolicy::default(),
                alloc::vec![20],
                alloc::vec![30],
            )
            .unwrap();
            history.push(result.report);
            expected = result.body;
        }
        let batch = subtract_through_cylinders_with_roles(
            &source,
            &tools,
            &MachiningPolicy::default(),
            alloc::vec![20],
            alloc::vec![30],
        )
        .unwrap();
        assert_eq!(source, original);
        assert_eq!(batch.body, expected);
        assert_eq!(batch.operations, history);
        for report in &batch.operations {
            assert_eq!(
                batch.body.faces()[report.wall.index()].role,
                alloc::vec![20]
            );
        }
    }
    #[test]
    fn pattern_failure_is_indexed_and_discards_all_intermediate_edits() {
        let source = stock();
        let original = source.clone();
        let tools = [-15.0, 15.0, -14.0]
            .map(|x| CylinderTool::new([x, 0.0, 0.0], [0.0, 0.0, 1.0], 2.0).unwrap());
        let error =
            subtract_through_cylinders(&source, &tools, &MachiningPolicy::default()).unwrap_err();
        assert_eq!(error.tool, Some(2));
        assert!(matches!(error.error, MachiningError::Clearance { .. }));
        assert_eq!(source, original);
        let unchanged =
            subtract_through_cylinders(&source, &[], &MachiningPolicy::default()).unwrap();
        assert_eq!(unchanged.body, source);
        assert!(unchanged.operations.is_empty());
        let error = subtract_through_cylinders(
            &source,
            &[],
            &MachiningPolicy {
                separation: 0.0,
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(error.tool, None);
        assert_eq!(error.error, MachiningError::InvalidPolicy);
        let error = subtract_through_cylinders(
            &Body::<(), ()>::default(),
            &[],
            &MachiningPolicy::default(),
        )
        .unwrap_err();
        assert_eq!(error.tool, None);
        assert_eq!(error.error, MachiningError::OpenBoundary);
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
