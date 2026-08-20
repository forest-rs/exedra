// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Independent finest-grid dual-contour witness for the safe H1 subset.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use exedra::{FaceBuildAttrs, Mesh, MeshBuilder, attr, op};
use exedra_isosurface::{
    DualContourParams, DualContourStats, HermiteIntersection, ScalarField,
    SemiAnalyticContourStats, SemiAnalyticFeature, SemiAnalyticField,
    SemiAnalyticProjectionOutcome, locate_edge_intersection,
};
use exedra_qef::{PlaneConstraint, QefBounds, QefSolver, SharpnessClass};
use exedra_spatial::Aabb;

use crate::fixture::{H1Field, H1Fixture};

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

#[derive(Clone, Debug)]
pub(crate) struct UniformResult {
    pub(crate) mesh: Mesh,
    pub(crate) stats: DualContourStats,
    pub(crate) semi_analytic: SemiAnalyticContourStats,
    pub(crate) final_leaf_depths: Vec<usize>,
    pub(crate) contributing_depths: Vec<usize>,
    pub(crate) work: UniformWork,
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct UniformWork {
    pub(crate) interval_calls: usize,
    pub(crate) lattice_samples: usize,
    pub(crate) lattice_bytes: usize,
    pub(crate) crossing_cells: usize,
    pub(crate) hermite_searches: usize,
    pub(crate) hermite_hits: usize,
    pub(crate) qef_solves: usize,
}

#[derive(Copy, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct CellKey {
    origin: [u32; 3],
    span: u32,
    depth: u8,
}

#[derive(Copy, Clone, Debug)]
struct ActiveCell {
    position: [f32; 3],
    sharpness: SharpnessClass,
    vertex: u32,
}

#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
struct FacePatchKey {
    axis: u8,
    plane: u32,
    u: u32,
    v: u32,
    span: u32,
}

struct Grid {
    bounds: Aabb,
    resolution: u32,
    step: [f32; 3],
    values: Vec<f32>,
}

pub(crate) fn extract(fixture: &H1Fixture) -> UniformResult {
    let mut work = UniformWork::default();
    let mut leaves = BTreeSet::new();
    let mut stored_cells = 0_usize;
    visit_forced_uniform(
        &fixture.field,
        &fixture.params,
        root_key(fixture.params.max_depth),
        &mut leaves,
        &mut stored_cells,
        &mut work,
    );
    balance_leaves(
        &fixture.field,
        &fixture.params,
        &mut leaves,
        &mut stored_cells,
        &mut work,
    );
    let final_leaf_depths = depth_histogram(leaves.iter().copied(), fixture.params.max_depth);

    let grid = sample_grid(&fixture.field, &fixture.params, &mut work);
    let mut active = solve_active_cells(&fixture.field, &fixture.params, &grid, &mut work);
    let mut semi_analytic = SemiAnalyticContourStats::default();
    project_active_cells(
        &fixture.field,
        &fixture.params,
        &mut active,
        &mut semi_analytic,
    );

    let mut builder = MeshBuilder::new();
    for cell in active.values_mut() {
        cell.vertex = builder.push_vertex(cell.position);
    }
    emit_faces(
        &fixture.field,
        &fixture.params,
        &grid,
        &active,
        &mut builder,
    );
    let mut mesh = builder
        .build()
        .expect("independent uniform mesh build")
        .mesh;
    populate_corner_normals(&fixture.field, &mut mesh);
    populate_region_boundary_seams(&mut mesh);
    let contributing_depths = {
        let mut depths = vec![0; usize::from(fixture.params.max_depth) + 1];
        depths[usize::from(fixture.params.max_depth)] = active.len();
        depths
    };
    UniformResult {
        stats: DualContourStats {
            octree_cells: stored_cells,
            active_cells: active.len(),
            vertices: mesh.vertices().count(),
            faces: mesh.faces().count(),
        },
        mesh,
        semi_analytic,
        final_leaf_depths,
        contributing_depths,
        work,
    }
}

