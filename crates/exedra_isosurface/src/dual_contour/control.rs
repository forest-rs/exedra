// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Extraction policy, evidence, and per-run accounting.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};
use core::fmt;

use exedra_mesh::{BuildError, op::CollapseEdgeError};
use exedra_qef::QefSolveError;
use exedra_spatial::RefineError;

use super::{DualContourParameter, DualContourStats};
use crate::{Aabb, ScalarField};

/// Hard limits on extractor-controlled storage and work.
///
/// `None` leaves a resource unrestricted. Limits are checked before the
/// corresponding allocation, callback batch, pass, or mesh operation. Exceeding
/// one returns an error, never an ordinary partial mesh. The separate
/// [`super::DualContourParams::cell_budget`] intentionally truncates output.
///
/// Storage limits count entries, not allocator bytes or process RSS. Temporary
/// structures are bounded by these counts; payloads and arbitrary work inside
/// caller-supplied field methods are outside this contract. Set all relevant
/// limits when a run needs bounded storage as well as bounded evaluation work.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct ExtractionLimits {
    /// Stored octree cells, including balancing and completion refinements.
    pub octree_cells: Option<usize>,
    /// Cached integer-grid corner samples.
    pub cached_corners: Option<usize>,
    /// Entries in any transition worklist, including intervals, endpoints,
    /// minimal segments and pending output patches. This is a per-list limit.
    pub transition_records: Option<usize>,
    /// Vertices in any generated mesh, including the intermediate join mesh.
    pub vertices: Option<usize>,
    /// Faces in any generated mesh. The triangle count is checked before
    /// creating the intermediate polygon mesh as well.
    pub faces: Option<usize>,
    /// Sum of interval queries, scalar/gradient rows, provenance queries and
    /// projection requests made to the supplied field through public methods.
    pub field_evaluations: Option<usize>,
    /// Balancing and transition-completion passes, including their final
    /// no-refinement passes.
    pub topology_passes: Option<usize>,
    /// Attempted checked vertex joins.
    pub vertex_joins: Option<usize>,
}

/// A resource whose limit stopped extraction.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ExtractionResource {
    /// Stored octree cells.
    OctreeCells,
    /// Cached grid samples.
    CachedCorners,
    /// Entries in a transition worklist.
    TransitionRecords,
    /// Vertices in a generated mesh.
    Vertices,
    /// Faces in a generated mesh.
    Faces,
    /// Logical field evaluations through caller methods.
    FieldEvaluations,
    /// Balancing and completion passes.
    TopologyPasses,
    /// Checked join attempts.
    VertexJoins,
}

/// Work performed and peak logical storage before success or failure.
///
/// Evaluations count rows actually passed to the field, including repeated
/// samples and invalid returned values. They do not inspect field internals.
/// A refused batch is excluded. Storage peaks include intermediate geometry.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct ExtractionWork {
    /// Spatial interval requests.
    pub interval_queries: usize,
    /// Scalar point rows.
    pub point_evaluations: usize,
    /// Value/gradient rows.
    pub gradient_evaluations: usize,
    /// Point provenance or primitive-identity requests.
    pub provenance_queries: usize,
    /// Semi-analytic projection requests.
    pub projection_queries: usize,
    /// Balancing and completion passes begun.
    pub topology_passes: usize,
    /// Checked join attempts begun.
    pub vertex_joins: usize,
    /// Greatest number of stored octree cells.
    pub peak_octree_cells: usize,
    /// Greatest number of cached grid sample slots reserved.
    pub peak_cached_corners: usize,
    /// Greatest number of entries reserved for a transition worklist.
    pub peak_transition_records: usize,
    /// Greatest number of vertex slots reserved for any generated mesh.
    pub peak_vertices: usize,
    /// Greatest number of faces reserved for any generated mesh.
    pub peak_faces: usize,
}

/// Stage where extraction stopped or produced a witness.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub enum ExtractionStage {
    /// Input validation, before any field evaluation.
    #[default]
    Validation,
    /// Initial interval and error-driven subdivision.
    Subdivision,
    /// Balancing neighboring leaf depths.
    Balancing,
    /// Resolving component routes around primal edges.
    TransitionCompletion,
    /// Optional projection of component vertices.
    Projection,
    /// Gathering and emitting transition patches.
    Emission,
    /// Building mesh connectivity.
    MeshBuild,
    /// Joining coincident component representatives.
    Joining,
    /// Triangulating the joined polygon mesh.
    Triangulation,
    /// Authoring corner normals.
    Normals,
    /// Marking region seams.
    RegionSeams,
}

