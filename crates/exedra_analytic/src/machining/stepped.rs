// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Finite coaxial bore profiles built in one retained-boundary transaction.
use super::*;
use crate::body::BodyParts;
use crate::geometry::intersection::AxialHarmonic;

/// How one cylindrical part of a bore ends, measured along its inward axis.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum BoreDepth {
    /// Cumulative positive distance from the entry point, in model units.
    ToDepth(f64),
    /// Continue through the opposite supported planar face; only valid last.
    Through,
}
/// One cylindrical portion of a stepped bore, ordered from the entry inward.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BoreStep {
    /// Positive radius; every later step must be strictly smaller.
    pub radius: f64,
    /// Cumulative end depth, or a final through termination.
    pub end: BoreDepth,
}
/// A checked finite or through coaxial cutting profile.
///
/// The entry point must lie on the source entry plane. Direction points into
/// material. Finite depths increase strictly and radii decrease strictly, so
/// shoulders are complete annuli. At most 256 steps are accepted. This describes
/// the whole cut, including shared shoulders; it is not overlapping cylinders.
#[derive(Clone, Debug, PartialEq)]
pub struct BoreTool {
    cylinder: CylinderTool,
    steps: Vec<BoreStep>,
}
impl BoreTool {
    /// Constructs a stepped bore in body-local model units.
    /// # Errors
    /// Refuses invalid geometry, an empty/oversized profile, nondecreasing
    /// radii, nonincreasing positive depths or a nonfinal through step.
    pub fn new(
        entry: [f64; 3],
        direction: [f64; 3],
        steps: &[BoreStep],
    ) -> Result<Self, BoreError> {
        if steps.is_empty() || steps.len() > 256 {
            return Err(BoreError::StepCount {
                found: steps.len(),
                maximum: 256,
            });
        }
        let mut radius = f64::INFINITY;
        let mut depth = 0.0;
        for (index, step) in steps.iter().enumerate() {
            if !step.radius.is_finite() || step.radius <= 0.0 || step.radius >= radius {
                return Err(BoreError::InvalidStep { step: index });
            }
            match step.end {
                BoreDepth::ToDepth(end) if end.is_finite() && end > depth => depth = end,
                BoreDepth::Through if index + 1 == steps.len() => (),
                _ => return Err(BoreError::InvalidStep { step: index }),
            }
            radius = step.radius;
        }
        let cylinder = CylinderTool::new(entry, direction, steps[0].radius)
            .map_err(|e| BoreError::Machining(MachiningError::Geometry(e)))?;
        Ok(Self {
            cylinder,
            steps: steps.to_vec(),
        })
    }
    /// Checked cylindrical portions, in entry-to-interior order.
    #[must_use]
    pub fn steps(&self) -> &[BoreStep] {
        &self.steps
    }
    /// Entry position in body-local model coordinates.
    #[must_use]
    pub fn entry(&self) -> [f64; 3] {
        col(self.cylinder.surface.frame(), 3)
    }
    /// Normalized axis pointing from the entry into material.
    #[must_use]
    pub fn direction(&self) -> [f64; 3] {
        col(self.cylinder.surface.frame(), 2)
    }
}
/// A bore could not be installed; the borrowed input remains unchanged.
#[derive(Clone, Debug, PartialEq)]
pub enum BoreError {
    /// The cutting profile is empty or exceeds the explicit step limit.
    StepCount {
        /// Requested portions.
        found: usize,
        /// Largest supported count.
        maximum: usize,
    },
    /// A step radius/depth/order is invalid.
    InvalidStep {
        /// Zero-based offending profile portion.
        step: usize,
    },
    /// Shared cylinder planning, topology or numeric validation failed.
    Machining(MachiningError),
    /// The declared entry is not on the first source plane along the inward axis.
    Entry {
        /// First intersected source face.
        face: FaceId,
        /// Absolute axial distance from the entry to that plane at the axis.
        distance: f64,
        /// Largest permitted distance, including arithmetic evidence.
        allowed: f64,
    },
    /// A shoulder/floor lacks clearance from another ring or the source boundary.
    DepthClearance {
        /// Zero-based finite profile portion, or final through portion.
        step: usize,
        /// Conservative material/axial clearance lower bound.
        bound: f64,
        /// Required positive clearance.
        required: f64,
    },
}
impl core::fmt::Display for BoreError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::StepCount { found, maximum } => write!(
                f,
                "a bore requires between 1 and {maximum} steps; received {found}"
            ),
            Self::InvalidStep { step } => write!(
                f,
                "bore step {} requires a positive radius smaller than the previous step and an increasing positive depth; through termination must be last",
                step.saturating_add(1)
            ),
            Self::Machining(error) => write!(f, "bore could not be cut: {error}"),
            Self::Entry {
                face,
                distance,
                allowed,
            } => write!(
                f,
                "bore entry is {distance} model units from source face {}; maximum allowed distance is {allowed} model units",
                face.index()
            ),
            Self::DepthClearance {
                step,
                bound,
                required,
            } => write!(
                f,
                "bore step {} requires clearance of {required} model units; bounded clearance is {bound} model units",
                step.saturating_add(1)
            ),
        }
    }
}
impl core::error::Error for BoreError {}
impl From<MachiningError> for BoreError {
    fn from(error: MachiningError) -> Self {
        Self::Machining(error)
    }
}
/// Source correspondence and exact-form integral changes from a complete bore.
#[derive(Clone, Debug, PartialEq)]
pub struct BoreReport {
    /// Source face images, including unchanged faces, in source table order.
    pub faces: Vec<FaceChange>,
    /// Every original edge, preserving its local identity.
    pub retained_edges: Vec<EdgeId>,
    /// Source planar face receiving the entry opening.
    pub entry: FaceId,
    /// Source planar exit face for a final through portion; absent for blind bores.
    pub exit: Option<FaceId>,
    /// New ring edges in construction order, shared by adjacent faces.
    pub rims: Vec<EdgeId>,
    /// Inward cylindrical walls, in profile order.
    pub walls: Vec<FaceId>,
    /// Annular shoulder faces, in profile order.
    pub shoulders: Vec<FaceId>,
    /// Planar terminal disk for a blind bore.
    pub floor: Option<FaceId>,
    /// Conservative minimum untouched-boundary/ring clearance in model units.
    pub min_clearance: f64,
    /// Largest complete-support agreement bound in model units.
    pub max_gap: f64,
    /// Analytic volume removed by the entire profile.
    pub removed_volume: f64,
    /// Analytic boundary-area change: added cavity minus removed openings.
    pub surface_area_delta: f64,
}
/// Complete retained output and its source correspondence; no partial mutation.
#[derive(Clone, Debug, PartialEq)]
pub struct BoreResult<F = (), E = ()> {
    /// New body with unchanged source identities and exact native supports/trims.
    pub body: Body<F, E>,
    /// Created geometry, source history and geometric evidence.
    pub report: BoreReport,
}

