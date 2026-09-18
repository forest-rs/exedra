// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use crate::analytic::SphereField;
use crate::{
    DualContourParams, EdgeSearchParams, ProvenanceField, QefParams, SemiAnalyticField,
    SemiAnalyticProjection, dual_contour, dual_contour_semi_analytic, dual_contour_with_regions,
};
use exedra_mesh::ExtractParams;

struct Measured {
    field: SphereField,
    work: Cell<ExtractionWork>,
}

impl Measured {
    fn new() -> Self {
        Self {
            field: SphereField {
                center: [0.13, -0.07, 0.11],
                radius: 0.73,
            },
            work: Cell::default(),
        }
    }

    fn assert_calls(&self, work: ExtractionWork) {
        let actual = self.work.get();
        assert_eq!(actual.interval_queries, work.interval_queries);
        assert_eq!(actual.point_evaluations, work.point_evaluations);
        assert_eq!(actual.gradient_evaluations, work.gradient_evaluations);
        assert_eq!(actual.provenance_queries, work.provenance_queries);
        assert_eq!(actual.projection_queries, work.projection_queries);
    }
}

impl ScalarField for Measured {
    fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]> {
        let mut work = self.work.get();
        work.interval_queries += 1;
        self.work.set(work);
        self.field.eval_interval(bounds)
    }
    fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
        let mut work = self.work.get();
        work.point_evaluations += points.len();
        self.work.set(work);
        self.field.eval_points(points, out);
    }
    fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
        let mut work = self.work.get();
        work.gradient_evaluations += points.len();
        self.work.set(work);
        self.field.eval_gradients(points, out);
    }
}

impl ProvenanceField for Measured {
    type Provenance = u32;
    fn eval_interval_with_provenance(&self, bounds: &Aabb) -> Option<([f32; 2], u32)> {
        self.eval_interval(bounds).map(|values| (values, 42))
    }
    fn point_provenance(&self, _: [f32; 3]) -> u32 {
        let mut work = self.work.get();
        work.provenance_queries += 1;
        self.work.set(work);
        42
    }
}
impl SemiAnalyticField for Measured {
    fn primitive_at(&self, point: [f32; 3]) -> u32 {
        self.point_provenance(point)
    }
    fn project_cell_vertex(&self, _: [f32; 3], _: &Aabb) -> Option<SemiAnalyticProjection> {
        let mut work = self.work.get();
        work.projection_queries += 1;
        self.work.set(work);
        None
    }
}

fn params() -> DualContourParams {
    DualContourParams {
        root_bounds: Aabb::new([-1.4; 3], [1.4; 3]).unwrap(),
        max_depth: 3,
        cell_budget: None,
        vertex_merge_tolerance: 0.0,
        edge_search: EdgeSearchParams::default(),
        qef: QefParams::default(),
        limits: ExtractionLimits::default(),
        witness_limit: 4,
    }
}
fn evaluations(work: ExtractionWork) -> usize {
    work.interval_queries
        + work.point_evaluations
        + work.gradient_evaluations
        + work.provenance_queries
        + work.projection_queries
}
fn assert_limit(error: &DualContourError, expected: ExtractionResource) {
    let DualContourErrorKind::LimitExceeded {
        resource,
        used,
        requested,
        limit,
    } = error.kind
    else {
        panic!("{error}")
    };
    assert_eq!(resource, expected);
    assert!(used <= limit);
    assert!(requested > limit);
}

#[test]
fn finite_limits_preserve_output_and_exact_callback_counts() {
    let baseline = Measured::new();
    let a = dual_contour_with_regions(&baseline, &params()).unwrap();
    baseline.assert_calls(a.work);
    let w = a.work;
    let mut p = params();
    p.limits = ExtractionLimits {
        octree_cells: Some(w.peak_octree_cells),
        cached_corners: Some(w.peak_cached_corners),
        transition_records: Some(w.peak_transition_records),
        vertices: Some(w.peak_vertices),
        faces: Some(w.peak_faces),
        field_evaluations: Some(evaluations(w)),
        topology_passes: Some(w.topology_passes),
        vertex_joins: Some(w.vertex_joins),
    };
    let bounded = Measured::new();
    let b = dual_contour_with_regions(&bounded, &p).unwrap();
    bounded.assert_calls(b.work);
    assert_eq!(a.work, b.work);
    assert_eq!(a.stats, b.stats);
    assert_eq!(a.report, b.report);
    let a = a.mesh.to_trimesh(&ExtractParams::default()).0;
    let b = b.mesh.to_trimesh(&ExtractParams::default()).0;
    assert_eq!(a.positions, b.positions);
    assert_eq!(a.indices, b.indices);
    assert_eq!(a.normals, b.normals);
}

