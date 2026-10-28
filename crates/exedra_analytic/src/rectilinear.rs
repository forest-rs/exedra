// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exact arrangement Booleans for retained axis-aligned plane/line boundaries.
//!
//! This is a deliberately bounded Boolean kernel. Every input edge must be an
//! axis-aligned line and every face an axis-aligned orthogonal polygon, with
//! optional holes. Input plane coordinates are retained exactly: distinct close
//! coordinates are refused, never snapped. The finite plane arrangement has no
//! boundary in a cell interior, so analytic ray winding at one interior reference
//! establishes the whole cell's membership. All three ray directions must agree.
//! This is neither mesh clipping nor a sampled approximation of curved surfaces.
//!
//! Coplanar output cells merge into polygon faces. Edge/vertex-only contacts are
//! rejected by oriented edge incidence and connected vertex links. Disconnected
//! positive-volume components and closed cavities are represented explicitly and
//! counted in the report; consumers requiring one shell must inspect those counts.
use crate::body::{
    Body, BodyError, BodyParts, BoundaryLoop, Coedge, Edge, EdgeId, Face, FaceId, LoopId, Sense,
    ValidationPolicy, VertexId,
};
use crate::geometry::{Curve, GeometryError, ParameterDomain, ParameterRange, Surface};
use alloc::{
    collections::{BTreeMap, BTreeSet},
    vec::Vec,
};
use exedra_math::{Placement3, sub};

type GridVertex = [usize; 3];
type GridEdge = (GridVertex, GridVertex);
/// Set operation on two retained rectilinear solids.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BooleanOperation {
    /// Material in either operand.
    Union,
    /// Material in both operands.
    Intersection,
    /// Left material excluding right material.
    Difference,
}
/// Operand identity local to one operation report.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Operand {
    /// First input.
    Left,
    /// Second input.
    Right,
}
/// A source patch whose support covers part of an output face.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FaceSource {
    /// Input operand.
    pub operand: Operand,
    /// Input body-local face identity.
    pub face: FaceId,
    /// Output outward sense opposes this source patch.
    pub reversed: bool,
}
/// Work and numerical limits checked before allocating an arrangement grid.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct RectilinearPolicy {
    /// Largest allowed Cartesian product of open coordinate intervals.
    pub max_cells: usize,
    /// Minimum allowed distance between distinct parallel input planes.
    pub minimum_spacing: f64,
    /// Maximum native support disagreement and conservative arithmetic allowance.
    pub geometry_gap: f64,
}
impl Default for RectilinearPolicy {
    fn default() -> Self {
        Self {
            max_cells: 1_000_000,
            minimum_spacing: 1e-6,
            geometry_gap: 1e-9,
        }
    }
}
/// Exact arrangement work, material topology, integrals and operation history.
#[derive(Clone, Debug, PartialEq)]
pub struct RectilinearReport {
    /// Requested set operation.
    pub operation: BooleanOperation,
    /// Distinct input coordinate plane counts along X, Y, Z.
    pub planes: [usize; 3],
    /// Cartesian arrangement cells classified for each operand.
    pub cells: usize,
    /// Positive-volume output material cells.
    pub material_cells: usize,
    /// Material components connected through positive-area faces.
    pub material_components: usize,
    /// Closed void components disconnected from the exterior.
    pub cavities: usize,
    /// Connected output boundary shells (outer boundaries and cavity boundaries).
    pub boundary_shells: usize,
    /// All output face origins, indexed by output face ID.
    pub face_sources: Vec<Vec<FaceSource>>,
    /// Output face images for each left input face, including empty images.
    pub left_faces: Vec<Vec<FaceId>>,
    /// Output face images for each right input face, including empty images.
    pub right_faces: Vec<Vec<FaceId>>,
    /// Output edge fragments on each left source edge, in source order.
    pub left_edges: Vec<Vec<EdgeId>>,
    /// Output edge fragments on each right source edge, in source order.
    pub right_edges: Vec<Vec<EdgeId>>,
    /// Output image of each left source vertex, absent when internal or removed.
    pub left_vertices: Vec<Option<VertexId>>,
    /// Output image of each right source vertex, absent when internal or removed.
    pub right_vertices: Vec<Option<VertexId>>,
    /// Native analytic volume, summed over exact arrangement boxes.
    pub volume: f64,
    /// Outward-rounded enclosure of exact arrangement-box volume arithmetic.
    pub volume_bounds: [f64; 2],
    /// Native analytic boundary area, summed over exposed arrangement rectangles.
    pub surface_area: f64,
    /// Outward-rounded enclosure of exact exposed-rectangle area arithmetic.
    pub surface_area_bounds: [f64; 2],
    /// Largest conservative arithmetic allowance or validated support gap.
    pub max_gap: f64,
}
/// Complete retained output and explicit geometric/provenance evidence.
#[derive(Clone, Debug, PartialEq)]
pub struct RectilinearResult {
    /// Reconstructed analytic boundary. Roles are intentionally empty; source
    /// metadata remains accessible through the report's typed correspondence.
    pub body: Body,
    /// Work, integral, connectivity and provenance report.
    pub report: RectilinearReport,
}
/// Structured transactional Boolean refusal.
#[derive(Clone, Debug, PartialEq)]
pub enum RectilinearError {
    /// Policy values must be finite and work/spacing limits strictly positive.
    InvalidPolicy,
    /// Native boundary/support validation failed for an input or output.
    Boundary {
        /// Input operand, or `None` for the constructed output.
        operand: Option<Operand>,
        /// Native failure.
        error: BodyError,
    },
    /// Input must be nonempty and oriented closed.
    OpenInput {
        /// Offending operand.
        operand: Operand,
    },
    /// Support, trim or exact axis-coordinate agreement is unsupported.
    UnsupportedFace {
        /// Offending operand.
        operand: Operand,
        /// Input face.
        face: FaceId,
    },
    /// Distinct planes are too close; no coordinate is snapped.
    PlaneSpacing {
        /// Coordinate axis.
        axis: usize,
        /// Lower exact input coordinate.
        lower: f64,
        /// Upper exact input coordinate.
        upper: f64,
        /// Required separation including arithmetic allowance.
        required: f64,
    },
    /// Grid product or required native table indexes exceed the budget.
    CellBudget {
        /// Required cells, absent if multiplication overflowed.
        required: Option<usize>,
        /// Caller limit.
        maximum: usize,
    },
    /// An input has inconsistent oriented material winding in the exact arrangement.
    InvalidMaterial {
        /// Offending operand.
        operand: Operand,
        /// Open arrangement cell coordinates.
        cell: [usize; 3],
    },
    /// Output has an edge/vertex-only contact or another nonmanifold boundary.
    NonManifold,
    /// A derived primitive violated the geometry contract.
    Geometry(GeometryError),
    /// Finite arithmetic or an interior reference cannot meet policy.
    NumericLimit,
}
impl core::fmt::Display for RectilinearError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "rectilinear Boolean: {self:?}")
    }
}
impl core::error::Error for RectilinearError {}