/// Subtracts a blind, stepped or counterbored passage with default generated roles.
///
/// Current planning proves a complete outer-radius corridor between two planar
/// source exits, even for a blind bore. Thus valid finite cuts may be refused
/// when their infinitely extended outer cylinder would meet another boundary.
/// Other cuts must remain separated. The source must be an embedded outward
/// regular solid; local incidence alone does not establish that premise.
/// # Errors
/// Returns [`BoreError`] transactionally, without changing `body`.
pub fn subtract_bore<F: Clone + Default, E: Clone + Default>(
    body: &Body<F, E>,
    tool: &BoreTool,
    policy: &MachiningPolicy,
) -> Result<BoreResult<F, E>, BoreError> {
    subtract_bore_with_roles(body, tool, policy, F::default(), E::default())
}
/// Subtracts a complete bore profile, assigning roles to generated faces/edges.
/// # Errors
/// Refuses unsupported source corridors, entry mismatch, insufficient shoulder
/// or floor clearance, invalid policy and nonrepresentable output geometry.
pub fn subtract_bore_with_roles<F: Clone, E: Clone>(
    body: &Body<F, E>,
    tool: &BoreTool,
    policy: &MachiningPolicy,
    face_role: F,
    edge_role: E,
) -> Result<BoreResult<F, E>, BoreError> {
    check_policy(policy)?;
    let plan = plan_cylinder(body, &tool.cylinder, policy)?;
    let entry = plan.exits[0].0;
    let entry_height = plan.exits[0].1.axial().coefficients()[0];
    if entry_height.abs() > policy.geometry_gap {
        return Err(BoreError::Entry {
            face: entry,
            distance: entry_height.abs(),
            allowed: policy.geometry_gap,
        });
    }
    let frame = tool.cylinder.surface.frame();
    let axis = col(frame, 2);
    let normal = plan.outward[1];
    let normal_axis = dot(normal, axis).abs() / norm(normal);
    let entry_axis = dot(plan.outward[0], axis).abs() / norm(plan.outward[0]);
    let numeric = 1024.0
        * f64::EPSILON
        * body
            .vertices()
            .iter()
            .flatten()
            .chain(tool.entry().iter())
            .copied()
            .map(f64::abs)
            .fold(tool.steps[0].radius, f64::max);
    let mut clearance = plan.clearance;
    let mut previous = plan.exits[0].1.axial();
    for (i, step) in tool.steps.iter().enumerate() {
        let bound = match step.end {
            BoreDepth::ToDepth(depth) => {
                let from_previous =
                    (depth - previous.bounds()[1]) * if i == 0 { entry_axis } else { 1.0 };
                let to_opposite = (plan.exits[1].1.axial().bounds()[0] - depth) * normal_axis;
                let radial = tool
                    .steps
                    .get(i + 1)
                    .map_or(f64::INFINITY, |next| step.radius - next.radius);
                previous = AxialHarmonic::new(depth, 0.0, 0.0).map_err(MachiningError::Geometry)?;
                from_previous.min(to_opposite).min(radial) - numeric
            }
            BoreDepth::Through => {
                (plan.exits[1].1.axial().bounds()[0] - previous.bounds()[1]) * normal_axis - numeric
            }
        };
        if !bound.is_finite() || bound < policy.separation {
            return Err(BoreError::DepthClearance {
                step: i,
                bound,
                required: policy.separation,
            });
        }
        clearance = clearance.min(bound);
    }
    // Explicit table preflight precedes the one source clone and all new geometry.
    let n = tool.steps.len();
    for (table, count, extra) in [
        ("vertices", body.vertices().len(), 2 * n),
        ("edges", body.edges().len(), 3 * n),
        ("loops", body.loops().len(), 4 * n + 1),
        ("faces", body.faces().len(), 2 * n),
    ] {
        let total = count
            .checked_add(extra)
            .ok_or(MachiningError::NumericLimit)?;
        if u32::try_from(total).is_err() {
            return Err(MachiningError::Boundary(BodyError::TooLarge {
                table,
                count: total,
            })
            .into());
        }
    }
    let source_faces = body.faces().len();
    let source_edges = body.edges().len();
    let mut parts = body.clone().into_parts();
    parts.vertices.reserve(2 * n);
    parts.edges.reserve(3 * n);
    parts.loops.reserve(3 * n);
    parts.faces.reserve(2 * n);
    parts.trims.reserve(3 * n);
    let mut report = BoreReport {
        faces: Vec::new(),
        retained_edges: (0..source_edges)
            .map(|i| EdgeId(crate::usize_to_u32(i)))
            .collect(),
        entry,
        exit: None,
        rims: Vec::with_capacity(2 * n),
        walls: Vec::with_capacity(n),
        shoulders: Vec::with_capacity(n - 1),
        floor: None,
        min_clearance: clearance,
        max_gap: 0.0,
        removed_volume: 0.0,
        surface_area_delta: 0.0,
    };
    let mut lower = add_section_ring(&mut parts, &plan.exits[0].1, edge_role.clone())?;
    report.rims.push(lower.edge);
    append_source_opening(&mut parts, entry, &lower, false)?;
    report.surface_area_delta -= lower.area;
    for (i, step) in tool.steps.iter().enumerate() {
        let upper = match step.end {
            BoreDepth::ToDepth(depth) => {
                add_circle_ring(&mut parts, frame, step.radius, depth, edge_role.clone())?
            }
            BoreDepth::Through => {
                let far = plan.exits[1].0;
                let Surface::Plane(plane) = &body.faces()[far.index()].surface else {
                    unreachable!()
                };
                let Surface::Cylinder(cylinder) =
                    Surface::cylinder(frame, step.radius).map_err(MachiningError::Geometry)?
                else {
                    unreachable!()
                };
                let section = plane_cylinder(
                    plane,
                    &cylinder,
                    &SectionPolicy {
                        min_axis_dot: policy.min_axis_dot,
                        max_gap: policy.geometry_gap,
                    },
                )
                .map_err(MachiningError::Section)?;
                report.max_gap = report.max_gap.max(section.max_gap());
                let ring = add_section_ring(&mut parts, &section, edge_role.clone())?;
                append_source_opening(&mut parts, far, &ring, true)?;
                report.exit = Some(far);
                report.surface_area_delta -= ring.area;
                ring
            }
        };
        report.rims.push(upper.edge);
        let height = upper.axial.coefficients()[0] - lower.axial.coefficients()[0];
        report.removed_volume += core::f64::consts::PI * step.radius * step.radius * height;
        report.surface_area_delta += core::f64::consts::TAU * step.radius * height;
        report.walls.push(append_wall(
            &mut parts,
            frame,
            step.radius,
            &lower,
            &upper,
            policy.geometry_gap,
            face_role.clone(),
            edge_role.clone(),
        )?);
        if let Some(next) = tool.steps.get(i + 1) {
            let BoreDepth::ToDepth(depth) = step.end else {
                unreachable!()
            };
            let inner = add_circle_ring(&mut parts, frame, next.radius, depth, edge_role.clone())?;
            report.rims.push(inner.edge);
            report.shoulders.push(append_floor(
                &mut parts,
                frame,
                depth,
                &upper,
                Some(&inner),
                face_role.clone(),
            )?);
            report.surface_area_delta += upper.area - inner.area;
            lower = inner;
        } else if let BoreDepth::ToDepth(depth) = step.end {
            report.floor = Some(append_floor(
                &mut parts,
                frame,
                depth,
                &upper,
                None,
                face_role.clone(),
            )?);
            report.surface_area_delta += upper.area;
        }
    }
    let output = Body { parts };
    if !output.is_closed() {
        return Err(MachiningError::OpenBoundary.into());
    }
    let checked = output
        .validate(&ValidationPolicy {
            max_gap: policy.geometry_gap,
        })
        .map_err(MachiningError::Boundary)?;
    report.max_gap = report
        .max_gap
        .max(checked.max_gap)
        .max(plan.exits[0].1.max_gap());
    if !report.removed_volume.is_finite() || !report.surface_area_delta.is_finite() {
        return Err(MachiningError::NumericLimit.into());
    }
    report.faces = (0..source_faces)
        .map(|i| {
            let id = FaceId(crate::usize_to_u32(i));
            FaceChange {
                source: id,
                result: id,
                kind: if id == entry || Some(id) == report.exit {
                    FaceChangeKind::Trimmed
                } else {
                    FaceChangeKind::Retained
                },
            }
        })
        .collect();
    Ok(BoreResult {
        body: output,
        report,
    })
}