#[test]
fn hard_limits_stop_before_work_and_preserve_failure_evidence() {
    for (resource, limits) in [
        (
            ExtractionResource::OctreeCells,
            ExtractionLimits {
                octree_cells: Some(0),
                ..Default::default()
            },
        ),
        (
            ExtractionResource::OctreeCells,
            ExtractionLimits {
                octree_cells: Some(8),
                ..Default::default()
            },
        ),
        (
            ExtractionResource::CachedCorners,
            ExtractionLimits {
                cached_corners: Some(0),
                ..Default::default()
            },
        ),
        (
            ExtractionResource::TransitionRecords,
            ExtractionLimits {
                transition_records: Some(0),
                ..Default::default()
            },
        ),
        (
            ExtractionResource::TopologyPasses,
            ExtractionLimits {
                topology_passes: Some(0),
                ..Default::default()
            },
        ),
        (
            ExtractionResource::Vertices,
            ExtractionLimits {
                vertices: Some(0),
                ..Default::default()
            },
        ),
        (
            ExtractionResource::Faces,
            ExtractionLimits {
                faces: Some(0),
                ..Default::default()
            },
        ),
        (
            ExtractionResource::FieldEvaluations,
            ExtractionLimits {
                field_evaluations: Some(0),
                ..Default::default()
            },
        ),
        (
            ExtractionResource::FieldEvaluations,
            ExtractionLimits {
                field_evaluations: Some(2),
                ..Default::default()
            },
        ),
    ] {
        let field = Measured::new();
        let mut p = params();
        p.limits = limits;
        let error = dual_contour(&field, &p).unwrap_err();
        assert_limit(&error, resource);
        field.assert_calls(error.context.work);
        assert!(error.context.witness.is_some(), "{resource:?}: {error}");
        if resource == ExtractionResource::Faces {
            let Some(ExtractionWitness::Transition(witness)) = error.context.witness.as_ref()
            else {
                panic!("{error}")
            };
            assert!(witness.candidates.iter().all(Option::is_some));
            assert_ne!(witness.start, witness.end);
            assert_eq!(witness.sampled_source, None);
        }
    }
}

#[test]
fn transition_corner_limit_preserves_refused_query_bounds() {
    let field = Measured::new();
    let mut p = params();
    p.limits.cached_corners = Some(228);
    let error = dual_contour(&field, &p).unwrap_err();
    assert_limit(&error, ExtractionResource::CachedCorners);
    assert_eq!(error.context.stage, ExtractionStage::TransitionCompletion);
    field.assert_calls(error.context.work);
    let Some(ExtractionWitness::Query { bounds }) = error.context.witness else {
        panic!("missing refused transition query: {error:?}")
    };
    assert!(p.root_bounds.contains(bounds.min));
    assert!(p.root_bounds.contains(bounds.max));
    assert_ne!(bounds.min, bounds.max);
}

#[test]
fn budgets_apply_to_projection_provenance_and_final_normals() {
    let baseline = Measured::new();
    let result = dual_contour_semi_analytic(&baseline, &params()).unwrap();
    baseline.assert_calls(result.work);
    assert!(result.work.projection_queries > 0 && result.work.provenance_queries > 0);
    let total = evaluations(result.work);
    // Sweep the whole run's callback budget at coarse steps and right before
    // completion. Counters must match even when the refused batch is partial.
    let mut stages = Vec::new();
    for limit in (0..total).step_by((total / 50).max(1)).chain([total - 1]) {
        let field = Measured::new();
        let mut p = params();
        p.limits.field_evaluations = Some(limit);
        let error = dual_contour_semi_analytic(&field, &p).unwrap_err();
        assert_limit(&error, ExtractionResource::FieldEvaluations);
        field.assert_calls(error.context.work);
        assert!(evaluations(error.context.work) <= limit);
        assert!(error.context.witness.is_some(), "{error}");
        stages.push(error.context.stage);
    }
    assert!(stages.contains(&ExtractionStage::Projection));
    assert!(stages.contains(&ExtractionStage::Emission));
    assert!(stages.contains(&ExtractionStage::Normals));
}