fn sample_grid(field: &H1Field, params: &DualContourParams, work: &mut UniformWork) -> Grid {
    let resolution = 1_u32 << params.max_depth;
    let width = usize::try_from(resolution + 1).expect("grid width fits usize");
    let sample_count = width.pow(3);
    let extent = params.root_bounds.extent();
    let step = extent.map(|value| value / resolution as f32);
    let mut values = vec![0.0; sample_count];
    let mut points = Vec::with_capacity(4096);
    let mut indices = Vec::with_capacity(4096);
    for x in 0..=resolution {
        for y in 0..=resolution {
            for z in 0..=resolution {
                points.push(point(params.root_bounds, step, resolution, [x, y, z]));
                indices.push(grid_index(width, x, y, z));
                if points.len() == points.capacity() {
                    sample_batch(field, &points, &indices, &mut values);
                    points.clear();
                    indices.clear();
                }
            }
        }
    }
    if !points.is_empty() {
        sample_batch(field, &points, &indices, &mut values);
    }
    work.lattice_samples = sample_count;
    work.lattice_bytes = values.len() * size_of::<f32>();
    Grid {
        bounds: params.root_bounds,
        resolution,
        step,
        values,
    }
}

fn sample_batch(field: &H1Field, points: &[[f32; 3]], indices: &[usize], values: &mut [f32]) {
    let mut output = vec![0.0; points.len()];
    field.eval_points(points, &mut output);
    for (&index, value) in indices.iter().zip(output) {
        values[index] = value;
    }
}

fn solve_active_cells(
    field: &H1Field,
    params: &DualContourParams,
    grid: &Grid,
    work: &mut UniformWork,
) -> BTreeMap<[u32; 3], ActiveCell> {
    let mut active = BTreeMap::new();
    for x in 0..grid.resolution {
        for y in 0..grid.resolution {
            for z in 0..grid.resolution {
                let origin = [x, y, z];
                let values: [f32; 8] = core::array::from_fn(|corner| {
                    grid.value([x + bit(corner, 0), y + bit(corner, 1), z + bit(corner, 2)])
                });
                if CUBE_EDGES
                    .iter()
                    .all(|&(start, end)| !edge_has_crossing(values[start], values[end]))
                {
                    continue;
                }
                work.crossing_cells += 1;
                let bounds = cell_bounds(grid, origin);
                let corners: [[f32; 3]; 8] = core::array::from_fn(|corner| {
                    grid.point([x + bit(corner, 0), y + bit(corner, 1), z + bit(corner, 2)])
                });
                let mut intersections = Vec::new();
                for (edge, &(start, end)) in CUBE_EDGES.iter().enumerate() {
                    if !edge_has_crossing(values[start], values[end]) {
                        continue;
                    }
                    work.hermite_searches += 1;
                    let hit = locate_edge_intersection(
                        field,
                        corners[start],
                        corners[end],
                        &params.edge_search,
                    )
                    .unwrap_or_else(|error| panic!("uniform edge {edge} failed: {error:?}"));
                    intersections.push(hit);
                    work.hermite_hits += 1;
                }
                let mut solver = QefSolver::new();
                for hit in &intersections {
                    assert!(
                        solver.add_constraint(PlaneConstraint {
                            position: hit.position,
                            normal: hit.normal,
                        }),
                        "uniform Hermite plane must be finite"
                    );
                }
                let anchor = mass_point(&intersections);
                let solved = solver
                    .solve_with_anchor(
                        QefBounds::new(bounds.min, bounds.max).expect("cell bounds"),
                        anchor,
                        &params.qef,
                    )
                    .expect("uniform QEF solve");
                work.qef_solves += 1;
                active.insert(
                    origin,
                    ActiveCell {
                        position: solved.position,
                        sharpness: solved.sharpness_class,
                        vertex: u32::MAX,
                    },
                );
            }
        }
    }
    active
}