/// A cell in this extraction's root-relative integer grid.
///
/// Coordinates are scoped to the caller-supplied root domain and depth policy. They
/// are not persistent feature identities or octree arena indices.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ExtractionCell {
    /// Minimum integer-grid corner.
    pub origin: [u32; 3],
    /// Cell width in finest-grid steps.
    pub span: u32,
    /// Octree depth.
    pub depth: u8,
    /// Bounds in field coordinates.
    pub bounds: Aabb,
}

/// One routed component's proposed output vertex.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ExtractionCandidate {
    /// Cell that supplied this component.
    pub cell: ExtractionCell,
    /// Cell-local component ordinal, or `None` for a compatibility representative.
    pub component: Option<u8>,
    /// Proposed position before joining.
    pub position: [f32; 3],
}

/// World and grid evidence for a failed transition.
#[derive(Clone, Debug, PartialEq)]
pub struct TransitionWitness {
    /// Primal edge's start in field coordinates.
    pub start: [f32; 3],
    /// Primal edge's end in field coordinates.
    pub end: [f32; 3],
    /// Axis of the primal edge (`0` = x, `1` = y, `2` = z).
    pub axis: u8,
    /// Starting integer-grid corner.
    pub grid_start: [u32; 3],
    /// Length in finest-grid steps.
    pub span: u32,
    /// Incident cell/component candidates in cyclic routing order. Missing
    /// candidates indicate an unresolved route, rather than an invented vertex.
    pub candidates: [Option<ExtractionCandidate>; 4],
    /// Source sampled at the generating zero crossing, when the entry point
    /// supplies provenance. This is sampled attribution, not a complete CSG trace.
    pub sampled_source: Option<u32>,
}

/// Spatial evidence available without inspecting extractor internals.
#[derive(Clone, Debug, PartialEq)]
#[expect(
    clippy::large_enum_variant,
    reason = "witnesses already live in the boxed failure context; a second allocation is unnecessary"
)]
pub enum ExtractionWitness {
    /// A cell being analyzed or allocated.
    Cell(ExtractionCell),
    /// A routed transition and its proposed vertices.
    Transition(TransitionWitness),
    /// A field request that could not begin.
    Query {
        /// Bounds containing the requested points, or interval query domain.
        bounds: Aabb,
    },
    /// An invalid scalar result at a particular query point.
    Sample {
        /// Query point in field coordinates.
        point: [f32; 3],
        /// Scalar value returned by the field.
        value: f32,
    },
    /// Invalid interval response over a spatial query.
    Interval {
        /// Query bounds in field coordinates.
        bounds: Aabb,
        /// Returned lower and upper values.
        values: [f32; 2],
    },
    /// Intermediate geometry involved in a joining or triangulation failure.
    MeshPatch {
        /// At most four candidate positions in loop order.
        positions: Vec<[f32; 3]>,
        /// Source region retained by the intermediate face, when present.
        source: Option<u32>,
    },
}

/// Whether output was intentionally omitted by the contributing-cell budget.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub enum ExtractionCompletion {
    /// Every eligible interior patch from this finite extraction was included.
    /// This does not certify closedness, topology, error, domain enclosure, or
    /// the discovery of features smaller than the sampling grid.
    #[default]
    Complete,
    /// One or more contributing cells were deliberately omitted. The mesh may
    /// be empty or open, even when the source field describes a closed object.
    TruncatedByCellBudget,
}

/// A reason to inspect a reported cell.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ExtractionCellIssue {
    /// The cell contributed geometry but the output budget omitted it.
    OmittedByCellBudget,
    /// Its interval allowed zero, but finest-grid corners had no crossing.
    /// This may be loose interval evidence or an unresolved small feature.
    NoCrossingAtMaxDepth,
    /// Corner signs crossed, but finest-cell intersection searches yielded no
    /// usable Hermite samples.
    NoHermiteAtMaxDepth,
    /// The finest cell used compatibility evidence without earning ordinary
    /// error-driven retention.
    MaxDepthCompatibility,
}

/// Bounded spatial detail for a successful but potentially unresolved run.
#[derive(Clone, Debug, PartialEq)]
pub struct ExtractionCellWitness {
    /// Cell location.
    pub cell: ExtractionCell,
    /// Reason this cell was reported.
    pub issue: ExtractionCellIssue,
}