#[test]
fn zero_and_partial_output_budgets_report_exact_omissions() {
    let field = Measured::new();
    let full = dual_contour(&field, &params()).unwrap();
    assert_eq!(full.report.completion, ExtractionCompletion::Complete);
    assert_eq!(full.report.omitted_cells, 0);
    assert_eq!(full.report.omitted_bounds, None);
    for budget in [0, 32] {
        let mut p = params();
        p.cell_budget = Some(budget);
        p.witness_limit = 0;
        let a = dual_contour(&field, &p).unwrap();
        assert_eq!(
            a.report.completion,
            ExtractionCompletion::TruncatedByCellBudget
        );
        assert_eq!(a.report.eligible_cells, full.report.eligible_cells);
        assert_eq!(a.report.omitted_cells, full.report.eligible_cells - budget);
        assert!(a.report.omitted_bounds.is_some());
        assert!(a.report.omitted_patches > 0);
        assert!(a.report.witnesses.is_empty());
        assert_eq!(
            a.report.unreported_witnesses,
            a.report.omitted_cells + a.report.unresolved_cells + a.report.compatibility_cells
        );
        p.witness_limit = 3;
        let b = dual_contour(&field, &p).unwrap();
        assert_eq!(b.report.witnesses.len(), 3);
        assert_eq!(
            b.report.unreported_witnesses + 3,
            a.report.unreported_witnesses
        );
        assert_eq!(a.stats, b.stats);
        assert_eq!(a.work, b.work);
        if budget == 0 {
            assert_eq!(a.stats.vertices, 0);
            assert!(a.work.interval_queries > 0);
        }
    }
}

struct InvalidField {
    interval: Option<[f32; 2]>,
    scalar: f32,
}
impl ScalarField for InvalidField {
    fn eval_interval(&self, _: &Aabb) -> Option<[f32; 2]> {
        self.interval
    }
    fn eval_points(&self, _: &[[f32; 3]], out: &mut [f32]) {
        out.fill(self.scalar);
    }
    fn eval_gradients(&self, _: &[[f32; 3]], out: &mut [[f32; 4]]) {
        out.fill([self.scalar, f32::NAN, f32::NAN, f32::NAN]);
    }
}

#[test]
fn invalid_field_values_are_failures_and_unknown_intervals_stay_unknown() {
    for interval in [Some([f32::NAN, 1.0]), Some([1.0, -1.0])] {
        let error = dual_contour(
            &InvalidField {
                interval,
                scalar: 1.0,
            },
            &params(),
        )
        .unwrap_err();
        assert_eq!(error.kind, DualContourErrorKind::InvalidInterval);
        assert!(matches!(
            error.context.witness,
            Some(ExtractionWitness::Interval { .. })
        ));
    }
    for scalar in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let error = dual_contour(
            &InvalidField {
                interval: None,
                scalar,
            },
            &params(),
        )
        .unwrap_err();
        assert_eq!(error.kind, DualContourErrorKind::NonFiniteScalar);
        assert!(matches!(
            error.context.witness,
            Some(ExtractionWitness::Sample { .. })
        ));
    }
    for interval in [None, Some([f32::NEG_INFINITY, f32::INFINITY])] {
        let mut p = params();
        p.max_depth = 1;
        let result = dual_contour(
            &InvalidField {
                interval,
                scalar: 1.0,
            },
            &p,
        )
        .unwrap();
        assert_eq!(result.report.completion, ExtractionCompletion::Complete);
        assert_eq!(result.report.eligible_cells, 0);
        assert_eq!(result.report.unresolved_cells, 8);
    }
}

#[test]
fn joining_budget_is_checked_before_the_first_kernel_edit() {
    let field = SphereField {
        center: [0.0; 3],
        radius: 0.02,
    };
    let mut p = params();
    p.root_bounds = Aabb::new([-0.032; 3], [0.032; 3]).unwrap();
    p.max_depth = 5;
    p.edge_search.bisection_steps = 14;
    let full = dual_contour(&field, &p).unwrap();
    assert!(full.work.vertex_joins > 0);
    for limit in [0, full.work.vertex_joins - 1] {
        p.limits.vertex_joins = Some(limit);
        let error = dual_contour(&field, &p).unwrap_err();
        assert_limit(&error, ExtractionResource::VertexJoins);
        assert_eq!(error.context.stage, ExtractionStage::Joining);
        assert_eq!(error.context.work.vertex_joins, limit);
        assert_eq!(error.context.stats.coincident_edge_collapses, limit);
        assert!(matches!(
            error.context.witness,
            Some(ExtractionWitness::MeshPatch { source: None, .. })
        ));
    }
    p.limits.vertex_joins = Some(full.work.vertex_joins);
    let bounded = dual_contour(&field, &p).unwrap();
    assert_eq!(bounded.stats, full.stats);
}

