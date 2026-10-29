// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exact circle arrangements for interacting parallel cylindrical passages.
use super::*;

/// A union of parallel through-cylinder tools could not be retained exactly.
#[derive(Clone, Debug, PartialEq)]
pub enum PassageError {
    /// At least one and at most 32 tools are supported per arrangement.
    ToolCount {
        /// Requested tools.
        found: usize,
    },
    /// An individual tool's entire corridor was refused by shared machining checks.
    Tool {
        /// Zero-based tool index.
        tool: usize,
        /// Exact existing machining refusal.
        error: MachiningError,
    },
    /// Tool axes must be exactly parallel in the retained input representation.
    Axis {
        /// Offending tool index.
        tool: usize,
    },
    /// Tools must share two planar exits perpendicular to their axes.
    Exits {
        /// Offending tool index.
        tool: usize,
    },
    /// A pair is too close to an internal/external tangency or coincidence.
    Contact {
        /// First tool index.
        left: usize,
        /// Second tool index.
        right: usize,
        /// Conservative distance from a circle tangency/coincidence.
        bound: f64,
        /// Requested minimum clearance.
        required: f64,
    },
    /// Close intersection vertices or a multiple junction cannot be separated.
    Junction,
    /// The tool union encloses a material island requiring source-face splitting.
    EnclosedIsland,
    /// Native topology, numerical precision or table construction was refused.
    Machining(MachiningError),
}
impl core::fmt::Display for PassageError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "analytic parallel passages: {self:?}")
    }
}
impl core::error::Error for PassageError {}
impl From<MachiningError> for PassageError {
    fn from(e: MachiningError) -> Self {
        Self::Machining(e)
    }
}
/// Shared-boundary provenance and analytic integrals for a union of passages.
#[derive(Clone, Debug, PartialEq)]
pub struct PassageReport {
    /// All original faces, with unchanged local identities.
    pub faces: Vec<FaceChange>,
    /// All original edges, with unchanged local identities.
    pub retained_edges: Vec<EdgeId>,
    /// Source entry/exit faces, in positive common-axis order.
    pub trimmed_faces: [FaceId; 2],
    /// One native wall for every exposed circle-arc fragment.
    pub walls: Vec<FaceId>,
    /// Requested tool index owning each output wall, parallel to `walls`.
    pub wall_tools: Vec<usize>,
    /// Connected opening components; enclosed islands are refused.
    pub opening_components: usize,
    /// Exact pairwise circle classifications performed before output allocation.
    pub pair_tests: usize,
    /// Shared boundary intersection vertices, before source-to-world placement.
    pub intersection_vertices: usize,
    /// Green-integral area of the union's retained circular boundary.
    pub removed_section_area: f64,
    /// Exact-form volume removed between the two perpendicular planes.
    pub removed_volume: f64,
    /// Exact-form boundary-area change for the whole union, counting overlap once.
    pub surface_area_delta: f64,
    /// Conservative clearance to source boundaries and arrangement degeneracies.
    pub min_clearance: f64,
    /// Maximum native support/arithmetic agreement bound in model units.
    pub max_gap: f64,
}
/// One complete native subtraction of a union of interacting parallel cylinders.
#[derive(Clone, Debug, PartialEq)]
pub struct PassageResult<F = (), E = ()> {
    /// Retained output; original source identities survive.
    pub body: Body<F, E>,
    /// Fragment ownership, integrals, work and numerical evidence.
    pub report: PassageReport,
}
/// Subtracts the union of parallel cylindrical passages, including overlaps.
///
/// Every full tool corridor must fit the source, and the shared planar exits
/// must be perpendicular to the common axis. Exact circle intersections split
/// rims and cylindrical walls; no internal wall remains between joined passages.
/// Source openings must be separated from the new tools. Up to 32 tools are
/// accepted. Tangencies, coincident circles, close/multiple intersection nodes,
/// and unions surrounding an uncut island are explicit refusals. The input must
/// already be an embedded outward-oriented regular solid.
/// # Errors
/// Returns an indexed [`PassageError`] without mutating the source.
pub fn subtract_parallel_passages<F: Clone + Default, E: Clone + Default>(
    body: &Body<F, E>,
    tools: &[CylinderTool],
    policy: &MachiningPolicy,
) -> Result<PassageResult<F, E>, PassageError> {
    subtract_parallel_passages_with_roles(body, tools, policy, F::default(), E::default())
}
/// Subtracts a union of parallel passages, assigning roles to generated entities.
/// # Errors
/// Refuses unsupported tools/source geometry, degenerate arrangements, exhausted
/// native table identity space and unmet numerical/clearance bounds.
pub fn subtract_parallel_passages_with_roles<F: Clone, E: Clone>(
    body: &Body<F, E>,
    tools: &[CylinderTool],
    policy: &MachiningPolicy,
    face_role: F,
    edge_role: E,
) -> Result<PassageResult<F, E>, PassageError> {
    check_policy(policy)?;
    if tools.is_empty() || tools.len() > 32 {
        return Err(PassageError::ToolCount { found: tools.len() });
    }
    let frame = tools[0].surface.frame();
    let axis = col(frame, 2);
    let mut circles = Vec::with_capacity(tools.len());
    let mut first: Option<CylinderPlan> = None;
    let mut clearance = f64::INFINITY;
    let mut gap = 0.0_f64;
    for (i, tool) in tools.iter().enumerate() {
        let other_axis = col(tool.surface.frame(), 2);
        if other_axis != axis && other_axis != scale(axis, -1.0) {
            return Err(PassageError::Axis { tool: i });
        }
        let plan = plan_cylinder(body, tool, policy)
            .map_err(|error| PassageError::Tool { tool: i, error })?;
        let ordered = if dot(other_axis, axis) > 0.0 {
            [plan.exits[0].0, plan.exits[1].0]
        } else {
            [plan.exits[1].0, plan.exits[0].0]
        };
        let expected = first
            .as_ref()
            .map(|p| [p.exits[0].0, p.exits[1].0])
            .unwrap_or(ordered);
        if ordered != expected {
            return Err(PassageError::Exits { tool: i });
        }
        for &(face, ref section) in &plan.exits {
            let normal = col(body.faces()[face.index()].surface.frame(), 2);
            if norm(cross(normal, axis)) != 0.0 {
                return Err(PassageError::Exits { tool: i });
            }
            gap = gap.max(section.max_gap());
        }
        clearance = clearance.min(plan.clearance);
        let center = crate::geometry::frame_coordinates(frame, col(tool.surface.frame(), 3))
            .ok_or(MachiningError::NumericLimit)?;
        let Surface::Cylinder(c) = &tool.surface else {
            unreachable!()
        };
        circles.push(Disk {
            center: [center[0], center[1]],
            radius: c.radius(),
        });
        if first.is_none() {
            first = Some(plan);
        }
    }
    let plan = first.expect("nonempty tools");
    let lower = plan.exits[0].1.axial().coefficients()[0];
    let upper = plan.exits[1].1.axial().coefficients()[0];
    let magnitude = body
        .vertices()
        .iter()
        .flatten()
        .copied()
        .map(f64::abs)
        .chain(
            circles
                .iter()
                .flat_map(|c| [c.center[0].abs(), c.center[1].abs(), c.radius]),
        )
        .fold(upper.abs().max(lower.abs()), f64::max);
    let numeric = 2048.0 * f64::EPSILON * magnitude;
    if !numeric.is_finite() || numeric > policy.geometry_gap {
        return Err(MachiningError::NumericLimit.into());
    }
    let arranged = arrange(&circles, policy.separation, numeric)?;
    clearance = clearance.min(arranged.clearance);
    let source_faces = body.faces().len();
    let source_edges = body.edges().len();
    let nodes = arranged.points.len();
    let arcs = arranged.arcs.len();
    let components = arranged.loops.len();
    for (table, count, extra) in [
        ("vertices", body.vertices().len(), 2 * nodes),
        ("edges", body.edges().len(), nodes + 2 * arcs),
        ("loops", body.loops().len(), arcs + 2 * components),
        ("faces", body.faces().len(), arcs),
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
    let mut parts = body.clone().into_parts();
    parts.vertices.reserve(2 * nodes);
    parts.edges.reserve(nodes + 2 * arcs);
    parts.loops.reserve(arcs + 2 * components);
    parts.faces.reserve(arcs);
    parts.trims.reserve(arcs + 2 * components);
    let vertex_base = parts.vertices.len();
    let seam_base = parts.edges.len();
    for &p in &arranged.points {
        parts
            .vertices
            .push(frame.transform_point([p[0], p[1], lower]));
        parts
            .vertices
            .push(frame.transform_point([p[0], p[1], upper]));
    }
    for node in 0..nodes {
        let a = VertexId(crate::usize_to_u32(vertex_base + 2 * node));
        let b = VertexId(crate::usize_to_u32(vertex_base + 2 * node + 1));
        parts.edges.push(Edge::new(
            Curve::line(
                parts.vertices[a.index()],
                sub(parts.vertices[b.index()], parts.vertices[a.index()]),
            )
            .map_err(MachiningError::Geometry)?,
            ParameterRange::UNIT,
            a,
            b,
            edge_role.clone(),
        ));
    }
    let mut rims = Vec::with_capacity(arcs);
    let mut walls = Vec::with_capacity(arcs);
    let mut wall_tools = Vec::with_capacity(arcs);
    for arc in &arranged.arcs {
        let disk = circles[arc.tool];
        let range = ParameterRange::new(arc.start, arc.end).map_err(MachiningError::Geometry)?;
        let edge_base = parts.edges.len();
        let ring_edges = [
            EdgeId(crate::usize_to_u32(edge_base)),
            EdgeId(crate::usize_to_u32(edge_base + 1)),
        ];
        for (side, height) in [lower, upper].into_iter().enumerate() {
            let center = frame.transform_point([disk.center[0], disk.center[1], height]);
            let circle = Curve::circle(
                Placement3::from_axes(col(frame, 0), col(frame, 1), axis, center),
                disk.radius,
            )
            .map_err(MachiningError::Geometry)?;
            parts.edges.push(Edge::new(
                circle,
                range,
                VertexId(crate::usize_to_u32(vertex_base + 2 * arc.from + side)),
                VertexId(crate::usize_to_u32(vertex_base + 2 * arc.to + side)),
                edge_role.clone(),
            ));
        }
        rims.push(ring_edges);
        let loop_id = LoopId(crate::usize_to_u32(parts.loops.len()));
        parts.loops.push(BoundaryLoop {
            coedges: alloc::vec![
                Coedge {
                    edge: ring_edges[0],
                    reversed: true
                },
                Coedge {
                    edge: EdgeId(crate::usize_to_u32(seam_base + arc.from)),
                    reversed: false
                },
                Coedge {
                    edge: ring_edges[1],
                    reversed: false
                },
                Coedge {
                    edge: EdgeId(crate::usize_to_u32(seam_base + arc.to)),
                    reversed: true
                }
            ],
        });
        parts.trims.push(LoopTrims {
            loop_id,
            curves: alloc::vec![
                TrimCurve::Line {
                    start: [arc.start, lower],
                    end: [arc.end, lower]
                },
                TrimCurve::Line {
                    start: [arc.start, lower],
                    end: [arc.start, upper]
                },
                TrimCurve::Line {
                    start: [arc.start, upper],
                    end: [arc.end, upper]
                },
                TrimCurve::Line {
                    start: [arc.end, lower],
                    end: [arc.end, upper]
                }
            ],
        });
        let cylinder_frame = Placement3::from_axes(
            col(frame, 0),
            col(frame, 1),
            axis,
            frame.transform_point([disk.center[0], disk.center[1], 0.0]),
        );
        let face = FaceId(crate::usize_to_u32(parts.faces.len()));
        walls.push(face);
        wall_tools.push(arc.tool);
        parts.faces.push(Face {
            surface: Surface::cylinder(cylinder_frame, disk.radius)
                .map_err(MachiningError::Geometry)?,
            domain: ParameterDomain {
                u: range,
                v: ParameterRange::new(lower, upper).map_err(MachiningError::Geometry)?,
            },
            sense: Sense::Reverse,
            loops: alloc::vec![loop_id],
            role: face_role.clone(),
        });
    }
    for component in &arranged.loops {
        for (side, &(face, _)) in plan.exits.iter().enumerate() {
            let loop_id = LoopId(crate::usize_to_u32(parts.loops.len()));
            let mut coedges: Vec<_> = component
                .iter()
                .map(|&i| Coedge {
                    edge: rims[i][side],
                    reversed: side == 1,
                })
                .collect();
            if side == 1 {
                coedges.reverse();
            }
            parts.loops.push(BoundaryLoop { coedges });
            parts.faces[face.index()].loops.push(loop_id);
            crate::operation_trims::append_planar_loop(&mut parts, face, loop_id)
                .map_err(MachiningError::Geometry)?;
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
    gap = gap.max(checked.max_gap).max(numeric);
    let height = upper - lower;
    let removed_volume = arranged.area * height;
    let surface_area_delta = arranged.perimeter * height - 2.0 * arranged.area;
    if !removed_volume.is_finite() || !surface_area_delta.is_finite() {
        return Err(MachiningError::NumericLimit.into());
    }
    let trimmed_faces = [plan.exits[0].0, plan.exits[1].0];
    Ok(PassageResult {
        body: output,
        report: PassageReport {
            faces: (0..source_faces)
                .map(|i| {
                    let id = FaceId(crate::usize_to_u32(i));
                    FaceChange {
                        source: id,
                        result: id,
                        kind: if trimmed_faces.contains(&id) {
                            FaceChangeKind::Trimmed
                        } else {
                            FaceChangeKind::Retained
                        },
                    }
                })
                .collect(),
            retained_edges: (0..source_edges)
                .map(|i| EdgeId(crate::usize_to_u32(i)))
                .collect(),
            trimmed_faces,
            walls,
            wall_tools,
            opening_components: components,
            pair_tests: tools.len() * (tools.len() - 1) / 2,
            intersection_vertices: arranged.intersections,
            removed_section_area: arranged.area,
            removed_volume,
            surface_area_delta,
            min_clearance: clearance,
            max_gap: gap,
        },
    })
}

#[derive(Copy, Clone)]
struct Disk {
    center: [f64; 2],
    radius: f64,
}
struct Arc {
    tool: usize,
    start: f64,
    end: f64,
    from: usize,
    to: usize,
}
struct Arrangement {
    points: Vec<[f64; 2]>,
    arcs: Vec<Arc>,
    loops: Vec<Vec<usize>>,
    area: f64,
    perimeter: f64,
    intersections: usize,
    clearance: f64,
}
fn arrange(disks: &[Disk], separation: f64, numeric: f64) -> Result<Arrangement, PassageError> {
    let mut cuts = alloc::vec![Vec::<(f64,usize)>::new();disks.len()];
    let mut points = Vec::<[f64; 2]>::new();
    let mut clearance = f64::INFINITY;
    for i in 0..disks.len() {
        for j in i + 1..disks.len() {
            let (a, b) = (disks[i], disks[j]);
            let delta = [b.center[0] - a.center[0], b.center[1] - a.center[1]];
            let d = libm::hypot(delta[0], delta[1]);
            let outer = a.radius + b.radius;
            let inner = (a.radius - b.radius).abs();
            let margin = (d - outer).abs().min((d - inner).abs()) - numeric;
            if margin < separation {
                return Err(PassageError::Contact {
                    left: i,
                    right: j,
                    bound: margin,
                    required: separation,
                });
            }
            clearance = clearance.min(margin);
            if d > outer || d < inner {
                continue;
            }
            let unit = [delta[0] / d, delta[1] / d];
            let scale = d.max(a.radius).max(b.radius);
            let (dd, ra, rb) = (d / scale, a.radius / scale, b.radius / scale);
            let xx = (dd * dd + ra * ra - rb * rb) / (2.0 * dd);
            let hh = libm::sqrt((ra - xx) * (ra + xx));
            if !xx.is_finite() || !hh.is_finite() {
                return Err(MachiningError::NumericLimit.into());
            }
            for sign in [-1.0, 1.0] {
                let p = [
                    a.center[0] + scale * (unit[0] * xx - sign * unit[1] * hh),
                    a.center[1] + scale * (unit[1] * xx + sign * unit[0] * hh),
                ];
                if !p.into_iter().all(f64::is_finite) {
                    return Err(MachiningError::NumericLimit.into());
                }
                for &previous in &points {
                    let distance = libm::hypot(p[0] - previous[0], p[1] - previous[1]) - numeric;
                    if distance < separation {
                        return Err(PassageError::Junction);
                    }
                    clearance = clearance.min(distance);
                }
                let node = points.len();
                points.push(p);
                for (index, c) in [(i, a), (j, b)] {
                    let mut angle = libm::atan2(p[1] - c.center[1], p[0] - c.center[0]);
                    if angle < 0.0 {
                        angle += core::f64::consts::TAU;
                    }
                    cuts[index].push((angle, node));
                }
            }
        }
    }
    let intersections = points.len();
    let mut arcs = Vec::new();
    for (i, c) in disks.iter().copied().enumerate() {
        cuts[i].sort_by(|a, b| a.0.total_cmp(&b.0));
        if cuts[i].is_empty() {
            let p = [c.center[0] + c.radius, c.center[1]];
            if disks.iter().enumerate().any(|(j, o)| {
                j != i && libm::hypot(p[0] - o.center[0], p[1] - o.center[1]) < o.radius
            }) {
                continue;
            }
            let node = points.len();
            points.push(p);
            arcs.push(Arc {
                tool: i,
                start: 0.0,
                end: core::f64::consts::TAU,
                from: node,
                to: node,
            });
            continue;
        }
        for k in 0..cuts[i].len() {
            let (start, from) = cuts[i][k];
            let (mut end, to) = cuts[i][(k + 1) % cuts[i].len()];
            if k + 1 == cuts[i].len() {
                end += core::f64::consts::TAU;
            }
            let mid = 0.5 * (start + end);
            let p = [
                c.center[0] + c.radius * libm::cos(mid),
                c.center[1] + c.radius * libm::sin(mid),
            ];
            let mut exposed = true;
            for (j, o) in disks.iter().enumerate() {
                if j == i {
                    continue;
                }
                let bound = libm::hypot(p[0] - o.center[0], p[1] - o.center[1]) - o.radius;
                if bound.abs() - numeric < separation {
                    return Err(PassageError::Contact {
                        left: i,
                        right: j,
                        bound: bound.abs() - numeric,
                        required: separation,
                    });
                }
                if bound < 0.0 {
                    exposed = false;
                }
            }
            if exposed {
                arcs.push(Arc {
                    tool: i,
                    start,
                    end,
                    from,
                    to,
                });
            }
        }
    }
    let mut outgoing = alloc::vec![None;points.len()];
    let mut incoming = alloc::vec![0_usize;points.len()];
    for (i, a) in arcs.iter().enumerate() {
        if outgoing[a.from].replace(i).is_some() {
            return Err(PassageError::Junction);
        }
        incoming[a.to] += 1;
    }
    for (i, out) in outgoing.iter().enumerate() {
        if out.is_some() && incoming[i] != 1 || out.is_none() && incoming[i] != 0 {
            return Err(PassageError::Junction);
        }
    }
    let mut visited = alloc::vec![false;arcs.len()];
    let mut loops = Vec::new();
    let mut area = 0.0;
    let mut perimeter = 0.0;
    for first in 0..arcs.len() {
        if visited[first] {
            continue;
        }
        let mut component = Vec::new();
        let mut current = first;
        let mut signed = 0.0;
        loop {
            if visited[current] {
                if current != first {
                    return Err(PassageError::Junction);
                }
                break;
            }
            visited[current] = true;
            component.push(current);
            let a = &arcs[current];
            let c = disks[a.tool];
            let span = a.end - a.start;
            signed += 0.5
                * (c.radius * c.radius * span
                    + c.radius * c.center[0] * (libm::sin(a.end) - libm::sin(a.start))
                    - c.radius * c.center[1] * (libm::cos(a.end) - libm::cos(a.start)));
            perimeter += c.radius * span;
            current = outgoing[a.to].ok_or(PassageError::Junction)?;
        }
        if !signed.is_finite() || signed <= 0.0 {
            return Err(PassageError::EnclosedIsland);
        }
        area += signed;
        loops.push(component);
    }
    if !area.is_finite() || !perimeter.is_finite() || area <= 0.0 {
        return Err(MachiningError::NumericLimit.into());
    }
    // Only exposed intersection ports belong to the output tables. Compact once
    // so fully covered pair intersections do not leave detached native edges.
    let mut compact = Vec::new();
    let mut map = alloc::vec![usize::MAX;points.len()];
    let mut exposed_intersections = 0;
    for a in &mut arcs {
        for node in [&mut a.from, &mut a.to] {
            if map[*node] == usize::MAX {
                map[*node] = compact.len();
                exposed_intersections += usize::from(*node < intersections);
                compact.push(points[*node]);
            }
            *node = map[*node];
        }
    }
    Ok(Arrangement {
        points: compact,
        arcs,
        loops,
        area,
        perimeter,
        intersections: exposed_intersections,
        clearance,
    })
}
