// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! First dual-contouring extraction path for implicit scalar fields.

extern crate alloc;

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

use exedra::{BuildError, FaceBuildAttrs, Mesh, MeshBuilder, attr, op};
use exedra_qef::{PlaneConstraint, QefBounds, QefParams, QefSolveError, QefSolver, SharpnessClass};
use exedra_spatial::{Aabb, CellRef, Octree, OctreeVisitor};

use crate::{
    CellHermiteData, EdgeSearchParams, ProvenanceField, ScalarField, locate_edge_intersection,
};

const CUBE_EDGES: [(usize, usize); 12] = [
    (0, 1),
    (1, 3),
    (2, 3),
    (0, 2),
    (4, 5),
    (5, 7),
    (6, 7),
    (4, 6),
    (0, 4),
    (1, 5),
    (3, 7),
    (2, 6),
];

const MIN_ADAPTIVE_DEPTH: u8 = 2;
const SIMPLE_CELL_MAX_CROSSINGS: usize = 3;

/// Parameters controlling dual-contouring extraction.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct DualContourParams {
    /// Root domain bounds.
    pub root_bounds: Aabb,
    /// Maximum octree depth and uniform extraction resolution.
    pub max_depth: u8,
    /// Optional cap on active cells emitted into the dual mesh.
    pub cell_budget: Option<usize>,
    /// Edge intersection search parameters.
    pub edge_search: EdgeSearchParams,
    /// QEF solve parameters.
    pub qef: QefParams,
}

/// Extraction statistics for one dual-contouring run.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct DualContourStats {
    /// Total octree cells stored after interval-driven subdivision.
    pub octree_cells: usize,
    /// Intersecting octree leaves that contributed one DC vertex.
    pub active_cells: usize,
    /// Output vertex count.
    pub vertices: usize,
    /// Output face count.
    pub faces: usize,
}

/// Successful dual-contouring result.
#[derive(Clone, Debug)]
pub struct DualContourResult {
    /// Output mesh.
    pub mesh: Mesh,
    /// Extraction statistics.
    pub stats: DualContourStats,
}

/// Dual-contouring extraction failure.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum DualContourError {
    /// The generated polygon mesh failed to build.
    Build(BuildError),
    /// QEF solve failed for an active cell.
    Solve(QefSolveError),
}

impl fmt::Display for DualContourError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Build(error) => write!(f, "dual contour mesh build failed: {error}"),
            Self::Solve(error) => write!(f, "dual contour QEF solve failed: {error:?}"),
        }
    }
}

impl core::error::Error for DualContourError {}

/// Extracts a mesh from `field`, defaulting all face regions to `0`.
pub fn dual_contour<F: ScalarField>(
    field: &F,
    params: &DualContourParams,
) -> Result<DualContourResult, DualContourError> {
    dual_contour_impl(field, params, |_| 0)
}