fn project_active_cells(
    field: &H1Field,
    params: &DualContourParams,
    active: &mut BTreeMap<[u32; 3], ActiveCell>,
    stats: &mut SemiAnalyticContourStats,
) {
    let resolution = 1_u32 << params.max_depth;
    let step = params
        .root_bounds
        .extent()
        .map(|extent| extent / resolution as f32);
    for (&origin, cell) in active.iter_mut() {
        let bounds = Aabb::new(
            point(params.root_bounds, step, resolution, origin),
            point(
                params.root_bounds,
                step,
                resolution,
                origin.map(|value| value + 1),
            ),
        )
        .expect("uniform active bounds");
        match field.project_cell_vertex_detailed(cell.position, &bounds) {
            SemiAnalyticProjectionOutcome::Projected(projection) => {
                let inside = (0..3).all(|axis| {
                    projection.position[axis] >= bounds.min[axis]
                        && projection.position[axis] <= bounds.max[axis]
                });
                let displacement = squared_distance(cell.position, projection.position);
                let budget = squared_distance(bounds.min, bounds.max);
                if !projection.position.iter().all(|value| value.is_finite()) {
                    stats.invalid_fallbacks += 1;
                } else if !inside || displacement > budget {
                    stats.over_budget_fallbacks += 1;
                } else {
                    cell.position = projection.position;
                    match projection.feature {
                        SemiAnalyticFeature::Surface => stats.surface_projections += 1,
                        SemiAnalyticFeature::Edge => {
                            stats.surface_projections += 1;
                            cell.sharpness = SharpnessClass::Edge;
                        }
                        SemiAnalyticFeature::Corner => {
                            stats.surface_projections += 1;
                            cell.sharpness = SharpnessClass::Corner;
                        }
                        SemiAnalyticFeature::IntersectionCurve => {
                            stats.feature_snaps += 1;
                            cell.sharpness = SharpnessClass::Edge;
                        }
                    }
                }
            }
            SemiAnalyticProjectionOutcome::Unsupported => stats.unsupported_fallbacks += 1,
            SemiAnalyticProjectionOutcome::Ambiguous => stats.ambiguous_fallbacks += 1,
            SemiAnalyticProjectionOutcome::Tangent => stats.tangent_fallbacks += 1,
            SemiAnalyticProjectionOutcome::Coincident => stats.coincident_fallbacks += 1,
            SemiAnalyticProjectionOutcome::OverBudget => stats.over_budget_fallbacks += 1,
            SemiAnalyticProjectionOutcome::Invalid => stats.invalid_fallbacks += 1,
        }
    }
}

fn emit_faces(
    field: &H1Field,
    params: &DualContourParams,
    grid: &Grid,
    active: &BTreeMap<[u32; 3], ActiveCell>,
    builder: &mut MeshBuilder,
) {
    let resolution = grid.resolution;
    let mut face_count = 0_usize;
    for axis in 0_u8..3 {
        for z in 0..=resolution {
            for y in 0..=resolution {
                for x in 0..=resolution {
                    let start = [x, y, z];
                    if start[usize::from(axis)] >= resolution
                        || orthogonal_boundary(start, axis, resolution)
                    {
                        continue;
                    }
                    let mut end = start;
                    end[usize::from(axis)] += 1;
                    let start_value = grid.value(start);
                    let end_value = grid.value(end);
                    if !edge_has_crossing(start_value, end_value) {
                        continue;
                    }
                    let origins = incident_origins(start, axis);
                    let mut entries = origins.map(|origin| {
                        active
                            .get(&origin)
                            .copied()
                            .unwrap_or_else(|| panic!("crossing segment missing cell {origin:?}"))
                    });
                    if start_value > 0.0 {
                        entries.reverse();
                    }
                    let hit = locate_edge_intersection(
                        field,
                        grid.point(start),
                        grid.point(end),
                        &params.edge_search,
                    )
                    .expect("region zero crossing");
                    let region = field.primitive_at(hit.position);
                    emit_quad(builder, entries, region, face_count);
                    face_count += 2;
                }
            }
        }
    }
}

fn emit_quad(builder: &mut MeshBuilder, entries: [ActiveCell; 4], region: u32, face: usize) {
    let points = entries.map(|entry| entry.position);
    let sharpness = [
        sharpness(entries[0].sharpness).max(sharpness(entries[1].sharpness)),
        sharpness(entries[1].sharpness).max(sharpness(entries[2].sharpness)),
        sharpness(entries[2].sharpness).max(sharpness(entries[3].sharpness)),
        sharpness(entries[3].sharpness).max(sharpness(entries[0].sharpness)),
    ];
    let zero_two_valid = triangle_is_nondegenerate([points[0], points[1], points[2]])
        && triangle_is_nondegenerate([points[0], points[2], points[3]]);
    let one_three_valid = triangle_is_nondegenerate([points[0], points[1], points[3]])
        && triangle_is_nondegenerate([points[1], points[2], points[3]]);
    let zero_two = squared_distance(points[0], points[2]);
    let one_three = squared_distance(points[1], points[3]);
    let coordinate_scale = points
        .iter()
        .flatten()
        .copied()
        .map(f32::abs)
        .fold(1.0_f32, f32::max);
    let coordinate_ulp = f32::EPSILON * coordinate_scale;
    let diagonal_scale = zero_two.max(one_three).sqrt();
    let tie_budget = 16.0 * coordinate_ulp * (diagonal_scale + coordinate_ulp);
    let use_one_three = match (zero_two_valid, one_three_valid) {
        (true, false) => false,
        (false, true) => true,
        (true, true) => one_three + tie_budget < zero_two,
        (false, false) => panic!("uniform quad {face} has no valid split"),
    };
    let vertices = entries.map(|entry| entry.vertex);
    let (triangles, edge_values) = if use_one_three {
        (
            [
                [vertices[0], vertices[1], vertices[3]],
                [vertices[1], vertices[2], vertices[3]],
            ],
            [
                [sharpness[0], 0.0, sharpness[3]],
                [sharpness[1], sharpness[2], 0.0],
            ],
        )
    } else {
        (
            [
                [vertices[0], vertices[1], vertices[2]],
                [vertices[0], vertices[2], vertices[3]],
            ],
            [
                [sharpness[0], sharpness[1], 0.0],
                [0.0, sharpness[2], sharpness[3]],
            ],
        )
    };
    for (triangle, edge_sharpness) in triangles.into_iter().zip(edge_values) {
        builder
            .add_face_with_attrs(
                &triangle,
                &FaceBuildAttrs {
                    region: Some(region),
                    edge_sharpness: Some(&edge_sharpness),
                    ..FaceBuildAttrs::default()
                },
            )
            .expect("uniform triangle build");
    }
}