/// Exact completion counts with caller-bounded spatial detail.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExtractionReport {
    /// Whether the contributing-cell budget omitted eligible output.
    pub completion: ExtractionCompletion,
    /// Contributing cells before applying the output budget.
    pub eligible_cells: usize,
    /// Contributing cells omitted by that budget.
    pub omitted_cells: usize,
    /// Interior crossing patches omitted because at least one incident cell
    /// was excluded by the output budget.
    pub omitted_patches: usize,
    /// Crossing primal-edge segments on the root boundary, where the extractor
    /// has no complete incident neighborhood and emits no cap.
    pub boundary_crossings: usize,
    /// Finest cells with no resolved crossing: no corner-sign crossing or no
    /// usable Hermite samples, despite interval evidence permitting a boundary.
    pub unresolved_cells: usize,
    /// Finest cells retained using compatibility evidence.
    pub compatibility_cells: usize,
    /// Bounding box of omitted contributing cells, or `None` when none were omitted.
    pub omitted_bounds: Option<Aabb>,
    /// Deterministic spatial witnesses, capped by the caller. Counts above
    /// remain exact when this list is truncated.
    pub witnesses: Vec<ExtractionCellWitness>,
    /// Witness records not retained because the detail cap was reached.
    pub unreported_witnesses: usize,
}

/// Machine-readable extraction failure category.
#[derive(Clone, Debug, PartialEq)]
pub enum DualContourErrorKind {
    /// Invalid octree refinement request.
    Spatial(RefineError),
    /// Invalid caller policy.
    InvalidParameter(DualContourParameter),
    /// Generated connectivity or triangulation failed.
    Build(BuildError),
    /// QEF solving failed.
    Solve(QefSolveError),
    /// Joining was refused by mesh topology checks.
    Collapse(CollapseEdgeError),
    /// A join involved a disconnected vertex link.
    DisconnectedVertexLink {
        /// Index in the intermediate mesh; use the spatial witness to locate it.
        vertex: u32,
    },
    /// A hard resource limit would be exceeded.
    LimitExceeded {
        /// Resource being reserved.
        resource: ExtractionResource,
        /// Current count, or current worklist length for a storage reservation.
        used: usize,
        /// Requested total count after the operation.
        requested: usize,
        /// Effective limit. Integer-capacity failures use the representable maximum.
        limit: usize,
    },
    /// A point or value/gradient callback returned a non-finite scalar value.
    /// Undefined gradient components are allowed by the field contract.
    NonFiniteScalar,
    /// An interval contained NaN or had its endpoints reversed. Infinite
    /// outward bounds remain valid conservative evidence.
    InvalidInterval,
}

/// Evidence preserved when extraction stops.
#[derive(Clone, Debug, PartialEq)]
pub struct ExtractionFailureContext {
    /// Stage at failure.
    pub stage: ExtractionStage,
    /// Accumulated work and peak storage, excluding a refused operation.
    pub work: ExtractionWork,
    /// Geometry progress before failure; no partial mesh is returned.
    pub stats: DualContourStats,
    /// Spatial evidence when the failed operation has a geometric location.
    pub witness: Option<ExtractionWitness>,
}

/// Extraction failure with typed cause, work evidence and spatial witnesses.
#[derive(Clone, Debug, PartialEq)]
pub struct DualContourError {
    /// Machine-readable failure category.
    pub kind: DualContourErrorKind,
    /// Detailed failure evidence, boxed to keep ordinary result values small.
    pub context: Box<ExtractionFailureContext>,
}

impl DualContourError {
    pub(crate) fn new(kind: DualContourErrorKind) -> Self {
        Self {
            kind,
            context: Box::new(ExtractionFailureContext {
                stage: ExtractionStage::Validation,
                work: ExtractionWork::default(),
                stats: DualContourStats::default(),
                witness: None,
            }),
        }
    }

    pub(crate) fn with_witness(mut self, witness: ExtractionWitness) -> Self {
        if self.context.witness.is_none() {
            self.context.witness = Some(witness);
        }
        self
    }

    pub(crate) fn build(error: BuildError) -> Self {
        Self::new(DualContourErrorKind::Build(error))
    }
    pub(crate) fn solve(error: QefSolveError) -> Self {
        Self::new(DualContourErrorKind::Solve(error))
    }
    pub(crate) fn collapse(error: CollapseEdgeError) -> Self {
        Self::new(DualContourErrorKind::Collapse(error))
    }
    pub(crate) fn invalid(parameter: DualContourParameter) -> Self {
        Self::new(DualContourErrorKind::InvalidParameter(parameter))
    }
    pub(crate) fn disconnected(vertex: u32) -> Self {
        Self::new(DualContourErrorKind::DisconnectedVertexLink { vertex })
    }
}