/// Extracts a mesh from `field`, sampling `FACE_REGION` from field provenance.
pub fn dual_contour_with_regions<F>(
    field: &F,
    params: &DualContourParams,
) -> Result<DualContourResult, DualContourError>
where
    F: ProvenanceField<Provenance = u32>,
{
    dual_contour_impl(field, params, |point| field.point_provenance(point))
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
struct LeafMarker {
    intersects_surface: bool,
}

#[derive(Copy, Clone, Debug)]
struct ActiveCell {
    coord: (u32, u32, u32),
    depth: u8,
    span: u32,
    position: [f32; 3],
    sharpness: SharpnessClass,
}

#[derive(Copy, Clone, Debug)]
struct VertexEntry {
    builder_index: u32,
    position: [f32; 3],
    sharpness: f32,
}

fn dual_contour_impl<F, R>(
    field: &F,
    params: &DualContourParams,
    region_at: R,
) -> Result<DualContourResult, DualContourError>
where
    F: ScalarField,
    R: Fn([f32; 3]) -> u32,
{
    let resolution = 1_u32 << params.max_depth;
    let lattice = sample_lattice(field, params.root_bounds, resolution);
    let mut visitor = IntervalVisitor {
        field,
        lattice: &lattice,
        root_bounds: params.root_bounds,
        resolution,
    };
    let tree = Octree::build(params.root_bounds, params.max_depth, &mut visitor);

    let mut active_cells = collect_active_cells(field, params, &tree, &lattice)?;

    if active_cells.is_empty() {
        return Ok(DualContourResult {
            mesh: MeshBuilder::new()
                .build()
                .expect("empty mesh build should succeed")
                .mesh,
            stats: DualContourStats {
                octree_cells: tree.len(),
                active_cells: 0,
                vertices: 0,
                faces: 0,
            },
        });
    }

    active_cells.sort_by_key(|cell| (cell.coord, cell.depth));

    let mut builder = MeshBuilder::new();
    let mut coverage = vec![None; finest_cell_count(resolution)];
    for cell in &active_cells {
        let builder_index = builder.push_vertex(cell.position);
        let entry = VertexEntry {
            builder_index,
            position: cell.position,
            sharpness: sharpness_value(cell.sharpness),
        };
        fill_coverage(&mut coverage, resolution, cell.coord, cell.span, entry);
    }

    let mut face_count = 0_usize;
    let mut emitted = BTreeSet::new();
    let mut edge_use = BTreeMap::new();
    for axis in 0..3 {
        emit_axis_faces(
            axis,
            resolution,
            &lattice,
            &coverage,
            &mut builder,
            &region_at,
            &mut emitted,
            &mut edge_use,
            &mut face_count,
        )?;
    }

    let result = builder.build().map_err(DualContourError::Build)?;
    let mut mesh = result.mesh;
    populate_corner_normals(field, &mut mesh);
    populate_region_boundary_seams(&mut mesh);
    Ok(DualContourResult {
        stats: DualContourStats {
            octree_cells: tree.len(),
            active_cells: active_cells.len(),
            vertices: mesh.vertices().count(),
            faces: mesh.faces().count(),
        },
        mesh,
    })
}

fn emit_axis_faces<R>(
    axis: usize,
    resolution: u32,
    lattice: &[f32],
    coverage: &[Option<VertexEntry>],
    builder: &mut MeshBuilder,
    region_at: &R,
    emitted: &mut BTreeSet<[u32; 3]>,
    edge_use: &mut BTreeMap<(u32, u32), u8>,
    face_count: &mut usize,
) -> Result<(), DualContourError>
where
    R: Fn([f32; 3]) -> u32,
{
    match axis {
        0 => {
            for k in 1..resolution {
                for j in 1..resolution {
                    for i in 0..resolution {
                        emit_one_face(
                            lattice,
                            resolution,
                            ((i, j, k), (i + 1, j, k)),
                            [(i, j - 1, k - 1), (i, j, k - 1), (i, j, k), (i, j - 1, k)],
                            coverage,
                            builder,
                            region_at,
                            emitted,
                            edge_use,
                            face_count,
                        )?;
                    }
                }
            }
        }
        1 => {
            for k in 1..resolution {
                for j in 0..resolution {
                    for i in 1..resolution {
                        emit_one_face(
                            lattice,
                            resolution,
                            ((i, j, k), (i, j + 1, k)),
                            [(i - 1, j, k - 1), (i - 1, j, k), (i, j, k), (i, j, k - 1)],
                            coverage,
                            builder,
                            region_at,
                            emitted,
                            edge_use,
                            face_count,
                        )?;
                    }
                }
            }
        }
        _ => {
            for k in 0..resolution {
                for j in 1..resolution {
                    for i in 1..resolution {
                        emit_one_face(
                            lattice,
                            resolution,
                            ((i, j, k), (i, j, k + 1)),
                            [(i - 1, j - 1, k), (i, j - 1, k), (i, j, k), (i - 1, j, k)],
                            coverage,
                            builder,
                            region_at,
                            emitted,
                            edge_use,
                            face_count,
                        )?;
                    }
                }
            }
        }
    }

    Ok(())
}

fn emit_one_face<R>(
    lattice: &[f32],
    resolution: u32,
    edge: ((u32, u32, u32), (u32, u32, u32)),
    surrounding: [(u32, u32, u32); 4],
    coverage: &[Option<VertexEntry>],
    builder: &mut MeshBuilder,
    region_at: &R,
    emitted: &mut BTreeSet<[u32; 3]>,
    edge_use: &mut BTreeMap<(u32, u32), u8>,
    face_count: &mut usize,
) -> Result<(), DualContourError>
where
    R: Fn([f32; 3]) -> u32,
{
    let start_value = lattice_value(lattice, resolution, edge.0);
    let end_value = lattice_value(lattice, resolution, edge.1);
    if !edge_has_crossing(start_value, end_value) {
        return Ok(());
    }

    let Some(mut face) = surrounding_vertices(surrounding, coverage, resolution) else {
        return Ok(());
    };
    if start_value > 0.0 {
        face.reverse();
    }

    let region = region_at(average_points(&face));
    match face.len() {
        3 => {
            let builder_loop = [
                face[0].builder_index,
                face[1].builder_index,
                face[2].builder_index,
            ];
            let sharpness = loop_sharpness3(&face);
            if try_mark_triangle(builder_loop, emitted, edge_use) {
                builder
                    .add_face_with_attrs(
                        &builder_loop,
                        &FaceBuildAttrs {
                            region: Some(region),
                            edge_sharpness: Some(&sharpness),
                            ..FaceBuildAttrs::default()
                        },
                    )
                    .map_err(DualContourError::Build)?;
                *face_count += 1;
            }
        }
        4 => {
            let positions = [
                face[0].position,
                face[1].position,
                face[2].position,
                face[3].position,
            ];
            let sharpness = loop_sharpness4(&face);
            let builder_loop = [
                face[0].builder_index,
                face[1].builder_index,
                face[2].builder_index,
                face[3].builder_index,
            ];
            match select_quad_diagonal(positions) {
                QuadDiagonal::ZeroTwo => {
                    let tri0 = [builder_loop[0], builder_loop[1], builder_loop[2]];
                    let tri0_sharpness = [sharpness[0], sharpness[1], 0.0];
                    if try_mark_triangle(tri0, emitted, edge_use) {
                        builder
                            .add_face_with_attrs(
                                &tri0,
                                &FaceBuildAttrs {
                                    region: Some(region),
                                    edge_sharpness: Some(&tri0_sharpness),
                                    ..FaceBuildAttrs::default()
                                },
                            )
                            .map_err(DualContourError::Build)?;
                        *face_count += 1;
                    }

                    let tri1 = [builder_loop[0], builder_loop[2], builder_loop[3]];
                    let tri1_sharpness = [0.0, sharpness[2], sharpness[3]];
                    if try_mark_triangle(tri1, emitted, edge_use) {
                        builder
                            .add_face_with_attrs(
                                &tri1,
                                &FaceBuildAttrs {
                                    region: Some(region),
                                    edge_sharpness: Some(&tri1_sharpness),
                                    ..FaceBuildAttrs::default()
                                },
                            )
                            .map_err(DualContourError::Build)?;
                        *face_count += 1;
                    }
                }
                QuadDiagonal::OneThree => {
                    let tri0 = [builder_loop[0], builder_loop[1], builder_loop[3]];
                    let tri0_sharpness = [sharpness[0], 0.0, sharpness[3]];
                    if try_mark_triangle(tri0, emitted, edge_use) {
                        builder
                            .add_face_with_attrs(
                                &tri0,
                                &FaceBuildAttrs {
                                    region: Some(region),
                                    edge_sharpness: Some(&tri0_sharpness),
                                    ..FaceBuildAttrs::default()
                                },
                            )
                            .map_err(DualContourError::Build)?;
                        *face_count += 1;
                    }

                    let tri1 = [builder_loop[1], builder_loop[2], builder_loop[3]];
                    let tri1_sharpness = [sharpness[1], sharpness[2], 0.0];
                    if try_mark_triangle(tri1, emitted, edge_use) {
                        builder
                            .add_face_with_attrs(
                                &tri1,
                                &FaceBuildAttrs {
                                    region: Some(region),
                                    edge_sharpness: Some(&tri1_sharpness),
                                    ..FaceBuildAttrs::default()
                                },
                            )
                            .map_err(DualContourError::Build)?;
                        *face_count += 1;
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum QuadDiagonal {
    ZeroTwo,
    OneThree,
}

fn select_quad_diagonal(points: [[f32; 3]; 4]) -> QuadDiagonal {
    if squared_distance(points[1], points[3]) < squared_distance(points[0], points[2]) {
        QuadDiagonal::OneThree
    } else {
        QuadDiagonal::ZeroTwo
    }
}

fn canonical_triangle(mut triangle: [u32; 3]) -> [u32; 3] {
    triangle.sort_unstable();
    triangle
}

fn try_mark_triangle(
    triangle: [u32; 3],
    emitted: &mut BTreeSet<[u32; 3]>,
    edge_use: &mut BTreeMap<(u32, u32), u8>,
) -> bool {
    let canonical = canonical_triangle(triangle);
    if !emitted.insert(canonical) {
        return false;
    }

    let edges = triangle_edges(triangle);
    if edges
        .iter()
        .any(|edge| edge_use.get(edge).copied().unwrap_or(0) >= 2)
    {
        emitted.remove(&canonical);
        return false;
    }

    for edge in edges {
        *edge_use.entry(edge).or_insert(0) += 1;
    }
    true
}

fn triangle_edges(triangle: [u32; 3]) -> [(u32, u32); 3] {
    [
        canonical_edge(triangle[0], triangle[1]),
        canonical_edge(triangle[1], triangle[2]),
        canonical_edge(triangle[2], triangle[0]),
    ]
}

fn canonical_edge(a: u32, b: u32) -> (u32, u32) {
    if a < b { (a, b) } else { (b, a) }
}

fn surrounding_vertices(
    coords: [(u32, u32, u32); 4],
    coverage: &[Option<VertexEntry>],
    resolution: u32,
) -> Option<Vec<VertexEntry>> {
    let mut out = Vec::with_capacity(4);
    for coord in coords {
        let entry = coverage_entry(coverage, resolution, coord)?;
        if out
            .iter()
            .any(|existing: &VertexEntry| existing.builder_index == entry.builder_index)
        {
            continue;
        }
        out.push(entry);
    }
    if out.len() < 3 {
        return None;
    }
    Some(out)
}

fn solve_active_cell<F: ScalarField>(
    field: &F,
    params: &DualContourParams,
    lattice: &[f32],
    bounds: Aabb,
    depth: u8,
) -> Result<Option<ActiveCell>, DualContourError> {
    let resolution = 1_u32 << params.max_depth;
    let coord = cell_coord(params.root_bounds, bounds, resolution);
    let span = cell_span(resolution, depth);
    let corner_values = cell_corner_values(lattice, resolution, coord, span);
    let corner_signs = corner_sign_mask(&corner_values);
    if corner_signs == 0 || corner_signs == 0xFF {
        return Ok(None);
    }

    let corner_positions = cube_corner_positions(bounds);
    let mut hermite = CellHermiteData::new(corner_signs);
    for (edge_index, &(start_corner, end_corner)) in CUBE_EDGES.iter().enumerate() {
        let start_value = corner_values[start_corner];
        let end_value = corner_values[end_corner];
        if !edge_has_crossing(start_value, end_value) {
            continue;
        }
        let intersection = locate_edge_intersection(
            field,
            corner_positions[start_corner],
            corner_positions[end_corner],
            &params.edge_search,
        );
        let Ok(intersection) = intersection else {
            continue;
        };
        hermite.push(
            u8::try_from(edge_index).expect("cube edge index fits into u8"),
            intersection,
        );
    }
    if hermite.intersections.is_empty() {
        return Ok(None);
    }

    let mut solver = QefSolver::new();
    for hit in &hermite.intersections {
        let _ = solver.add_constraint(PlaneConstraint {
            position: hit.intersection.position,
            normal: hit.intersection.normal,
        });
    }
    if solver.constraint_count() == 0 {
        return Ok(Some(ActiveCell {
            coord,
            depth,
            span,
            position: bounds.center(),
            sharpness: SharpnessClass::Smooth,
        }));
    }
    let result = solver
        .solve_with_anchor(
            QefBounds::new(bounds.min, bounds.max).expect("cell bounds are valid"),
            hermite_mass_point(&hermite),
            &params.qef,
        )
        .map_err(DualContourError::Solve)?;

    Ok(Some(ActiveCell {
        coord,
        depth,
        span,
        position: result.position,
        sharpness: result.sharpness_class,
    }))
}

fn collect_active_cells<F: ScalarField>(
    field: &F,
    params: &DualContourParams,
    tree: &Octree<LeafMarker>,
    lattice: &[f32],
) -> Result<Vec<ActiveCell>, DualContourError> {
    let mut active_cells = Vec::new();
    for leaf_id in tree.leaf_ids() {
        if params
            .cell_budget
            .is_some_and(|budget| active_cells.len() >= budget)
        {
            break;
        }
        let Some(cell) = tree.cell(leaf_id) else {
            continue;
        };
        if !cell
            .payload()
            .copied()
            .is_some_and(|payload| payload.intersects_surface)
        {
            continue;
        }
        let Some(cell_data) = solve_active_cell(field, params, lattice, cell.bounds, cell.depth)?
        else {
            continue;
        };
        active_cells.push(cell_data);
    }
    Ok(active_cells)
}

fn sample_lattice<F: ScalarField>(field: &F, root_bounds: Aabb, resolution: u32) -> Vec<f32> {
    let step = step_size(root_bounds, resolution);
    let point_count = usize::try_from((resolution + 1).pow(3)).expect("grid fits in usize");
    let mut points = Vec::with_capacity(point_count);
    for z in 0..=resolution {
        for y in 0..=resolution {
            for x in 0..=resolution {
                points.push(grid_point(root_bounds, step, (x, y, z)));
            }
        }
    }
    let mut values = vec![0.0_f32; points.len()];
    field.eval_points(&points, &mut values);
    values
}

fn lattice_value(lattice: &[f32], resolution: u32, coord: (u32, u32, u32)) -> f32 {
    let stride = usize::try_from(resolution + 1).expect("resolution fits usize");
    let x = coord.0 as usize;
    let y = coord.1 as usize;
    let z = coord.2 as usize;
    lattice[(z * stride + y) * stride + x]
}

fn step_size(root_bounds: Aabb, resolution: u32) -> [f32; 3] {
    let extent = root_bounds.extent();
    [
        extent[0] / resolution as f32,
        extent[1] / resolution as f32,
        extent[2] / resolution as f32,
    ]
}

fn grid_point(root_bounds: Aabb, step: [f32; 3], coord: (u32, u32, u32)) -> [f32; 3] {
    [
        root_bounds.min[0] + step[0] * coord.0 as f32,
        root_bounds.min[1] + step[1] * coord.1 as f32,
        root_bounds.min[2] + step[2] * coord.2 as f32,
    ]
}

fn cell_span(resolution: u32, depth: u8) -> u32 {
    resolution >> depth
}

fn cell_corner_values(
    lattice: &[f32],
    resolution: u32,
    coord: (u32, u32, u32),
    span: u32,
) -> [f32; 8] {
    core::array::from_fn(|corner| {
        lattice_value(
            lattice,
            resolution,
            (
                coord.0 + bit_u32(corner, 0) * span,
                coord.1 + bit_u32(corner, 1) * span,
                coord.2 + bit_u32(corner, 2) * span,
            ),
        )
    })
}

fn corner_sign_mask(corner_values: &[f32; 8]) -> u8 {
    corner_values
        .iter()
        .enumerate()
        .fold(0_u8, |mask, (corner, value)| {
            if *value <= 0.0 {
                mask | (1_u8 << corner)
            } else {
                mask
            }
        })
}

fn crossing_edge_count(corner_values: &[f32; 8]) -> usize {
    CUBE_EDGES
        .iter()
        .filter(|&&(start_corner, end_corner)| {
            edge_has_crossing(corner_values[start_corner], corner_values[end_corner])
        })
        .count()
}

fn cube_corner_positions(bounds: Aabb) -> [[f32; 3]; 8] {
    core::array::from_fn(|corner| {
        [
            if (corner & 0b001) != 0 {
                bounds.max[0]
            } else {
                bounds.min[0]
            },
            if (corner & 0b010) != 0 {
                bounds.max[1]
            } else {
                bounds.min[1]
            },
            if (corner & 0b100) != 0 {
                bounds.max[2]
            } else {
                bounds.min[2]
            },
        ]
    })
}

fn sharpness_value(sharpness: SharpnessClass) -> f32 {
    match sharpness {
        SharpnessClass::Smooth => 0.0,
        SharpnessClass::Edge => 1.0,
        SharpnessClass::Corner => 2.0,
    }
}

fn average_points(points: &[VertexEntry]) -> [f32; 3] {
    let sum = points.iter().fold([0.0; 3], |mut acc, entry| {
        let point = entry.position;
        acc[0] += point[0];
        acc[1] += point[1];
        acc[2] += point[2];
        acc
    });
    let inv = 1.0 / points.len() as f32;
    [sum[0] * inv, sum[1] * inv, sum[2] * inv]
}

fn loop_sharpness3(face: &[VertexEntry]) -> [f32; 3] {
    [
        face[0].sharpness.max(face[1].sharpness),
        face[1].sharpness.max(face[2].sharpness),
        face[2].sharpness.max(face[0].sharpness),
    ]
}

fn loop_sharpness4(face: &[VertexEntry]) -> [f32; 4] {
    [
        face[0].sharpness.max(face[1].sharpness),
        face[1].sharpness.max(face[2].sharpness),
        face[2].sharpness.max(face[3].sharpness),
        face[3].sharpness.max(face[0].sharpness),
    ]
}

fn squared_distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    dx * dx + dy * dy + dz * dz
}

fn edge_has_crossing(start: f32, end: f32) -> bool {
    (start <= 0.0 && end > 0.0) || (start > 0.0 && end <= 0.0)
}

fn bit_u32(value: usize, shift: u32) -> u32 {
    if (value & (1_usize << shift)) != 0 {
        1
    } else {
        0
    }
}

fn cell_coord(root_bounds: Aabb, bounds: Aabb, resolution: u32) -> (u32, u32, u32) {
    let step = step_size(root_bounds, resolution);
    (
        coordinate_axis(root_bounds.min[0], bounds.min[0], step[0]),
        coordinate_axis(root_bounds.min[1], bounds.min[1], step[1]),
        coordinate_axis(root_bounds.min[2], bounds.min[2], step[2]),
    )
}

fn coordinate_axis(root_min: f32, value: f32, step: f32) -> u32 {
    let mut coord = 0_u32;
    let mut best_distance = abs(value - root_min);
    loop {
        let next = coord + 1;
        let next_value = root_min + step * next as f32;
        let next_distance = abs(value - next_value);
        if next_distance + 1.0e-6 < best_distance {
            coord = next;
            best_distance = next_distance;
        } else {
            break;
        }
    }
    coord
}

struct IntervalVisitor<'a, F> {
    field: &'a F,
    lattice: &'a [f32],
    root_bounds: Aabb,
    resolution: u32,
}

impl<F: ScalarField> OctreeVisitor for IntervalVisitor<'_, F> {
    type Payload = LeafMarker;

    fn should_subdivide(&mut self, cell: CellRef) -> bool {
        let intersects = self
            .field
            .eval_interval(&cell.bounds)
            .is_none_or(interval_crosses_zero);
        if !intersects {
            return false;
        }
        if cell.depth < MIN_ADAPTIVE_DEPTH {
            return true;
        }
        let coord = cell_coord(self.root_bounds, cell.bounds, self.resolution);
        let span = cell_span(self.resolution, cell.depth);
        let corner_values = cell_corner_values(self.lattice, self.resolution, coord, span);
        let signs = corner_sign_mask(&corner_values);
        if signs == 0 || signs == 0xFF {
            return false;
        }
        crossing_edge_count(&corner_values) > SIMPLE_CELL_MAX_CROSSINGS
    }

    fn make_leaf_payload(&mut self, cell: CellRef) -> Self::Payload {
        LeafMarker {
            intersects_surface: self
                .field
                .eval_interval(&cell.bounds)
                .is_none_or(interval_crosses_zero),
        }
    }
}

fn finest_cell_count(resolution: u32) -> usize {
    usize::try_from(resolution)
        .expect("resolution fits usize")
        .pow(3)
}

fn coverage_index(resolution: u32, coord: (u32, u32, u32)) -> usize {
    let stride = usize::try_from(resolution).expect("resolution fits usize");
    let x = coord.0 as usize;
    let y = coord.1 as usize;
    let z = coord.2 as usize;
    (z * stride + y) * stride + x
}

fn fill_coverage(
    coverage: &mut [Option<VertexEntry>],
    resolution: u32,
    coord: (u32, u32, u32),
    span: u32,
    entry: VertexEntry,
) {
    for z in coord.2..coord.2 + span {
        for y in coord.1..coord.1 + span {
            for x in coord.0..coord.0 + span {
                coverage[coverage_index(resolution, (x, y, z))] = Some(entry);
            }
        }
    }
}

fn coverage_entry(
    coverage: &[Option<VertexEntry>],
    resolution: u32,
    coord: (u32, u32, u32),
) -> Option<VertexEntry> {
    coverage
        .get(coverage_index(resolution, coord))
        .copied()
        .flatten()
}

fn interval_crosses_zero(interval: [f32; 2]) -> bool {
    interval[0] <= 0.0 && interval[1] >= 0.0
}

fn populate_corner_normals<F: ScalarField>(field: &F, mesh: &mut Mesh) {
    let mut corners = Vec::new();
    let mut sample_points = Vec::new();
    for face in mesh.faces() {
        let loop_corners = mesh.face_loop(face).collect::<Vec<_>>();
        let face_center = face_centroid(mesh, &loop_corners);
        for corner in loop_corners {
            let Some(vertex) = mesh.to_vertex(corner) else {
                continue;
            };
            let Some(position) = mesh.vertex_position(vertex).copied() else {
                continue;
            };
            corners.push(corner);
            sample_points.push(inset_corner_sample(position, face_center));
        }
    }

    if corners.is_empty() {
        return;
    }

    let mut gradients = vec![[0.0_f32; 4]; sample_points.len()];
    field.eval_gradients(&sample_points, &mut gradients);

    let mut session = mesh.edit();
    for (corner, gradient) in corners.into_iter().zip(gradients) {
        if let Some(normal) = normalize_gradient(gradient) {
            op::set_corner_normal_override(&mut session, corner, Some(normal))
                .expect("collected corner must stay live during normal population");
        }
    }
    let _: () = session.finish();
}

fn populate_region_boundary_seams(mesh: &mut Mesh) {
    let Some(region_layer) = mesh.attrs().dense(attr::FACE_REGION) else {
        return;
    };

    let mut seam_edges = Vec::new();
    for face in mesh.faces() {
        let Some(face_region) = region_layer.get(face.as_id()).copied() else {
            continue;
        };
        for corner in mesh.face_loop(face) {
            let Some(twin) = mesh.twin(corner) else {
                continue;
            };
            if twin < corner {
                continue;
            }
            let Some(other_face) = mesh.face(twin) else {
                continue;
            };
            if other_face == exedra::FaceId::OUTSIDE {
                continue;
            }
            let Some(other_region) = region_layer.get(other_face.as_id()).copied() else {
                continue;
            };
            if face_region != other_region {
                seam_edges.push(corner);
            }
        }
    }

    if seam_edges.is_empty() {
        return;
    }

    let mut session = mesh.edit();
    for half_edge in seam_edges {
        op::set_edge_seam(&mut session, half_edge, true)
            .expect("collected seam edge must stay live during seam tagging");
    }
    let _: () = session.finish();
}

fn hermite_mass_point(hermite: &CellHermiteData) -> [f32; 3] {
    let sum = hermite
        .intersections
        .iter()
        .fold([0.0_f32; 3], |mut acc, hit| {
            acc[0] += hit.intersection.position[0];
            acc[1] += hit.intersection.position[1];
            acc[2] += hit.intersection.position[2];
            acc
        });
    let inv = 1.0 / hermite.intersections.len() as f32;
    [sum[0] * inv, sum[1] * inv, sum[2] * inv]
}

fn abs(value: f32) -> f32 {
    #[cfg(feature = "std")]
    {
        value.abs()
    }
    #[cfg(all(not(feature = "std"), feature = "libm"))]
    {
        libm::fabsf(value)
    }
}

fn sqrt(value: f32) -> f32 {
    #[cfg(feature = "std")]
    {
        value.sqrt()
    }
    #[cfg(all(not(feature = "std"), feature = "libm"))]
    {
        libm::sqrtf(value)
    }
}

fn face_centroid(mesh: &Mesh, corners: &[exedra::CornerId]) -> [f32; 3] {
    let mut sum = [0.0_f32; 3];
    let mut count = 0_u32;
    for &corner in corners {
        let Some(vertex) = mesh.to_vertex(corner) else {
            continue;
        };
        let Some(position) = mesh.vertex_position(vertex) else {
            continue;
        };
        sum[0] += position[0];
        sum[1] += position[1];
        sum[2] += position[2];
        count += 1;
    }
    if count == 0 {
        return [0.0; 3];
    }
    let inv = 1.0 / count as f32;
    [sum[0] * inv, sum[1] * inv, sum[2] * inv]
}

fn inset_corner_sample(position: [f32; 3], face_center: [f32; 3]) -> [f32; 3] {
    const FACE_INSET: f32 = 0.125;
    [
        position[0] + (face_center[0] - position[0]) * FACE_INSET,
        position[1] + (face_center[1] - position[1]) * FACE_INSET,
        position[2] + (face_center[2] - position[2]) * FACE_INSET,
    ]
}

fn normalize_gradient(sample: [f32; 4]) -> Option<[f32; 3]> {
    let gradient = [sample[1], sample[2], sample[3]];
    let length_squared =
        gradient[0] * gradient[0] + gradient[1] * gradient[1] + gradient[2] * gradient[2];
    if !length_squared.is_finite() || length_squared <= 0.0 {
        return None;
    }
    let inv_length = sqrt(length_squared).recip();
    Some([
        gradient[0] * inv_length,
        gradient[1] * inv_length,
        gradient[2] * inv_length,
    ])
}

#[cfg(test)]
mod tests {
    use super::{
        DualContourParams, IntervalVisitor, QuadDiagonal, collect_active_cells, dual_contour,
        dual_contour_with_regions, sample_lattice, select_quad_diagonal, solve_active_cell,
        squared_distance,
    };
    use crate::EdgeSearchParams;
    use crate::analytic::{BoxField, CylinderField, SphereField, TaggedField, Union};
    use crate::{ProvenanceField, ScalarField};
    use exedra::{ExtractParams, attr};
    use exedra_qef::QefParams;
    use exedra_spatial::{Aabb, Octree};

    fn params(bounds: Aabb, max_depth: u8) -> DualContourParams {
        DualContourParams {
            root_bounds: bounds,
            max_depth,
            cell_budget: None,
            edge_search: EdgeSearchParams {
                bisection_steps: 10,
            },
            qef: QefParams::default(),
        }
    }

    struct AxisTaggedUnion {
        field: Union<BoxField, BoxField>,
    }

    impl ScalarField for AxisTaggedUnion {
        fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]> {
            self.field.eval_interval(bounds)
        }

        fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
            self.field.eval_points(points, out);
        }

        fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
            self.field.eval_gradients(points, out);
        }
    }

    impl ProvenanceField for AxisTaggedUnion {
        type Provenance = u32;

        fn eval_interval_with_provenance(
            &self,
            bounds: &Aabb,
        ) -> Option<([f32; 2], Self::Provenance)> {
            self.field
                .eval_interval(bounds)
                .map(|interval| (interval, self.point_provenance(bounds.center())))
        }

        fn point_provenance(&self, point: [f32; 3]) -> Self::Provenance {
            if point[0] < 0.0 { 1 } else { 2 }
        }
    }

    #[test]
    fn dual_contour_sphere_builds_valid_deterministic_mesh() {
        let field = SphereField {
            center: [0.0, 0.0, 0.0],
            radius: 1.0,
        };
        let bounds = Aabb::new([-1.5, -1.5, -1.5], [1.5, 1.5, 1.5]).expect("bounds");

        let first =
            dual_contour(&field, &params(bounds, 4)).expect("sphere extraction should work");
        let second =
            dual_contour(&field, &params(bounds, 4)).expect("sphere extraction should be stable");

        assert!(first.mesh.validate_deep().is_empty());
        assert_eq!(first.stats, second.stats);
        let (tri_a, stats_a) = first.mesh.to_trimesh(&ExtractParams::default());
        let (tri_b, stats_b) = second.mesh.to_trimesh(&ExtractParams::default());
        assert_eq!(stats_a, stats_b);
        assert_eq!(tri_a.indices, tri_b.indices);
        assert_eq!(tri_a.positions, tri_b.positions);

        let normal_layer = first
            .mesh
            .attrs()
            .sparse(attr::CORNER_NORMAL_OVERRIDE)
            .expect("corner normal layer should exist");
        assert!(
            first
                .mesh
                .faces()
                .flat_map(|face| first.mesh.face_loop(face))
                .all(|corner| {
                    let Some(normal) = normal_layer.get(corner.as_id()).copied() else {
                        return false;
                    };
                    let Some(vertex) = first.mesh.to_vertex(corner) else {
                        return false;
                    };
                    let Some(position) = first.mesh.vertex_position(vertex) else {
                        return false;
                    };
                    dot3(normal, *position) > 0.5 && (length3(normal) - 1.0).abs() < 1.0e-3
                })
        );
    }

    #[test]
    fn dual_contour_box_sets_some_sharp_edges() {
        let field = BoxField {
            center: [0.0, 0.0, 0.0],
            half_extents: [0.8, 0.8, 0.8],
        };
        let bounds = Aabb::new([-1.2, -1.2, -1.2], [1.2, 1.2, 1.2]).expect("bounds");
        let result = dual_contour(&field, &params(bounds, 4)).expect("box extraction should work");

        assert!(result.mesh.validate_deep().is_empty());
        let sharp_layer = result
            .mesh
            .attrs()
            .sparse(attr::EDGE_SHARPNESS)
            .expect("sharpness layer should exist");
        assert!(result.mesh.faces().any(|face| {
            result.mesh.face_loop(face).any(|half_edge| {
                sharp_layer
                    .get(half_edge.as_id())
                    .is_some_and(|value| *value >= 1.0)
            })
        }));
    }

    #[test]
    fn dual_contour_cylinder_preserves_rim_sharpness() {
        let field = CylinderField {
            center: [0.0, 0.0, 0.0],
            axis: [0.0, 0.0, 1.0],
            radius: 0.7,
            half_height: 0.8,
        };
        let bounds = Aabb::new([-1.2, -1.2, -1.2], [1.2, 1.2, 1.2]).expect("bounds");
        let result =
            dual_contour(&field, &params(bounds, 4)).expect("cylinder extraction should work");

        assert!(result.mesh.validate_deep().is_empty());
        let sharp_layer = result
            .mesh
            .attrs()
            .sparse(attr::EDGE_SHARPNESS)
            .expect("sharpness layer should exist");
        assert!(result.mesh.faces().any(|face| {
            result.mesh.face_loop(face).any(|half_edge| {
                sharp_layer
                    .get(half_edge.as_id())
                    .is_some_and(|value| *value >= 1.0)
            })
        }));
    }

    #[test]
    fn dual_contour_with_regions_tags_faces_from_provenance() {
        let field = TaggedField {
            field: SphereField {
                center: [0.0, 0.0, 0.0],
                radius: 0.9,
            },
            provenance: 3_u32,
        };
        let bounds = Aabb::new([-1.2, -1.2, -1.2], [1.2, 1.2, 1.2]).expect("bounds");
        let result = dual_contour_with_regions(&field, &params(bounds, 4))
            .expect("provenance extraction should work");

        assert!(result.mesh.validate_deep().is_empty());
        let regions = result
            .mesh
            .attrs()
            .dense(attr::FACE_REGION)
            .expect("FACE_REGION layer should exist");
        assert!(
            result
                .mesh
                .faces()
                .all(|face| regions.get(face.as_id()) == Some(&3))
        );
    }

    #[test]
    fn dual_contour_with_regions_marks_seams_between_tagged_operands() {
        let field = AxisTaggedUnion {
            field: Union::new(
                BoxField {
                    center: [-0.35, 0.0, 0.0],
                    half_extents: [0.55, 0.55, 0.55],
                },
                BoxField {
                    center: [0.35, 0.0, 0.0],
                    half_extents: [0.55, 0.55, 0.55],
                },
            ),
        };
        let bounds = Aabb::new([-1.4, -1.1, -1.1], [1.4, 1.1, 1.1]).expect("bounds");
        let result = dual_contour_with_regions(&field, &params(bounds, 4))
            .expect("tagged union extraction should work");

        assert!(result.mesh.validate_deep().is_empty());
        let regions = result
            .mesh
            .attrs()
            .dense(attr::FACE_REGION)
            .expect("FACE_REGION layer should exist");
        assert!(
            result
                .mesh
                .faces()
                .any(|face| regions.get(face.as_id()) == Some(&1))
        );
        assert!(
            result
                .mesh
                .faces()
                .any(|face| regions.get(face.as_id()) == Some(&2))
        );

        let seams = result
            .mesh
            .attrs()
            .sparse(attr::EDGE_SEAM)
            .expect("EDGE_SEAM layer should exist");
        assert!(result.mesh.faces().any(|face| {
            result
                .mesh
                .face_loop(face)
                .any(|corner| seams.get(corner.as_id()).copied().unwrap_or(false))
        }));
    }

    #[test]
    fn dual_contour_respects_active_cell_budget() {
        let field = SphereField {
            center: [0.0, 0.0, 0.0],
            radius: 1.0,
        };
        let bounds = Aabb::new([-1.5, -1.5, -1.5], [1.5, 1.5, 1.5]).expect("bounds");
        let mut params = params(bounds, 4);
        params.cell_budget = Some(32);

        let result = dual_contour(&field, &params).expect("budgeted extraction should work");

        assert!(result.stats.active_cells <= 32);
        assert!(result.mesh.validate_deep().is_empty());
    }

    #[test]
    fn dual_contour_csg_union_builds_valid_sharp_mesh() {
        let field = Union::new(
            BoxField {
                center: [-0.35, 0.0, 0.0],
                half_extents: [0.55, 0.55, 0.55],
            },
            BoxField {
                center: [0.35, 0.0, 0.0],
                half_extents: [0.55, 0.55, 0.55],
            },
        );
        let bounds = Aabb::new([-1.4, -1.1, -1.1], [1.4, 1.1, 1.1]).expect("bounds");
        let result =
            dual_contour(&field, &params(bounds, 4)).expect("union extraction should work");

        assert!(result.mesh.validate_deep().is_empty());
        let sharp_layer = result
            .mesh
            .attrs()
            .sparse(attr::EDGE_SHARPNESS)
            .expect("sharpness layer should exist");
        assert!(result.mesh.faces().any(|face| {
            result.mesh.face_loop(face).any(|half_edge| {
                sharp_layer
                    .get(half_edge.as_id())
                    .is_some_and(|value| *value >= 1.0)
            })
        }));
        assert!(
            result
                .mesh
                .faces()
                .all(|face| result.mesh.face_loop(face).count() == 3)
        );
    }

    #[test]
    fn sphere_active_cells_do_not_all_collapse_to_cell_centers() {
        let field = SphereField {
            center: [0.0, 0.0, 0.0],
            radius: 1.0,
        };
        let params = params(
            Aabb::new([-1.5, -1.5, -1.5], [1.5, 1.5, 1.5]).expect("bounds"),
            4,
        );
        let resolution = 1_u32 << params.max_depth;
        let lattice = sample_lattice(&field, params.root_bounds, resolution);
        let mut visitor = IntervalVisitor {
            field: &field,
            lattice: &lattice,
            root_bounds: params.root_bounds,
            resolution,
        };
        let tree = Octree::build(params.root_bounds, params.max_depth, &mut visitor);

        let mut found_non_center = false;
        for leaf_id in tree.leaf_ids() {
            let Some(cell) = tree.cell(leaf_id) else {
                continue;
            };
            if cell.depth != params.max_depth {
                continue;
            }
            if !cell
                .payload()
                .copied()
                .is_some_and(|payload| payload.intersects_surface)
            {
                continue;
            }

            let Some(active) =
                solve_active_cell(&field, &params, &lattice, cell.bounds, cell.depth)
                    .expect("cell solve should work")
            else {
                continue;
            };
            if squared_distance(active.position, cell.bounds.center()) > 1.0e-8 {
                found_non_center = true;
                break;
            }
        }

        assert!(found_non_center);
    }

    #[test]
    fn multiscale_active_cells_span_multiple_depths() {
        let field = Union::new(
            SphereField {
                center: [0.0, 0.0, 0.0],
                radius: 1.0,
            },
            SphereField {
                center: [1.2, 0.0, 0.0],
                radius: 0.18,
            },
        );
        let params = params(
            Aabb::new([-1.6, -1.5, -1.5], [1.6, 1.5, 1.5]).expect("bounds"),
            6,
        );
        let resolution = 1_u32 << params.max_depth;
        let lattice = sample_lattice(&field, params.root_bounds, resolution);
        let mut visitor = IntervalVisitor {
            field: &field,
            lattice: &lattice,
            root_bounds: params.root_bounds,
            resolution,
        };
        let tree = Octree::build(params.root_bounds, params.max_depth, &mut visitor);
        let active =
            collect_active_cells(&field, &params, &tree, &lattice).expect("active cells collect");

        assert!(active.iter().any(|cell| cell.depth < params.max_depth));
        assert!(active.iter().any(|cell| cell.depth == params.max_depth));

        let result = dual_contour(&field, &params).expect("multiscale extraction should work");
        assert!(result.mesh.validate_deep().is_empty());
    }

    #[test]
    fn select_quad_diagonal_prefers_shorter_diagonal() {
        let skewed = [
            [0.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [3.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ];

        assert_eq!(select_quad_diagonal(skewed), QuadDiagonal::OneThree);
    }

    fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }

    fn length3(vector: [f32; 3]) -> f32 {
        (dot3(vector, vector)).sqrt()
    }
}