fn visit_forced_uniform(
    field: &H1Field,
    params: &DualContourParams,
    key: CellKey,
    leaves: &mut BTreeSet<CellKey>,
    stored_cells: &mut usize,
    work: &mut UniformWork,
) {
    *stored_cells += 1;
    work.interval_calls += 1;
    let bounds = key_bounds(params, key);
    let intersects = field
        .eval_interval(&bounds)
        .is_none_or(|interval| interval[0] <= 0.0 && interval[1] >= 0.0);
    if intersects && key.depth < params.max_depth {
        for child in children(key) {
            visit_forced_uniform(field, params, child, leaves, stored_cells, work);
        }
    } else {
        leaves.insert(key);
    }
}

fn balance_leaves(
    field: &H1Field,
    params: &DualContourParams,
    leaves: &mut BTreeSet<CellKey>,
    stored_cells: &mut usize,
    work: &mut UniformWork,
) {
    loop {
        let mut faces = HashMap::<FacePatchKey, [Option<CellKey>; 2]>::new();
        for &key in leaves.iter() {
            for axis in 0_u8..3 {
                for side in 0_u8..2 {
                    let (patch, owner) = face_patch(key, axis, side);
                    faces.entry(patch).or_insert([None; 2])[owner] = Some(key);
                }
            }
        }
        let mut refine = BTreeSet::new();
        for &key in leaves.iter() {
            for axis in 0_u8..3 {
                for side in 0_u8..2 {
                    let (patch, owner) = face_patch(key, axis, side);
                    let mut span = patch.span;
                    loop {
                        let query = FacePatchKey {
                            u: align_down(patch.u, span),
                            v: align_down(patch.v, span),
                            span,
                            ..patch
                        };
                        if let Some(neighbor) = faces.get(&query).and_then(|pair| pair[1 - owner]) {
                            if key.depth > neighbor.depth + 1 {
                                refine.insert(neighbor);
                            }
                            break;
                        }
                        if span >= 1_u32 << params.max_depth {
                            break;
                        }
                        span *= 2;
                    }
                }
            }
        }
        if refine.is_empty() {
            return;
        }
        for key in refine {
            assert!(leaves.remove(&key), "balance candidate is a leaf");
            for child in children(key) {
                visit_forced_uniform(field, params, child, leaves, stored_cells, work);
            }
        }
    }
}

fn face_patch(key: CellKey, axis: u8, side: u8) -> (FacePatchKey, usize) {
    let (plane, u, v) = match axis {
        0 => (
            key.origin[0] + u32::from(side) * key.span,
            key.origin[1],
            key.origin[2],
        ),
        1 => (
            key.origin[1] + u32::from(side) * key.span,
            key.origin[0],
            key.origin[2],
        ),
        _ => (
            key.origin[2] + u32::from(side) * key.span,
            key.origin[0],
            key.origin[1],
        ),
    };
    (
        FacePatchKey {
            axis,
            plane,
            u,
            v,
            span: key.span,
        },
        usize::from(1 - side),
    )
}

