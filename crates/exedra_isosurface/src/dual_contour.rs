// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! First dual-contouring extraction path for implicit scalar fields.

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

use exedra::{BuildError, FaceBuildAttrs, FaceLoopErrorKind, Mesh, MeshBuilder, attr, op};
use exedra_qef::{
    PlaneConstraint, QefBounds, QefParams, QefResult, QefSolveError, QefSolver, SharpnessClass,
};
use exedra_spatial::{Aabb, CellId, CellRef, Octree, OctreeVisitor};
use hashbrown::HashMap;

use crate::adaptive_transition::{
    AdaptiveGrid, BalanceContext, CellKey, ComponentRoute, EdgeSegmentKey, LeafLocator, LeafSet,
    balance_tree, enumerate_segments, leaf_keys, segment_end,
};
use crate::cell_topology::{CUBE_EDGES, CellTopology, classify_cell};
use crate::hermite::locate_edge_zero;
use crate::{
    CellHermiteData, EdgeSearchParams, ProvenanceField, ScalarField, SemiAnalyticFeature,
    SemiAnalyticField, SemiAnalyticProjectionOutcome, locate_edge_intersection,
};

const MIN_EMITTER_DEPTH: u8 = 2;
const LEGACY_SIMPLE_CELL_MAX_CROSSINGS: usize = 3;
const ADAPTIVE_ERROR_FRACTION: f32 = 0.25;

/// Parameters controlling dual-contouring extraction.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct DualContourParams {
    /// Root domain bounds, preserved exactly at integer-grid coordinates `0`
    /// and `2^max_depth`.
    pub root_bounds: Aabb,
    /// Maximum octree depth and finest integer-grid resolution.
    pub max_depth: u8,
    /// Optional cap on contributing leaves emitted into the dual mesh.
    ///
    /// The cap does not bound interval analysis, balancing, or sparse corner
    /// sampling. Truncation is deterministic but may leave an open mesh.
    pub cell_budget: Option<usize>,
    /// Edge intersection search parameters.
    ///
    /// If an exact edge-endpoint crossing has an undefined gradient, the
    /// extractor uses the oriented primal-edge direction as its Hermite normal.
    /// Undefined gradients at interior crossings remain invalid evidence.
    pub edge_search: EdgeSearchParams,
    /// QEF solve parameters.
    pub qef: QefParams,
}

/// Extraction statistics for one dual-contouring run.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct DualContourStats {
    /// Total octree cells stored after initial interval/error subdivision plus
    /// transition balancing and completion refinement.
    pub octree_cells: usize,
    /// Intersecting octree leaves that contributed one or more DC vertices.
    pub active_cells: usize,
    /// Output component/compatibility-vertex count, which may differ from
    /// `active_cells`.
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

/// Semi-analytic projection and fallback counts for one extraction.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct SemiAnalyticContourStats {
    /// Contributing leaves projected onto one dominant primitive surface.
    pub surface_projections: usize,
    /// Contributing leaves snapped onto a supported primitive intersection.
    pub feature_snaps: usize,
    /// Contributing leaves whose primitive pair has no exact feature solver.
    pub unsupported_fallbacks: usize,
    /// Contributing leaves containing more than one surface component.
    pub ambiguous_fallbacks: usize,
    /// Contributing leaves containing a tangent primitive contact.
    pub tangent_fallbacks: usize,
    /// Contributing leaves containing coincident primitive surface patches.
    pub coincident_fallbacks: usize,
    /// Projections rejected for leaving the cell or exceeding its displacement
    /// budget.
    pub over_budget_fallbacks: usize,
    /// Projections rejected for invalid parameters or non-finite output.
    pub invalid_fallbacks: usize,
}

/// Successful semi-analytic dual-contouring result.
#[derive(Clone, Debug)]
pub struct SemiAnalyticContourResult {
    /// Output mesh.
    pub mesh: Mesh,
    /// Ordinary dual-contouring statistics.
    pub stats: DualContourStats,
    /// Semi-analytic projection and fallback statistics.
    pub semi_analytic: SemiAnalyticContourStats,
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
    dual_contour_impl(field, params, |_, _, _| 0)
}

/// Extracts a mesh from `field`, sampling `FACE_REGION` from field provenance
/// at each emitted patch's generating zero crossing.
pub fn dual_contour_with_regions<F>(
    field: &F,
    params: &DualContourParams,
) -> Result<DualContourResult, DualContourError>
where
    F: ProvenanceField<Provenance = u32>,
{
    dual_contour_impl(field, params, |start, end, fallback| {
        let point = locate_edge_zero(field, start, end, &params.edge_search)
            .map_or(fallback, |(point, _)| point);
        field.point_provenance(point)
    })
}

/// Extracts a mesh while projecting supported cells onto analytic primitive
/// surfaces and feature curves.
///
/// This path is opt-in. Unsupported, ambiguous, tangent, coincident,
/// out-of-cell, and invalid projections retain their bounded QEF position and
/// are counted in [`SemiAnalyticContourResult::semi_analytic`]. Faces receive
/// the dominating primitive identity in `FACE_REGION`.
pub fn dual_contour_semi_analytic<F>(
    field: &F,
    params: &DualContourParams,
) -> Result<SemiAnalyticContourResult, DualContourError>
where
    F: SemiAnalyticField,
{
    let (result, semi_analytic) = dual_contour_projected_impl(
        field,
        params,
        |start, end, fallback| {
            let point = locate_edge_zero(field, start, end, &params.edge_search)
                .map_or(fallback, |(point, _)| point);
            field.primitive_at(point)
        },
        |point, cell| Some(field.project_cell_vertex_detailed(point, cell)),
        RefinementMode::ErrorDriven,
    )?;
    Ok(SemiAnalyticContourResult {
        mesh: result.mesh,
        stats: result.stats,
        semi_analytic,
    })
}

#[derive(Clone, Debug, PartialEq)]
struct LeafMarker {
    decision: RefinementDecision,
    analysis: Option<Box<CellAnalysis>>,
}

#[derive(Clone, Debug)]
struct ActiveCell {
    id: CellId,
    key: CellKey,
    bounds: Aabb,
    position: [f32; 3],
    sharpness: SharpnessClass,
    topology: CellTopology,
    components: Vec<ComponentVertex>,
    compatibility: ComponentVertex,
    compatibility_fallback: bool,
    emitted: Vec<Option<VertexEntry>>,
    compatibility_emitted: Option<VertexEntry>,
}

#[derive(Clone, Debug)]
struct ActiveSelection {
    cells: Vec<ActiveCell>,
    omitted_by_budget: Vec<CellId>,
}

#[derive(Copy, Clone, Debug, PartialEq)]
struct ComponentVertex {
    position: [f32; 3],
    sharpness: SharpnessClass,
    constraint_count: u32,
    qef: Option<QefResult>,
}

#[derive(Clone, Debug, PartialEq)]
struct CellVertices {
    components: Vec<ComponentVertex>,
    compatibility: ComponentVertex,
}

#[derive(Clone, Debug, PartialEq)]
struct CellAnalysis {
    corner_values: [f32; 8],
    topology: CellTopology,
    hermite: CellHermiteData,
    vertices: CellVertices,
    evidence: RefinementEvidence,
}