#[test]
fn completion_does_not_hide_domain_boundary_crossings() {
    struct Plane;
    impl ScalarField for Plane {
        fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]> {
            Some([bounds.min[0] - 0.1, bounds.max[0] - 0.1])
        }
        fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
            for (point, value) in points.iter().zip(out) {
                *value = point[0] - 0.1;
            }
        }
        fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
            for (point, row) in points.iter().zip(out) {
                *row = [point[0] - 0.1, 1.0, 0.0, 0.0];
            }
        }
    }
    let full = dual_contour(&Plane, &params()).unwrap();
    assert_eq!(full.report.completion, ExtractionCompletion::Complete);
    assert!(full.report.boundary_crossings > 0);
    assert!(full.stats.faces > 0);
    let mut p = params();
    p.cell_budget = Some(0);
    let omitted = dual_contour(&Plane, &p).unwrap();
    assert_eq!(
        omitted.report.boundary_crossings,
        full.report.boundary_crossings
    );
    assert!(omitted.report.omitted_patches > 0);
}

#[test]
fn gradient_scalar_is_checked_but_undefined_directions_are_allowed() {
    let field = InvalidField {
        interval: None,
        scalar: 1.0,
    };
    let run = RunContext::new(ExtractionLimits::default());
    let checked = CheckedField {
        field: &field,
        run: &run,
    };
    let mut rows = [[0.0; 4]; 1];
    checked.eval_gradients(&[[0.0; 3]], &mut rows);
    assert!(run.check().is_ok());
    assert!(rows[0][1].is_nan());
    let field = InvalidField {
        interval: None,
        scalar: f32::NAN,
    };
    let checked = CheckedField {
        field: &field,
        run: &run,
    };
    checked.eval_gradients(&[[1.0; 3]], &mut rows);
    let error = run.check().unwrap_err();
    assert_eq!(error.kind, DualContourErrorKind::NonFiniteScalar);
    assert!(matches!(
        error.context.witness,
        Some(ExtractionWitness::Sample {
            point: [1.0, 1.0, 1.0],
            ..
        })
    ));
}

#[test]
fn analysis_limits_cover_later_balancing_and_completion() {
    use crate::adaptive_transition::AdaptiveGrid;
    use crate::analytic::BoxField;
    use crate::dual_contour::{IntervalVisitor, RefinementMode};
    use exedra_spatial::Octree;

    let field = BoxField {
        center: [0.1375, -0.08125, 0.10625],
        half_extents: [0.81, 0.59, 0.43],
    };
    let mut p = params();
    p.max_depth = 5;
    let run = RunContext::new(ExtractionLimits::default());
    let mut visitor = IntervalVisitor {
        field: &field,
        params: &p,
        refinement_mode: RefinementMode::ErrorDriven,
        grid: AdaptiveGrid::new(&field, p.root_bounds, 1 << p.max_depth),
        pending: None,
        run: &run,
    };
    let initial = Octree::build(p.root_bounds, p.max_depth, None, &mut visitor).unwrap();
    let full = dual_contour(&field, &p).unwrap();
    assert!(
        full.stats.octree_cells > initial.len(),
        "fixture must require later refinement"
    );
    p.limits.octree_cells = Some(initial.len());
    let error = dual_contour(&field, &p).unwrap_err();
    assert_limit(&error, ExtractionResource::OctreeCells);
    assert_eq!(error.context.stage, ExtractionStage::Balancing);
    assert_eq!(error.context.stats.octree_cells, initial.len());
    assert_eq!(error.context.work.peak_octree_cells, initial.len());
    assert!(matches!(
        error.context.witness,
        Some(ExtractionWitness::Cell(_))
    ));

    p.limits = ExtractionLimits {
        topology_passes: Some(full.work.topology_passes - 1),
        ..ExtractionLimits::default()
    };
    let error = dual_contour(&field, &p).unwrap_err();
    assert_limit(&error, ExtractionResource::TopologyPasses);
    assert_eq!(error.context.stage, ExtractionStage::TransitionCompletion);
    assert_eq!(error.context.stats.octree_cells, full.stats.octree_cells);
    assert!(error.context.work.peak_cached_corners > 0);
}