struct Ring {
    edge: EdgeId,
    vertex: VertexId,
    trim: TrimCurve,
    forward: bool,
    axial: AxialHarmonic,
    area: f64,
}
fn add_section_ring<F, E>(
    parts: &mut BodyParts<F, E>,
    section: &PlaneCylinderSection,
    role: E,
) -> Result<Ring, MachiningError> {
    let sign = if section.reversed() { -1.0 } else { 1.0 };
    let start = -sign * section.phase();
    let range = ParameterRange::new(start, start + core::f64::consts::TAU)
        .map_err(MachiningError::Geometry)?;
    let curve = section.curve().clone();
    let vertex = VertexId(crate::usize_to_u32(parts.vertices.len()));
    parts.vertices.push(
        curve
            .evaluate(start)
            .ok_or(MachiningError::NumericLimit)?
            .point,
    );
    let edge = EdgeId(crate::usize_to_u32(parts.edges.len()));
    parts
        .edges
        .push(Edge::new(curve, range, vertex, vertex, role));
    let mut trim = section_trim(section, range);
    if section.reversed()
        && let TrimCurve::Harmonic { origin, .. } = &mut trim
    {
        origin[0] += core::f64::consts::TAU;
    }
    Ok(Ring {
        edge,
        vertex,
        trim,
        forward: !section.reversed(),
        axial: section.axial(),
        area: core::f64::consts::PI
            * section.ellipse().semi_major()
            * section.ellipse().semi_minor(),
    })
}
fn add_circle_ring<F, E>(
    parts: &mut BodyParts<F, E>,
    frame: Placement3,
    radius: f64,
    depth: f64,
    role: E,
) -> Result<Ring, MachiningError> {
    let center = add(col(frame, 3), scale(col(frame, 2), depth));
    let frame = Placement3::from_axes(col(frame, 0), col(frame, 1), col(frame, 2), center);
    let curve = Curve::circle(frame, radius).map_err(MachiningError::Geometry)?;
    let range =
        ParameterRange::new(0.0, core::f64::consts::TAU).map_err(MachiningError::Geometry)?;
    let vertex = VertexId(crate::usize_to_u32(parts.vertices.len()));
    parts.vertices.push(
        curve
            .evaluate(0.0)
            .ok_or(MachiningError::NumericLimit)?
            .point,
    );
    let edge = EdgeId(crate::usize_to_u32(parts.edges.len()));
    parts
        .edges
        .push(Edge::new(curve, range, vertex, vertex, role));
    Ok(Ring {
        edge,
        vertex,
        trim: TrimCurve::Line {
            start: [0.0, depth],
            end: [core::f64::consts::TAU, depth],
        },
        forward: true,
        axial: AxialHarmonic::new(depth, 0.0, 0.0).map_err(MachiningError::Geometry)?,
        area: core::f64::consts::PI * radius * radius,
    })
}
fn append_source_opening<F, E>(
    parts: &mut BodyParts<F, E>,
    face: FaceId,
    ring: &Ring,
    far: bool,
) -> Result<(), MachiningError> {
    let loop_id = LoopId(crate::usize_to_u32(parts.loops.len()));
    parts.loops.push(BoundaryLoop {
        coedges: alloc::vec![Coedge {
            edge: ring.edge,
            reversed: if far { ring.forward } else { !ring.forward }
        }],
    });
    parts.faces[face.index()].loops.push(loop_id);
    crate::operation_trims::append_planar_loop(parts, face, loop_id)
        .map_err(MachiningError::Geometry)
}
fn append_wall<F, E>(
    parts: &mut BodyParts<F, E>,
    frame: Placement3,
    radius: f64,
    lower: &Ring,
    upper: &Ring,
    gap: f64,
    role: F,
    edge_role: E,
) -> Result<FaceId, MachiningError> {
    let seam = EdgeId(crate::usize_to_u32(parts.edges.len()));
    let a = parts.vertices[lower.vertex.index()];
    let b = parts.vertices[upper.vertex.index()];
    parts.edges.push(Edge::new(
        Curve::line(a, sub(b, a)).map_err(MachiningError::Geometry)?,
        ParameterRange::UNIT,
        lower.vertex,
        upper.vertex,
        edge_role,
    ));
    let loop_id = LoopId(crate::usize_to_u32(parts.loops.len()));
    parts.loops.push(BoundaryLoop {
        coedges: alloc::vec![
            Coedge {
                edge: lower.edge,
                reversed: lower.forward
            },
            Coedge {
                edge: seam,
                reversed: false
            },
            Coedge {
                edge: upper.edge,
                reversed: !upper.forward
            },
            Coedge {
                edge: seam,
                reversed: true
            }
        ],
    });
    let at = |ring: &Ring, high: bool| -> Result<[f64; 2], MachiningError> {
        ring.trim
            .evaluate(if high == ring.forward { 1.0 } else { 0.0 })
            .ok_or(MachiningError::NumericLimit)
    };
    parts.trims.push(LoopTrims {
        loop_id,
        curves: alloc::vec![
            lower.trim.clone(),
            TrimCurve::Line {
                start: at(lower, false)?,
                end: at(upper, false)?
            },
            upper.trim.clone(),
            TrimCurve::Line {
                start: at(lower, true)?,
                end: at(upper, true)?
            }
        ],
    });
    let face = FaceId(crate::usize_to_u32(parts.faces.len()));
    parts.faces.push(Face {
        surface: Surface::cylinder(frame, radius).map_err(MachiningError::Geometry)?,
        domain: ParameterDomain {
            u: ParameterRange::new(0.0, core::f64::consts::TAU)
                .map_err(MachiningError::Geometry)?,
            v: ParameterRange::new(
                (lower.axial.bounds()[0] - 4.0 * gap).next_down(),
                (upper.axial.bounds()[1] + 4.0 * gap).next_up(),
            )
            .map_err(MachiningError::Geometry)?,
        },
        sense: Sense::Reverse,
        loops: alloc::vec![loop_id],
        role,
    });
    Ok(face)
}
fn append_floor<F, E>(
    parts: &mut BodyParts<F, E>,
    frame: Placement3,
    depth: f64,
    outer: &Ring,
    inner: Option<&Ring>,
    role: F,
) -> Result<FaceId, MachiningError> {
    let center = add(col(frame, 3), scale(col(frame, 2), depth));
    let frame = Placement3::from_axes(col(frame, 0), col(frame, 1), col(frame, 2), center);
    let mut loops = Vec::with_capacity(if inner.is_some() { 2 } else { 1 });
    for (ring, reversed) in core::iter::once((outer, true)).chain(inner.map(|r| (r, false))) {
        let loop_id = LoopId(crate::usize_to_u32(parts.loops.len()));
        loops.push(loop_id);
        parts.loops.push(BoundaryLoop {
            coedges: alloc::vec![Coedge {
                edge: ring.edge,
                reversed
            }],
        });
    }
    let Some((Curve::Circle(circle), _)) = parts.edges[outer.edge.index()].regular() else {
        return Err(MachiningError::NumericLimit);
    };
    let radius = circle.radius().next_up();
    let face = FaceId(crate::usize_to_u32(parts.faces.len()));
    parts.faces.push(Face {
        surface: Surface::plane(frame).map_err(MachiningError::Geometry)?,
        domain: ParameterDomain {
            u: ParameterRange::new(-radius, radius).map_err(MachiningError::Geometry)?,
            v: ParameterRange::new(-radius, radius).map_err(MachiningError::Geometry)?,
        },
        sense: Sense::Reverse,
        loops: loops.clone(),
        role,
    });
    for loop_id in loops {
        crate::operation_trims::append_planar_loop(parts, face, loop_id)
            .map_err(MachiningError::Geometry)?;
    }
    Ok(face)
}