#[derive(Copy, Clone, Debug, PartialEq)]
struct RefinementEvidence {
    expected_crossings: usize,
    hermite_hits: usize,
    complete_hermite: bool,
    usable_constraints: u32,
    qef_rms: f32,
    curvature_error: f32,
    was_clamped: bool,
    finite: bool,
    component_count: usize,
    ambiguous_face: bool,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum RefinementDecision {
    Inactive(InactiveReason),
    Refine(RefinementReason),
    Retain,
    RetainRedundantHermitePlanes,
    MaxDepthCompatibility,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum InactiveReason {
    IntervalExcluded,
    NoCrossingAtMaxDepth,
    LegacyHomogeneous,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum RefinementReason {
    #[cfg(test)]
    ForcedUniform,
    EmitterMinimumDepth,
    EnclosedNoEdge,
    PartialHermite,
    NonFinite,
    TopologyUnsafe,
    Clamped,
    Residual,
    Curvature,
    LegacyCrossingCount,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum RefinementMode {
    ErrorDriven,
    Legacy,
    #[cfg(test)]
    ForcedUniform,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum RedundantHermitePlanesEvidence {
    Satisfied,
    MissingQef,
    InvalidRank,
    InvalidNormal,
    CoplanarityMismatch,
    RankMismatch,
    SingletonGroup,
    NonzeroResidual,
}

#[derive(Copy, Clone)]
struct HermitePlaneGroup {
    normal: [u32; 3],
    offset: f32,
    edge_mask: u16,
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
    R: Fn([f32; 3], [f32; 3], [f32; 3]) -> u32,
{
    dual_contour_projected_impl(
        field,
        params,
        region_at,
        |_, _| None,
        RefinementMode::ErrorDriven,
    )
    .map(|(result, _)| result)
}

fn dual_contour_projected_impl<F, R, P>(
    field: &F,
    params: &DualContourParams,
    region_at: R,
    project: P,
    refinement_mode: RefinementMode,
) -> Result<(DualContourResult, SemiAnalyticContourStats), DualContourError>
where
    F: ScalarField,
    R: Fn([f32; 3], [f32; 3], [f32; 3]) -> u32,
    P: Fn([f32; 3], &Aabb) -> Option<SemiAnalyticProjectionOutcome>,
{
    let resolution = 1_u32 << params.max_depth;
    let mut visitor = IntervalVisitor {
        field,
        params,
        refinement_mode,
        grid: AdaptiveGrid::new(field, params.root_bounds, resolution),
        pending: None,
        failure: None,
    };
    let mut tree = Octree::build(params.root_bounds, params.max_depth, &mut visitor);
    if let Some(error) = visitor.failure {
        return Err(error);
    }
    debug_assert!(
        visitor.pending.is_none(),
        "octree construction must consume every retained leaf payload"
    );
    let (leaf_keys, segments) = prepare_transitions(&mut tree, &mut visitor)?;
    if let Some(error) = visitor.failure {
        return Err(error);
    }

    let octree_cells = tree.len();
    let ActiveSelection {
        cells: mut active_cells,
        omitted_by_budget,
    } = collect_active_cells(params, &tree, &visitor.grid);
    let locator = LeafLocator::new(&leaf_keys, resolution);
    drop(tree);

    if active_cells.is_empty() {
        return Ok((
            DualContourResult {
                mesh: MeshBuilder::new()
                    .build()
                    .expect("empty mesh build should succeed")
                    .mesh,
                stats: DualContourStats {
                    octree_cells,
                    active_cells: 0,
                    vertices: 0,
                    faces: 0,
                },
            },
            SemiAnalyticContourStats::default(),
        ));
    }

    active_cells.sort_by_key(|cell| (cell.key, cell.id));
    let mut semi_analytic = SemiAnalyticContourStats::default();
    for cell in &mut active_cells {
        project_active_cell(cell, &project, &mut semi_analytic);
    }

    let mut builder = MeshBuilder::new();
    emit_transition_faces(
        &segments,
        &locator,
        &visitor.grid,
        &omitted_by_budget,
        &mut active_cells,
        &mut builder,
        &region_at,
    )?;

    let result = builder.build().map_err(DualContourError::Build)?;
    let mut mesh = result.mesh;
    populate_corner_normals(field, &mut mesh);
    populate_region_boundary_seams(&mut mesh);
    Ok((
        DualContourResult {
            stats: DualContourStats {
                octree_cells,
                active_cells: active_cells.len(),
                vertices: mesh.vertices().count(),
                faces: mesh.faces().count(),
            },
            mesh,
        },
        semi_analytic,
    ))
}

fn project_active_cell<P>(cell: &mut ActiveCell, project: &P, stats: &mut SemiAnalyticContourStats)
where
    P: Fn([f32; 3], &Aabb) -> Option<SemiAnalyticProjectionOutcome>,
{
    if cell.components.len() > 1 {
        stats.ambiguous_fallbacks += 1;
        return;
    }
    let project_component = cell.components.first().is_some_and(component_is_usable);
    if project_component {
        let component = cell.components[0];
        cell.position = component.position;
        cell.sharpness = component.sharpness;
    } else if !cell.components.is_empty() {
        if !component_is_usable(&cell.compatibility) {
            stats.invalid_fallbacks += 1;
            return;
        }
        cell.position = cell.compatibility.position;
        cell.sharpness = cell.compatibility.sharpness;
    }
    if let Some(outcome) = project(cell.position, &cell.bounds) {
        apply_projection(cell, outcome, stats);
    }
    if project_component {
        let component = &mut cell.components[0];
        component.position = cell.position;
        component.sharpness = cell.sharpness;
    } else if !cell.components.is_empty() {
        cell.compatibility.position = cell.position;
        cell.compatibility.sharpness = cell.sharpness;
    }
}

fn emit_transition_faces<F, R>(
    segments: &[EdgeSegmentKey],
    locator: &LeafLocator,
    grid: &AdaptiveGrid<'_, F>,
    omitted_by_budget: &[CellId],
    active_cells: &mut [ActiveCell],
    builder: &mut MeshBuilder,
    region_at: &R,
) -> Result<(), DualContourError>
where
    F: ScalarField,
    R: Fn([f32; 3], [f32; 3], [f32; 3]) -> u32,
{
    let mut active_by_id = HashMap::with_capacity(active_cells.len());
    for (index, cell) in active_cells.iter_mut().enumerate() {
        active_by_id.insert(cell.id, index);
        cell.emitted.clear();
        cell.compatibility_emitted = None;
        if cell.components.is_empty() {
            let builder_index = builder.push_vertex(cell.position);
            cell.compatibility_emitted = Some(VertexEntry {
                builder_index,
                position: cell.position,
                sharpness: sharpness_value(cell.sharpness),
            });
        } else {
            for component in &cell.components {
                cell.emitted.push(component_is_usable(component).then(|| {
                    let builder_index = builder.push_vertex(component.position);
                    VertexEntry {
                        builder_index,
                        position: component.position,
                        sharpness: sharpness_value(component.sharpness),
                    }
                }));
            }
            if cell.emitted.iter().any(Option::is_none)
                && (component_is_usable(&cell.compatibility) || cell.compatibility_fallback)
            {
                let builder_index = builder.push_vertex(cell.compatibility.position);
                cell.compatibility_emitted = Some(VertexEntry {
                    builder_index,
                    position: cell.compatibility.position,
                    sharpness: sharpness_value(cell.compatibility.sharpness),
                });
            }
        }
    }

    let mut face_count = 0_usize;
    for &segment in segments {
        let start_value = grid
            .value(segment.start)
            .expect("prepared segment start must be cached");
        let end_key = segment_end(segment);
        let end_value = grid
            .value(end_key)
            .expect("prepared segment end must be cached");
        if !edge_has_crossing(start_value, end_value) {
            continue;
        }
        let incident = match locator.incident_leaves(segment) {
            Ok(Some(incident)) => incident,
            Ok(None) => continue,
            Err(()) => {
                return Err(invalid_transition(face_count, FaceLoopErrorKind::TooShort));
            }
        };
        if incident
            .iter()
            .any(|leaf| omitted_by_budget.binary_search(leaf).is_ok())
        {
            continue;
        }

        let mut entries = Vec::with_capacity(4);
        for leaf in incident {
            let Some(&cell_index) = active_by_id.get(&leaf) else {
                return Err(invalid_transition(face_count, FaceLoopErrorKind::TooShort));
            };
            let cell = &active_cells[cell_index];
            let route = locator.component_route(leaf, segment);
            let Some(entry) = component_entry(cell, route) else {
                return Err(invalid_transition(face_count, FaceLoopErrorKind::TooShort));
            };
            entries.push(entry);
        }
        let mut face = cyclic_vertex_entries(entries, face_count)?;
        if start_value > 0.0 {
            face.reverse();
        }
        let region = region_at(
            grid.point(segment.start),
            grid.point(end_key),
            average_points(&face),
        );
        emit_transition_polygon(builder, &face, region, &mut face_count)?;
    }
    Ok(())
}

fn component_entry(cell: &ActiveCell, route: ComponentRoute) -> Option<VertexEntry> {
    match route {
        ComponentRoute::LocalEdge(edge) => {
            let component = cell.topology.component_for_edge(edge)?;
            component_vertex_entry(cell, usize::from(component))
        }
        ComponentRoute::OnlyComponent
            if cell.components.is_empty() && cell.compatibility_emitted.is_some() =>
        {
            cell.compatibility_emitted
        }
        ComponentRoute::OnlyComponent if cell.components.len() == 1 => {
            component_vertex_entry(cell, 0)
        }
        ComponentRoute::OnlyComponent => None,
    }
}

fn component_vertex_entry(cell: &ActiveCell, component: usize) -> Option<VertexEntry> {
    if component_is_usable(cell.components.get(component)?) {
        cell.emitted.get(component).copied().flatten()
    } else {
        cell.compatibility_emitted
    }
}

fn component_is_usable(component: &ComponentVertex) -> bool {
    component.constraint_count > 0 && component.qef.is_some_and(qef_result_is_finite)
}

fn constraintless_component(component: &ComponentVertex) -> bool {
    component.constraint_count == 0 && component.qef.is_none()
}

fn max_depth_center_fallback_allowed(analysis: &CellAnalysis) -> bool {
    analysis.evidence.complete_hermite
        && analysis.corner_values.iter().all(|value| value.is_finite())
        && analysis.hermite.intersections.iter().all(|hit| {
            hit.intersection
                .position
                .iter()
                .all(|value| value.is_finite())
        })
        && !analysis.hermite.intersections.is_empty()
        && constraintless_component(&analysis.vertices.compatibility)
        && analysis
            .vertices
            .components
            .iter()
            .all(constraintless_component)
        && analysis
            .vertices
            .compatibility
            .position
            .iter()
            .all(|value| value.is_finite())
}

fn cyclic_vertex_entries(
    entries: Vec<VertexEntry>,
    face: usize,
) -> Result<Vec<VertexEntry>, DualContourError> {
    let mut out = Vec::with_capacity(entries.len());
    for entry in entries {
        if out
            .last()
            .is_some_and(|previous: &VertexEntry| previous.builder_index == entry.builder_index)
        {
            continue;
        }
        out.push(entry);
    }
    if out.len() > 1 && out[0].builder_index == out[out.len() - 1].builder_index {
        out.pop();
    }
    if out.len() < 3 {
        return Err(invalid_transition(face, FaceLoopErrorKind::TooShort));
    }
    for first in 0..out.len() {
        if out[first + 1..]
            .iter()
            .any(|entry| entry.builder_index == out[first].builder_index)
        {
            return Err(invalid_transition(face, FaceLoopErrorKind::RepeatedVertex));
        }
    }
    Ok(out)
}

fn emit_transition_polygon(
    builder: &mut MeshBuilder,
    face: &[VertexEntry],
    region: u32,
    face_count: &mut usize,
) -> Result<(), DualContourError> {
    match face {
        [a, b, c] => {
            if !triangle_is_nondegenerate([a.position, b.position, c.position]) {
                return Err(DualContourError::Build(BuildError::DegenerateTriangle {
                    triangle: *face_count,
                }));
            }
            let builder_loop = [a.builder_index, b.builder_index, c.builder_index];
            let sharpness = loop_sharpness3(face);
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
        [a, b, c, d] => {
            let positions = [a.position, b.position, c.position, d.position];
            let sharpness = loop_sharpness4(face);
            let builder_loop = [
                a.builder_index,
                b.builder_index,
                c.builder_index,
                d.builder_index,
            ];
            let (triangles, triangle_sharpness) = match select_quad_diagonal(positions) {
                Some(QuadDiagonal::ZeroTwo) => (
                    [
                        [builder_loop[0], builder_loop[1], builder_loop[2]],
                        [builder_loop[0], builder_loop[2], builder_loop[3]],
                    ],
                    [
                        [sharpness[0], sharpness[1], 0.0],
                        [0.0, sharpness[2], sharpness[3]],
                    ],
                ),
                Some(QuadDiagonal::OneThree) => (
                    [
                        [builder_loop[0], builder_loop[1], builder_loop[3]],
                        [builder_loop[1], builder_loop[2], builder_loop[3]],
                    ],
                    [
                        [sharpness[0], 0.0, sharpness[3]],
                        [sharpness[1], sharpness[2], 0.0],
                    ],
                ),
                None => {
                    return Err(DualContourError::Build(BuildError::DegenerateTriangle {
                        triangle: *face_count,
                    }));
                }
            };
            for (triangle, sharpness) in triangles.into_iter().zip(triangle_sharpness) {
                builder
                    .add_face_with_attrs(
                        &triangle,
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
        _ => return Err(invalid_transition(*face_count, FaceLoopErrorKind::TooShort)),
    }
    Ok(())
}

fn invalid_transition(face: usize, kind: FaceLoopErrorKind) -> DualContourError {
    DualContourError::Build(BuildError::InvalidFaceLoop { face, kind })
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum QuadDiagonal {
    ZeroTwo,
    OneThree,
}

fn select_quad_diagonal(points: [[f32; 3]; 4]) -> Option<QuadDiagonal> {
    let zero_two_valid = triangle_is_nondegenerate([points[0], points[1], points[2]])
        && triangle_is_nondegenerate([points[0], points[2], points[3]]);
    let one_three_valid = triangle_is_nondegenerate([points[0], points[1], points[3]])
        && triangle_is_nondegenerate([points[1], points[2], points[3]]);
    match (zero_two_valid, one_three_valid) {
        (true, false) => return Some(QuadDiagonal::ZeroTwo),
        (false, true) => return Some(QuadDiagonal::OneThree),
        (false, false) => return None,
        (true, true) => {}
    }

    let zero_two = squared_distance(points[0], points[2]);
    let one_three = squared_distance(points[1], points[3]);
    let coordinate_scale = points
        .iter()
        .flatten()
        .copied()
        .map(abs)
        .fold(1.0_f32, f32::max);
    let coordinate_ulp = f32::EPSILON * coordinate_scale;
    let diagonal_scale = sqrt(zero_two.max(one_three));
    let tie_budget = 16.0 * coordinate_ulp * (diagonal_scale + coordinate_ulp);
    if one_three + tie_budget < zero_two {
        Some(QuadDiagonal::OneThree)
    } else {
        Some(QuadDiagonal::ZeroTwo)
    }
}

fn triangle_is_nondegenerate(points: [[f32; 3]; 3]) -> bool {
    let points = points.map(|point| point.map(f64::from));
    let ab = sub3_f64(points[1], points[0]);
    let ac = sub3_f64(points[2], points[0]);
    let bc = sub3_f64(points[2], points[1]);
    let cross = [
        ab[1] * ac[2] - ab[2] * ac[1],
        ab[2] * ac[0] - ab[0] * ac[2],
        ab[0] * ac[1] - ab[1] * ac[0],
    ];
    let area_squared = dot3_f64(cross, cross);
    let longest_edge_squared = dot3_f64(ab, ab).max(dot3_f64(ac, ac)).max(dot3_f64(bc, bc));
    let relative_epsilon = 16.0 * f64::from(f32::EPSILON);
    let minimum_area_squared =
        relative_epsilon * relative_epsilon * longest_edge_squared * longest_edge_squared;
    area_squared.is_finite()
        && longest_edge_squared.is_finite()
        && longest_edge_squared > 0.0
        && area_squared > minimum_area_squared
}

fn sub3_f64(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot3_f64(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn analyze_crossing_cell<F: ScalarField>(
    field: &F,
    params: &DualContourParams,
    corner_values: [f32; 8],
    bounds: Aabb,
) -> Result<CellAnalysis, DualContourError> {
    let corner_signs = corner_sign_mask(&corner_values);
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
        let Ok(mut intersection) = intersection else {
            continue;
        };
        repair_endpoint_normal(
            &mut intersection,
            corner_positions[start_corner],
            corner_positions[end_corner],
            start_value,
            end_value,
        );
        hermite.push(
            u8::try_from(edge_index).expect("cube edge index fits into u8"),
            intersection,
        );
    }
    let topology = classify_cell(&corner_values);
    let vertices = solve_cell_vertices(&topology, &hermite, bounds, &params.qef)?;
    let compatibility = vertices.compatibility;
    let qef_rms = hermite_rms(&hermite, compatibility.position).unwrap_or(f32::NAN);
    let curvature_error = normal_turn_error(&hermite, bounds).unwrap_or(f32::NAN);
    let finite = corner_values.iter().all(|value| value.is_finite())
        && hermite.intersections.iter().all(|hit| {
            hit.intersection
                .position
                .iter()
                .chain(hit.intersection.normal.iter())
                .all(|value| value.is_finite())
        })
        && compatibility.position.iter().all(|value| value.is_finite())
        && compatibility.constraint_count
            == u32::try_from(hermite.intersections.len()).expect("at most 12 Hermite hits")
        && compatibility.qef.is_some_and(qef_result_is_finite)
        && qef_rms.is_finite()
        && curvature_error.is_finite();
    let evidence = RefinementEvidence {
        expected_crossings: crossing_edge_count(&corner_values),
        hermite_hits: hermite.intersections.len(),
        complete_hermite: hermite_edge_mask(&hermite) == crossing_edge_mask(&corner_values),
        usable_constraints: compatibility.constraint_count,
        qef_rms,
        curvature_error,
        was_clamped: compatibility.qef.is_some_and(|result| result.was_clamped),
        finite,
        component_count: vertices.components.len(),
        ambiguous_face: topology.has_ambiguous_face(),
    };

    Ok(CellAnalysis {
        corner_values,
        topology,
        hermite,
        vertices,
        evidence,
    })
}

fn repair_endpoint_normal(
    intersection: &mut crate::HermiteIntersection,
    start: [f32; 3],
    end: [f32; 3],
    start_value: f32,
    end_value: f32,
) {
    if normalize_vector(intersection.normal).is_some() {
        return;
    }
    let at_endpoint = intersection.t.is_finite()
        && ((0.0..=f32::EPSILON).contains(&intersection.t)
            || (1.0 - f32::EPSILON..=1.0).contains(&intersection.t));
    if !at_endpoint {
        return;
    }
    let direction = [end[0] - start[0], end[1] - start[1], end[2] - start[2]];
    let Some(mut normal) = normalize_vector(direction) else {
        return;
    };
    if end_value < start_value {
        normal = [-normal[0], -normal[1], -normal[2]];
    }
    intersection.normal = normal;
}

fn solve_cell_vertices(
    topology: &CellTopology,
    hermite: &CellHermiteData,
    bounds: Aabb,
    params: &QefParams,
) -> Result<CellVertices, DualContourError> {
    let component_hermite = topology.partition_hermite(hermite);
    let components = solve_component_vertices(&component_hermite, bounds, params)?;

    // Refinement evidence retains the original combined-QEF compatibility
    // result for multi-component and checkerboard cells. Transition emission
    // consumes the component results directly; unambiguous cells can share the
    // same solve for both roles.
    let compatibility = if components.len() == 1
        && !topology.has_ambiguous_face()
        && usize::try_from(components[0].constraint_count).ok() == Some(hermite.intersections.len())
    {
        components[0]
    } else {
        solve_hermite_vertex(hermite, bounds, params)?
    };

    Ok(CellVertices {
        components,
        compatibility,
    })
}

fn solve_component_vertices(
    component_hermite: &[CellHermiteData],
    bounds: Aabb,
    params: &QefParams,
) -> Result<Vec<ComponentVertex>, DualContourError> {
    component_hermite
        .iter()
        .map(|hermite| solve_hermite_vertex(hermite, bounds, params))
        .collect()
}

fn solve_hermite_vertex(
    hermite: &CellHermiteData,
    bounds: Aabb,
    params: &QefParams,
) -> Result<ComponentVertex, DualContourError> {
    let mut solver = QefSolver::new();
    for hit in &hermite.intersections {
        let _ = solver.add_constraint(PlaneConstraint {
            position: hit.intersection.position,
            normal: hit.intersection.normal,
        });
    }
    if solver.constraint_count() == 0 {
        return Ok(ComponentVertex {
            position: bounds.center(),
            sharpness: SharpnessClass::Smooth,
            constraint_count: 0,
            qef: None,
        });
    }
    let result = solver
        .solve_with_anchor(
            QefBounds::new(bounds.min, bounds.max).expect("cell bounds are valid"),
            hermite_mass_point(hermite),
            params,
        )
        .map_err(DualContourError::Solve)?;

    Ok(ComponentVertex {
        position: result.position,
        sharpness: result.sharpness_class,
        constraint_count: solver.constraint_count(),
        qef: Some(result),
    })
}

fn prepare_transitions<F: ScalarField>(
    tree: &mut Octree<LeafMarker>,
    visitor: &mut IntervalVisitor<'_, F>,
) -> Result<(LeafSet, Vec<EdgeSegmentKey>), DualContourError> {
    loop {
        balance_tree(tree, visitor);
        if let Some(error) = visitor.failure {
            return Err(error);
        }
        let leaves = leaf_keys(tree, &visitor.grid);
        let segments = enumerate_segments(&leaves);
        let mut endpoints = Vec::with_capacity(segments.len() * 2);
        for &segment in &segments {
            endpoints.push(segment.start);
            endpoints.push(segment_end(segment));
        }
        visitor.grid.sample_keys(&endpoints);

        let locator = LeafLocator::new(&leaves, visitor.grid.resolution());
        let mut refine = Vec::new();
        let mut unresolved = false;
        for &segment in &segments {
            let start = visitor
                .grid
                .value(segment.start)
                .expect("segment start must be cached");
            let end = visitor
                .grid
                .value(segment_end(segment))
                .expect("segment end must be cached");
            if !edge_has_crossing(start, end) {
                continue;
            }
            let incident = match locator.incident_leaves(segment) {
                Ok(Some(incident)) => incident,
                Ok(None) => continue,
                Err(()) => {
                    return Err(invalid_transition(0, FaceLoopErrorKind::TooShort));
                }
            };
            let mut tokens = [None; 4];
            for (slot, leaf) in incident.into_iter().enumerate() {
                let route = locator.component_route(leaf, segment);
                tokens[slot] = transition_component_token(tree, leaf, route);
                if tokens[slot].is_none() {
                    unresolved |= !push_refinement_candidate(
                        tree,
                        &locator,
                        leaf,
                        visitor.params.max_depth,
                        &mut refine,
                    );
                }
            }
            if tokens.iter().all(Option::is_some) {
                let tokens = tokens.map(|token| token.expect("checked all transition tokens"));
                if cyclic_distinct(tokens).is_err() {
                    for leaf in incident {
                        unresolved |= !push_refinement_candidate(
                            tree,
                            &locator,
                            leaf,
                            visitor.params.max_depth,
                            &mut refine,
                        );
                    }
                }
            }
        }

        refine.sort_unstable_by_key(|&(key, id)| (key, id));
        refine.dedup_by_key(|entry| entry.1);
        if refine.is_empty() {
            if unresolved {
                return Err(invalid_transition(0, FaceLoopErrorKind::TooShort));
            }
            return Ok((leaves, segments));
        }
        for (_, id) in refine {
            tree.refine_leaf(id, visitor.params.max_depth, visitor)
                .expect("completion candidates are current octree leaves");
        }
        if let Some(error) = visitor.failure {
            return Err(error);
        }
    }
}

fn transition_component_token(
    tree: &Octree<LeafMarker>,
    leaf: CellId,
    route: ComponentRoute,
) -> Option<(CellId, u8)> {
    let payload = tree.cell(leaf)?.payload()?;
    if !matches!(
        payload.decision,
        RefinementDecision::Retain
            | RefinementDecision::RetainRedundantHermitePlanes
            | RefinementDecision::MaxDepthCompatibility
    ) {
        return None;
    }
    let analysis = payload.analysis.as_ref()?;
    let center_fallback = matches!(payload.decision, RefinementDecision::MaxDepthCompatibility)
        && max_depth_center_fallback_allowed(analysis);
    let component = match route {
        ComponentRoute::LocalEdge(edge) => analysis.topology.component_for_edge(edge)?,
        ComponentRoute::OnlyComponent if analysis.vertices.components.len() == 1 => 0,
        ComponentRoute::OnlyComponent if analysis.vertices.components.is_empty() => {
            return (component_is_usable(&analysis.vertices.compatibility) || center_fallback)
                .then_some((leaf, u8::MAX));
        }
        ComponentRoute::OnlyComponent => return None,
    };
    let routed = analysis.vertices.components.get(usize::from(component))?;
    if component_is_usable(routed) {
        Some((leaf, component))
    } else if center_fallback && constraintless_component(routed) {
        Some((leaf, u8::MAX))
    } else {
        component_is_usable(&analysis.vertices.compatibility).then_some((leaf, u8::MAX))
    }
}

fn push_refinement_candidate(
    tree: &Octree<LeafMarker>,
    locator: &LeafLocator,
    leaf: CellId,
    max_depth: u8,
    out: &mut Vec<(CellKey, CellId)>,
) -> bool {
    let Some(cell) = tree.cell(leaf) else {
        return false;
    };
    if cell.depth < max_depth {
        out.push((locator.key(leaf), leaf));
        true
    } else {
        false
    }
}

fn cyclic_distinct<T: Copy + Eq>(values: [T; 4]) -> Result<Vec<T>, ()> {
    let mut out = Vec::with_capacity(4);
    for value in values {
        if out.last().copied() != Some(value) {
            out.push(value);
        }
    }
    if out.len() > 1 && out.first() == out.last() {
        out.pop();
    }
    if out.len() < 3 {
        return Err(());
    }
    for first in 0..out.len() {
        if out[first + 1..].contains(&out[first]) {
            return Err(());
        }
    }
    Ok(out)
}

fn collect_active_cells<F: ScalarField>(
    params: &DualContourParams,
    tree: &Octree<LeafMarker>,
    grid: &AdaptiveGrid<'_, F>,
) -> ActiveSelection {
    let mut active_cells = Vec::new();
    let mut omitted_by_budget = Vec::new();
    for leaf_id in tree.leaf_ids() {
        let Some(cell) = tree.cell(leaf_id) else {
            continue;
        };
        let Some(payload) = cell.payload() else {
            continue;
        };
        if !matches!(
            payload.decision,
            RefinementDecision::Retain
                | RefinementDecision::RetainRedundantHermitePlanes
                | RefinementDecision::MaxDepthCompatibility
        ) {
            continue;
        }
        let Some(analysis) = payload.analysis.as_ref() else {
            continue;
        };
        let compatibility = analysis.vertices.compatibility;
        let active = ActiveCell {
            id: cell.id,
            key: grid.cell_key(cell.id),
            bounds: grid.cell_bounds(grid.cell_key(cell.id)),
            position: compatibility.position,
            sharpness: compatibility.sharpness,
            topology: analysis.topology.clone(),
            components: analysis.vertices.components.clone(),
            compatibility,
            compatibility_fallback: matches!(
                payload.decision,
                RefinementDecision::MaxDepthCompatibility
            ) && max_depth_center_fallback_allowed(analysis),
            emitted: Vec::new(),
            compatibility_emitted: None,
        };
        if params
            .cell_budget
            .is_some_and(|budget| active_cells.len() >= budget)
        {
            omitted_by_budget.push(cell.id);
        } else {
            active_cells.push(active);
        }
    }
    active_cells.sort_by_key(|cell| (cell.key, cell.id));
    omitted_by_budget.sort_unstable();
    ActiveSelection {
        cells: active_cells,
        omitted_by_budget,
    }
}

fn step_size(root_bounds: Aabb, resolution: u32) -> [f32; 3] {
    let extent = root_bounds.extent();
    [
        extent[0] / resolution as f32,
        extent[1] / resolution as f32,
        extent[2] / resolution as f32,
    ]
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

fn crossing_edge_mask(corner_values: &[f32; 8]) -> u16 {
    CUBE_EDGES
        .iter()
        .enumerate()
        .fold(0_u16, |mask, (edge, &(start, end))| {
            if edge_has_crossing(corner_values[start], corner_values[end]) {
                mask | (1_u16 << edge)
            } else {
                mask
            }
        })
}

fn hermite_edge_mask(hermite: &CellHermiteData) -> u16 {
    hermite.intersections.iter().fold(0_u16, |mask, hit| {
        if hit.edge_index < 12 {
            mask | (1_u16 << hit.edge_index)
        } else {
            mask
        }
    })
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

fn apply_projection(
    cell: &mut ActiveCell,
    outcome: SemiAnalyticProjectionOutcome,
    stats: &mut SemiAnalyticContourStats,
) {
    match outcome {
        SemiAnalyticProjectionOutcome::Projected(projection) => {
            if !projection
                .position
                .iter()
                .all(|component| component.is_finite())
            {
                stats.invalid_fallbacks += 1;
                return;
            }
            if !point_within_bounds(projection.position, cell.bounds) {
                stats.over_budget_fallbacks += 1;
                return;
            }
            let cell_diagonal_squared = squared_distance(cell.bounds.min, cell.bounds.max);
            if squared_distance(cell.position, projection.position) > cell_diagonal_squared {
                stats.over_budget_fallbacks += 1;
                return;
            }
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
        SemiAnalyticProjectionOutcome::Unsupported => stats.unsupported_fallbacks += 1,
        SemiAnalyticProjectionOutcome::Ambiguous => stats.ambiguous_fallbacks += 1,
        SemiAnalyticProjectionOutcome::Tangent => stats.tangent_fallbacks += 1,
        SemiAnalyticProjectionOutcome::Coincident => stats.coincident_fallbacks += 1,
        SemiAnalyticProjectionOutcome::OverBudget => stats.over_budget_fallbacks += 1,
        SemiAnalyticProjectionOutcome::Invalid => stats.invalid_fallbacks += 1,
    }
}

fn point_within_bounds(point: [f32; 3], bounds: Aabb) -> bool {
    let scale = bounds
        .min
        .into_iter()
        .chain(bounds.max)
        .fold(1.0_f32, |scale, value| scale.max(abs(value)));
    let tolerance = 64.0 * f32::EPSILON * scale;
    (0..3).all(|axis| {
        point[axis] >= bounds.min[axis] - tolerance && point[axis] <= bounds.max[axis] + tolerance
    })
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

struct IntervalVisitor<'a, F> {
    field: &'a F,
    params: &'a DualContourParams,
    refinement_mode: RefinementMode,
    grid: AdaptiveGrid<'a, F>,
    pending: Option<(CellId, LeafMarker)>,
    failure: Option<DualContourError>,
}

impl<F: ScalarField> IntervalVisitor<'_, F> {
    fn inspect_cell(
        &mut self,
        cell: CellRef,
        at_max_depth: bool,
    ) -> Result<LeafMarker, DualContourError> {
        let cell_key = self.grid.locate_cell(cell);
        let cell_bounds = self.grid.cell_bounds(cell_key);
        let intersects = self
            .field
            .eval_interval(&cell_bounds)
            .is_none_or(interval_crosses_zero);
        if !intersects {
            return Ok(LeafMarker {
                decision: RefinementDecision::Inactive(InactiveReason::IntervalExcluded),
                analysis: None,
            });
        }

        #[cfg(test)]
        if self.refinement_mode == RefinementMode::ForcedUniform && !at_max_depth {
            return Ok(LeafMarker {
                decision: RefinementDecision::Refine(RefinementReason::ForcedUniform),
                analysis: None,
            });
        }

        if self.refinement_mode == RefinementMode::Legacy
            && !at_max_depth
            && cell.depth < MIN_EMITTER_DEPTH
        {
            return Ok(LeafMarker {
                decision: RefinementDecision::Refine(RefinementReason::EmitterMinimumDepth),
                analysis: None,
            });
        }

        let corner_values = self.grid.sample_cell_corners(cell_key);
        let expected_crossings = crossing_edge_count(&corner_values);

        if expected_crossings == 0 {
            let decision = if at_max_depth {
                RefinementDecision::Inactive(InactiveReason::NoCrossingAtMaxDepth)
            } else if self.refinement_mode == RefinementMode::Legacy {
                RefinementDecision::Inactive(InactiveReason::LegacyHomogeneous)
            } else if corner_values.iter().all(|value| value.is_finite()) {
                RefinementDecision::Refine(RefinementReason::EnclosedNoEdge)
            } else {
                RefinementDecision::Refine(RefinementReason::NonFinite)
            };
            return Ok(LeafMarker {
                decision,
                analysis: None,
            });
        }

        if !at_max_depth
            && self.refinement_mode == RefinementMode::ErrorDriven
            && cell.depth < MIN_EMITTER_DEPTH
        {
            return Ok(LeafMarker {
                decision: RefinementDecision::Refine(RefinementReason::EmitterMinimumDepth),
                analysis: None,
            });
        }

        if self.refinement_mode == RefinementMode::Legacy
            && !at_max_depth
            && expected_crossings > LEGACY_SIMPLE_CELL_MAX_CROSSINGS
        {
            return Ok(LeafMarker {
                decision: RefinementDecision::Refine(RefinementReason::LegacyCrossingCount),
                analysis: None,
            });
        }

        let analysis = analyze_crossing_cell(self.field, self.params, corner_values, cell_bounds)?;
        let decision = if at_max_depth {
            if analysis.hermite.intersections.is_empty() {
                RefinementDecision::Inactive(InactiveReason::NoCrossingAtMaxDepth)
            } else {
                RefinementDecision::MaxDepthCompatibility
            }
        } else if self.refinement_mode == RefinementMode::Legacy {
            RefinementDecision::Retain
        } else {
            refinement_decision(&analysis.evidence, adaptive_error_target(self.params))
        };
        let decision = if decision == RefinementDecision::Refine(RefinementReason::Curvature)
            && redundant_hermite_planes_evidence(&analysis, cell_bounds)
                == RedundantHermitePlanesEvidence::Satisfied
        {
            RefinementDecision::RetainRedundantHermitePlanes
        } else {
            decision
        };
        Ok(LeafMarker {
            decision,
            analysis: Some(Box::new(analysis)),
        })
    }

    fn record_failure(&mut self, error: DualContourError) -> LeafMarker {
        self.failure.get_or_insert(error);
        LeafMarker {
            decision: RefinementDecision::Inactive(InactiveReason::NoCrossingAtMaxDepth),
            analysis: None,
        }
    }
}

fn redundant_hermite_planes_evidence(
    analysis: &CellAnalysis,
    bounds: Aabb,
) -> RedundantHermitePlanesEvidence {
    classify_redundant_hermite_planes(
        &analysis.hermite,
        analysis.vertices.compatibility.qef,
        bounds.center(),
    )
}

fn classify_redundant_hermite_planes(
    hermite: &CellHermiteData,
    qef: Option<QefResult>,
    center: [f32; 3],
) -> RedundantHermitePlanesEvidence {
    let Some(qef) = qef else {
        return RedundantHermitePlanesEvidence::MissingQef;
    };
    if !(1..=3).contains(&qef.rank) {
        return RedundantHermitePlanesEvidence::InvalidRank;
    }

    let mut groups: [Option<HermitePlaneGroup>; 12] = [None; 12];
    let mut group_count = 0;
    for hit in &hermite.intersections {
        let Some(normal) = normalize_vector(hit.intersection.normal) else {
            return RedundantHermitePlanesEvidence::InvalidNormal;
        };
        let normal_key = normal.map(|component| {
            if component == 0.0 {
                0.0_f32.to_bits()
            } else {
                component.to_bits()
            }
        });
        let local =
            core::array::from_fn::<_, 3, _>(|axis| hit.intersection.position[axis] - center[axis]);
        let offset = dot3_f32(normal, local);
        let edge_bit = 1_u16.checked_shl(u32::from(hit.edge_index)).unwrap_or(0);

        if let Some(group) = groups[..group_count]
            .iter_mut()
            .flatten()
            .find(|group| group.normal == normal_key)
        {
            if group.offset != offset {
                return RedundantHermitePlanesEvidence::CoplanarityMismatch;
            }
            group.edge_mask |= edge_bit;
        } else {
            let Some(slot) = groups.get_mut(group_count) else {
                return RedundantHermitePlanesEvidence::RankMismatch;
            };
            *slot = Some(HermitePlaneGroup {
                normal: normal_key,
                offset,
                edge_mask: edge_bit,
            });
            group_count += 1;
        }

        let displacement = core::array::from_fn::<_, 3, _>(|axis| {
            qef.position[axis] - hit.intersection.position[axis]
        });
        if dot3_f32(normal, displacement) != 0.0 {
            return RedundantHermitePlanesEvidence::NonzeroResidual;
        }
    }

    if group_count != usize::from(qef.rank) {
        return RedundantHermitePlanesEvidence::RankMismatch;
    }
    if groups[..group_count]
        .iter()
        .flatten()
        .any(|group| group.edge_mask.count_ones() < 2)
    {
        return RedundantHermitePlanesEvidence::SingletonGroup;
    }
    RedundantHermitePlanesEvidence::Satisfied
}

#[inline]
fn dot3_f32(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

impl<F: ScalarField> OctreeVisitor for IntervalVisitor<'_, F> {
    type Payload = LeafMarker;

    fn should_subdivide(&mut self, cell: CellRef) -> bool {
        debug_assert!(
            self.pending.is_none(),
            "the prior retained leaf payload must be consumed before visiting another cell"
        );
        let inspection = match self.inspect_cell(cell, false) {
            Ok(inspection) => inspection,
            Err(error) => self.record_failure(error),
        };
        if matches!(inspection.decision, RefinementDecision::Refine(_)) {
            true
        } else {
            self.pending = Some((cell.id, inspection));
            false
        }
    }

    fn make_leaf_payload(&mut self, cell: CellRef) -> Self::Payload {
        // `Octree::build_subtree` calls `make_leaf_payload` immediately after
        // `should_subdivide` returns false. The exact-ID assertion makes that
        // sequencing dependency visible. Max-depth cells bypass
        // `should_subdivide` and are analyzed explicitly below.
        if let Some((pending_id, payload)) = self.pending.take() {
            debug_assert_eq!(
                pending_id, cell.id,
                "pending refinement evidence must be consumed by the same cell"
            );
            if pending_id == cell.id {
                return payload;
            }
        }

        match self.inspect_cell(cell, true) {
            Ok(payload) => payload,
            Err(error) => self.record_failure(error),
        }
    }
}

impl<F: ScalarField> BalanceContext for IntervalVisitor<'_, F> {
    type Field = F;

    fn transition_grid(&self) -> &AdaptiveGrid<'_, Self::Field> {
        &self.grid
    }

    fn global_max_depth(&self) -> u8 {
        self.params.max_depth
    }

    fn failed(&self) -> bool {
        self.failure.is_some()
    }
}

fn refinement_decision(evidence: &RefinementEvidence, target: f32) -> RefinementDecision {
    if evidence.hermite_hits != evidence.expected_crossings || !evidence.complete_hermite {
        return RefinementDecision::Refine(RefinementReason::PartialHermite);
    }
    if !evidence.finite
        || evidence.usable_constraints
            != u32::try_from(evidence.hermite_hits).expect("at most 12 Hermite hits")
    {
        return RefinementDecision::Refine(RefinementReason::NonFinite);
    }
    if evidence.component_count != 1 || evidence.ambiguous_face {
        return RefinementDecision::Refine(RefinementReason::TopologyUnsafe);
    }
    if evidence.was_clamped {
        return RefinementDecision::Refine(RefinementReason::Clamped);
    }
    if evidence.qef_rms > target {
        return RefinementDecision::Refine(RefinementReason::Residual);
    }
    if evidence.curvature_error > target {
        return RefinementDecision::Refine(RefinementReason::Curvature);
    }
    RefinementDecision::Retain
}

fn adaptive_error_target(params: &DualContourParams) -> f32 {
    let resolution = 1_u32 << params.max_depth;
    ADAPTIVE_ERROR_FRACTION * vector_length(step_size(params.root_bounds, resolution))
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

fn hermite_rms(hermite: &CellHermiteData, position: [f32; 3]) -> Option<f32> {
    if hermite.intersections.is_empty() || !position.iter().all(|value| value.is_finite()) {
        return None;
    }

    let mut squared_error = 0.0_f32;
    for hit in &hermite.intersections {
        let normal = normalize_vector(hit.intersection.normal)?;
        if !hit
            .intersection
            .position
            .iter()
            .all(|value| value.is_finite())
        {
            return None;
        }
        let displacement = [
            position[0] - hit.intersection.position[0],
            position[1] - hit.intersection.position[1],
            position[2] - hit.intersection.position[2],
        ];
        let distance =
            normal[0] * displacement[0] + normal[1] * displacement[1] + normal[2] * displacement[2];
        squared_error += distance * distance;
    }
    let rms = sqrt(squared_error / hermite.intersections.len() as f32);
    rms.is_finite().then_some(rms)
}

fn normal_turn_error(hermite: &CellHermiteData, bounds: Aabb) -> Option<f32> {
    let mut normals = Vec::with_capacity(hermite.intersections.len());
    for hit in &hermite.intersections {
        normals.push(normalize_vector(hit.intersection.normal)?);
    }

    let mut minimum_dot = 1.0_f32;
    for first in 0..normals.len() {
        for second in first + 1..normals.len() {
            let dot = normals[first][0] * normals[second][0]
                + normals[first][1] * normals[second][1]
                + normals[first][2] * normals[second][2];
            minimum_dot = minimum_dot.min(dot.clamp(-1.0, 1.0));
        }
    }
    let half_angle_sine = sqrt(((1.0 - minimum_dot) * 0.5).max(0.0));
    Some(0.5 * vector_length(bounds.extent()) * half_angle_sine)
}

fn normalize_vector(vector: [f32; 3]) -> Option<[f32; 3]> {
    let length = vector_length(vector);
    if !length.is_finite() || length <= 1.0e-8 {
        return None;
    }
    Some([vector[0] / length, vector[1] / length, vector[2] / length])
}

fn vector_length(vector: [f32; 3]) -> f32 {
    sqrt(vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2])
}

fn qef_result_is_finite(result: QefResult) -> bool {
    result
        .position
        .iter()
        .chain(result.eigenvalues.iter())
        .all(|value| value.is_finite())
        && result.residual_error.is_finite()
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
    use alloc::boxed::Box;
    use alloc::vec;
    use alloc::vec::Vec;
    use core::mem::size_of;

    use super::{
        ActiveCell, AdaptiveGrid, CellAnalysis, CellKey, ComponentRoute, ComponentVertex,
        DualContourParams, InactiveReason, IntervalVisitor, LeafMarker, QuadDiagonal,
        RedundantHermitePlanesEvidence, RefinementDecision, RefinementEvidence, RefinementMode,
        RefinementReason, SemiAnalyticContourStats, VertexEntry, adaptive_error_target,
        analyze_crossing_cell, apply_projection, classify_redundant_hermite_planes,
        collect_active_cells, constraintless_component, cube_corner_positions, cyclic_distinct,
        dual_contour, dual_contour_projected_impl, dual_contour_semi_analytic,
        dual_contour_with_regions, emit_transition_polygon, hermite_rms, normal_turn_error,
        prepare_transitions, project_active_cell, refinement_decision, repair_endpoint_normal,
        select_quad_diagonal, solve_cell_vertices, squared_distance, transition_component_token,
        triangle_is_nondegenerate,
    };
    use crate::analytic::{
        BoxField, CylinderField, Difference, HalfSpaceField, SphereField, TaggedField, Union,
    };
    use crate::cell_topology::classify_cell;
    use crate::{
        AnalyticCylinder, AnalyticPrimitive, CellHermiteData, EdgeSearchParams,
        HermiteIntersection, ProvenanceField, ScalarField, SemiAnalyticField,
        SemiAnalyticProjection, SemiAnalyticProjectionOutcome,
    };
    use exedra::{BuildError, ExtractParams, FaceLoopErrorKind, attr};
    use exedra_qef::{QefParams, QefResult};
    use exedra_spatial::{Aabb, CellId, CellRef, Octree, OctreeVisitor};
    use hashbrown::HashSet;

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

    fn assert_closed_transition_mesh(mesh: &exedra::Mesh) {
        assert!(mesh.validate_deep().is_empty());
        assert!(
            mesh.boundary_loops()
                .expect("closed fixture boundary traversal")
                .is_empty()
        );

        let mut referenced = HashSet::new();
        for face in mesh.faces() {
            let corners = mesh.face_loop(face).collect::<Vec<_>>();
            assert_eq!(corners.len(), 3, "transition output must be triangulated");
            let mut positions = [[0.0_f32; 3]; 3];
            for (slot, corner) in corners.into_iter().enumerate() {
                let vertex = mesh.to_vertex(corner).expect("face corner has a vertex");
                referenced.insert(vertex);
                positions[slot] = *mesh.vertex_position(vertex).expect("vertex has a position");
                let twin = mesh.twin(corner).expect("closed edge has a twin");
                assert_ne!(
                    mesh.face(twin),
                    Some(exedra::FaceId::OUTSIDE),
                    "every emitted edge must have exactly two incident faces"
                );
            }
            assert!(
                triangle_is_nondegenerate(positions),
                "degenerate face {face:?}"
            );
        }
        assert_eq!(
            referenced.len(),
            mesh.vertices().count(),
            "every emitted closed-fixture vertex must be face-referenced"
        );
    }

    fn degenerate_face_count(mesh: &exedra::Mesh) -> usize {
        mesh.faces()
            .filter(|&face| {
                let positions = mesh
                    .face_loop(face)
                    .map(|corner| {
                        let vertex = mesh.to_vertex(corner).expect("face corner vertex");
                        *mesh.vertex_position(vertex).expect("vertex position")
                    })
                    .collect::<Vec<_>>();
                let [a, b, c] = positions.as_slice() else {
                    return true;
                };
                !triangle_is_nondegenerate([*a, *b, *c])
            })
            .count()
    }

    fn semi_analytic_counter_total(stats: SemiAnalyticContourStats) -> usize {
        stats.surface_projections
            + stats.feature_snaps
            + stats.unsupported_fallbacks
            + stats.ambiguous_fallbacks
            + stats.tangent_fallbacks
            + stats.coincident_fallbacks
            + stats.over_budget_fallbacks
            + stats.invalid_fallbacks
    }

    struct AxisTaggedUnion {
        field: Union<BoxField, BoxField>,
    }

    struct SurfaceTaggedSphere {
        field: SphereField,
        tolerance: f32,
    }

    struct PartiallyNonFinitePlane;

    struct SliceSensitivePlane {
        retain_some_edge_hits: bool,
    }

    struct UnknownIntervalBox {
        field: BoxField,
    }

    struct QuantizedGradientSphere {
        field: SphereField,
    }

    struct ConstantGradientSphere {
        field: SphereField,
    }

    struct CheckerboardPairField;

    struct FixedMarkerVisitor {
        marker: LeafMarker,
    }

    impl ScalarField for QuantizedGradientSphere {
        fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]> {
            self.field.eval_interval(bounds)
        }

        fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
            self.field.eval_points(points, out);
        }

        fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
            self.field.eval_gradients(points, out);
            for row in out {
                for component in &mut row[1..] {
                    *component = if *component > 0.0 {
                        1.0
                    } else if *component < 0.0 {
                        -1.0
                    } else {
                        0.0
                    };
                }
            }
        }
    }

    impl ScalarField for ConstantGradientSphere {
        fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]> {
            self.field.eval_interval(bounds)
        }

        fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
            self.field.eval_points(points, out);
        }

        fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
            self.field.eval_gradients(points, out);
            for row in out {
                row[1..].copy_from_slice(&[1.0, 0.0, 0.0]);
            }
        }
    }

    impl OctreeVisitor for FixedMarkerVisitor {
        type Payload = LeafMarker;

        fn should_subdivide(&mut self, _cell: CellRef) -> bool {
            false
        }

        fn make_leaf_payload(&mut self, _cell: CellRef) -> Self::Payload {
            self.marker.clone()
        }
    }

    impl ScalarField for CheckerboardPairField {
        fn eval_interval(&self, _bounds: &Aabb) -> Option<[f32; 2]> {
            None
        }

        fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
            let frequency = 2.0 * core::f32::consts::PI;
            for (point, value) in points.iter().zip(out) {
                *value = test_cos(frequency * point[0]) * test_cos(frequency * point[1]);
            }
        }

        fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
            let frequency = 2.0 * core::f32::consts::PI;
            for (point, gradient) in points.iter().zip(out) {
                let x = frequency * point[0];
                let y = frequency * point[1];
                let value = test_cos(x) * test_cos(y);
                *gradient = [
                    value,
                    -frequency * test_sin(x) * test_cos(y),
                    -frequency * test_cos(x) * test_sin(y),
                    0.0,
                ];
            }
        }
    }

    impl SemiAnalyticField for CheckerboardPairField {
        fn project_cell_vertex(
            &self,
            _point: [f32; 3],
            _cell: &Aabb,
        ) -> Option<SemiAnalyticProjection> {
            None
        }

        fn primitive_at(&self, _point: [f32; 3]) -> u32 {
            0
        }
    }

    impl ScalarField for PartiallyNonFinitePlane {
        fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]> {
            Some([bounds.min[0], bounds.max[0]])
        }

        fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
            for (point, value) in points.iter().zip(out) {
                *value = point[0];
            }
        }

        fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
            for (point, gradient) in points.iter().zip(out) {
                *gradient = if point[1] < 0.0 {
                    [point[0], 1.0, 0.0, 0.0]
                } else {
                    [point[0], f32::NAN, 0.0, 0.0]
                };
            }
        }
    }

    impl ScalarField for SliceSensitivePlane {
        fn eval_interval(&self, _bounds: &Aabb) -> Option<[f32; 2]> {
            Some([-1.0, 1.0])
        }

        fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
            let expose_crossing = points.len() > 2
                || (self.retain_some_edge_hits
                    && points.first().is_some_and(|point| point[1] < 0.0));
            for (point, value) in points.iter().zip(out) {
                *value = if expose_crossing { point[0] } else { 1.0 };
            }
        }

        fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
            for (point, gradient) in points.iter().zip(out) {
                *gradient = [point[0], 1.0, 0.0, 0.0];
            }
        }
    }

    impl ScalarField for UnknownIntervalBox {
        fn eval_interval(&self, _bounds: &Aabb) -> Option<[f32; 2]> {
            None
        }

        fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
            self.field.eval_points(points, out);
        }

        fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
            self.field.eval_gradients(points, out);
        }
    }

    #[derive(Copy, Clone)]
    struct MalformedCylinderLeaf {
        field: TaggedField<CylinderField, u32>,
    }

    impl ScalarField for MalformedCylinderLeaf {
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

    impl SemiAnalyticField for MalformedCylinderLeaf {
        fn project_cell_vertex(
            &self,
            point: [f32; 3],
            cell: &Aabb,
        ) -> Option<SemiAnalyticProjection> {
            self.field.project_cell_vertex(point, cell)
        }

        fn primitive_at(&self, point: [f32; 3]) -> u32 {
            self.field.primitive_at(point)
        }

        fn leaf_primitive(&self) -> Option<AnalyticPrimitive> {
            Some(AnalyticPrimitive::Cylinder(AnalyticCylinder {
                radius: -self.field.field.radius,
                primitive: self.field.provenance,
                center: self.field.field.center,
                axis: self.field.field.axis,
                half_height: self.field.field.half_height,
            }))
        }
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

    impl ScalarField for SurfaceTaggedSphere {
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

    impl ProvenanceField for SurfaceTaggedSphere {
        type Provenance = u32;

        fn eval_interval_with_provenance(
            &self,
            bounds: &Aabb,
        ) -> Option<([f32; 2], Self::Provenance)> {
            self.eval_interval(bounds)
                .map(|interval| (interval, self.point_provenance(bounds.center())))
        }

        fn point_provenance(&self, point: [f32; 3]) -> Self::Provenance {
            let mut value = [0.0_f32; 1];
            self.eval_points(&[point], &mut value);
            if value[0].abs() <= self.tolerance {
                7
            } else {
                99
            }
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

        assert_closed_transition_mesh(&first.mesh);
        assert_eq!(
            first.stats,
            super::DualContourStats {
                octree_cells: 585,
                active_cells: 320,
                vertices: 320,
                faces: 636,
            }
        );
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

        assert_closed_transition_mesh(&result.mesh);
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
    fn dual_contour_attributes_faces_at_primal_zero_crossings() {
        let field = SurfaceTaggedSphere {
            field: SphereField {
                center: [0.0, 0.0, 0.0],
                radius: 1.0,
            },
            tolerance: 1.0e-3,
        };
        let bounds = Aabb::new([-1.5, -1.5, -1.5], [1.5, 1.5, 1.5]).expect("bounds");

        let first = dual_contour_with_regions(&field, &params(bounds, 4))
            .expect("surface-attributed sphere should extract");
        let second = dual_contour_with_regions(&field, &params(bounds, 4))
            .expect("surface attribution should be deterministic");

        let first_regions = first
            .mesh
            .attrs()
            .dense(attr::FACE_REGION)
            .expect("FACE_REGION layer should exist");
        let second_regions = second
            .mesh
            .attrs()
            .dense(attr::FACE_REGION)
            .expect("FACE_REGION layer should exist");
        let first_values = first
            .mesh
            .faces()
            .map(|face| {
                first_regions
                    .get(face.as_id())
                    .copied()
                    .expect("every face should be attributed")
            })
            .collect::<Vec<_>>();
        let second_values = second
            .mesh
            .faces()
            .map(|face| {
                second_regions
                    .get(face.as_id())
                    .copied()
                    .expect("every face should be attributed")
            })
            .collect::<Vec<_>>();

        assert!(!first_values.is_empty());
        assert!(first_values.iter().all(|region| *region == 7));
        assert_eq!(first_values, second_values);
        assert!(first.mesh.faces().any(|face| {
            let corners = first.mesh.face_loop(face).collect::<Vec<_>>();
            field.point_provenance(super::face_centroid(&first.mesh, &corners)) == 99
        }));
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
        let uncapped =
            dual_contour(&field, &params(bounds, 4)).expect("uncapped extraction should work");
        let mut capped_params = params(bounds, 4);
        capped_params.cell_budget = Some(32);

        let first = dual_contour(&field, &capped_params).expect("budgeted extraction should work");
        let second = dual_contour(&field, &capped_params).expect("budgeted extraction repeats");

        assert_eq!(first.stats.active_cells, 32);
        assert!(first.stats.faces < uncapped.stats.faces);
        assert!(first.mesh.validate_deep().is_empty());
        assert_eq!(mesh_geometry(&first.mesh), mesh_geometry(&second.mesh));
    }

    #[test]
    fn nonbinding_budget_is_bit_identical_to_uncapped_output() {
        let field = SphereField {
            center: [0.0, 0.0, 0.0],
            radius: 1.0,
        };
        let bounds = Aabb::new([-1.5; 3], [1.5; 3]).expect("bounds");
        let uncapped = dual_contour(&field, &params(bounds, 4)).expect("uncapped extraction");
        let mut nonbinding_params = params(bounds, 4);
        nonbinding_params.cell_budget = Some(usize::MAX);
        let nonbinding =
            dual_contour(&field, &nonbinding_params).expect("nonbinding budget extraction");

        assert_eq!(nonbinding.stats, uncapped.stats);
        assert_eq!(
            mesh_geometry(&nonbinding.mesh),
            mesh_geometry(&uncapped.mesh)
        );
    }

    #[test]
    fn budget_selects_contributors_in_octree_leaf_storage_order() {
        let field = SphereField {
            center: [0.0, 0.0, 0.0],
            radius: 1.0,
        };
        let bounds = Aabb::new([-1.5; 3], [1.5; 3]).expect("bounds");
        let params = params(bounds, 4);
        let resolution = 1_u32 << params.max_depth;
        let mut visitor = IntervalVisitor {
            field: &field,
            params: &params,
            refinement_mode: RefinementMode::Legacy,
            grid: AdaptiveGrid::new(&field, params.root_bounds, resolution),
            pending: None,
            failure: None,
        };
        let mut tree = Octree::build(params.root_bounds, params.max_depth, &mut visitor);
        prepare_transitions(&mut tree, &mut visitor).expect("transition completion");
        let contributors = tree
            .leaf_ids()
            .into_iter()
            .filter(|&id| {
                tree.cell(id)
                    .and_then(|cell| cell.payload())
                    .is_some_and(|payload| {
                        matches!(
                            payload.decision,
                            RefinementDecision::Retain
                                | RefinementDecision::RetainRedundantHermitePlanes
                                | RefinementDecision::MaxDepthCompatibility
                        ) && payload.analysis.is_some()
                    })
            })
            .collect::<Vec<_>>();
        let budget = 11;
        let mut capped_params = params;
        capped_params.cell_budget = Some(budget);
        let selection = collect_active_cells(&capped_params, &tree, &visitor.grid);

        let mut expected_selected = contributors[..budget].to_vec();
        expected_selected.sort_unstable_by_key(|&id| (visitor.grid.cell_key(id), id));
        let mut expected_omitted = contributors[budget..].to_vec();
        expected_omitted.sort_unstable();
        assert_eq!(
            selection
                .cells
                .iter()
                .map(|cell| cell.id)
                .collect::<Vec<_>>(),
            expected_selected
        );
        assert_eq!(selection.omitted_by_budget, expected_omitted);
    }

    #[test]
    fn unresolved_max_depth_transition_is_not_hidden_by_nonbinding_budget() {
        let field = SliceSensitivePlane {
            retain_some_edge_hits: false,
        };
        let bounds = Aabb::new([-1.0; 3], [1.0; 3]).expect("bounds");
        for budget in [None, Some(usize::MAX)] {
            let mut params = params(bounds, 2);
            params.cell_budget = budget;
            assert!(matches!(
                dual_contour(&field, &params),
                Err(super::DualContourError::Build(
                    BuildError::InvalidFaceLoop {
                        face: 0,
                        kind: FaceLoopErrorKind::TooShort,
                    }
                ))
            ));
        }
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

        assert_closed_transition_mesh(&result.mesh);
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
    fn semi_analytic_box_minus_cylinder_snaps_features_and_attributes_primitives() {
        let bounds = Aabb::new([-1.4; 3], [1.4; 3]).expect("bounds");
        for depth in [4, 5, 6] {
            let field = tagged_box_minus_cylinder([0.0, 0.0, 1.0]);
            let result = dual_contour_semi_analytic(&field, &params(bounds, depth))
                .expect("semi-analytic through-cut should extract");

            assert_closed_transition_mesh(&result.mesh);
            assert!(result.semi_analytic.feature_snaps > 0);
            assert!(result.semi_analytic.surface_projections > 0);
            assert_eq!(
                semi_analytic_counter_total(result.semi_analytic),
                result.stats.active_cells,
                "projection outcomes partition contributing leaves"
            );
            assert!(result.stats.vertices >= result.stats.active_cells);
            let rim_vertices = result
                .mesh
                .vertices()
                .filter(|&vertex| {
                    let Some(position) = result.mesh.vertex_position(vertex) else {
                        return false;
                    };
                    let radius = (position[0] * position[0] + position[1] * position[1]).sqrt();
                    (position[2].abs() - 1.0).abs() <= 2.0e-5 && (radius - 0.6).abs() <= 2.0e-5
                })
                .count();
            assert!(rim_vertices >= result.semi_analytic.feature_snaps);
            let regions = result
                .mesh
                .attrs()
                .dense(attr::FACE_REGION)
                .expect("primitive regions should exist");
            assert!(
                result
                    .mesh
                    .faces()
                    .any(|face| regions.get(face.as_id()) == Some(&10))
            );
            assert!(
                result
                    .mesh
                    .faces()
                    .any(|face| regions.get(face.as_id()) == Some(&20))
            );
            let sharpness = result
                .mesh
                .attrs()
                .sparse(attr::EDGE_SHARPNESS)
                .expect("feature sharpness should exist");
            assert!(result.mesh.faces().any(|face| {
                result.mesh.face_loop(face).any(|edge| {
                    sharpness
                        .get(edge.as_id())
                        .is_some_and(|value| *value >= 1.0)
                })
            }));

            let repeated = dual_contour_semi_analytic(&field, &params(bounds, depth))
                .expect("semi-analytic extraction should repeat");
            let (first_tri, first_stats) = result.mesh.to_trimesh(&ExtractParams::default());
            let (second_tri, second_stats) = repeated.mesh.to_trimesh(&ExtractParams::default());
            assert_eq!(result.stats, repeated.stats);
            assert_eq!(result.semi_analytic, repeated.semi_analytic);
            assert_eq!(first_stats, second_stats);
            assert_eq!(first_tri.positions, second_tri.positions);
            assert_eq!(first_tri.indices, second_tri.indices);
        }
    }

    #[test]
    fn budgeted_multi_component_cells_emit_all_components_and_count_once_per_leaf() {
        let mut extraction_params = params(Aabb::new([-1.0; 3], [1.0; 3]).expect("bounds"), 2);
        extraction_params.cell_budget = Some(1);

        let result = dual_contour_semi_analytic(&CheckerboardPairField, &extraction_params)
            .expect("budgeted multi-component extraction");

        assert_eq!(result.stats.active_cells, 1);
        assert_eq!(result.semi_analytic.ambiguous_fallbacks, 1);
        assert!(result.stats.vertices > result.stats.active_cells);
        assert_eq!(
            semi_analytic_counter_total(result.semi_analytic),
            result.stats.active_cells
        );
        assert_eq!(
            result.stats.faces, 0,
            "the explicit leaf cap permits a hole"
        );
    }

    #[test]
    fn semi_analytic_rotated_box_cylinder_pair_counts_unsupported_fallbacks() {
        let field = tagged_box_minus_cylinder([1.0, 1.0, 0.0]);
        let bounds = Aabb::new([-1.4; 3], [1.4; 3]).expect("bounds");

        let result = dual_contour_semi_analytic(&field, &params(bounds, 5))
            .expect("unsupported pair should retain QEF output");

        assert!(result.mesh.validate_deep().is_empty());
        assert!(result.semi_analytic.unsupported_fallbacks > 0);
        assert_eq!(result.semi_analytic.feature_snaps, 0);
    }

    #[test]
    fn malformed_pair_counts_invalid_and_retains_qef_output() {
        let box_field = TaggedField {
            field: BoxField {
                center: [0.0; 3],
                half_extents: [1.0; 3],
            },
            provenance: 10,
        };
        let cylinder = MalformedCylinderLeaf {
            field: TaggedField {
                field: CylinderField {
                    center: [0.0; 3],
                    axis: [0.0, 0.0, 1.0],
                    radius: 0.6,
                    half_height: 2.0,
                },
                provenance: 20,
            },
        };
        let field = Difference::new(box_field, cylinder);
        let extraction_params = params(Aabb::new([-1.4; 3], [1.4; 3]).expect("bounds"), 4);

        let ordinary = dual_contour(&field, &extraction_params).expect("ordinary QEF extraction");
        let semi = dual_contour_semi_analytic(&field, &extraction_params)
            .expect("malformed analytic metadata must retain QEF output");
        let (ordinary_tri, _) = ordinary.mesh.to_trimesh(&ExtractParams::default());
        let (semi_tri, _) = semi.mesh.to_trimesh(&ExtractParams::default());

        assert!(semi.mesh.validate_deep().is_empty());
        assert!(
            semi.semi_analytic.invalid_fallbacks > 0,
            "malformed primitive metadata must count Invalid"
        );
        assert_eq!(
            semi.semi_analytic.unsupported_fallbacks, 0,
            "malformed metadata must not be classified as unsupported orientation"
        );
        assert_eq!(
            ordinary_tri.positions, semi_tri.positions,
            "Invalid fallback must retain QEF positions"
        );
        assert_eq!(
            ordinary_tri.indices, semi_tri.indices,
            "Invalid fallback must retain QEF topology"
        );
    }

    #[test]
    fn typed_pair_fallbacks_do_not_move_the_active_cell() {
        let bounds = Aabb::new([0.0; 3], [1.0; 3]).expect("cell bounds");
        let box_field = TaggedField {
            field: BoxField {
                center: [0.0; 3],
                half_extents: [1.0; 3],
            },
            provenance: 10,
        };
        let cylinder = TaggedField {
            field: CylinderField {
                center: [0.0; 3],
                axis: [0.0, 0.0, 1.0],
                radius: 0.6,
                half_height: 2.0,
            },
            provenance: 20,
        };
        let thin_strip = Aabb::new([-0.1, -0.7, 0.9], [0.1, 0.7, 1.1]).expect("thin strip");
        let disconnected_arcs = Difference::new(box_field, cylinder)
            .project_cell_vertex_detailed([0.0, 0.58, 0.98], &thin_strip);
        assert_eq!(
            disconnected_arcs,
            SemiAnalyticProjectionOutcome::Ambiguous,
            "two clipped circle arcs must classify as one ambiguous cell"
        );

        for (outcome, expected) in [
            (
                disconnected_arcs,
                SemiAnalyticContourStats {
                    ambiguous_fallbacks: 1,
                    ..SemiAnalyticContourStats::default()
                },
            ),
            (
                SemiAnalyticProjectionOutcome::Tangent,
                SemiAnalyticContourStats {
                    tangent_fallbacks: 1,
                    ..SemiAnalyticContourStats::default()
                },
            ),
            (
                SemiAnalyticProjectionOutcome::Coincident,
                SemiAnalyticContourStats {
                    coincident_fallbacks: 1,
                    ..SemiAnalyticContourStats::default()
                },
            ),
            (
                SemiAnalyticProjectionOutcome::OverBudget,
                SemiAnalyticContourStats {
                    over_budget_fallbacks: 1,
                    ..SemiAnalyticContourStats::default()
                },
            ),
        ] {
            let mut cell = ActiveCell {
                id: CellId::from_index(0),
                key: CellKey {
                    origin: crate::adaptive_transition::CornerKey::new(0, 0, 0),
                    span: 1,
                    depth: 1,
                },
                bounds,
                position: [0.25, 0.5, 0.75],
                sharpness: exedra_qef::SharpnessClass::Smooth,
                topology: classify_cell(&values_for_mask(1)),
                components: Vec::new(),
                compatibility: ComponentVertex {
                    position: [0.25, 0.5, 0.75],
                    sharpness: exedra_qef::SharpnessClass::Smooth,
                    constraint_count: 0,
                    qef: None,
                },
                compatibility_fallback: false,
                emitted: Vec::new(),
                compatibility_emitted: None,
            };
            let original = cell.position;
            let mut stats = SemiAnalyticContourStats::default();

            apply_projection(&mut cell, outcome, &mut stats);

            assert_eq!(cell.position, original, "typed fallback moved QEF point");
            assert_eq!(stats, expected, "typed fallback incremented wrong counter");
        }
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
        let mut visitor = IntervalVisitor {
            field: &field,
            params: &params,
            refinement_mode: RefinementMode::ErrorDriven,
            grid: AdaptiveGrid::new(&field, params.root_bounds, resolution),
            pending: None,
            failure: None,
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
            let Some(analysis) = cell.payload().and_then(|payload| payload.analysis.as_ref())
            else {
                continue;
            };
            if squared_distance(
                analysis.vertices.compatibility.position,
                cell.bounds.center(),
            ) > 1.0e-8
            {
                found_non_center = true;
                break;
            }
        }

        assert!(found_non_center);
    }

    #[test]
    fn multi_component_cell_solves_each_component_without_cross_talk() {
        let values = values_for_mask(0b1000_0001);
        let topology = classify_cell(&values);
        let hermite = separated_component_hermite();
        let groups = topology.partition_hermite(&hermite);
        let bounds = Aabb::new([0.0; 3], [1.0; 3]).expect("unit bounds");
        let solved = solve_cell_vertices(&topology, &hermite, bounds, &QefParams::default())
            .expect("component QEFs solve");

        assert_eq!(groups.len(), 2);
        assert_eq!(
            groups[0]
                .intersections
                .iter()
                .map(|hit| hit.edge_index)
                .collect::<Vec<_>>(),
            vec![0, 3, 8]
        );
        assert_eq!(
            groups[1]
                .intersections
                .iter()
                .map(|hit| hit.edge_index)
                .collect::<Vec<_>>(),
            vec![5, 6, 10]
        );
        assert_eq!(solved.components.len(), 2);
        assert_eq!(solved.components[0].position, [0.25; 3]);
        assert_eq!(solved.components[1].position, [0.75; 3]);
        assert_eq!(
            solved.components[0].sharpness,
            exedra_qef::SharpnessClass::Corner
        );
        assert_eq!(
            solved.components[1].sharpness,
            exedra_qef::SharpnessClass::Corner
        );
        assert_eq!(
            solved.compatibility.position, [0.5; 3],
            "analysis must retain the combined-QEF compatibility representative"
        );
    }

    #[test]
    fn constraintless_component_routes_to_one_compatibility_token() {
        let values = values_for_mask(0b1000_0001);
        let topology = classify_cell(&values);
        let mut hermite = separated_component_hermite();
        hermite
            .intersections
            .retain(|hit| topology.component_for_edge(hit.edge_index) == Some(0));
        let bounds = Aabb::new([0.0; 3], [1.0; 3]).expect("unit bounds");
        let vertices = solve_cell_vertices(&topology, &hermite, bounds, &QefParams::default())
            .expect("remaining component QEF solves");
        assert_eq!(vertices.components.len(), 2);
        assert_eq!(vertices.components[1].constraint_count, 0);
        assert!(vertices.components[1].qef.is_none());
        assert_eq!(vertices.components[1].position, bounds.center());
        assert_eq!(vertices.compatibility.constraint_count, 3);
        assert_ne!(vertices.compatibility.position, bounds.center());

        let invalid_component = vertices.components[1];
        let mut projected = ActiveCell {
            id: CellId::from_index(0),
            key: CellKey {
                origin: crate::adaptive_transition::CornerKey::new(0, 0, 0),
                span: 1,
                depth: 0,
            },
            bounds,
            position: invalid_component.position,
            sharpness: invalid_component.sharpness,
            topology: topology.clone(),
            components: vec![invalid_component],
            compatibility: vertices.compatibility,
            compatibility_fallback: false,
            emitted: Vec::new(),
            compatibility_emitted: None,
        };
        let expected_input = vertices.compatibility.position;
        let expected_output = [0.3, 0.25, 0.25];
        let mut projection_stats = SemiAnalyticContourStats::default();
        project_active_cell(
            &mut projected,
            &|point, _| {
                assert_eq!(point, expected_input);
                Some(SemiAnalyticProjectionOutcome::Projected(
                    SemiAnalyticProjection {
                        position: expected_output,
                        feature: crate::SemiAnalyticFeature::Surface,
                        primitive: 1,
                    },
                ))
            },
            &mut projection_stats,
        );
        assert_eq!(projected.components[0], invalid_component);
        assert_eq!(projected.compatibility.position, expected_output);
        assert_eq!(projection_stats.surface_projections, 1);

        let marker = LeafMarker {
            decision: RefinementDecision::MaxDepthCompatibility,
            analysis: Some(Box::new(CellAnalysis {
                corner_values: values,
                topology: topology.clone(),
                hermite,
                vertices,
                evidence: RefinementEvidence {
                    expected_crossings: 6,
                    hermite_hits: 3,
                    complete_hermite: false,
                    usable_constraints: 3,
                    qef_rms: 0.0,
                    curvature_error: 0.0,
                    was_clamped: false,
                    finite: true,
                    component_count: 2,
                    ambiguous_face: false,
                },
            })),
        };
        let mut visitor = FixedMarkerVisitor { marker };
        let tree = Octree::build(bounds, 0, &mut visitor);
        let root = tree.root_id();
        let valid_edge = (0_u8..12)
            .find(|&edge| topology.component_for_edge(edge) == Some(0))
            .expect("first component edge");
        let missing_edge = (0_u8..12)
            .find(|&edge| topology.component_for_edge(edge) == Some(1))
            .expect("second component edge");

        assert_eq!(
            transition_component_token(&tree, root, ComponentRoute::LocalEdge(valid_edge)),
            Some((root, 0))
        );
        assert_eq!(
            transition_component_token(&tree, root, ComponentRoute::LocalEdge(missing_edge)),
            Some((root, u8::MAX)),
            "constraintless components must alias the one combined-QEF compatibility token"
        );
    }

    #[test]
    fn complete_max_depth_constraintless_cell_uses_one_center_compatibility_token() {
        let values = values_for_mask(1);
        let topology = classify_cell(&values);
        let bounds = Aabb::new([0.0; 3], [1.0; 3]).expect("unit bounds");
        let mut hermite = CellHermiteData::new(1);
        for edge in [0_u8, 3, 8] {
            hermite.push(
                edge,
                HermiteIntersection {
                    position: [0.5; 3],
                    normal: [f32::NAN; 3],
                    t: 0.5,
                },
            );
        }
        let vertices = solve_cell_vertices(&topology, &hermite, bounds, &QefParams::default())
            .expect("constraintless QEF solve should produce center representatives");
        assert!(vertices.components.iter().all(constraintless_component));
        assert!(constraintless_component(&vertices.compatibility));
        assert_eq!(vertices.compatibility.position, bounds.center());

        let marker = LeafMarker {
            decision: RefinementDecision::MaxDepthCompatibility,
            analysis: Some(Box::new(CellAnalysis {
                corner_values: values,
                topology: topology.clone(),
                hermite,
                vertices,
                evidence: RefinementEvidence {
                    expected_crossings: 3,
                    hermite_hits: 3,
                    complete_hermite: true,
                    usable_constraints: 0,
                    qef_rms: f32::NAN,
                    curvature_error: f32::NAN,
                    was_clamped: false,
                    finite: false,
                    component_count: 1,
                    ambiguous_face: false,
                },
            })),
        };
        let mut visitor = FixedMarkerVisitor { marker };
        let tree = Octree::build(bounds, 0, &mut visitor);
        let root = tree.root_id();
        let edge = (0_u8..12)
            .find(|&edge| topology.component_for_edge(edge) == Some(0))
            .expect("crossing edge");
        assert_eq!(
            transition_component_token(&tree, root, ComponentRoute::LocalEdge(edge)),
            Some((root, u8::MAX))
        );

        let mut partial = tree
            .cell(root)
            .and_then(|cell| cell.payload())
            .cloned()
            .expect("fixture marker");
        partial
            .analysis
            .as_mut()
            .expect("analysis")
            .evidence
            .complete_hermite = false;
        let mut partial_visitor = FixedMarkerVisitor { marker: partial };
        let partial_tree = Octree::build(bounds, 0, &mut partial_visitor);
        assert_eq!(
            transition_component_token(
                &partial_tree,
                partial_tree.root_id(),
                ComponentRoute::LocalEdge(edge)
            ),
            None,
            "partial scalar crossing masks must not earn the center fallback"
        );

        let mut retained = partial_tree
            .cell(partial_tree.root_id())
            .and_then(|cell| cell.payload())
            .cloned()
            .expect("partial fixture marker");
        retained.decision = RefinementDecision::Retain;
        let mut retained_visitor = FixedMarkerVisitor { marker: retained };
        let retained_tree = Octree::build(bounds, 0, &mut retained_visitor);
        assert_eq!(
            transition_component_token(
                &retained_tree,
                retained_tree.root_id(),
                ComponentRoute::LocalEdge(edge)
            ),
            None,
            "non-max-depth leaves must not earn the center fallback"
        );
    }

    #[test]
    fn cyclic_distinct_only_collapses_adjacent_repetitions() {
        assert_eq!(cyclic_distinct([1, 1, 2, 3]), Ok(vec![1, 2, 3]));
        assert_eq!(cyclic_distinct([1, 2, 3, 1]), Ok(vec![1, 2, 3]));
        assert_eq!(cyclic_distinct([1, 2, 1, 3]), Err(()));
        assert_eq!(cyclic_distinct([1, 1, 2, 2]), Err(()));
    }

    #[test]
    fn endpoint_normal_repair_is_oriented_and_does_not_hide_interior_nan() {
        let mut endpoint = HermiteIntersection {
            position: [1.0, 0.0, 0.0],
            normal: [f32::NAN; 3],
            t: 1.0,
        };
        repair_endpoint_normal(&mut endpoint, [0.0, 0.0, 0.0], [1.0, 0.0, 0.0], -1.0, 1.0);
        assert_eq!(endpoint.normal, [1.0, 0.0, 0.0]);

        let mut reversed = endpoint;
        reversed.normal = [f32::NAN; 3];
        repair_endpoint_normal(&mut reversed, [0.0, 0.0, 0.0], [1.0, 0.0, 0.0], 1.0, -1.0);
        assert_eq!(reversed.normal, [-1.0, 0.0, 0.0]);

        let mut interior = endpoint;
        interior.normal = [f32::NAN; 3];
        interior.t = 0.5;
        repair_endpoint_normal(&mut interior, [0.0, 0.0, 0.0], [1.0, 0.0, 0.0], -1.0, 1.0);
        assert!(interior.normal.iter().all(|value| value.is_nan()));

        interior.t = f32::NAN;
        repair_endpoint_normal(&mut interior, [0.0, 0.0, 0.0], [1.0, 0.0, 0.0], -1.0, 1.0);
        assert!(interior.normal.iter().all(|value| value.is_nan()));
    }

    #[test]
    fn one_component_cell_reuses_the_component_result_bit_for_bit() {
        let values = values_for_mask(0b0000_0001);
        let topology = classify_cell(&values);
        let mut hermite = CellHermiteData::new(0b0000_0001);
        push_plane_hit(&mut hermite, 0, [0.25, 0.0, 0.0], [1.0, 0.0, 0.0]);
        push_plane_hit(&mut hermite, 3, [0.0, 0.25, 0.0], [0.0, 1.0, 0.0]);
        push_plane_hit(&mut hermite, 8, [0.0, 0.0, 0.25], [0.0, 0.0, 1.0]);
        let bounds = Aabb::new([0.0; 3], [1.0; 3]).expect("unit bounds");

        let solved = solve_cell_vertices(&topology, &hermite, bounds, &QefParams::default())
            .expect("component QEF solves");

        assert_eq!(solved.components.len(), 1);
        assert_eq!(solved.compatibility, solved.components[0]);
        assert_eq!(solved.compatibility.position, [0.25; 3]);
    }

    #[test]
    fn refinement_reason_precedence_is_explicit() {
        let target = 0.1;
        let good = RefinementEvidence {
            expected_crossings: 4,
            hermite_hits: 4,
            complete_hermite: true,
            usable_constraints: 4,
            qef_rms: 0.01,
            curvature_error: 0.02,
            was_clamped: false,
            finite: true,
            component_count: 1,
            ambiguous_face: false,
        };
        assert_eq!(
            refinement_decision(&good, target),
            RefinementDecision::Retain
        );

        let partial = RefinementEvidence {
            hermite_hits: 3,
            complete_hermite: false,
            finite: false,
            component_count: 2,
            was_clamped: true,
            qef_rms: 1.0,
            curvature_error: 1.0,
            ..good
        };
        assert_eq!(
            refinement_decision(&partial, target),
            RefinementDecision::Refine(RefinementReason::PartialHermite)
        );
        let partial_but_solvable = RefinementEvidence {
            hermite_hits: 3,
            complete_hermite: false,
            usable_constraints: 3,
            ..good
        };
        assert_eq!(
            refinement_decision(&partial_but_solvable, target),
            RefinementDecision::Refine(RefinementReason::PartialHermite),
            "a finite solvable subset must not stand in for a missing crossing edge"
        );
        let wrong_edge_set = RefinementEvidence {
            complete_hermite: false,
            ..good
        };
        assert_eq!(
            refinement_decision(&wrong_edge_set, target),
            RefinementDecision::Refine(RefinementReason::PartialHermite)
        );
        let non_finite = RefinementEvidence {
            finite: false,
            component_count: 2,
            was_clamped: true,
            qef_rms: 1.0,
            curvature_error: 1.0,
            ..good
        };
        assert_eq!(
            refinement_decision(&non_finite, target),
            RefinementDecision::Refine(RefinementReason::NonFinite)
        );
        let unsafe_topology = RefinementEvidence {
            component_count: 2,
            was_clamped: true,
            qef_rms: 1.0,
            curvature_error: 1.0,
            ..good
        };
        assert_eq!(
            refinement_decision(&unsafe_topology, target),
            RefinementDecision::Refine(RefinementReason::TopologyUnsafe)
        );
        let ambiguous_topology = RefinementEvidence {
            ambiguous_face: true,
            ..good
        };
        assert_eq!(
            refinement_decision(&ambiguous_topology, target),
            RefinementDecision::Refine(RefinementReason::TopologyUnsafe)
        );
        let clamped = RefinementEvidence {
            was_clamped: true,
            qef_rms: 1.0,
            curvature_error: 1.0,
            ..good
        };
        assert_eq!(
            refinement_decision(&clamped, target),
            RefinementDecision::Refine(RefinementReason::Clamped)
        );
        let residual = RefinementEvidence {
            qef_rms: 1.0,
            curvature_error: 1.0,
            ..good
        };
        assert_eq!(
            refinement_decision(&residual, target),
            RefinementDecision::Refine(RefinementReason::Residual)
        );
        let curvature = RefinementEvidence {
            curvature_error: 1.0,
            ..good
        };
        assert_eq!(
            refinement_decision(&curvature, target),
            RefinementDecision::Refine(RefinementReason::Curvature)
        );
    }

    #[test]
    fn rms_and_normal_turn_evidence_have_known_geometric_values() {
        let mut hermite = CellHermiteData::new(0b0000_0011);
        push_plane_hit(&mut hermite, 0, [0.0, 0.0, 0.0], [2.0, 0.0, 0.0]);
        push_plane_hit(&mut hermite, 1, [2.0, 0.0, 0.0], [-3.0, 0.0, 0.0]);
        assert_eq!(hermite_rms(&hermite, [1.0, 0.0, 0.0]), Some(1.0));

        let mut quarter_turn = CellHermiteData::new(0b0000_0011);
        push_plane_hit(&mut quarter_turn, 0, [0.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
        push_plane_hit(&mut quarter_turn, 1, [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        let bounds = Aabb::new([0.0; 3], [1.0; 3]).expect("unit bounds");
        let turn = normal_turn_error(&quarter_turn, bounds).expect("finite normal turn");
        assert!((0.6123..0.6124).contains(&turn), "{turn}");
    }

    #[test]
    fn redundant_hermite_plane_evidence_has_strict_typed_failures() {
        for (scale, translation) in [
            (1.0e-3, [0.0; 3]),
            (1.0, [128.0, -64.0, 32.0]),
            (1.0e3, [0.0; 3]),
        ] {
            let (hermite, qef, center) = redundant_plane_fixture(scale, translation);
            assert_eq!(
                classify_redundant_hermite_planes(&hermite, Some(qef), center),
                RedundantHermitePlanesEvidence::Satisfied,
                "scale={scale} translation={translation:?}"
            );

            let mut reversed = hermite.clone();
            reversed.intersections.reverse();
            assert_eq!(
                classify_redundant_hermite_planes(&reversed, Some(qef), center),
                RedundantHermitePlanesEvidence::Satisfied
            );
        }

        let (hermite, qef, center) = redundant_plane_fixture(1.0, [0.0; 3]);
        assert_eq!(
            classify_redundant_hermite_planes(&hermite, None, center),
            RedundantHermitePlanesEvidence::MissingQef
        );

        let mut invalid_rank = qef;
        invalid_rank.rank = 0;
        assert_eq!(
            classify_redundant_hermite_planes(&hermite, Some(invalid_rank), center),
            RedundantHermitePlanesEvidence::InvalidRank
        );

        let mut invalid_normal = hermite.clone();
        invalid_normal.intersections[0].intersection.normal[0] = f32::NAN;
        assert_eq!(
            classify_redundant_hermite_planes(&invalid_normal, Some(qef), center),
            RedundantHermitePlanesEvidence::InvalidNormal
        );

        let mut offset_mismatch = hermite.clone();
        offset_mismatch.intersections[1].intersection.position[0] += 0.125;
        assert_eq!(
            classify_redundant_hermite_planes(&offset_mismatch, Some(qef), center),
            RedundantHermitePlanesEvidence::CoplanarityMismatch
        );

        let mut rank_mismatch = qef;
        rank_mismatch.rank = 2;
        assert_eq!(
            classify_redundant_hermite_planes(&hermite, Some(rank_mismatch), center),
            RedundantHermitePlanesEvidence::RankMismatch
        );

        let mut singleton = hermite.clone();
        singleton.intersections.remove(1);
        assert_eq!(
            classify_redundant_hermite_planes(&singleton, Some(qef), center),
            RedundantHermitePlanesEvidence::SingletonGroup
        );

        let mut duplicate_edge = hermite.clone();
        duplicate_edge.intersections[1].edge_index = duplicate_edge.intersections[0].edge_index;
        assert_eq!(
            classify_redundant_hermite_planes(&duplicate_edge, Some(qef), center),
            RedundantHermitePlanesEvidence::SingletonGroup
        );

        let mut nonzero_residual = qef;
        nonzero_residual.position[0] += 0.125;
        assert_eq!(
            classify_redundant_hermite_planes(&hermite, Some(nonzero_residual), center),
            RedundantHermitePlanesEvidence::NonzeroResidual
        );
    }

    #[test]
    fn analyzed_nonfinite_hermite_data_refines_before_topology_or_error_budgets() {
        let field = PartiallyNonFinitePlane;
        let bounds = Aabb::new([-1.0; 3], [1.0; 3]).expect("bounds");
        let params = params(bounds, 5);
        let corners = cube_corner_positions(bounds);
        let mut corner_values = [0.0_f32; 8];
        field.eval_points(&corners, &mut corner_values);

        let analysis = analyze_crossing_cell(&field, &params, corner_values, bounds)
            .expect("the two finite constraints keep the compatibility QEF solvable");

        assert_eq!(analysis.evidence.expected_crossings, 4);
        assert_eq!(analysis.evidence.hermite_hits, 4);
        assert_eq!(analysis.evidence.usable_constraints, 2);
        assert!(!analysis.evidence.finite);
        assert_eq!(
            refinement_decision(&analysis.evidence, adaptive_error_target(&params)),
            RefinementDecision::Refine(RefinementReason::NonFinite)
        );
    }

    #[test]
    fn inactive_leaf_markers_do_not_embed_cell_analysis_storage() {
        assert!(
            size_of::<LeafMarker>() <= 4 * size_of::<usize>(),
            "LeafMarker grew to {} bytes",
            size_of::<LeafMarker>()
        );
    }

    #[test]
    fn interval_enclosure_and_max_depth_have_distinct_decisions() {
        let bounds = Aabb::new([-1.0; 3], [1.0; 3]).expect("bounds");
        let params = params(bounds, 5);

        let far_sphere = SphereField {
            center: [5.0; 3],
            radius: 0.25,
        };
        assert_eq!(
            inspect_root(&far_sphere, &params, false).decision,
            RefinementDecision::Inactive(InactiveReason::IntervalExcluded)
        );

        let enclosed_sphere = SphereField {
            center: [0.0; 3],
            radius: 0.1,
        };
        assert_eq!(
            inspect_root(&enclosed_sphere, &params, false).decision,
            RefinementDecision::Refine(RefinementReason::EnclosedNoEdge)
        );
        assert_eq!(
            inspect_root(&enclosed_sphere, &params, true).decision,
            RefinementDecision::Inactive(InactiveReason::NoCrossingAtMaxDepth)
        );

        let plane = HalfSpaceField {
            point: [0.1375, -0.08125, 0.10625],
            normal: [1.0, 2.0, -1.0],
        };
        assert_eq!(
            inspect_root(&plane, &params, false).decision,
            RefinementDecision::Refine(RefinementReason::EmitterMinimumDepth)
        );
        assert_eq!(
            inspect_root(&plane, &params, true).decision,
            RefinementDecision::MaxDepthCompatibility
        );
    }

    #[test]
    fn forced_uniform_refines_an_intersecting_nonmax_cell() {
        let field = HalfSpaceField {
            point: [0.1375, -0.08125, 0.10625],
            normal: [1.0, 2.0, -1.0],
        };
        let bounds = Aabb::new([-1.0; 3], [1.0; 3]).expect("bounds");
        let extraction_params = params(bounds, 5);
        let mut visitor = IntervalVisitor {
            field: &field,
            params: &extraction_params,
            refinement_mode: RefinementMode::ForcedUniform,
            grid: AdaptiveGrid::new(&field, bounds, 1 << extraction_params.max_depth),
            pending: None,
            failure: None,
        };

        let marker = visitor
            .inspect_cell(
                CellRef {
                    id: CellId::from_index(0),
                    bounds,
                    depth: 0,
                    parent: None,
                },
                false,
            )
            .expect("forced-uniform inspection");

        assert_eq!(
            marker.decision,
            RefinementDecision::Refine(RefinementReason::ForcedUniform)
        );
        assert!(marker.analysis.is_none());
    }

    #[test]
    fn max_depth_decision_is_authoritative_for_partial_hermite_payloads() {
        let bounds = Aabb::new([-1.0; 3], [1.0; 3]).expect("bounds");
        let params = params(bounds, 0);
        let no_hits = SliceSensitivePlane {
            retain_some_edge_hits: false,
        };
        let inactive = inspect_root(&no_hits, &params, true);
        assert_eq!(
            inactive.decision,
            RefinementDecision::Inactive(InactiveReason::NoCrossingAtMaxDepth)
        );
        assert!(
            inactive.analysis.is_some(),
            "the regression requires an analyzed but inactive payload"
        );
        assert_eq!(
            dual_contour(&no_hits, &params)
                .expect("inconsistent no-hit extraction")
                .stats
                .active_cells,
            0,
            "inactive analysis must not leak through collection"
        );

        let some_hits = SliceSensitivePlane {
            retain_some_edge_hits: true,
        };
        let compatible = inspect_root(&some_hits, &params, true);
        assert_eq!(
            compatible.decision,
            RefinementDecision::MaxDepthCompatibility
        );
        let evidence = compatible.analysis.expect("partial analysis").evidence;
        assert!(evidence.hermite_hits > 0);
        assert!(evidence.hermite_hits < evidence.expected_crossings);
        assert_eq!(
            dual_contour(&some_hits, &params)
                .expect("inconsistent partial extraction")
                .stats
                .active_cells,
            1,
            "a partial cell with some Hermite data preserves compatibility at max depth"
        );
    }

    #[test]
    fn exact_plane_has_zero_curvature_and_is_retained_within_budget() {
        let bounds = Aabb::new([-1.4, -1.2, -1.1], [1.6, 1.3, 1.4]).expect("bounds");
        let params = params(bounds, 6);
        let plane = HalfSpaceField {
            point: [0.1375, -0.08125, 0.10625],
            normal: [1.0, 2.0, -1.0],
        };
        let inspected = bounds
            .split()
            .into_iter()
            .flat_map(|child| child.split())
            .map(|candidate| inspect_candidate(&plane, &params, candidate, 2, false))
            .find(|inspection| inspection.decision == RefinementDecision::Retain)
            .expect("at least one depth-two plane cell is retained");
        let evidence = inspected
            .analysis
            .expect("plane has active evidence")
            .evidence;

        assert_eq!(inspected.decision, RefinementDecision::Retain);
        assert!(
            evidence.qef_rms <= adaptive_error_target(&params),
            "{evidence:?}"
        );
        assert!(evidence.curvature_error <= 1.0e-6, "{evidence:?}");
    }

    #[test]
    fn exact_plane_retention_still_provides_an_emitter_neighborhood() {
        let bounds = Aabb::new([-1.4, -1.2, -1.1], [1.6, 1.3, 1.4]).expect("bounds");
        let params = params(bounds, 6);
        let plane = HalfSpaceField {
            point: [0.1375, -0.08125, 0.10625],
            normal: [1.0, 2.0, -1.0],
        };

        let result = dual_contour(&plane, &params).expect("plane extraction");

        assert!(result.stats.active_cells >= 3);
        assert!(result.stats.vertices >= 3);
        assert!(result.stats.faces > 0);
        let (triangles, _) = result.mesh.to_trimesh(&ExtractParams::default());
        assert!(!triangles.indices.is_empty());
    }

    #[test]
    fn curved_sphere_candidate_refines_above_the_finest_scale_budget() {
        let bounds = Aabb::new([-1.0; 3], [1.0; 3]).expect("bounds");
        let params = params(bounds, 6);
        let sphere = SphereField {
            center: [0.0; 3],
            radius: 0.73,
        };
        let inspected = bounds
            .split()
            .into_iter()
            .flat_map(|child| child.split())
            .map(|candidate| inspect_candidate(&sphere, &params, candidate, 2, false))
            .find(|inspection| {
                matches!(
                    inspection.decision,
                    RefinementDecision::Refine(RefinementReason::Residual)
                        | RefinementDecision::Refine(RefinementReason::Curvature)
                )
            })
            .expect("at least one depth-two sphere cell exceeds the error budget");
        let evidence = inspected
            .analysis
            .as_ref()
            .expect("sphere octant has active evidence")
            .evidence;

        assert!(matches!(
            inspected.decision,
            RefinementDecision::Refine(RefinementReason::Residual)
                | RefinementDecision::Refine(RefinementReason::Curvature)
        ));
        assert!(
            evidence.qef_rms > adaptive_error_target(&params)
                || evidence.curvature_error > adaptive_error_target(&params),
            "{evidence:?}"
        );
    }

    #[test]
    fn off_lattice_box_reduces_elements_before_mesh_construction() {
        let bounds = Aabb::new([-1.4, -1.2, -1.1], [1.6, 1.3, 1.4]).expect("bounds");
        let params = params(bounds, 6);
        let field = BoxField {
            center: [0.1375, -0.08125, 0.10625],
            half_extents: [0.81, 0.59, 0.43],
        };
        let legacy = contour_with_mode(&field, &params, RefinementMode::Legacy);
        let adaptive = contour_with_mode(&field, &params, RefinementMode::ErrorDriven);
        let repeated = contour_with_mode(&field, &params, RefinementMode::ErrorDriven);

        assert!(adaptive.stats.active_cells < legacy.stats.active_cells);
        assert!(adaptive.stats.vertices < legacy.stats.vertices);
        assert!(adaptive.stats.faces < legacy.stats.faces);
        assert_eq!(adaptive.stats, repeated.stats);
        let adaptive_tri = adaptive.mesh.to_trimesh(&ExtractParams::default());
        let repeated_tri = repeated.mesh.to_trimesh(&ExtractParams::default());
        assert_eq!(adaptive_tri.0.positions, repeated_tri.0.positions);
        assert_eq!(adaptive_tri.0.indices, repeated_tri.0.indices);
        assert_closed_transition_mesh(&adaptive.mesh);
        assert!(
            adaptive
                .mesh
                .boundary_loops()
                .expect("adaptive box boundary traversal")
                .is_empty()
        );
    }

    #[test]
    fn sparse_corner_sampling_stays_below_the_finest_lattice() {
        let field = BoxField {
            center: [0.1375, -0.08125, 0.10625],
            half_extents: [0.81, 0.59, 0.43],
        };
        let extraction_params = params(
            Aabb::new([-1.4, -1.2, -1.1], [1.6, 1.3, 1.4]).expect("bounds"),
            6,
        );
        let resolution = 1_u32 << extraction_params.max_depth;
        let mut visitor = IntervalVisitor {
            field: &field,
            params: &extraction_params,
            refinement_mode: RefinementMode::ErrorDriven,
            grid: AdaptiveGrid::new(&field, extraction_params.root_bounds, resolution),
            pending: None,
            failure: None,
        };
        let mut tree = Octree::build(
            extraction_params.root_bounds,
            extraction_params.max_depth,
            &mut visitor,
        );
        let (leaves, segments) =
            prepare_transitions(&mut tree, &mut visitor).expect("transition preparation");
        let active = collect_active_cells(&extraction_params, &tree, &visitor.grid).cells;
        let sparse_samples = visitor.grid.sample_count();
        let finest_lattice = usize::try_from(resolution + 1)
            .expect("resolution fits usize")
            .pow(3);

        assert_eq!(
            (
                tree.len(),
                leaves.len(),
                active.len(),
                active
                    .iter()
                    .map(|cell| cell.components.len())
                    .sum::<usize>(),
                segments.len(),
                sparse_samples,
                finest_lattice,
            ),
            (1_361, 1_191, 246, 246, 5_403, 2_182, 274_625)
        );
        assert!(sparse_samples * 5 < finest_lattice);
        assert_eq!(leaves.len(), tree.leaf_ids().len());
    }

    #[test]
    fn sparse_transition_output_stays_closed_and_deterministic_after_transform() {
        let base_bounds = Aabb::new([-1.4, -1.2, -1.1], [1.6, 1.3, 1.4]).expect("bounds");
        let base_field = BoxField {
            center: [0.1375, -0.08125, 0.10625],
            half_extents: [0.81, 0.59, 0.43],
        };
        let scale = 3.0_f32;
        let translation = [13.0_f32, -7.0, 5.0];
        let transform = |point: [f32; 3]| {
            [
                point[0] * scale + translation[0],
                point[1] * scale + translation[1],
                point[2] * scale + translation[2],
            ]
        };
        let transformed_bounds =
            Aabb::new(transform(base_bounds.min), transform(base_bounds.max)).expect("bounds");
        let transformed_field = BoxField {
            center: transform(base_field.center),
            half_extents: base_field.half_extents.map(|extent| extent * scale),
        };

        let base = dual_contour(&base_field, &params(base_bounds, 6)).expect("base extraction");
        let transformed = dual_contour(&transformed_field, &params(transformed_bounds, 6))
            .expect("transformed extraction");
        let transformed_repeat = dual_contour(&transformed_field, &params(transformed_bounds, 6))
            .expect("repeated transformed extraction");
        assert_closed_transition_mesh(&base.mesh);
        assert_closed_transition_mesh(&transformed.mesh);
        assert_closed_transition_mesh(&transformed_repeat.mesh);
        assert_eq!(base.stats, transformed.stats);
        assert_eq!(transformed.stats, transformed_repeat.stats);

        let transformed_triangles = transformed.mesh.to_trimesh(&ExtractParams::default()).0;
        let repeated_triangles = transformed_repeat
            .mesh
            .to_trimesh(&ExtractParams::default())
            .0;
        assert_eq!(transformed_triangles.indices, repeated_triangles.indices);
        assert_eq!(
            transformed_triangles.positions,
            repeated_triangles.positions
        );
    }

    #[test]
    fn unknown_intervals_preserve_closed_deterministic_conservative_output() {
        let bounds = Aabb::new([-1.4, -1.2, -1.1], [1.6, 1.3, 1.4]).expect("bounds");
        let params = params(bounds, 4);
        let box_field = BoxField {
            center: [0.1375, -0.08125, 0.10625],
            half_extents: [0.81, 0.59, 0.43],
        };
        let unknown = UnknownIntervalBox { field: box_field };

        let bounded = dual_contour(&box_field, &params).expect("bounded interval extraction");
        let conservative = dual_contour(&unknown, &params).expect("unknown interval extraction");
        let repeated =
            dual_contour(&unknown, &params).expect("repeated unknown interval extraction");

        assert!(conservative.stats.octree_cells > bounded.stats.octree_cells);
        assert!(conservative.stats.active_cells >= bounded.stats.active_cells);
        assert!(conservative.stats.faces >= bounded.stats.faces);
        assert_eq!(conservative.stats, repeated.stats);
        assert_closed_transition_mesh(&bounded.mesh);
        assert_closed_transition_mesh(&conservative.mesh);
        assert_closed_transition_mesh(&repeated.mesh);
        let conservative_triangles = conservative.mesh.to_trimesh(&ExtractParams::default()).0;
        let repeated_triangles = repeated.mesh.to_trimesh(&ExtractParams::default()).0;
        assert_eq!(
            conservative_triangles.positions,
            repeated_triangles.positions
        );
        assert_eq!(conservative_triangles.indices, repeated_triangles.indices);
    }

    #[test]
    fn scaled_boxes_make_the_same_refinement_decisions() {
        let unit_depths = active_depths_for_box(1.0);
        assert_eq!(active_depths_for_box(1.0e-3), unit_depths);
        assert_eq!(active_depths_for_box(1.0e3), unit_depths);
        assert_eq!(
            active_depths_for_box_transform(1.0, [128.0, -64.0, 32.0]),
            unit_depths
        );

        let redundant = redundant_retention_depths_for_box_transform(1.0, [0.0; 3]);
        assert!(!redundant.is_empty());
        assert_eq!(
            redundant_retention_depths_for_box_transform(1.0e-3, [0.0; 3]),
            redundant
        );
        assert_eq!(
            redundant_retention_depths_for_box_transform(1.0e3, [0.0; 3]),
            redundant
        );
        assert_eq!(
            redundant_retention_depths_for_box_transform(1.0, [128.0, -64.0, 32.0]),
            redundant
        );
    }

    #[test]
    fn curved_gradient_adversaries_never_earn_redundant_plane_retention() {
        let sphere = SphereField {
            center: [0.137, -0.083, 0.109],
            radius: 0.731,
        };
        let bounds = Aabb::new([-1.4, -1.2, -1.1], [1.6, 1.3, 1.4]).expect("bounds");
        let extraction_params = params(bounds, 6);
        assert!(
            redundant_retention_depths(&sphere, &extraction_params).is_empty(),
            "the smooth sphere must bypass the exact-plane witness"
        );
        assert!(
            redundant_retention_depths(
                &QuantizedGradientSphere { field: sphere },
                &extraction_params
            )
            .is_empty(),
            "quantized curved gradients must not mimic redundant planes"
        );
        assert!(
            redundant_retention_depths(
                &ConstantGradientSphere { field: sphere },
                &extraction_params
            )
            .is_empty(),
            "constant gradients over curved offsets must fail coplanarity"
        );
    }

    #[test]
    fn pending_evidence_is_consumed_by_the_exact_leaf() {
        let field = SphereField {
            center: [0.1, -0.05, 0.025],
            radius: 0.73,
        };
        let params = params(
            Aabb::new([-1.4, -1.2, -1.1], [1.6, 1.3, 1.4]).expect("bounds"),
            5,
        );
        let resolution = 1_u32 << params.max_depth;
        let mut visitor = IntervalVisitor {
            field: &field,
            params: &params,
            refinement_mode: RefinementMode::ErrorDriven,
            grid: AdaptiveGrid::new(&field, params.root_bounds, resolution),
            pending: None,
            failure: None,
        };

        let _tree = Octree::build(params.root_bounds, params.max_depth, &mut visitor);

        assert!(visitor.pending.is_none());
        assert_eq!(visitor.failure, None);
    }

    fn inspect_root<F: ScalarField>(
        field: &F,
        params: &DualContourParams,
        at_max_depth: bool,
    ) -> LeafMarker {
        inspect_candidate(field, params, params.root_bounds, 0, at_max_depth)
    }

    fn inspect_candidate<F: ScalarField>(
        field: &F,
        params: &DualContourParams,
        bounds: Aabb,
        depth: u8,
        at_max_depth: bool,
    ) -> LeafMarker {
        let resolution = 1_u32 << params.max_depth;
        let mut visitor = IntervalVisitor {
            field,
            params,
            refinement_mode: RefinementMode::ErrorDriven,
            grid: AdaptiveGrid::new(field, params.root_bounds, resolution),
            pending: None,
            failure: None,
        };
        visitor
            .inspect_cell(
                CellRef {
                    id: CellId::from_index(0),
                    bounds,
                    depth,
                    parent: None,
                },
                at_max_depth,
            )
            .expect("root inspection")
    }

    fn contour_with_mode<F: ScalarField>(
        field: &F,
        params: &DualContourParams,
        mode: RefinementMode,
    ) -> super::DualContourResult {
        dual_contour_projected_impl(field, params, |_, _, _| 0, |_, _| None, mode)
            .expect("test extraction")
            .0
    }

    fn active_depths_for_box(scale: f32) -> Vec<u8> {
        active_depths_for_box_transform(scale, [0.0; 3])
    }

    fn active_depths_for_box_transform(scale: f32, translation: [f32; 3]) -> Vec<u8> {
        let (field, params) = box_refinement_fixture(scale, translation);
        let resolution = 1_u32 << params.max_depth;
        let mut visitor = IntervalVisitor {
            field: &field,
            params: &params,
            refinement_mode: RefinementMode::ErrorDriven,
            grid: AdaptiveGrid::new(&field, params.root_bounds, resolution),
            pending: None,
            failure: None,
        };
        let tree = Octree::build(params.root_bounds, params.max_depth, &mut visitor);
        collect_active_cells(&params, &tree, &visitor.grid)
            .cells
            .into_iter()
            .map(|cell| cell.key.depth)
            .collect()
    }

    fn redundant_retention_depths_for_box_transform(scale: f32, translation: [f32; 3]) -> Vec<u8> {
        let (field, params) = box_refinement_fixture(scale, translation);
        redundant_retention_depths(&field, &params)
    }

    fn box_refinement_fixture(scale: f32, translation: [f32; 3]) -> (BoxField, DualContourParams) {
        let bounds = Aabb::new(
            [
                -1.4 * scale + translation[0],
                -1.2 * scale + translation[1],
                -1.1 * scale + translation[2],
            ],
            [
                1.6 * scale + translation[0],
                1.3 * scale + translation[1],
                1.4 * scale + translation[2],
            ],
        )
        .expect("scaled bounds");
        let field = BoxField {
            center: [
                0.1375 * scale + translation[0],
                -0.08125 * scale + translation[1],
                0.10625 * scale + translation[2],
            ],
            half_extents: [0.81 * scale, 0.59 * scale, 0.43 * scale],
        };
        (field, params(bounds, 5))
    }

    fn redundant_retention_depths<F: ScalarField>(
        field: &F,
        params: &DualContourParams,
    ) -> Vec<u8> {
        let resolution = 1_u32 << params.max_depth;
        let mut visitor = IntervalVisitor {
            field,
            params,
            refinement_mode: RefinementMode::ErrorDriven,
            grid: AdaptiveGrid::new(field, params.root_bounds, resolution),
            pending: None,
            failure: None,
        };
        let tree = Octree::build(params.root_bounds, params.max_depth, &mut visitor);
        assert_eq!(visitor.failure, None);
        tree.leaf_ids()
            .into_iter()
            .filter_map(|id| {
                let cell = tree.cell(id)?;
                (cell.payload()?.decision == RefinementDecision::RetainRedundantHermitePlanes)
                    .then_some(cell.depth)
            })
            .collect()
    }

    fn redundant_plane_fixture(
        scale: f32,
        translation: [f32; 3],
    ) -> (CellHermiteData, QefResult, [f32; 3]) {
        let transform =
            |point: [f32; 3]| core::array::from_fn(|axis| translation[axis] + scale * point[axis]);
        let mut hermite = CellHermiteData::new(0b1000_0001);
        for (edge, position, normal) in [
            (0, [0.25, 0.0, 0.0], [1.0, -0.0, 0.0]),
            (1, [0.25, 1.0, 0.0], [1.0, 0.0, -0.0]),
            (2, [0.0, 0.25, 0.0], [-0.0, 1.0, 0.0]),
            (3, [1.0, 0.25, 0.0], [0.0, 1.0, -0.0]),
            (8, [0.0, 0.0, 0.25], [-0.0, 0.0, 1.0]),
            (9, [1.0, 0.0, 0.25], [0.0, -0.0, 1.0]),
        ] {
            push_plane_hit(&mut hermite, edge, transform(position), normal);
        }
        let qef = QefResult {
            position: transform([0.25; 3]),
            residual_error: 0.0,
            rank: 3,
            sharpness_class: exedra_qef::SharpnessClass::Corner,
            eigenvalues: [1.0; 3],
            was_clamped: false,
        };
        (hermite, qef, transform([0.5; 3]))
    }

    fn separated_component_hermite() -> CellHermiteData {
        let mut hermite = CellHermiteData::new(0b1000_0001);
        push_plane_hit(&mut hermite, 0, [0.25, 0.0, 0.0], [1.0, 0.0, 0.0]);
        push_plane_hit(&mut hermite, 3, [0.0, 0.25, 0.0], [0.0, 1.0, 0.0]);
        push_plane_hit(&mut hermite, 8, [0.0, 0.0, 0.25], [0.0, 0.0, 1.0]);
        push_plane_hit(&mut hermite, 5, [0.75, 0.0, 0.0], [1.0, 0.0, 0.0]);
        push_plane_hit(&mut hermite, 6, [0.0, 0.75, 0.0], [0.0, 1.0, 0.0]);
        push_plane_hit(&mut hermite, 10, [0.0, 0.0, 0.75], [0.0, 0.0, 1.0]);
        hermite
    }

    fn push_plane_hit(
        hermite: &mut CellHermiteData,
        edge: u8,
        position: [f32; 3],
        normal: [f32; 3],
    ) {
        hermite.push(
            edge,
            HermiteIntersection {
                position,
                normal,
                t: 0.5,
            },
        );
    }

    fn values_for_mask(mask: u8) -> [f32; 8] {
        core::array::from_fn(|corner| {
            if mask & (1_u8 << corner) != 0 {
                -1.0
            } else {
                1.0
            }
        })
    }

    #[test]
    fn multiscale_active_cells_span_multiple_depths() {
        let field = Union::new(
            SphereField {
                center: [0.013, -0.017, 0.011],
                radius: 1.0,
            },
            SphereField {
                center: [1.213, -0.017, 0.011],
                radius: 0.18,
            },
        );
        let params = params(
            Aabb::new([-1.6, -1.5, -1.5], [1.6, 1.5, 1.5]).expect("bounds"),
            6,
        );
        let resolution = 1_u32 << params.max_depth;
        let mut visitor = IntervalVisitor {
            field: &field,
            params: &params,
            refinement_mode: RefinementMode::ErrorDriven,
            grid: AdaptiveGrid::new(&field, params.root_bounds, resolution),
            pending: None,
            failure: None,
        };
        let tree = Octree::build(params.root_bounds, params.max_depth, &mut visitor);
        let active = collect_active_cells(&params, &tree, &visitor.grid).cells;

        assert!(active.iter().any(|cell| cell.key.depth < params.max_depth));
        assert!(active.iter().any(|cell| cell.key.depth == params.max_depth));

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

        assert_eq!(select_quad_diagonal(skewed), Some(QuadDiagonal::OneThree));
    }

    #[test]
    fn select_quad_diagonal_avoids_the_degenerate_split() {
        let trap = [
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.6, 0.0],
        ];

        assert_eq!(select_quad_diagonal(trap), Some(QuadDiagonal::OneThree));
        assert_eq!(
            select_quad_diagonal([trap[1], trap[2], trap[3], trap[0]]),
            Some(QuadDiagonal::ZeroTwo),
            "a cyclic shift must preserve the physical diagonal"
        );
        assert_eq!(
            select_quad_diagonal([trap[3], trap[2], trap[1], trap[0]]),
            Some(QuadDiagonal::ZeroTwo),
            "reversing winding must preserve the physical diagonal"
        );
    }

    #[test]
    fn transition_polygon_reports_when_neither_quad_split_is_valid() {
        let positions = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [3.0, 0.0, 0.0],
        ];
        assert_eq!(select_quad_diagonal(positions), None);

        let mut builder = exedra::MeshBuilder::new();
        let mut face = Vec::new();
        for position in positions {
            face.push(VertexEntry {
                builder_index: builder.push_vertex(position),
                position,
                sharpness: 0.0,
            });
        }
        let mut face_count = 0;
        assert!(matches!(
            emit_transition_polygon(&mut builder, &face, 0, &mut face_count),
            Err(super::DualContourError::Build(
                BuildError::DegenerateTriangle { triangle: 0 }
            ))
        ));
    }

    #[test]
    fn aligned_coincident_box_union_has_no_degenerate_transition_faces() {
        let scale = 2.75_f32;
        let translation = [2.3_f32, -1.1, 0.7];
        let transform = |local: [f32; 3]| {
            [
                translation[0] + scale * local[0],
                translation[1] + scale * local[1],
                translation[2] + scale * local[2],
            ]
        };
        let field = Union::new(
            BoxField {
                center: transform([-0.35, -0.15, 0.0]),
                half_extents: [0.9 * scale, 0.7 * scale, 0.55 * scale],
            },
            BoxField {
                center: transform([0.4, 0.3, 0.1]),
                half_extents: [0.65 * scale, 0.8 * scale, 0.45 * scale],
            },
        );
        let extraction_params = params(
            Aabb::new(transform([-1.6, -1.5, -1.3]), transform([1.6, 1.5, 1.3]))
                .expect("trap bounds"),
            6,
        );

        for mode in [RefinementMode::ForcedUniform, RefinementMode::ErrorDriven] {
            let result = contour_with_mode(&field, &extraction_params, mode);
            assert_eq!(degenerate_face_count(&result.mesh), 0, "mode={mode:?}");
            assert_closed_transition_mesh(&result.mesh);
        }
    }

    fn tagged_box_minus_cylinder(
        axis: [f32; 3],
    ) -> Difference<TaggedField<BoxField, u32>, TaggedField<CylinderField, u32>> {
        Difference::new(
            TaggedField {
                field: BoxField {
                    center: [0.0; 3],
                    half_extents: [1.0; 3],
                },
                provenance: 10,
            },
            TaggedField {
                field: CylinderField {
                    center: [0.0; 3],
                    axis,
                    radius: 0.6,
                    half_height: 2.0,
                },
                provenance: 20,
            },
        )
    }

    fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }

    fn length3(vector: [f32; 3]) -> f32 {
        (dot3(vector, vector)).sqrt()
    }

    fn mesh_geometry(mesh: &exedra::Mesh) -> (Vec<[f32; 3]>, Vec<Vec<u32>>) {
        let positions = mesh
            .vertices()
            .map(|vertex| *mesh.vertex_position(vertex).expect("vertex position"))
            .collect();
        let faces = mesh
            .faces()
            .map(|face| {
                mesh.face_loop(face)
                    .map(|corner| mesh.to_vertex(corner).expect("face vertex").index())
                    .collect()
            })
            .collect();
        (positions, faces)
    }

    fn test_sin(value: f32) -> f32 {
        #[cfg(feature = "std")]
        {
            value.sin()
        }
        #[cfg(all(not(feature = "std"), feature = "libm"))]
        {
            libm::sinf(value)
        }
    }

    fn test_cos(value: f32) -> f32 {
        #[cfg(feature = "std")]
        {
            value.cos()
        }
        #[cfg(all(not(feature = "std"), feature = "libm"))]
        {
            libm::cosf(value)
        }
    }

    type ForcedPinField = Union<TaggedField<BoxField, u32>, TaggedField<BoxField, u32>>;

    struct ForcedPinMeasured<F> {
        field: F,
        interval_cells: core::cell::RefCell<Vec<Aabb>>,
        projection_cells: core::cell::RefCell<Vec<Aabb>>,
    }

    impl<F> ForcedPinMeasured<F> {
        fn new(field: F) -> Self {
            Self {
                field,
                interval_cells: core::cell::RefCell::new(Vec::new()),
                projection_cells: core::cell::RefCell::new(Vec::new()),
            }
        }
    }

    impl<F: ScalarField> ScalarField for ForcedPinMeasured<F> {
        fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]> {
            self.interval_cells.borrow_mut().push(*bounds);
            self.field.eval_interval(bounds)
        }

        fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
            self.field.eval_points(points, out);
        }

        fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
            self.field.eval_gradients(points, out);
        }
    }

    impl<F: SemiAnalyticField> SemiAnalyticField for ForcedPinMeasured<F> {
        fn project_cell_vertex(
            &self,
            point: [f32; 3],
            cell: &Aabb,
        ) -> Option<SemiAnalyticProjection> {
            self.field.project_cell_vertex(point, cell)
        }

        fn project_cell_vertex_detailed(
            &self,
            point: [f32; 3],
            cell: &Aabb,
        ) -> SemiAnalyticProjectionOutcome {
            self.projection_cells.borrow_mut().push(*cell);
            self.field.project_cell_vertex_detailed(point, cell)
        }

        fn primitive_at(&self, point: [f32; 3]) -> u32 {
            self.field.primitive_at(point)
        }

        fn leaf_primitive(&self) -> Option<AnalyticPrimitive> {
            self.field.leaf_primitive()
        }
    }

    #[derive(Copy, Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
    struct ForcedPinCell {
        origin: [u32; 3],
        span: u32,
        depth: u8,
    }

    fn forced_pin_h1_fixture() -> (ForcedPinField, DualContourParams) {
        let scale = 2.75_f32;
        let translation = [2.3_f32, -1.1, 0.7];
        let transform = |local: [f32; 3]| {
            [
                translation[0] + scale * local[0],
                translation[1] + scale * local[1],
                translation[2] + scale * local[2],
            ]
        };
        let field = Union::new(
            TaggedField {
                field: BoxField {
                    center: transform([-0.347, -0.153, 0.017]),
                    half_extents: [0.893 * scale, 0.697 * scale, 0.541 * scale],
                },
                provenance: 10,
            },
            TaggedField {
                field: BoxField {
                    center: transform([0.407, 0.293, 0.123]),
                    half_extents: [0.647 * scale, 0.787 * scale, 0.397 * scale],
                },
                provenance: 20,
            },
        );
        let root_bounds = Aabb::new(transform([-1.6, -1.5, -1.3]), transform([1.6, 1.5, 1.3]))
            .expect("forced-uniform H1 root bounds");
        (field, params(root_bounds, 7))
    }

    fn forced_pin_extract(
        measured: &ForcedPinMeasured<ForcedPinField>,
        extraction_params: &DualContourParams,
    ) -> (super::DualContourResult, SemiAnalyticContourStats) {
        dual_contour_projected_impl(
            measured,
            extraction_params,
            |start, end, fallback| {
                let point = crate::hermite::locate_edge_zero(
                    measured,
                    start,
                    end,
                    &extraction_params.edge_search,
                )
                .map_or(fallback, |(point, _)| point);
                measured.primitive_at(point)
            },
            |point, cell| Some(measured.project_cell_vertex_detailed(point, cell)),
            RefinementMode::ForcedUniform,
        )
        .expect("forced-uniform H1 extraction")
    }

    fn forced_pin_histograms(
        measured: &ForcedPinMeasured<ForcedPinField>,
        root: Aabb,
        max_depth: u8,
    ) -> (
        Vec<usize>,
        Vec<usize>,
        alloc::collections::BTreeSet<ForcedPinCell>,
    ) {
        let cells = measured
            .interval_cells
            .borrow()
            .iter()
            .copied()
            .map(|bounds| forced_pin_cell(bounds, root, max_depth))
            .collect::<alloc::collections::BTreeSet<_>>();
        assert_eq!(
            cells.len(),
            measured.interval_cells.borrow().len(),
            "the traversal must interval-test each stored cell once"
        );
        let mut final_depths = vec![0; usize::from(max_depth) + 1];
        for cell in &cells {
            if cell.depth == max_depth
                || !forced_pin_children(*cell)
                    .iter()
                    .any(|child| cells.contains(child))
            {
                final_depths[usize::from(cell.depth)] += 1;
            }
        }

        let projection_cells = measured
            .projection_cells
            .borrow()
            .iter()
            .copied()
            .map(|bounds| forced_pin_cell(bounds, root, max_depth))
            .collect::<Vec<_>>();
        let projection_cell_set = projection_cells
            .iter()
            .copied()
            .collect::<alloc::collections::BTreeSet<_>>();
        assert_eq!(
            projection_cell_set.len(),
            projection_cells.len(),
            "each contributing leaf must be projected exactly once"
        );
        let mut contributing_depths = vec![0; usize::from(max_depth) + 1];
        for cell in projection_cells {
            contributing_depths[usize::from(cell.depth)] += 1;
        }
        (final_depths, contributing_depths, projection_cell_set)
    }

    fn forced_pin_cell(bounds: Aabb, root: Aabb, max_depth: u8) -> ForcedPinCell {
        let resolution = 1_u32 << max_depth;
        let origin: [u32; 3] = core::array::from_fn(|axis| {
            forced_pin_coordinate(root, resolution, axis, bounds.min[axis])
        });
        let maximum: [u32; 3] = core::array::from_fn(|axis| {
            forced_pin_coordinate(root, resolution, axis, bounds.max[axis])
        });
        let spans = core::array::from_fn::<_, 3, _>(|axis| maximum[axis] - origin[axis]);
        assert_eq!(
            spans, [spans[0]; 3],
            "octree cell must be cubic in key space"
        );
        assert!(spans[0].is_power_of_two(), "octree span must be dyadic");
        let span = spans[0];
        ForcedPinCell {
            origin,
            span,
            depth: max_depth - u8::try_from(span.ilog2()).expect("depth fits u8"),
        }
    }

    fn forced_pin_coordinate(root: Aabb, resolution: u32, axis: usize, target: f32) -> u32 {
        let mut low = 0;
        let mut high = resolution;
        while low <= high {
            let middle = low + (high - low) / 2;
            match forced_pin_axis_point(root, resolution, axis, middle).total_cmp(&target) {
                core::cmp::Ordering::Less => low = middle + 1,
                core::cmp::Ordering::Greater => {
                    assert!(middle > 0, "target must lie inside root bounds");
                    high = middle - 1;
                }
                core::cmp::Ordering::Equal => return middle,
            }
        }
        panic!("cell endpoint {target:?} is not an exact integer-grid coordinate")
    }

    fn forced_pin_axis_point(root: Aabb, resolution: u32, axis: usize, key: u32) -> f32 {
        if key == 0 {
            root.min[axis]
        } else if key == resolution {
            root.max[axis]
        } else {
            let step = root.extent()[axis] / resolution as f32;
            root.min[axis] + step * key as f32
        }
    }

    fn forced_pin_assert_coordinate_round_trip(root: Aabb, resolution: u32) {
        for axis in 0..3 {
            for key in 0..=resolution {
                let point = forced_pin_axis_point(root, resolution, axis, key);
                assert_eq!(
                    forced_pin_coordinate(root, resolution, axis, point),
                    key,
                    "axis {axis} key {key} must round-trip exactly"
                );
            }
        }
    }

    fn forced_pin_children(cell: ForcedPinCell) -> [ForcedPinCell; 8] {
        let child_span = cell.span / 2;
        core::array::from_fn(|corner| ForcedPinCell {
            origin: core::array::from_fn(|axis| {
                cell.origin[axis] + child_span * u32::from(((corner >> axis) & 1) != 0)
            }),
            span: child_span,
            depth: cell.depth + 1,
        })
    }

    fn forced_pin_assert_common_subset(
        field: &ForcedPinField,
        extraction_params: &DualContourParams,
        projected_cells: &alloc::collections::BTreeSet<ForcedPinCell>,
    ) {
        use super::{
            component_is_usable as forced_pin_component_is_usable,
            qef_result_is_finite as forced_pin_qef_result_is_finite,
        };

        let resolution = 1_u32 << extraction_params.max_depth;
        let mut grid = AdaptiveGrid::new(field, extraction_params.root_bounds, resolution);
        let mut crossing_cells = alloc::collections::BTreeSet::new();
        for x in 0..resolution {
            for y in 0..resolution {
                for z in 0..resolution {
                    let key = CellKey {
                        origin: crate::adaptive_transition::CornerKey::new(x, y, z),
                        span: 1,
                        depth: extraction_params.max_depth,
                    };
                    let corner_values = grid.sample_cell_corners(key);
                    if super::crossing_edge_count(&corner_values) == 0 {
                        continue;
                    }
                    crossing_cells.insert(ForcedPinCell {
                        origin: [x, y, z],
                        span: 1,
                        depth: extraction_params.max_depth,
                    });
                }
            }
        }
        assert_eq!(
            &crossing_cells, projected_cells,
            "the actual projected leaf set must exactly equal the independently scanned crossing set"
        );

        for cell in projected_cells {
            let key = CellKey {
                origin: crate::adaptive_transition::CornerKey::new(
                    cell.origin[0],
                    cell.origin[1],
                    cell.origin[2],
                ),
                span: cell.span,
                depth: cell.depth,
            };
            let corner_values = grid.sample_cell_corners(key);
            let analysis = analyze_crossing_cell(
                field,
                extraction_params,
                corner_values,
                grid.cell_bounds(key),
            )
            .expect("forced-uniform H1 cell analysis");
            assert_eq!(analysis.vertices.components.len(), 1);
            assert!(!analysis.topology.has_ambiguous_face());
            assert_eq!(
                analysis.evidence.expected_crossings,
                analysis.evidence.hermite_hits
            );
            assert!(analysis.evidence.complete_hermite);
            assert!(analysis.evidence.finite);
            assert!(analysis.corner_values.iter().all(|value| *value != 0.0));
            assert_eq!(
                super::hermite_edge_mask(&analysis.hermite),
                super::crossing_edge_mask(&analysis.corner_values)
            );
            let component = analysis.vertices.components[0];
            assert!(forced_pin_component_is_usable(&component));
            assert!(component.qef.is_some_and(forced_pin_qef_result_is_finite));
            assert_eq!(component, analysis.vertices.compatibility);
        }
    }

    fn forced_pin_regions(mesh: &exedra::Mesh) -> Vec<(u32, usize)> {
        use alloc::collections::BTreeMap as ForcedPinBTreeMap;

        let regions = mesh
            .attrs()
            .dense(attr::FACE_REGION)
            .expect("forced-uniform FACE_REGION");
        let mut histogram = ForcedPinBTreeMap::new();
        for face in mesh.faces() {
            let region = regions
                .get(face.as_id())
                .copied()
                .expect("every face has a region");
            *histogram.entry(region).or_insert(0) += 1;
        }
        histogram.into_iter().collect()
    }

    #[test]
    fn forced_uniform_h1_semantic_contract_is_fully_pinned() {
        let (field, extraction_params) = forced_pin_h1_fixture();
        let measured = ForcedPinMeasured::new(field);
        let (result, semi_analytic) = forced_pin_extract(&measured, &extraction_params);
        let (final_depths, contributing_depths, projected_cells) = forced_pin_histograms(
            &measured,
            extraction_params.root_bounds,
            extraction_params.max_depth,
        );

        assert_eq!(
            result.stats,
            super::DualContourStats {
                octree_cells: 100_937,
                active_cells: 30_122,
                vertices: 30_122,
                faces: 60_240,
            }
        );
        assert_eq!(
            semi_analytic,
            SemiAnalyticContourStats {
                unsupported_fallbacks: 30_122,
                ..SemiAnalyticContourStats::default()
            }
        );
        assert_eq!(
            semi_analytic_counter_total(semi_analytic),
            result.stats.active_cells
        );
        assert_eq!(final_depths, [0, 0, 0, 199, 1_388, 5_277, 21_744, 59_712]);
        assert_eq!(contributing_depths, [0, 0, 0, 0, 0, 0, 0, 30_122]);
        assert_eq!(
            forced_pin_regions(&result.mesh),
            [(10, 38_884), (20, 21_356)]
        );
        assert_eq!(projected_cells.len(), result.stats.active_cells);
        forced_pin_assert_coordinate_round_trip(
            extraction_params.root_bounds,
            1_u32 << extraction_params.max_depth,
        );
        forced_pin_assert_common_subset(&measured.field, &extraction_params, &projected_cells);
        assert_closed_transition_mesh(&result.mesh);
    }
}