fn children(key: CellKey) -> [CellKey; 8] {
    let span = key.span / 2;
    core::array::from_fn(|corner| CellKey {
        origin: [
            key.origin[0] + bit(corner, 0) * span,
            key.origin[1] + bit(corner, 1) * span,
            key.origin[2] + bit(corner, 2) * span,
        ],
        span,
        depth: key.depth + 1,
    })
}

fn root_key(max_depth: u8) -> CellKey {
    CellKey {
        origin: [0; 3],
        span: 1_u32 << max_depth,
        depth: 0,
    }
}

fn key_bounds(params: &DualContourParams, key: CellKey) -> Aabb {
    let resolution = 1_u32 << params.max_depth;
    let step = params
        .root_bounds
        .extent()
        .map(|extent| extent / resolution as f32);
    Aabb::new(
        point(params.root_bounds, step, resolution, key.origin),
        point(
            params.root_bounds,
            step,
            resolution,
            key.origin.map(|value| value + key.span),
        ),
    )
    .expect("key bounds")
}

fn depth_histogram(keys: impl Iterator<Item = CellKey>, max_depth: u8) -> Vec<usize> {
    let mut depths = vec![0; usize::from(max_depth) + 1];
    for key in keys {
        depths[usize::from(key.depth)] += 1;
    }
    depths
}

impl Grid {
    fn point(&self, key: [u32; 3]) -> [f32; 3] {
        point(self.bounds, self.step, self.resolution, key)
    }

    fn value(&self, key: [u32; 3]) -> f32 {
        let width = usize::try_from(self.resolution + 1).expect("grid width fits usize");
        self.values[grid_index(width, key[0], key[1], key[2])]
    }
}

fn cell_bounds(grid: &Grid, origin: [u32; 3]) -> Aabb {
    Aabb::new(
        grid.point(origin),
        grid.point(origin.map(|value| value + 1)),
    )
    .expect("grid cell bounds")
}

fn point(bounds: Aabb, step: [f32; 3], resolution: u32, key: [u32; 3]) -> [f32; 3] {
    core::array::from_fn(|axis| {
        if key[axis] == 0 {
            bounds.min[axis]
        } else if key[axis] == resolution {
            bounds.max[axis]
        } else {
            bounds.min[axis] + step[axis] * key[axis] as f32
        }
    })
}

fn grid_index(width: usize, x: u32, y: u32, z: u32) -> usize {
    (usize::try_from(x).expect("x fits") * width + usize::try_from(y).expect("y fits")) * width
        + usize::try_from(z).expect("z fits")
}

fn bit(corner: usize, axis: u32) -> u32 {
    u32::from((corner & (1_usize << axis)) != 0)
}

fn edge_has_crossing(start: f32, end: f32) -> bool {
    (start <= 0.0 && end > 0.0) || (start > 0.0 && end <= 0.0)
}

fn orthogonal_boundary(start: [u32; 3], axis: u8, resolution: u32) -> bool {
    (0..3).any(|other| {
        other != usize::from(axis) && (start[other] == 0 || start[other] == resolution)
    })
}

fn incident_origins(start: [u32; 3], axis: u8) -> [[u32; 3]; 4] {
    let [x, y, z] = start;
    match axis {
        0 => [[x, y - 1, z - 1], [x, y, z - 1], [x, y, z], [x, y - 1, z]],
        1 => [[x - 1, y, z - 1], [x - 1, y, z], [x, y, z], [x, y, z - 1]],
        _ => [[x - 1, y - 1, z], [x, y - 1, z], [x, y, z], [x - 1, y, z]],
    }
}

fn mass_point(intersections: &[HermiteIntersection]) -> [f32; 3] {
    let sum = intersections.iter().fold([0.0; 3], |mut sum, hit| {
        for (axis, component) in sum.iter_mut().enumerate() {
            *component += hit.position[axis];
        }
        sum
    });
    let inverse = 1.0 / intersections.len() as f32;
    sum.map(|value| value * inverse)
}

fn sharpness(value: SharpnessClass) -> f32 {
    match value {
        SharpnessClass::Smooth => 0.0,
        SharpnessClass::Edge => 1.0,
        SharpnessClass::Corner => 2.0,
    }
}

fn triangle_is_nondegenerate(points: [[f32; 3]; 3]) -> bool {
    let points = points.map(|point| point.map(f64::from));
    let ab = sub3(points[1], points[0]);
    let ac = sub3(points[2], points[0]);
    let bc = sub3(points[2], points[1]);
    let cross = [
        ab[1] * ac[2] - ab[2] * ac[1],
        ab[2] * ac[0] - ab[0] * ac[2],
        ab[0] * ac[1] - ab[1] * ac[0],
    ];
    let area_squared = dot3(cross, cross);
    let longest = dot3(ab, ab).max(dot3(ac, ac)).max(dot3(bc, bc));
    let relative = 16.0 * f64::from(f32::EPSILON);
    area_squared.is_finite()
        && longest.is_finite()
        && longest > 0.0
        && area_squared > relative * relative * longest * longest
}