impl fmt::Display for DualContourError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "dual contour extraction failed during {:?}: {:?}",
            self.context.stage, self.kind
        )?;
        if let Some(witness) = &self.context.witness {
            write!(f, " at {witness:?}")?;
        }
        Ok(())
    }
}

impl core::error::Error for DualContourError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match &self.kind {
            DualContourErrorKind::Spatial(error) => Some(error),
            DualContourErrorKind::Build(error) => Some(error),
            DualContourErrorKind::Solve(error) => Some(error),
            DualContourErrorKind::Collapse(error) => Some(error),
            _ => None,
        }
    }
}

pub(crate) struct RunContext {
    pub(crate) limits: ExtractionLimits,
    pub(crate) stage: Cell<ExtractionStage>,
    pub(crate) work: Cell<ExtractionWork>,
    pub(crate) stats: Cell<DualContourStats>,
    failure: RefCell<Option<DualContourError>>,
}

impl RunContext {
    pub(crate) fn new(limits: ExtractionLimits) -> Self {
        Self {
            limits,
            stage: Cell::new(ExtractionStage::Validation),
            work: Cell::default(),
            stats: Cell::default(),
            failure: RefCell::new(None),
        }
    }

    pub(crate) fn decorate(&self, mut error: DualContourError) -> DualContourError {
        error.context.stage = self.stage.get();
        error.context.work = self.work.get();
        error.context.stats = self.stats.get();
        error
    }

    pub(crate) fn check(&self) -> Result<(), DualContourError> {
        self.failure
            .borrow()
            .as_ref()
            .map_or(Ok(()), |error| Err(error.clone()))
    }

    pub(crate) fn fail(&self, error: DualContourError) {
        self.failure
            .borrow_mut()
            .get_or_insert_with(|| self.decorate(error));
    }

    pub(crate) fn reserve(
        &self,
        resource: ExtractionResource,
        used: usize,
        requested: usize,
    ) -> Result<(), DualContourError> {
        self.check()?;
        let limit = match resource {
            ExtractionResource::OctreeCells => self.limits.octree_cells,
            ExtractionResource::CachedCorners => self.limits.cached_corners,
            ExtractionResource::TransitionRecords => self.limits.transition_records,
            ExtractionResource::Vertices => self.limits.vertices,
            ExtractionResource::Faces => self.limits.faces,
            ExtractionResource::FieldEvaluations => self.limits.field_evaluations,
            ExtractionResource::TopologyPasses => self.limits.topology_passes,
            ExtractionResource::VertexJoins => self.limits.vertex_joins,
        }
        .unwrap_or(usize::MAX);
        if requested > limit {
            return Err(self.decorate(DualContourError::new(
                DualContourErrorKind::LimitExceeded {
                    resource,
                    used,
                    requested,
                    limit,
                },
            )));
        }
        let mut work = self.work.get();
        let peak = match resource {
            ExtractionResource::OctreeCells => &mut work.peak_octree_cells,
            ExtractionResource::CachedCorners => &mut work.peak_cached_corners,
            ExtractionResource::TransitionRecords => &mut work.peak_transition_records,
            ExtractionResource::Vertices => &mut work.peak_vertices,
            ExtractionResource::Faces => &mut work.peak_faces,
            ExtractionResource::TopologyPasses => &mut work.topology_passes,
            ExtractionResource::VertexJoins => &mut work.vertex_joins,
            ExtractionResource::FieldEvaluations => return Ok(()),
        };
        *peak = (*peak).max(requested);
        self.work.set(work);
        Ok(())
    }

    pub(crate) fn record_tree(&self, count: usize) -> Result<(), DualContourError> {
        self.reserve(ExtractionResource::OctreeCells, 0, count)?;
        let mut stats = self.stats.get();
        stats.octree_cells = count;
        self.stats.set(stats);
        Ok(())
    }

    pub(crate) fn grow(
        &self,
        resource: ExtractionResource,
        used: usize,
        additional: usize,
    ) -> Result<usize, DualContourError> {
        let requested = used.checked_add(additional).ok_or_else(|| {
            self.decorate(DualContourError::new(DualContourErrorKind::LimitExceeded {
                resource,
                used,
                requested: usize::MAX,
                limit: usize::MAX,
            }))
        })?;
        self.reserve(resource, used, requested)?;
        Ok(requested)
    }