#[derive(Clone, Debug)]
struct AxisFace {
    id: FaceId,
    axis: usize,
    coordinate: f64,
    sign: i8,
    loops: Vec<Vec<[f64; 2]>>,
}
#[derive(Default)]
struct Patch {
    edges: BTreeMap<GridEdge, ()>,
    cells: Vec<[usize; 3]>,
}

/// Performs a bounded Boolean on native orthogonal planar solids.
///
/// Exact equal coordinates represent coplanarity. Distinct coordinates remain
/// distinct. Output faces combine coplanar pieces and carry typed source images
/// in the report. Curves and arbitrary oblique planes are explicitly outside
/// this kernel. Empty, disconnected and cavity-bearing results are legitimate;
/// their material and boundary component counts are always reported.
/// # Errors
/// Refuses unsupported input, insufficient spacing, budget/finite arithmetic
/// failures, inconsistent material winding, and nonmanifold contacts.
pub fn boolean_rectilinear<LF, LE, RF, RE>(
    left: &Body<LF, LE>,
    right: &Body<RF, RE>,
    operation: BooleanOperation,
    policy: &RectilinearPolicy,
) -> Result<RectilinearResult, RectilinearError> {
    if policy.max_cells == 0
        || !policy.minimum_spacing.is_finite()
        || policy.minimum_spacing <= 0.0
        || !policy.geometry_gap.is_finite()
        || policy.geometry_gap < 0.0
    {
        return Err(RectilinearError::InvalidPolicy);
    }
    let validation = ValidationPolicy {
        max_gap: policy.geometry_gap,
    };
    let left_faces = inspect(left, Operand::Left, &validation)?;
    let right_faces = inspect(right, Operand::Right, &validation)?;
    let mut coordinates: [Vec<f64>; 3] = core::array::from_fn(|_| Vec::new());
    for point in used_points(left).chain(used_points(right)) {
        for axis in 0..3 {
            coordinates[axis].push(point[axis]);
        }
    }
    let magnitude = coordinates
        .iter()
        .flatten()
        .map(|v| v.abs())
        .fold(0.0, f64::max);
    let arithmetic = 512.0 * f64::EPSILON * magnitude;
    if !arithmetic.is_finite() || arithmetic > policy.geometry_gap {
        return Err(RectilinearError::NumericLimit);
    }
    for (axis, values) in coordinates.iter_mut().enumerate() {
        values.sort_by(f64::total_cmp);
        values.dedup_by(|a, b| *a == *b);
        for pair in values.windows(2) {
            let distance = pair[1] - pair[0];
            if !distance.is_finite() {
                return Err(RectilinearError::NumericLimit);
            }
            if distance < policy.minimum_spacing + arithmetic {
                return Err(RectilinearError::PlaneSpacing {
                    axis,
                    lower: pair[0],
                    upper: pair[1],
                    required: policy.minimum_spacing + arithmetic,
                });
            }
        }
        if values.len() < 2 {
            return Err(RectilinearError::NumericLimit);
        }
    }
    let planes = coordinates.each_ref().map(Vec::len);
    let dimensions = planes.map(|n| n - 1);
    let cells = dimensions[0]
        .checked_mul(dimensions[1])
        .and_then(|n| n.checked_mul(dimensions[2]));
    let Some(cells) = cells else {
        return Err(RectilinearError::CellBudget {
            required: None,
            maximum: policy.max_cells,
        });
    };
    if cells > policy.max_cells
        || cells
            .checked_mul(24)
            .is_none_or(|n| u32::try_from(n).is_err())
    {
        return Err(RectilinearError::CellBudget {
            required: Some(cells),
            maximum: policy.max_cells,
        });
    }
    let references: [Vec<f64>; 3] = coordinates
        .each_ref()
        .map(|v| {
            v.windows(2)
                .map(|p| {
                    let m = p[0] + (p[1] - p[0]) * 0.5;
                    if !m.is_finite() || m <= p[0] || m >= p[1] {
                        Err(RectilinearError::NumericLimit)
                    } else {
                        Ok(m)
                    }
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?
        .try_into()
        .map_err(|_| RectilinearError::NumericLimit)?;
    let mut material = alloc::vec![false;cells];
    let mut material_cells = 0;
    let mut volume = 0.0;
    let mut volume_bounds = [0.0; 2];
    for (index, filled) in material.iter_mut().enumerate() {
        let cell = decode(index, dimensions);
        let point = core::array::from_fn(|i| references[i][cell[i]]);
        let a = classify(&left_faces, point, Operand::Left, cell)?;
        let b = classify(&right_faces, point, Operand::Right, cell)?;
        *filled = match operation {
            BooleanOperation::Union => a || b,
            BooleanOperation::Intersection => a && b,
            BooleanOperation::Difference => a && !b,
        };
        if *filled {
            material_cells += 1;
            let size = core::array::from_fn::<_, 3, _>(|i| {
                coordinates[i][cell[i] + 1] - coordinates[i][cell[i]]
            });
            let dv = size[0] * size[1] * size[2];
            if !dv.is_finite() || dv <= 0.0 {
                return Err(RectilinearError::NumericLimit);
            }
            volume += dv;
            let mut bounds = [1.0, 1.0];
            for length in size {
                bounds = interval_product(bounds, [length.next_down(), length.next_up()]);
            }
            volume_bounds = interval_sum(volume_bounds, bounds);
        }
    }
    let (material_components, cavities) = component_counts(&material, dimensions);
    let mut patches: BTreeMap<(usize, usize, i8), Patch> = BTreeMap::new();
    let mut surface_area = 0.0;
    let mut surface_area_bounds = [0.0; 2];
    for (index, &filled) in material.iter().enumerate() {
        if !filled {
            continue;
        }
        let cell = decode(index, dimensions);
        for axis in 0..3 {
            for side in 0..2 {
                let neighbor = neighbor(cell, axis, side == 1, dimensions);
                if neighbor.is_some_and(|n| material[encode(n, dimensions)]) {
                    continue;
                }
                let sign = if side == 0 { -1 } else { 1 };
                let plane = cell[axis] + side;
                let patch = patches.entry((axis, plane, sign)).or_default();
                let u = (axis + 1) % 3;
                let v = (axis + 2) % 3;
                let mut corners = [cell; 4];
                for c in &mut corners {
                    c[axis] = plane;
                }
                corners[1][u] += 1;
                corners[2][u] += 1;
                corners[2][v] += 1;
                corners[3][v] += 1;
                if sign < 0 {
                    corners.reverse();
                }
                for i in 0..4 {
                    let edge = (corners[i], corners[(i + 1) % 4]);
                    if patch.edges.remove(&(edge.1, edge.0)).is_none()
                        && patch.edges.insert(edge, ()).is_some()
                    {
                        return Err(RectilinearError::NonManifold);
                    }
                }
                patch.cells.push(cell);
                surface_area += (coordinates[u][cell[u] + 1] - coordinates[u][cell[u]])
                    * (coordinates[v][cell[v] + 1] - coordinates[v][cell[v]]);
                let a = coordinates[u][cell[u] + 1] - coordinates[u][cell[u]];
                let b = coordinates[v][cell[v] + 1] - coordinates[v][cell[v]];
                surface_area_bounds = interval_sum(
                    surface_area_bounds,
                    interval_product([a.next_down(), a.next_up()], [b.next_down(), b.next_up()]),
                );
            }
        }
    }
    if !volume.is_finite()
        || !surface_area.is_finite()
        || !volume_bounds
            .into_iter()
            .chain(surface_area_bounds)
            .all(f64::is_finite)
    {
        return Err(RectilinearError::NumericLimit);
    }
    let mut parts = BodyParts::default();
    let mut vertices: BTreeMap<GridVertex, VertexId> = BTreeMap::new();
    let mut edges: BTreeMap<(VertexId, VertexId), EdgeId> = BTreeMap::new();
    let mut face_sources: Vec<Vec<FaceSource>> = Vec::new();
    for ((axis, plane, sign), mut patch) in patches {
        let contours = contours(&mut patch.edges)?;
        let mut outers = Vec::new();
        let mut holes = Vec::new();
        for contour in contours {
            let area = grid_area(&contour, axis)?;
            if area == 0 {
                return Err(RectilinearError::NonManifold);
            }
            if (area > 0) == (sign > 0) {
                outers.push((contour, area.unsigned_abs()));
            } else {
                holes.push(contour);
            }
        }
        let mut assignments = alloc::vec![Vec::new();outers.len()];
        for hole in holes {
            let p = project_grid_point(hole[0], axis, &coordinates);
            let mut owner = None;
            for (i, (outer, area)) in outers.iter().enumerate() {
                if point_in_grid_loop(p, outer, axis, &coordinates)
                    && owner.is_none_or(|(_, old)| *area < old)
                {
                    owner = Some((i, *area));
                }
            }
            let Some((owner, _)) = owner else {
                return Err(RectilinearError::NonManifold);
            };
            assignments[owner].push(hole);
        }
        for (i, (outer, _)) in outers.into_iter().enumerate() {
            let mut source_set = BTreeSet::new();
            for cell in &patch.cells {
                let p = [
                    references[(axis + 1) % 3][cell[(axis + 1) % 3]],
                    references[(axis + 2) % 3][cell[(axis + 2) % 3]],
                ];
                if !point_in_grid_loop(p, &outer, axis, &coordinates)
                    || assignments[i]
                        .iter()
                        .any(|h| point_in_grid_loop(p, h, axis, &coordinates))
                {
                    continue;
                }
                for (operand, faces) in
                    [(Operand::Left, &left_faces), (Operand::Right, &right_faces)]
                {
                    for face in faces {
                        if face.axis == axis
                            && face.coordinate == coordinates[axis][plane]
                            && contains(face, p)
                        {
                            source_set.insert(FaceSource {
                                operand,
                                face: face.id,
                                reversed: face.sign != sign,
                            });
                        }
                    }
                }
            }
            if source_set.is_empty() {
                return Err(RectilinearError::NumericLimit);
            }
            let mut loops = Vec::new();
            let mut bounds = [[f64::INFINITY; 2], [f64::NEG_INFINITY; 2]];
            for contour in core::iter::once(&outer).chain(assignments[i].iter()) {
                let mut coedges = Vec::new();
                for j in 0..contour.len() {
                    let a = grid_vertex(contour[j], &coordinates, &mut vertices, &mut parts);
                    let b = grid_vertex(
                        contour[(j + 1) % contour.len()],
                        &coordinates,
                        &mut vertices,
                        &mut parts,
                    );
                    let key = if a < b { (a, b) } else { (b, a) };
                    let edge = if let Some(&edge) = edges.get(&key) {
                        edge
                    } else {
                        let edge = EdgeId(crate::usize_to_u32(parts.edges.len()));
                        let start = parts.vertices[key.0.index()];
                        let end = parts.vertices[key.1.index()];
                        parts.edges.push(Edge::new(
                            Curve::line(start, sub(end, start))
                                .map_err(RectilinearError::Geometry)?,
                            ParameterRange::UNIT,
                            key.0,
                            key.1,
                            (),
                        ));
                        edges.insert(key, edge);
                        edge
                    };
                    coedges.push(Coedge {
                        edge,
                        reversed: a != key.0,
                    });
                    let p = project_grid_point(contour[j], axis, &coordinates);
                    for (k, value) in p.into_iter().enumerate() {
                        bounds[0][k] = bounds[0][k].min(value);
                        bounds[1][k] = bounds[1][k].max(value);
                    }
                }
                let id = LoopId(crate::usize_to_u32(parts.loops.len()));
                parts.loops.push(BoundaryLoop { coedges });
                loops.push(id);
            }
            let mut origin = [0.0; 3];
            origin[axis] = coordinates[axis][plane];
            let mut u = [0.0; 3];
            u[(axis + 1) % 3] = 1.0;
            let mut v = [0.0; 3];
            v[(axis + 2) % 3] = 1.0;
            let mut z = [0.0; 3];
            z[axis] = 1.0;
            parts.faces.push(Face {
                surface: Surface::plane(Placement3::from_axes(u, v, z, origin))
                    .map_err(RectilinearError::Geometry)?,
                domain: ParameterDomain {
                    u: ParameterRange::new(bounds[0][0], bounds[1][0])
                        .map_err(RectilinearError::Geometry)?,
                    v: ParameterRange::new(bounds[0][1], bounds[1][1])
                        .map_err(RectilinearError::Geometry)?,
                },
                sense: if sign > 0 {
                    Sense::Forward
                } else {
                    Sense::Reverse
                },
                loops,
                role: (),
            });
            face_sources.push(source_set.into_iter().collect());
        }
    }
    crate::operation_trims::append_all_planar(&mut parts).map_err(RectilinearError::Geometry)?;
    let body =
        Body::try_from_parts(parts, &validation).map_err(|error| RectilinearError::Boundary {
            operand: None,
            error,
        })?;
    if material_cells > 0 && (!body.is_closed() || !manifold_vertices(&body)) {
        return Err(RectilinearError::NonManifold);
    }
    let boundary_shells = shell_count(&body);
    if boundary_shells != material_components + cavities {
        return Err(RectilinearError::NonManifold);
    }
    let mut left_images = alloc::vec![Vec::new();left.faces().len()];
    let mut right_images = alloc::vec![Vec::new();right.faces().len()];
    for (i, sources) in face_sources.iter().enumerate() {
        let id = FaceId(crate::usize_to_u32(i));
        for source in sources {
            let images = match source.operand {
                Operand::Left => &mut left_images,
                Operand::Right => &mut right_images,
            };
            images[source.face.index()].push(id);
        }
    }
    let checked = body
        .validate(&validation)
        .map_err(|error| RectilinearError::Boundary {
            operand: None,
            error,
        })?;
    let report = RectilinearReport {
        operation,
        planes,
        cells,
        material_cells,
        material_components,
        cavities,
        boundary_shells,
        face_sources,
        left_faces: left_images,
        right_faces: right_images,
        left_edges: edge_images(left, &body),
        right_edges: edge_images(right, &body),
        left_vertices: vertex_images(left, &body),
        right_vertices: vertex_images(right, &body),
        volume,
        volume_bounds,
        surface_area,
        surface_area_bounds,
        max_gap: arithmetic.max(checked.max_gap),
    };
    Ok(RectilinearResult { body, report })
}

fn interval_sum(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [(a[0] + b[0]).next_down().max(0.0), (a[1] + b[1]).next_up()]
}
fn interval_product(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [(a[0] * b[0]).next_down().max(0.0), (a[1] * b[1]).next_up()]
}
fn used_points<F, E>(body: &Body<F, E>) -> impl Iterator<Item = [f64; 3]> + '_ {
    body.faces()
        .iter()
        .flat_map(|f| f.loops.iter())
        .flat_map(|id| body.loops()[id.index()].coedges.iter())
        .map(|co| {
            body.vertices()[if co.reversed {
                body.edges()[co.edge.index()].end
            } else {
                body.edges()[co.edge.index()].start
            }
            .index()]
        })
}
fn inspect<F, E>(
    body: &Body<F, E>,
    operand: Operand,
    policy: &ValidationPolicy,
) -> Result<Vec<AxisFace>, RectilinearError> {
    body.validate(policy)
        .map_err(|error| RectilinearError::Boundary {
            operand: Some(operand),
            error,
        })?;
    if !body.is_closed() {
        return Err(RectilinearError::OpenInput { operand });
    }
    let mut result = Vec::new();
    for (index, face) in body.faces().iter().enumerate() {
        let id = FaceId(crate::usize_to_u32(index));
        let unsupported = || RectilinearError::UnsupportedFace { operand, face: id };
        let Surface::Plane(plane) = &face.surface else {
            return Err(unsupported());
        };
        let normal = plane.frame().rows.map(|r| r[2] * face.sense.sign());
        let Some(axis) =
            (0..3).find(|&i| normal[i].abs() == 1.0 && (0..3).all(|j| j == i || normal[j] == 0.0))
        else {
            return Err(unsupported());
        };
        let sign = if normal[axis] > 0.0 { 1 } else { -1 };
        if plane.frame().rows[axis][0] != 0.0 || plane.frame().rows[axis][1] != 0.0 {
            return Err(unsupported());
        }
        let coordinate = plane.frame().rows[axis][3];
        let mut loops = Vec::new();
        for (loop_index, loop_id) in face.loops.iter().enumerate() {
            let coedges = &body.loops()[loop_id.index()].coedges;
            if coedges.len() < 4 {
                return Err(unsupported());
            }
            let mut polygon = Vec::new();
            for co in coedges {
                if !matches!(
                    body.edges()[co.edge.index()].regular(),
                    Some((Curve::Line(_), _))
                ) {
                    return Err(unsupported());
                }
                let ids = body
                    .coedge_vertices(*co)
                    .ok_or(RectilinearError::NumericLimit)?;
                let a = body.vertices()[ids[0].index()];
                let b = body.vertices()[ids[1].index()];
                if a[axis] != coordinate || b[axis] != coordinate {
                    return Err(unsupported());
                }
                let p = [a[(axis + 1) % 3], a[(axis + 2) % 3]];
                let q = [b[(axis + 1) % 3], b[(axis + 2) % 3]];
                if (p[0] == q[0]) == (p[1] == q[1]) {
                    return Err(unsupported());
                }
                let (Curve::Line(line), _) = body.edges()[co.edge.index()]
                    .regular()
                    .ok_or_else(unsupported)?
                else {
                    return Err(unsupported());
                };
                let varying = (0..3).find(|&i| a[i] != b[i]).ok_or_else(unsupported)?;
                if (0..3).any(|i| {
                    i != varying && (line.direction()[i] != 0.0 || line.origin()[i] != a[i])
                }) {
                    return Err(unsupported());
                }
                polygon.push(p);
            }
            let anchor = polygon[0];
            let area = polygon
                .iter()
                .zip(polygon.iter().cycle().skip(1))
                .take(polygon.len())
                .map(|(a, b)| {
                    (a[0] - anchor[0]) * (b[1] - anchor[1])
                        - (a[1] - anchor[1]) * (b[0] - anchor[0])
                })
                .sum::<f64>();
            if !area.is_finite() || area == 0.0 || (area > 0.0) != ((sign > 0) == (loop_index == 0))
            {
                return Err(unsupported());
            }
            if !simple(&polygon) {
                return Err(unsupported());
            }
            loops.push(polygon);
        }
        for i in 0..loops.len() {
            for j in i + 1..loops.len() {
                if loops_touch(&loops[i], &loops[j]) {
                    return Err(unsupported());
                }
            }
            if i > 0
                && (!point_in_loop(loops[i][0], &loops[0])
                    || (1..i).any(|j| {
                        point_in_loop(loops[i][0], &loops[j])
                            || point_in_loop(loops[j][0], &loops[i])
                    }))
            {
                return Err(unsupported());
            }
        }
        result.push(AxisFace {
            id,
            axis,
            coordinate,
            sign,
            loops,
        });
    }
    Ok(result)
}
fn point_in_loop(p: [f64; 2], polygon: &[[f64; 2]]) -> bool {
    let mut inside = false;
    for i in 0..polygon.len() {
        let a = polygon[i];
        let b = polygon[(i + 1) % polygon.len()];
        if (a[1] > p[1]) != (b[1] > p[1]) && a[0] > p[0] {
            inside = !inside;
        }
    }
    inside
}
fn contains(face: &AxisFace, p: [f64; 2]) -> bool {
    point_in_loop(p, &face.loops[0]) && face.loops[1..].iter().all(|h| !point_in_loop(p, h))
}
fn classify(
    faces: &[AxisFace],
    point: [f64; 3],
    operand: Operand,
    cell: [usize; 3],
) -> Result<bool, RectilinearError> {
    let mut result = None;
    for axis in 0..3 {
        let p = [point[(axis + 1) % 3], point[(axis + 2) % 3]];
        let winding = faces
            .iter()
            .filter(|f| f.axis == axis && f.coordinate > point[axis] && contains(f, p))
            .map(|f| i64::from(f.sign))
            .sum::<i64>();
        if !(0..=1).contains(&winding) {
            return Err(RectilinearError::InvalidMaterial { operand, cell });
        }
        let inside = winding == 1;
        if result.is_some_and(|r| r != inside) {
            return Err(RectilinearError::InvalidMaterial { operand, cell });
        }
        result = Some(inside);
    }
    Ok(result.unwrap_or(false))
}
fn interval_touch(a: f64, b: f64, c: f64, d: f64) -> bool {
    a.min(b) <= c.max(d) && c.min(d) <= a.max(b)
}
fn segment_touch(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    if a[0] == b[0] {
        if c[0] == d[0] {
            a[0] == c[0] && interval_touch(a[1], b[1], c[1], d[1])
        } else {
            a[0] >= c[0].min(d[0])
                && a[0] <= c[0].max(d[0])
                && c[1] >= a[1].min(b[1])
                && c[1] <= a[1].max(b[1])
        }
    } else if c[1] == d[1] {
        a[1] == c[1] && interval_touch(a[0], b[0], c[0], d[0])
    } else {
        c[0] >= a[0].min(b[0])
            && c[0] <= a[0].max(b[0])
            && a[1] >= c[1].min(d[1])
            && a[1] <= c[1].max(d[1])
    }
}
fn simple(p: &[[f64; 2]]) -> bool {
    for i in 0..p.len() {
        for j in i + 1..p.len() {
            let a = p[i];
            let b = p[(i + 1) % p.len()];
            let c = p[j];
            let d = p[(j + 1) % p.len()];
            if !segment_touch(a, b, c, d) {
                continue;
            }
            let adjacent = j == i + 1 || (i == 0 && j == p.len() - 1);
            if !adjacent {
                return false;
            }
            if a[0] == b[0]
                && c[0] == d[0]
                && a[0] == c[0]
                && a[1].min(b[1]).max(c[1].min(d[1])) < a[1].max(b[1]).min(c[1].max(d[1]))
            {
                return false;
            }
            if a[1] == b[1]
                && c[1] == d[1]
                && a[1] == c[1]
                && a[0].min(b[0]).max(c[0].min(d[0])) < a[0].max(b[0]).min(c[0].max(d[0]))
            {
                return false;
            }
        }
    }
    true
}
fn loops_touch(a: &[[f64; 2]], b: &[[f64; 2]]) -> bool {
    (0..a.len()).any(|i| {
        (0..b.len()).any(|j| segment_touch(a[i], a[(i + 1) % a.len()], b[j], b[(j + 1) % b.len()]))
    })
}
fn encode(p: [usize; 3], d: [usize; 3]) -> usize {
    (p[0] * d[1] + p[1]) * d[2] + p[2]
}
fn decode(i: usize, d: [usize; 3]) -> [usize; 3] {
    [i / (d[1] * d[2]), (i / d[2]) % d[1], i % d[2]]
}
fn neighbor(mut p: [usize; 3], axis: usize, positive: bool, d: [usize; 3]) -> Option<[usize; 3]> {
    if positive {
        if p[axis] + 1 >= d[axis] {
            return None;
        }
        p[axis] += 1;
    } else {
        p[axis] = p[axis].checked_sub(1)?;
    }
    Some(p)
}
fn component_counts(material: &[bool], d: [usize; 3]) -> (usize, usize) {
    let mut seen = alloc::vec![false;material.len()];
    let mut components = 0;
    let mut cavities = 0;
    let mut stack = Vec::new();
    for start in 0..material.len() {
        if seen[start] {
            continue;
        }
        let filled = material[start];
        let mut exterior = false;
        seen[start] = true;
        stack.push(start);
        while let Some(i) = stack.pop() {
            let cell = decode(i, d);
            for axis in 0..3 {
                if cell[axis] == 0 || cell[axis] + 1 == d[axis] {
                    exterior = true;
                }
                for side in [false, true] {
                    if let Some(next) = neighbor(cell, axis, side, d) {
                        let j = encode(next, d);
                        if !seen[j] && material[j] == filled {
                            seen[j] = true;
                            stack.push(j);
                        }
                    }
                }
            }
        }
        if filled {
            components += 1;
        } else if !exterior {
            cavities += 1;
        }
    }
    (components, cavities)
}
fn contours(edges: &mut BTreeMap<GridEdge, ()>) -> Result<Vec<Vec<GridVertex>>, RectilinearError> {
    let mut degrees: BTreeMap<GridVertex, [usize; 2]> = BTreeMap::new();
    for &(a, b) in edges.keys() {
        degrees.entry(a).or_default()[0] += 1;
        degrees.entry(b).or_default()[1] += 1;
    }
    if degrees.values().any(|&d| d != [1, 1]) {
        return Err(RectilinearError::NonManifold);
    }
    let mut result = Vec::new();
    while let Some((&(start, _), _)) = edges.first_key_value() {
        let mut contour = Vec::new();
        let mut current = start;
        loop {
            let Some((&(a, b), _)) = edges
                .range((current, [0; 3])..=(current, [usize::MAX; 3]))
                .next()
            else {
                return Err(RectilinearError::NonManifold);
            };
            edges.remove(&(a, b));
            contour.push(a);
            current = b;
            if current == start {
                break;
            }
            if contour.len() > degrees.len() {
                return Err(RectilinearError::NonManifold);
            }
        }
        if contour.len() < 4 {
            return Err(RectilinearError::NonManifold);
        }
        result.push(contour);
    }
    Ok(result)
}
fn grid_area(contour: &[GridVertex], axis: usize) -> Result<i128, RectilinearError> {
    let u = (axis + 1) % 3;
    let v = (axis + 2) % 3;
    let mut area = 0;
    for i in 0..contour.len() {
        let a = contour[i];
        let b = contour[(i + 1) % contour.len()];
        let au = i128::try_from(a[u]).map_err(|_| RectilinearError::NumericLimit)?;
        let av = i128::try_from(a[v]).map_err(|_| RectilinearError::NumericLimit)?;
        let bu = i128::try_from(b[u]).map_err(|_| RectilinearError::NumericLimit)?;
        let bv = i128::try_from(b[v]).map_err(|_| RectilinearError::NumericLimit)?;
        area += au * bv - av * bu;
    }
    Ok(area)
}
fn project_grid_point(p: GridVertex, axis: usize, coordinates: &[Vec<f64>; 3]) -> [f64; 2] {
    [
        coordinates[(axis + 1) % 3][p[(axis + 1) % 3]],
        coordinates[(axis + 2) % 3][p[(axis + 2) % 3]],
    ]
}
fn point_in_grid_loop(
    p: [f64; 2],
    polygon: &[GridVertex],
    axis: usize,
    coordinates: &[Vec<f64>; 3],
) -> bool {
    let mut inside = false;
    for i in 0..polygon.len() {
        let a = project_grid_point(polygon[i], axis, coordinates);
        let b = project_grid_point(polygon[(i + 1) % polygon.len()], axis, coordinates);
        if (a[1] > p[1]) != (b[1] > p[1]) && a[0] > p[0] {
            inside = !inside;
        }
    }
    inside
}
fn grid_vertex(
    p: GridVertex,
    coordinates: &[Vec<f64>; 3],
    map: &mut BTreeMap<GridVertex, VertexId>,
    parts: &mut BodyParts,
) -> VertexId {
    *map.entry(p).or_insert_with(|| {
        let id = VertexId(crate::usize_to_u32(parts.vertices.len()));
        parts
            .vertices
            .push(core::array::from_fn(|i| coordinates[i][p[i]]));
        id
    })
}
fn manifold_vertices(body: &Body) -> bool {
    let mut links = alloc::vec![Vec::new();body.vertices().len()];
    for face in body.faces() {
        for l in &face.loops {
            let coedges = &body.loops()[l.index()].coedges;
            for i in 0..coedges.len() {
                let co = coedges[i];
                let prev = coedges[(i + coedges.len() - 1) % coedges.len()];
                let Some([v, _]) = body.coedge_vertices(co) else {
                    return false;
                };
                links[v.index()].push((prev.edge, co.edge));
            }
        }
    }
    for pairs in links {
        if pairs.is_empty() {
            continue;
        }
        let mut reached = BTreeSet::new();
        reached.insert(pairs[0].0);
        loop {
            let before = reached.len();
            for &(a, b) in &pairs {
                if reached.contains(&a) || reached.contains(&b) {
                    reached.insert(a);
                    reached.insert(b);
                }
            }
            if reached.len() == before {
                break;
            }
        }
        if pairs
            .iter()
            .any(|&(a, b)| !reached.contains(&a) || !reached.contains(&b))
        {
            return false;
        }
    }
    true
}
fn shell_count(body: &Body) -> usize {
    let mut reached = alloc::vec![false;body.faces().len()];
    let mut count = 0;
    for start in 0..body.faces().len() {
        if reached[start] {
            continue;
        }
        count += 1;
        reached[start] = true;
        loop {
            let mut progress = false;
            for i in 0..body.faces().len() {
                if !reached[i] {
                    continue;
                }
                for (j, face) in body.faces().iter().enumerate() {
                    if reached[j] {
                        continue;
                    }
                    if face
                        .loops
                        .iter()
                        .flat_map(|l| &body.loops()[l.index()].coedges)
                        .any(|co| {
                            body.faces()[i]
                                .loops
                                .iter()
                                .flat_map(|l| &body.loops()[l.index()].coedges)
                                .any(|other| other.edge == co.edge)
                        })
                    {
                        reached[j] = true;
                        progress = true;
                    }
                }
            }
            if !progress {
                break;
            }
        }
    }
    count
}
fn vertex_images<F, E>(source: &Body<F, E>, output: &Body) -> Vec<Option<VertexId>> {
    source
        .vertices()
        .iter()
        .map(|p| {
            output
                .vertices()
                .iter()
                .position(|q| p == q)
                .map(|i| VertexId(crate::usize_to_u32(i)))
        })
        .collect()
}
fn edge_images<F, E>(source: &Body<F, E>, output: &Body) -> Vec<Vec<EdgeId>> {
    source
        .edges()
        .iter()
        .map(|edge| {
            let a = source.vertices()[edge.start.index()];
            let b = source.vertices()[edge.end.index()];
            let axis = (0..3).find(|&i| a[i] != b[i]);
            output
                .edges()
                .iter()
                .enumerate()
                .filter_map(|(i, e)| {
                    let axis = axis?;
                    let p = output.vertices()[e.start.index()];
                    let q = output.vertices()[e.end.index()];
                    if (0..3).all(|j| j == axis || (a[j] == p[j] && a[j] == q[j]))
                        && p[axis] >= a[axis].min(b[axis])
                        && p[axis] <= a[axis].max(b[axis])
                        && q[axis] >= a[axis].min(b[axis])
                        && q[axis] <= a[axis].max(b[axis])
                    {
                        Some(EdgeId(crate::usize_to_u32(i)))
                    } else {
                        None
                    }
                })
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn block(min: [f64; 3], max: [f64; 3]) -> Body {
        let mut parts = BodyParts::default();
        let mut edge_ids = BTreeMap::new();
        for bits in 0..8 {
            parts.vertices.push(core::array::from_fn(|i| {
                if bits & (1 << i) == 0 { min[i] } else { max[i] }
            }));
        }
        for axis in 0..3 {
            for side in 0..2 {
                let u = (axis + 1) % 3;
                let v = (axis + 2) % 3;
                let bits = side << axis;
                let mut corners = [
                    bits,
                    bits | (1 << u),
                    bits | (1 << u) | (1 << v),
                    bits | (1 << v),
                ];
                if side == 0 {
                    corners.reverse();
                }
                let mut coedges = Vec::new();
                for i in 0..4 {
                    let a = VertexId(corners[i]);
                    let b = VertexId(corners[(i + 1) % 4]);
                    let key = if a < b { (a, b) } else { (b, a) };
                    let edge = *edge_ids.entry(key).or_insert_with(|| {
                        let edge = EdgeId(crate::usize_to_u32(parts.edges.len()));
                        let a = parts.vertices[key.0.index()];
                        let b = parts.vertices[key.1.index()];
                        parts.edges.push(Edge::new(
                            Curve::line(a, sub(b, a)).unwrap(),
                            ParameterRange::UNIT,
                            key.0,
                            key.1,
                            (),
                        ));
                        edge
                    });
                    coedges.push(Coedge {
                        edge,
                        reversed: a != key.0,
                    });
                }
                let l = LoopId(crate::usize_to_u32(parts.loops.len()));
                parts.loops.push(BoundaryLoop { coedges });
                let mut origin = [0.0; 3];
                origin[axis] = if side == 0 { min[axis] } else { max[axis] };
                let mut x = [0.0; 3];
                x[u] = 1.0;
                let mut y = [0.0; 3];
                y[v] = 1.0;
                let mut z = [0.0; 3];
                z[axis] = 1.0;
                parts.faces.push(Face {
                    surface: Surface::plane(Placement3::from_axes(x, y, z, origin)).unwrap(),
                    domain: ParameterDomain {
                        u: ParameterRange::new(min[u], max[u]).unwrap(),
                        v: ParameterRange::new(min[v], max[v]).unwrap(),
                    },
                    sense: if side == 0 {
                        Sense::Reverse
                    } else {
                        Sense::Forward
                    },
                    loops: alloc::vec![l],
                    role: (),
                });
            }
        }
        Body::try_from_parts(parts, &ValidationPolicy { max_gap: 1e-9 }).unwrap()
    }
    fn boolean(a: &Body, b: &Body, op: BooleanOperation) -> RectilinearResult {
        boolean_rectilinear(a, b, op, &RectilinearPolicy::default()).unwrap()
    }
    fn verify(result: &RectilinearResult, volume: f64, area: f64) {
        assert!(
            (result.report.volume - volume).abs() < 1e-9,
            "volume {:?}",
            result.report
        );
        assert!(
            (result.report.surface_area - area).abs() < 1e-9,
            "area {:?}",
            result.report
        );
        if volume > 0.0 {
            assert!(result.body.is_closed());
            assert!((0..result.body.faces().len()).all(|i| {
                result
                    .body
                    .has_explicit_trims(FaceId(crate::usize_to_u32(i)))
            }));
            let mesh = result
                .body
                .realize(
                    Placement3::IDENTITY,
                    &crate::realize::RealizationPolicy::default(),
                )
                .unwrap();
            assert!(mesh.mesh.validate_deep().is_empty());
            assert!(mesh.mesh.boundary_loops().unwrap().is_empty());
            assert!(mesh.report.within_tolerance());
        } else {
            assert!(result.body.faces().is_empty());
        }
    }
    #[test]
    fn exhaustive_two_by_two_prismatic_box_boolean_matches_independent_cells() {
        let ranges = [(0.0, 1.0), (0.0, 2.0), (1.0, 2.0)];
        for &(ax0, ax1) in &ranges {
            for &(ay0, ay1) in &ranges {
                for &(bx0, bx1) in &ranges {
                    for &(by0, by1) in &ranges {
                        let a = block([ax0, ay0, 0.0], [ax1, ay1, 1.0]);
                        let b = block([bx0, by0, 0.0], [bx1, by1, 1.0]);
                        for op in [
                            BooleanOperation::Union,
                            BooleanOperation::Intersection,
                            BooleanOperation::Difference,
                        ] {
                            let mut occupied = [[false; 2]; 2];
                            for (x, column) in occupied.iter_mut().enumerate() {
                                for (y, cell) in column.iter_mut().enumerate() {
                                    let px = f64::from(u32::try_from(x).unwrap()) + 0.5;
                                    let py = f64::from(u32::try_from(y).unwrap()) + 0.5;
                                    let left = px > ax0 && px < ax1 && py > ay0 && py < ay1;
                                    let right = px > bx0 && px < bx1 && py > by0 && py < by1;
                                    *cell = match op {
                                        BooleanOperation::Union => left || right,
                                        BooleanOperation::Intersection => left && right,
                                        BooleanOperation::Difference => left && !right,
                                    };
                                }
                            }
                            let mut volume = 0.0;
                            let mut area = 0.0;
                            for x in 0..2 {
                                for y in 0..2 {
                                    if !occupied[x][y] {
                                        continue;
                                    }
                                    volume += 1.0;
                                    area += 2.0;
                                    for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                                        let nx = i32::try_from(x).unwrap() + dx;
                                        let ny = i32::try_from(y).unwrap() + dy;
                                        if !(0..2).contains(&nx)
                                            || !(0..2).contains(&ny)
                                            || !occupied[usize::try_from(nx).unwrap()]
                                                [usize::try_from(ny).unwrap()]
                                        {
                                            area += 1.0;
                                        }
                                    }
                                }
                            }
                            let result =
                                boolean_rectilinear(&a, &b, op, &RectilinearPolicy::default());
                            let diagonal = (occupied[0][0]
                                && occupied[1][1]
                                && !occupied[0][1]
                                && !occupied[1][0])
                                || (occupied[0][1]
                                    && occupied[1][0]
                                    && !occupied[0][0]
                                    && !occupied[1][1]);
                            if diagonal {
                                assert!(matches!(result, Err(RectilinearError::NonManifold)));
                                continue;
                            }
                            let result = result.unwrap();
                            assert_eq!(result.report.volume, volume);
                            assert_eq!(result.report.surface_area, area);
                            assert!(
                                result.report.volume_bounds[0] <= volume
                                    && result.report.volume_bounds[1] >= volume
                            );
                            assert!(
                                result.report.surface_area_bounds[0] <= area
                                    && result.report.surface_area_bounds[1] >= area
                            );
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn exhaustive_two_by_two_by_two_box_boolean_matches_independent_cells() {
        let ranges = [(0.0, 1.0), (0.0, 2.0), (1.0, 2.0)];
        let mut boxes = Vec::new();
        for &x in &ranges {
            for &y in &ranges {
                for &z in &ranges {
                    boxes.push(([x.0, y.0, z.0], [x.1, y.1, z.1]));
                }
            }
        }
        for &(amin, amax) in &boxes {
            for &(bmin, bmax) in &boxes {
                let a = block(amin, amax);
                let b = block(bmin, bmax);
                for op in [
                    BooleanOperation::Union,
                    BooleanOperation::Intersection,
                    BooleanOperation::Difference,
                ] {
                    let occupied = core::array::from_fn::<_, 8, _>(|bits| {
                        let p = core::array::from_fn::<_, 3, _>(|i| {
                            if bits & (1 << i) == 0 { 0.5 } else { 1.5 }
                        });
                        let left = (0..3).all(|i| p[i] > amin[i] && p[i] < amax[i]);
                        let right = (0..3).all(|i| p[i] > bmin[i] && p[i] < bmax[i]);
                        match op {
                            BooleanOperation::Union => left || right,
                            BooleanOperation::Intersection => left && right,
                            BooleanOperation::Difference => left && !right,
                        }
                    });
                    let mut volume = 0.0;
                    let mut area = 0.0;
                    for bits in 0..8 {
                        if !occupied[bits] {
                            continue;
                        }
                        volume += 1.0;
                        for i in 0..3 {
                            let neighbor = bits ^ (1 << i);
                            area += 1.0;
                            if !occupied[neighbor] {
                                area += 1.0;
                            }
                        }
                    }
                    let widths = core::array::from_fn::<_, 3, _>(|i| {
                        amax[i].min(bmax[i]) - amin[i].max(bmin[i])
                    });
                    let contact = op == BooleanOperation::Union
                        && widths.iter().all(|&w| w >= 0.0)
                        && widths.iter().filter(|&&w| w == 0.0).count() >= 2;
                    let result = boolean_rectilinear(&a, &b, op, &RectilinearPolicy::default());
                    if contact {
                        assert!(matches!(result, Err(RectilinearError::NonManifold)));
                        continue;
                    }
                    let result = result.unwrap();
                    assert_eq!(result.report.volume, volume);
                    assert_eq!(result.report.surface_area, area);
                    assert!(
                        result.report.volume_bounds[0] <= volume
                            && result.report.volume_bounds[1] >= volume
                    );
                    assert!(
                        result.report.surface_area_bounds[0] <= area
                            && result.report.surface_area_bounds[1] >= area
                    );
                }
            }
        }
    }
    #[test]
    fn overlapping_coplanar_blocks_union_intersect_and_subtract() {
        let a = block([0.0; 3], [4.0, 4.0, 4.0]);
        let b = block([2.0, 0.0, 0.0], [6.0, 4.0, 4.0]);
        let union = boolean(&a, &b, BooleanOperation::Union);
        verify(&union, 96.0, 128.0);
        assert_eq!(union.body.faces().len(), 6);
        assert_eq!(union.report.material_components, 1);
        assert_eq!(union.report.boundary_shells, 1);
        let intersection = boolean(&a, &b, BooleanOperation::Intersection);
        verify(&intersection, 32.0, 64.0);
        let difference = boolean(&a, &b, BooleanOperation::Difference);
        verify(&difference, 32.0, 64.0);
        assert!(
            difference
                .report
                .face_sources
                .iter()
                .flatten()
                .any(|s| s.operand == Operand::Right && s.reversed)
        );
        assert_eq!(a, block([0.0; 3], [4.0, 4.0, 4.0]));
    }
    #[test]
    fn cavities_disconnected_and_empty_material_are_explicit() {
        let a = block([0.0; 3], [10.0; 3]);
        let cavity = block([2.0; 3], [8.0; 3]);
        let cut = boolean(&a, &cavity, BooleanOperation::Difference);
        verify(&cut, 784.0, 816.0);
        assert_eq!(cut.report.material_components, 1);
        assert_eq!(cut.report.cavities, 1);
        assert_eq!(cut.report.boundary_shells, 2);
        let far = block([20.0, 0.0, 0.0], [24.0, 4.0, 4.0]);
        let union = boolean(&a, &far, BooleanOperation::Union);
        verify(&union, 1064.0, 696.0);
        assert_eq!(union.report.material_components, 2);
        assert_eq!(union.report.boundary_shells, 2);
        verify(&boolean(&a, &far, BooleanOperation::Intersection), 0.0, 0.0);
        verify(&boolean(&a, &a, BooleanOperation::Difference), 0.0, 0.0);
    }
    #[test]
    fn nonorthogonal_support_and_far_thin_intervals_refuse() {
        let outline = exedra_profile::Loop::new(alloc::vec![
            exedra_profile::Segment::line([2.0, 0.0]),
            exedra_profile::Segment::line([0.0, 2.0]),
            exedra_profile::Segment::line([-2.0, 0.0]),
            exedra_profile::Segment::line([0.0, -2.0])
        ])
        .unwrap();
        let diagonal = crate::extrusion::Extrusion::new(
            exedra_profile::Profile::new(
                outline,
                alloc::vec![],
                &exedra_profile::ProfilePolicy::new(1e-9),
            )
            .unwrap(),
            Placement3::IDENTITY,
            2.0,
        )
        .unwrap();
        let source = block([0.0; 3], [4.0; 3]);
        assert!(matches!(
            boolean_rectilinear(
                &source,
                diagonal.body(),
                BooleanOperation::Union,
                &RectilinearPolicy::default()
            ),
            Err(RectilinearError::UnsupportedFace { .. })
        ));
        let huge = block([1e16, 0.0, 0.0], [1e16 + 2.0, 4.0, 4.0]);
        assert!(matches!(
            boolean_rectilinear(
                &huge,
                &huge,
                BooleanOperation::Union,
                &RectilinearPolicy {
                    geometry_gap: 1e6,
                    ..Default::default()
                }
            ),
            Err(RectilinearError::NumericLimit) | Err(RectilinearError::PlaneSpacing { .. })
        ));
    }
    #[test]
    fn edge_vertex_contacts_and_subresolution_planes_refuse() {
        let a = block([0.0; 3], [4.0; 3]);
        for b in [
            block([4.0, 4.0, 0.0], [8.0, 8.0, 4.0]),
            block([4.0; 3], [8.0; 3]),
        ] {
            assert!(matches!(
                boolean_rectilinear(
                    &a,
                    &b,
                    BooleanOperation::Union,
                    &RectilinearPolicy::default()
                ),
                Err(RectilinearError::NonManifold)
            ));
        }
        let b = block([4.0 + 1e-8, 0.0, 0.0], [8.0, 4.0, 4.0]);
        assert!(matches!(
            boolean_rectilinear(
                &a,
                &b,
                BooleanOperation::Union,
                &RectilinearPolicy::default()
            ),
            Err(RectilinearError::PlaneSpacing { .. })
        ));
        assert!(matches!(
            boolean_rectilinear(
                &a,
                &a,
                BooleanOperation::Union,
                &RectilinearPolicy {
                    max_cells: 0,
                    ..Default::default()
                }
            ),
            Err(RectilinearError::InvalidPolicy)
        ));
        let b = block([1.0; 3], [5.0; 3]);
        assert!(matches!(
            boolean_rectilinear(
                &a,
                &b,
                BooleanOperation::Union,
                &RectilinearPolicy {
                    max_cells: 1,
                    ..Default::default()
                }
            ),
            Err(RectilinearError::CellBudget { .. })
        ));
    }
    #[test]
    fn face_contact_union_is_manifold_and_reusable_for_cylinder_machining() {
        let base = block([-20.0, -15.0, 0.0], [20.0, 15.0, 10.0]);
        let lug = block([20.0, -5.0, 0.0], [35.0, 5.0, 10.0]);
        let united = boolean(&base, &lug, BooleanOperation::Union);
        verify(&united, 13500.0, 4400.0);
        assert_eq!(united.report.material_components, 1);
        let drilled = crate::machining::subtract_through_cylinder(
            &united.body,
            &crate::machining::CylinderTool::new([27.5, 0.0, 0.0], [0.0, 0.0, 1.0], 2.0).unwrap(),
            &crate::machining::MachiningPolicy::default(),
        )
        .unwrap();
        assert!(drilled.body.is_closed());
        let mesh = drilled
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
    }
}