fn populate_corner_normals(field: &H1Field, mesh: &mut Mesh) {
    let mut corners = Vec::new();
    let mut points = Vec::new();
    for face in mesh.faces() {
        let loop_corners = mesh.face_loop(face).collect::<Vec<_>>();
        let center = face_centroid(mesh, &loop_corners);
        for corner in loop_corners {
            let vertex = mesh.to_vertex(corner).expect("face corner vertex");
            let position = *mesh.vertex_position(vertex).expect("vertex position");
            corners.push(corner);
            points.push([
                position[0] + (center[0] - position[0]) * 0.125,
                position[1] + (center[1] - position[1]) * 0.125,
                position[2] + (center[2] - position[2]) * 0.125,
            ]);
        }
    }
    let mut gradients = vec![[0.0; 4]; points.len()];
    field.eval_gradients(&points, &mut gradients);
    let mut session = mesh.edit();
    for (corner, gradient) in corners.into_iter().zip(gradients) {
        let vector = [gradient[1], gradient[2], gradient[3]];
        let length_squared = dot3_f32(vector, vector);
        if length_squared.is_finite() && length_squared > 0.0 {
            let inverse = length_squared.sqrt().recip();
            op::set_corner_normal_override(
                &mut session,
                corner,
                Some(vector.map(|value| value * inverse)),
            )
            .expect("corner normal");
        }
    }
    let _: () = session.finish();
}

fn populate_region_boundary_seams(mesh: &mut Mesh) {
    let regions = mesh
        .attrs()
        .dense(attr::FACE_REGION)
        .expect("uniform regions");
    let mut edges = Vec::new();
    for face in mesh.faces() {
        let region = regions.get(face.as_id()).copied().expect("face region");
        for corner in mesh.face_loop(face) {
            let Some(twin) = mesh.twin(corner) else {
                continue;
            };
            if twin < corner {
                continue;
            }
            let Some(other) = mesh.face(twin) else {
                continue;
            };
            if other != exedra::FaceId::OUTSIDE
                && regions
                    .get(other.as_id())
                    .copied()
                    .is_some_and(|value| value != region)
            {
                edges.push(corner);
            }
        }
    }
    let mut session = mesh.edit();
    for edge in edges {
        op::set_edge_seam(&mut session, edge, true).expect("uniform seam");
    }
    let _: () = session.finish();
}

fn face_centroid(mesh: &Mesh, corners: &[exedra::CornerId]) -> [f32; 3] {
    let mut sum = [0.0; 3];
    for &corner in corners {
        let vertex = mesh.to_vertex(corner).expect("corner vertex");
        let point = mesh.vertex_position(vertex).expect("vertex position");
        for axis in 0..3 {
            sum[axis] += point[axis];
        }
    }
    let inverse = 1.0 / corners.len() as f32;
    sum.map(|value| value * inverse)
}

fn squared_distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    dot3_f32(
        [a[0] - b[0], a[1] - b[1], a[2] - b[2]],
        [a[0] - b[0], a[1] - b[1], a[2] - b[2]],
    )
}

fn sub3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn dot3_f32(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn align_down(value: u32, span: u32) -> u32 {
    value & !(span - 1)
}

#[cfg(test)]
mod tests {
    use crate::fixture::h1;
    use crate::report::{extraction_signature, region_histogram, topology};

    use super::extract;

    #[test]
    fn depth_five_uniform_witness_is_deterministic_and_closed() {
        let fixture = h1(5);
        let first = extract(&fixture);
        let second = extract(&fixture);

        assert_eq!(
            extraction_signature(&first.mesh),
            extraction_signature(&second.mesh)
        );
        assert_eq!(first.stats, second.stats);
        assert_eq!(first.semi_analytic, second.semi_analytic);
        assert_eq!(first.final_leaf_depths, second.final_leaf_depths);
        assert_eq!(
            region_histogram(&first.mesh),
            region_histogram(&second.mesh)
        );
        let topology = topology(&first.mesh);
        assert!(topology.is_closed_clean(), "{topology:?}");
    }
}