    pub(crate) fn records(&self, count: usize, width: usize) -> Result<usize, DualContourError> {
        let requested = count.checked_mul(width).ok_or_else(|| {
            self.decorate(DualContourError::new(DualContourErrorKind::LimitExceeded {
                resource: ExtractionResource::TransitionRecords,
                used: 0,
                requested: usize::MAX,
                limit: usize::MAX,
            }))
        })?;
        self.reserve(ExtractionResource::TransitionRecords, 0, requested)?;
        Ok(requested)
    }

    pub(crate) fn evaluate(&self, kind: Evaluation, count: usize) -> Result<(), DualContourError> {
        let mut work = self.work.get();
        let used = work.interval_queries
            + work.point_evaluations
            + work.gradient_evaluations
            + work.provenance_queries
            + work.projection_queries;
        let requested = used.checked_add(count).ok_or_else(|| {
            self.decorate(DualContourError::new(DualContourErrorKind::LimitExceeded {
                resource: ExtractionResource::FieldEvaluations,
                used,
                requested: usize::MAX,
                limit: self.limits.field_evaluations.unwrap_or(usize::MAX),
            }))
        })?;
        self.reserve(ExtractionResource::FieldEvaluations, used, requested)?;
        match kind {
            Evaluation::Interval => work.interval_queries += count,
            Evaluation::Point => work.point_evaluations += count,
            Evaluation::Gradient => work.gradient_evaluations += count,
            Evaluation::Provenance => work.provenance_queries += count,
            Evaluation::Projection => work.projection_queries += count,
        }
        self.work.set(work);
        Ok(())
    }
}

#[derive(Copy, Clone)]
pub(crate) enum Evaluation {
    Interval,
    Point,
    Gradient,
    Provenance,
    Projection,
}

pub(crate) struct CheckedField<'a, F> {
    pub(crate) field: &'a F,
    pub(crate) run: &'a RunContext,
}

impl<F: ScalarField> ScalarField for CheckedField<'_, F> {
    fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]> {
        if let Err(error) = self.run.evaluate(Evaluation::Interval, 1) {
            self.run
                .fail(error.with_witness(ExtractionWitness::Query { bounds: *bounds }));
            return None;
        }
        let result = self.field.eval_interval(bounds);
        if let Some(values) = result
            && (values[0].is_nan() || values[1].is_nan() || values[0] > values[1])
        {
            self.run.fail(
                DualContourError::new(DualContourErrorKind::InvalidInterval).with_witness(
                    ExtractionWitness::Interval {
                        bounds: *bounds,
                        values,
                    },
                ),
            );
        }
        result
    }

    fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
        out.fill(f32::NAN);
        if points.is_empty() {
            return;
        }
        if let Err(error) = self.run.evaluate(Evaluation::Point, points.len()) {
            self.run.fail(error.with_witness(ExtractionWitness::Query {
                bounds: query_bounds(points.iter().copied()),
            }));
            return;
        }
        self.field.eval_points(points, out);
        if let Some((&point, &value)) = points
            .iter()
            .zip(out.iter())
            .find(|(_, value)| !value.is_finite())
        {
            self.run.fail(
                DualContourError::new(DualContourErrorKind::NonFiniteScalar)
                    .with_witness(ExtractionWitness::Sample { point, value }),
            );
        }
    }

    fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
        out.fill([f32::NAN; 4]);
        if points.is_empty() {
            return;
        }
        if let Err(error) = self.run.evaluate(Evaluation::Gradient, points.len()) {
            self.run.fail(error.with_witness(ExtractionWitness::Query {
                bounds: query_bounds(points.iter().copied()),
            }));
            return;
        }
        self.field.eval_gradients(points, out);
        if let Some((&point, row)) = points
            .iter()
            .zip(out.iter())
            .find(|(_, row)| !row[0].is_finite())
        {
            self.run.fail(
                DualContourError::new(DualContourErrorKind::NonFiniteScalar).with_witness(
                    ExtractionWitness::Sample {
                        point,
                        value: row[0],
                    },
                ),
            );
        }
    }
}

pub(crate) fn query_bounds(mut points: impl Iterator<Item = [f32; 3]>) -> Aabb {
    let first = points
        .next()
        .expect("query bounds require a nonempty batch");
    let mut bounds = Aabb {
        min: first,
        max: first,
    };
    for point in points {
        for (axis, value) in point.iter().enumerate() {
            bounds.min[axis] = bounds.min[axis].min(*value);
            bounds.max[axis] = bounds.max[axis].max(*value);
        }
    }
    bounds
}

#[cfg(test)]
mod tests;
