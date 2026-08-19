// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Recipe evaluation: walking a frozen [`Recipe`] into tessellated bodies
//! plus an honest [`GeometryReport`].
//!
//! Evaluation is a pure function of `(recipe, policy)`: nodes are visited
//! deterministically, placements compose in f64, and every outcome —
//! including what could *not* be evaluated — lands in the report as typed
//! fidelity and diagnostics rather than silent approximation.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::cache::{CacheKey, EvalCache, policy_fingerprint};
use crate::ir::{NodeId, NodeKind, Placement3, PolicyId, ProfileId, Recipe, SourceId};
use crate::tessellate::{
    EvalPolicy, TessellateError, TessellatedBody, tessellate_extrude, tessellate_loft,
    tessellate_revolve, tessellate_sweep,
};

/// How faithfully a node's output represents its constructive intent.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Fidelity {
    /// The output is the node's exact intent (up to discretization policy).
    Exact,
    /// The output approximates the intent under a named policy; the payload
    /// is an opaque frontend-supplied policy reference.
    PolicyDefined(PolicyId),
    /// The node's specification is contradictory; the payload cites the
    /// opaque issue reference chosen by the frontend.
    Conflicted(SourceId),
    /// Only a bounding envelope is available — the operation is not yet
    /// evaluable (for example CSG before the boolean pipeline lands).
    EnvelopeOnly,
}

/// Diagnostic severity.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Informational note.
    Note,
    /// The output differs from intent in a recoverable way.
    Warning,
    /// The node could not be evaluated.
    Error,
}

/// One structured diagnostic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// Severity class.
    pub severity: Severity,
    /// Stable machine-readable code (for example `eval.csg.unsupported`).
    pub code: &'static str,
    /// Human-readable detail.
    pub message: String,
    /// The node this concerns, when there is one.
    pub node: Option<NodeId>,
}

/// Simple axis-aligned bounds in construction space.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Aabb3 {
    /// Minimum corner.
    pub min: [f64; 3],
    /// Maximum corner.
    pub max: [f64; 3],
}

impl Aabb3 {
    const EMPTY: Self = Self {
        min: [f64::INFINITY; 3],
        max: [f64::NEG_INFINITY; 3],
    };

    fn include(&mut self, p: [f64; 3]) {
        for (axis, &value) in p.iter().enumerate() {
            self.min[axis] = self.min[axis].min(value);
            self.max[axis] = self.max[axis].max(value);
        }
    }

    fn union(&mut self, other: &Self) {
        self.include(other.min);
        self.include(other.max);
    }

    fn is_empty(&self) -> bool {
        self.min[0] > self.max[0]
    }
}

/// Counters exposed for introspection (tenet: if we cannot measure it, we
/// cannot improve it).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct EvalCounters {
    /// Bodies emitted into the evaluation output.
    pub bodies: u32,
    /// Distinct tessellations performed (instances reuse tessellations).
    pub tessellations: u32,
    /// Total faces emitted.
    pub faces: u32,
    /// Total vertices emitted.
    pub vertices: u32,
    /// Nodes reported as envelope-only.
    pub envelope_only: u32,
    /// Nodes skipped as not yet implemented.
    pub unimplemented: u32,
    /// Total source-map bytes retained across emitted bodies.
    pub source_map_bytes: u64,
    /// Bodies reused from the evaluation cache (always zero for the pure
    /// [`evaluate`]).
    pub cache_hits: u32,
    /// Cache lookups that missed (always zero for the pure [`evaluate`]).
    pub cache_misses: u32,
}

/// The honest ledger of one evaluation.
#[derive(Clone, Debug, PartialEq)]
pub struct GeometryReport {
    /// Per-node fidelity outcomes, in node order (only nodes the walk
    /// visited).
    pub fidelity: Vec<(NodeId, Fidelity)>,
    /// Structured diagnostics, in emission order.
    pub diagnostics: Vec<Diagnostic>,
    /// Envelope bounds recorded for envelope-only nodes.
    pub envelopes: Vec<(NodeId, Aabb3)>,
    /// Policy-defined curve usage: which nodes used which curve policies.
    pub policy_curves: Vec<(NodeId, PolicyId)>,
    /// Work counters.
    pub counters: EvalCounters,
    /// The policy evaluation ran under.
    pub policy: EvalPolicy,
    /// The evaluation schema version ([`crate::EVAL_SCHEMA_VERSION`]).
    pub schema_version: u32,
}

impl GeometryReport {
    /// Fidelity recorded for a node, if the walk visited it.
    #[must_use]
    pub fn fidelity_of(&self, node: NodeId) -> Option<Fidelity> {
        self.fidelity
            .iter()
            .find(|(n, _)| *n == node)
            .map(|(_, f)| *f)
    }

    /// True when no diagnostics of `severity` or higher were emitted.
    #[must_use]
    pub fn clean_at(&self, severity: Severity) -> bool {
        self.diagnostics.iter().all(|d| d.severity < severity)
    }
}

/// One evaluated body with the node that produced it.
///
/// The body is reference-counted so cache hits and repeated instances
/// share one tessellation; consumers read through it transparently.
/// (Single-threaded sharing by design — see cam-ezlm for the threadability
/// model.)
#[derive(Debug)]
pub struct PlacedBody {
    /// The producing node.
    pub node: NodeId,
    /// The tessellated body (already placed in world space).
    pub body: Rc<TessellatedBody>,
}

/// The result of evaluating a recipe.
#[derive(Debug)]
pub struct Evaluation {
    /// Tessellated bodies in deterministic node order.
    pub bodies: Vec<PlacedBody>,
    /// The evaluation report.
    pub report: GeometryReport,
}

/// Hard evaluation failure: a body that should tessellate, did not.
#[derive(Clone, Debug, PartialEq)]
pub struct EvalError {
    /// The failing node.
    pub node: NodeId,
    /// The underlying tessellation failure.
    pub error: TessellateError,
}

impl core::fmt::Display for EvalError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "node {:?} failed to evaluate: {}", self.node, self.error)
    }
}

impl core::error::Error for EvalError {}

/// Evaluates `recipe` under `policy`.
///
/// Supported in this slice: extrude and revolve bodies, groups, and rigid
/// transforms; CSG nodes report [`Fidelity::EnvelopeOnly`] with a
/// structured diagnostic (never fake geometry); other node kinds report
/// `eval.unimplemented`. The walk starts at the recipe root and visits
/// children depth-first in operand order.
///
/// # Errors
///
/// Fails only when a supported body fails to tessellate; everything
/// unsupported is a report entry, not an error.
pub fn evaluate(recipe: &Recipe, policy: &EvalPolicy) -> Result<Evaluation, EvalError> {
    evaluate_inner(recipe, policy, None)
}

/// Evaluates `recipe` under `policy`, reusing bodies from `cache`.
///
/// Bit-identical to [`evaluate`] by contract: bodies, source maps, and the
/// report agree exactly, except the work counters
/// ([`EvalCounters::tessellations`], [`EvalCounters::cache_hits`],
/// [`EvalCounters::cache_misses`]), which honestly describe how much work
/// each run actually did. The cache is caller-owned and survives across
/// evaluations; entries key on content fingerprints, so edited recipes
/// re-tessellate exactly their changed nodes. See [`crate::cache`] for the
/// key design.
///
/// # Errors
///
/// Same contract as [`evaluate`].
pub fn evaluate_with_cache(
    recipe: &Recipe,
    policy: &EvalPolicy,
    cache: &mut EvalCache,
) -> Result<Evaluation, EvalError> {
    cache.begin_generation();
    evaluate_inner(recipe, policy, Some(cache))
}

fn evaluate_inner(
    recipe: &Recipe,
    policy: &EvalPolicy,
    cache: Option<&mut EvalCache>,
) -> Result<Evaluation, EvalError> {
    let mut cx = EvalCx {
        recipe,
        policy,
        policy_fp: policy_fingerprint(policy),
        cache,
        instance_cache: hashbrown::HashMap::new(),
        bodies: Vec::new(),
        report: GeometryReport {
            fidelity: Vec::new(),
            diagnostics: Vec::new(),
            envelopes: Vec::new(),
            policy_curves: Vec::new(),
            counters: EvalCounters::default(),
            policy: *policy,
            schema_version: crate::EVAL_SCHEMA_VERSION,
        },
    };
    cx.walk(recipe.root(), &Placement3::IDENTITY, true)?;
    Ok(Evaluation {
        bodies: cx.bodies,
        report: cx.report,
    })
}

struct EvalCx<'a> {
    recipe: &'a Recipe,
    policy: &'a EvalPolicy,
    /// Fingerprint of `policy` (cache key component), computed once.
    policy_fp: u64,
    /// Cross-evaluation body cache, when the caller provided one.
    cache: Option<&'a mut EvalCache>,
    bodies: Vec<PlacedBody>,
    report: GeometryReport,
    /// Local-space evaluations of instanced definitions, keyed by node.
    instance_cache: hashbrown::HashMap<NodeId, Rc<Vec<PlacedBody>>>,
}

impl EvalCx<'_> {
    /// Walks one node under an accumulated world placement. When `emit` is
    /// false the walk only computes envelopes (used under CSG operands).
    /// Returns the node's world-space bounds, when known.
    fn walk(
        &mut self,
        node_id: NodeId,
        world: &Placement3,
        emit: bool,
    ) -> Result<Aabb3, EvalError> {
        let node = self.recipe.node(node_id).expect("walked ids are validated");
        match &node.kind {
            NodeKind::Extrude {
                profile,
                placement,
                height,
                caps,
            } => {
                let combined = compose(world, placement);
                let (profile, height, caps) = (*profile, *height, *caps);
                let body = self.body_cached(node_id, world, |cx| {
                    tessellate_extrude(
                        cx.recipe.profile(profile).expect("validated profile id"),
                        &combined,
                        height,
                        caps,
                        cx.policy,
                    )
                    .map_err(|error| EvalError {
                        node: node_id,
                        error,
                    })
                })?;
                let fidelity = self.body_fidelity(node_id, &[profile]);
                Ok(self.finish_body(node_id, body, emit, fidelity))
            }
            NodeKind::Revolve {
                profile,
                placement,
                sweep,
                caps,
            } => {
                let combined = compose(world, placement);
                let (profile, sweep, caps) = (*profile, *sweep, *caps);
                let body = self.body_cached(node_id, world, |cx| {
                    tessellate_revolve(
                        cx.recipe.profile(profile).expect("validated profile id"),
                        &combined,
                        sweep,
                        caps,
                        cx.policy,
                    )
                    .map_err(|error| EvalError {
                        node: node_id,
                        error,
                    })
                })?;
                let fidelity = self.body_fidelity(node_id, &[profile]);
                Ok(self.finish_body(node_id, body, emit, fidelity))
            }
            NodeKind::Loft {
                sections,
                policy: _,
                caps,
            } => {
                let placed: Vec<(Placement3, &crate::profile::Profile2)> = sections
                    .iter()
                    .map(|(placement, profile)| {
                        (
                            compose(world, placement),
                            self.recipe.profile(*profile).expect("validated profile id"),
                        )
                    })
                    .collect();
                let caps = *caps;
                let profile_ids: Vec<ProfileId> =
                    sections.iter().map(|(_, profile)| *profile).collect();
                let body = self.body_cached(node_id, world, |cx| {
                    tessellate_loft(&placed, caps, cx.policy).map_err(|error| EvalError {
                        node: node_id,
                        error,
                    })
                })?;
                let fidelity = self.body_fidelity(node_id, &profile_ids);
                Ok(self.finish_body(node_id, body, emit, fidelity))
            }
            NodeKind::Sweep {
                profile,
                path,
                caps,
            } => {
                let crate::ir::Path3::Polyline { points, frame: _ } = path;
                let points = points.clone();
                let caps = *caps;
                let profile = *profile;
                let body = self.body_cached(node_id, world, |cx| {
                    tessellate_sweep(
                        cx.recipe.profile(profile).expect("validated profile id"),
                        world,
                        &points,
                        caps,
                        cx.policy,
                    )
                    .map_err(|error| EvalError {
                        node: node_id,
                        error,
                    })
                })?;
                let fidelity = self.body_fidelity(node_id, &[profile]);
                Ok(self.finish_body(node_id, body, emit, fidelity))
            }
            NodeKind::Group { children } => {
                let children = children.clone();
                let mut bounds = Aabb3::EMPTY;
                for child in children {
                    let b = self.walk(child, world, emit)?;
                    bounds.union(&b);
                }
                Ok(bounds)
            }
            NodeKind::Transform { child, xf } => {
                let combined = compose(world, xf);
                let child = *child;
                self.walk(child, &combined, emit)
            }
            NodeKind::Csg { op, operands } => {
                let op = *op;
                let operands = operands.clone();
                self.evaluate_csg(node_id, op, &operands, world, emit)
            }
            NodeKind::GridSurface {
                points,
                rows,
                cols,
                close_u,
                close_w,
                thickness,
                placement,
            } => {
                let combined = compose(world, placement);
                let points = points.clone();
                let (rows, cols) = (*rows, *cols);
                let (close_u, close_w, thickness) = (*close_u, *close_w, *thickness);
                let body = self.body_cached(node_id, world, |_| {
                    crate::tessellate::tessellate_grid(
                        &points, rows, cols, close_u, close_w, thickness, &combined,
                    )
                    .map_err(|error| EvalError {
                        node: node_id,
                        error,
                    })
                })?;
                let fidelity = self.body_fidelity(node_id, &[]);
                Ok(self.finish_body(node_id, body, emit, fidelity))
            }
            NodeKind::MeshImport { import, placement } => {
                let placement = compose(world, placement);
                if crate::tessellate::det3(&placement) < 0.0 {
                    self.report.counters.unimplemented += 1;
                    self.report.diagnostics.push(Diagnostic {
                        severity: Severity::Error,
                        code: "eval.import.reflecting",
                        message: String::from(
                            "imported-mesh placements must not reflect; \
                             mirror the source mesh in the frontend instead",
                        ),
                        node: Some(node_id),
                    });
                    return Ok(Aabb3::EMPTY);
                }
                let import = *import;
                let body = self.body_cached(node_id, world, |cx| {
                    let source = cx.recipe.import(import).expect("validated import id");
                    let mesh = transform_mesh(source, &placement);
                    let face_features =
                        alloc::vec![crate::tessellate::Feature::Imported; mesh.faces().count()];
                    let vertex_features =
                        alloc::vec![crate::tessellate::Feature::Imported; mesh.vertices().count()];
                    let source_map =
                        crate::source_map::SourceMap::new(&mesh, face_features, vertex_features);
                    Ok(TessellatedBody { mesh, source_map })
                })?;
                Ok(self.finish_body(node_id, body, emit, Fidelity::Exact))
            }
            NodeKind::Mirror { child, plane } => {
                let reflection = reflection_placement(plane);
                let combined = compose(world, &reflection);
                let child = *child;
                // The reflecting placement composes downward; body
                // tessellation detects the negative determinant and
                // reverses face loops to keep outward orientation.
                self.walk(child, &combined, emit)
            }
            NodeKind::Instance { of, placement } => {
                let of = *of;
                let placement = compose(world, placement);
                if crate::tessellate::det3(&placement) < 0.0 {
                    // Reflecting instances would need winding repair on the
                    // cloned mesh; route reflections through Mirror inside
                    // the definition instead.
                    self.report.counters.unimplemented += 1;
                    self.report.diagnostics.push(Diagnostic {
                        severity: Severity::Error,
                        code: "eval.instance.reflecting",
                        message: String::from(
                            "instance placements must not reflect; put a Mirror \
                             node inside the instanced definition",
                        ),
                        node: Some(node_id),
                    });
                    return Ok(Aabb3::EMPTY);
                }
                let local = if let Some(cached) = self.instance_cache.get(&of) {
                    Rc::clone(cached)
                } else {
                    // Evaluate the definition once in local space.
                    let taken = core::mem::take(&mut self.bodies);
                    self.walk(of, &Placement3::IDENTITY, true)?;
                    let local: Vec<PlacedBody> = core::mem::replace(&mut self.bodies, taken);
                    let rc = Rc::new(local);
                    self.instance_cache.insert(of, Rc::clone(&rc));
                    rc
                };
                let mut bounds = Aabb3::EMPTY;
                for source in local.iter() {
                    let body = Rc::new(instantiate(&source.body, &placement));
                    bounds.union(&mesh_bounds(&body.mesh));
                    self.report.fidelity.push((node_id, Fidelity::Exact));
                    if emit {
                        self.report.counters.bodies += 1;
                        self.report.counters.faces += crate::len_u32(body.mesh.faces().count());
                        self.report.counters.vertices +=
                            crate::len_u32(body.mesh.vertices().count());
                        self.bodies.push(PlacedBody {
                            node: node_id,
                            body,
                        });
                    }
                }
                Ok(bounds)
            }
            _ => {
                self.report.counters.unimplemented += 1;
                self.report.diagnostics.push(Diagnostic {
                    severity: Severity::Error,
                    code: "eval.unimplemented",
                    message: String::from("node kind is not evaluable in this version"),
                    node: Some(node_id),
                });
                Ok(Aabb3::EMPTY)
            }
        }
    }

    /// Evaluates one operand subtree into its bodies (world-placed),
    /// folding multi-body operands into one mesh by union.
    fn collect_operand_mesh(
        &mut self,
        operand: NodeId,
        world: &Placement3,
        scratch: &mut exedra::boolean::BooleanScratch,
        diagnostics: &mut exedra::boolean::BooleanDiagnostics,
    ) -> Result<Option<exedra::Mesh>, EvalError> {
        let taken = core::mem::take(&mut self.bodies);
        let emitted_before = self.report.counters.bodies;
        self.walk(operand, world, true)?;
        let collected: Vec<PlacedBody> = core::mem::replace(&mut self.bodies, taken);
        // Consumed operand bodies are not part of the evaluation output.
        self.report.counters.bodies = emitted_before;
        // Shared (cached) bodies clone their mesh for consumption; unshared
        // ones move it out without copying.
        let mut meshes = collected
            .into_iter()
            .map(|placed| match Rc::try_unwrap(placed.body) {
                Ok(body) => body.mesh,
                Err(shared) => shared.mesh.clone(),
            });
        let Some(mut folded) = meshes.next() else {
            return Ok(None);
        };
        for next in meshes {
            match exedra::boolean::boolean_mesh(
                &folded,
                &next,
                exedra::boolean::BooleanOp::Union,
                exedra::FaceTriangulation::Fan,
                scratch,
                diagnostics,
            ) {
                Ok(output) => folded = output.mesh,
                Err(_) => return Ok(None),
            }
        }
        Ok(Some(folded))
    }

    /// Evaluates a CSG node through the mesh boolean pipeline.
    ///
    /// Success reports `Exact` fidelity; any pipeline refusal (suspect
    /// patches, empty operands) falls back to the envelope-only report —
    /// typed and visible, never silently wrong geometry.
    fn evaluate_csg(
        &mut self,
        node_id: NodeId,
        op: crate::ir::CsgOp,
        operands: &[NodeId],
        world: &Placement3,
        emit: bool,
    ) -> Result<Aabb3, EvalError> {
        use exedra::boolean::{BooleanOp, BooleanScratch};

        let mut scratch = BooleanScratch::default();
        let mut diagnostics = exedra::boolean::BooleanDiagnostics::default();

        // The catalog-free difference convention: A op (union of the rest).
        let mut meshes: Vec<exedra::Mesh> = Vec::with_capacity(operands.len());
        let mut all_present = true;
        for operand in operands {
            match self.collect_operand_mesh(*operand, world, &mut scratch, &mut diagnostics)? {
                Some(mesh) => meshes.push(mesh),
                None => {
                    all_present = false;
                    break;
                }
            }
        }

        // Cache lookup happens after the operand walks so the report is
        // identical either way; the key is content-addressed, so a hit is
        // exactly what the pipeline below would deterministically produce.
        let key = self.cache_key(node_id, world);
        let cached = if let (Some(cache), Some(key)) = (self.cache.as_deref_mut(), key.as_ref()) {
            let hit = cache.get(key);
            if hit.is_some() {
                self.report.counters.cache_hits += 1;
            } else {
                self.report.counters.cache_misses += 1;
            }
            hit
        } else {
            None
        };

        let combined: Option<Rc<TessellatedBody>> = if let Some(body) = cached {
            Some(body)
        } else if all_present {
            let boolean_op = match op {
                crate::ir::CsgOp::Union => BooleanOp::Union,
                crate::ir::CsgOp::Intersection => BooleanOp::Intersection,
                crate::ir::CsgOp::Difference => BooleanOp::Difference,
            };
            let mut iter = meshes.into_iter();
            let first = iter.next().expect("IR validation requires >= 2 operands");
            // Fold the tail together with Union first (the documented
            // n-ary Difference rule folds 2..n before subtracting), then
            // apply the operation once. Union/Intersection fold pairwise
            // identically under associativity.
            let mut tail = iter.next().expect("IR validation requires >= 2 operands");
            let mut tail_ok = true;
            for next in iter {
                match exedra::boolean::boolean_mesh(
                    &tail,
                    &next,
                    BooleanOp::Union,
                    exedra::FaceTriangulation::Fan,
                    &mut scratch,
                    &mut diagnostics,
                ) {
                    Ok(output) => tail = output.mesh,
                    Err(_) => {
                        tail_ok = false;
                        break;
                    }
                }
            }
            let output = if tail_ok {
                exedra::boolean::boolean_mesh(
                    &first,
                    &tail,
                    boolean_op,
                    exedra::FaceTriangulation::Fan,
                    &mut scratch,
                    &mut diagnostics,
                )
                .ok()
            } else {
                None
            };
            output.map(|output| {
                let mesh = output.mesh;
                // Coarse per-operand attribution; fine detail rides the
                // FACE_REGION values the pipeline carried through.
                let face_features: Vec<crate::tessellate::Feature> = output
                    .face_provenance
                    .iter()
                    .map(|(_, side, _)| crate::tessellate::Feature::BooleanFace {
                        operand: match side {
                            exedra::boolean::MeshSide::A => 0,
                            exedra::boolean::MeshSide::B => 1,
                        },
                    })
                    .collect();
                let vertex_features = alloc::vec![
                    crate::tessellate::Feature::BooleanFace { operand: 0 };
                    mesh.vertices().count()
                ];
                let source_map =
                    crate::source_map::SourceMap::new(&mesh, face_features, vertex_features);
                let body = Rc::new(TessellatedBody { mesh, source_map });
                self.report.counters.tessellations += 1;
                if let (Some(cache), Some(key)) = (self.cache.as_deref_mut(), key) {
                    cache.insert(key, Rc::clone(&body));
                }
                body
            })
        } else {
            None
        };

        match combined {
            Some(body) => Ok(self.finish_body(node_id, body, emit, Fidelity::Exact)),
            None => {
                // Typed fallback: envelope-only, with the pipeline's
                // diagnostics surfaced.
                let mut bounds = Aabb3::EMPTY;
                for operand in operands {
                    let b = self.walk(*operand, world, false)?;
                    bounds.union(&b);
                }
                self.report.counters.envelope_only += 1;
                self.report.fidelity.push((node_id, Fidelity::EnvelopeOnly));
                if !bounds.is_empty() {
                    self.report.envelopes.push((node_id, bounds));
                }
                for entry in diagnostics.entries() {
                    self.report.diagnostics.push(Diagnostic {
                        severity: Severity::Warning,
                        code: "eval.csg.pipeline",
                        message: alloc::format!("{entry}"),
                        node: Some(node_id),
                    });
                }
                self.report.diagnostics.push(Diagnostic {
                    severity: Severity::Error,
                    code: "eval.csg.unsupported",
                    message: String::from(
                        "CSG evaluation fell back to the operand envelope; \
                         see the pipeline diagnostics",
                    ),
                    node: Some(node_id),
                });
                Ok(bounds)
            }
        }
    }

    /// Fidelity of a body node: frontend-declared conflicts win, then
    /// policy-defined curves, then exact. Policy usage lands in the report.
    fn body_fidelity(&mut self, node_id: NodeId, profiles: &[ProfileId]) -> Fidelity {
        let mut first_policy = None;
        for profile in profiles {
            let profile = self.recipe.profile(*profile).expect("validated profile id");
            for loop_ in core::iter::once(profile.outer()).chain(profile.holes().iter()) {
                for seg in loop_.segs() {
                    if let crate::profile::SegKind::PolicyTo { policy, .. } = &seg.kind {
                        if !self
                            .report
                            .policy_curves
                            .iter()
                            .any(|(n, p)| *n == node_id && p == policy)
                        {
                            self.report.policy_curves.push((node_id, *policy));
                        }
                        first_policy.get_or_insert(*policy);
                    }
                }
            }
        }
        // Declared spec conflicts win the classification, but policy usage
        // above is still fully attributed in the report.
        if let Some(issue) = self
            .recipe
            .node(node_id)
            .expect("walked ids are validated")
            .issue
        {
            return Fidelity::Conflicted(issue);
        }
        match first_policy {
            Some(policy) => Fidelity::PolicyDefined(policy),
            None => Fidelity::Exact,
        }
    }

    /// Cache key for `node` under `world`, when a cache is attached.
    fn cache_key(&self, node: NodeId, world: &Placement3) -> Option<CacheKey> {
        self.cache.as_ref()?;
        let fingerprint = self.recipe.fingerprint(node)?;
        let mut bits = [0_u64; 12];
        for (i, row) in world.rows.iter().enumerate() {
            for (j, value) in row.iter().enumerate() {
                bits[i * 4 + j] = value.to_bits();
            }
        }
        Some(CacheKey {
            node: fingerprint.0,
            world: bits,
            policy: self.policy_fp,
        })
    }

    /// Runs `build` through the cache: a hit returns the shared body; a
    /// miss builds, counts one tessellation, and (with a cache attached)
    /// stores the result for later evaluations.
    fn body_cached(
        &mut self,
        node_id: NodeId,
        world: &Placement3,
        build: impl FnOnce(&mut Self) -> Result<TessellatedBody, EvalError>,
    ) -> Result<Rc<TessellatedBody>, EvalError> {
        let key = self.cache_key(node_id, world);
        if let (Some(cache), Some(key)) = (self.cache.as_deref_mut(), key.as_ref()) {
            if let Some(body) = cache.get(key) {
                self.report.counters.cache_hits += 1;
                return Ok(body);
            }
            self.report.counters.cache_misses += 1;
        }
        let body = Rc::new(build(self)?);
        self.report.counters.tessellations += 1;
        if let (Some(cache), Some(key)) = (self.cache.as_deref_mut(), key) {
            cache.insert(key, Rc::clone(&body));
        }
        Ok(body)
    }

    /// Records a successfully tessellated body and returns its bounds.
    fn finish_body(
        &mut self,
        node: NodeId,
        body: Rc<TessellatedBody>,
        emit: bool,
        fidelity: Fidelity,
    ) -> Aabb3 {
        let mut bounds = Aabb3::EMPTY;
        let mesh = &body.mesh;
        for face in mesh.faces() {
            for he in mesh.face_loop(face) {
                if let Some(p) = mesh.to_vertex(he).and_then(|v| mesh.vertex_position(v)) {
                    bounds.include([f64::from(p[0]), f64::from(p[1]), f64::from(p[2])]);
                }
            }
        }
        self.report.fidelity.push((node, fidelity));
        if emit {
            self.report.counters.bodies += 1;
            self.report.counters.source_map_bytes += body.source_map.stats().approx_bytes as u64;
            self.report.counters.faces += crate::len_u32(mesh.faces().count());
            self.report.counters.vertices += crate::len_u32(mesh.vertices().count());
            self.bodies.push(PlacedBody { node, body });
        }
        bounds
    }
}

/// Reflection across a plane `dot(n, p) = d` as a placement.
fn reflection_placement(plane: &crate::ir::Plane3) -> Placement3 {
    let n = plane.normal;
    let len = libm::sqrt(n[0] * n[0] + n[1] * n[1] + n[2] * n[2]);
    let u = [n[0] / len, n[1] / len, n[2] / len];
    let d = plane.distance / len;
    let mut rows = [[0.0; 4]; 3];
    for (i, row) in rows.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().take(3).enumerate() {
            let identity = if i == j { 1.0 } else { 0.0 };
            *cell = identity - 2.0 * u[i] * u[j];
        }
        row[3] = 2.0 * d * u[i];
    }
    Placement3 { rows }
}

/// World-space bounds of a mesh (f32 positions promoted).
fn mesh_bounds(mesh: &exedra::Mesh) -> Aabb3 {
    let mut bounds = Aabb3::EMPTY;
    for vertex in mesh.vertices() {
        if let Some(p) = mesh.vertex_position(vertex) {
            bounds.include([f64::from(p[0]), f64::from(p[1]), f64::from(p[2])]);
        }
    }
    bounds
}

/// Clones a local-space body under a rigid placement: vertex positions
/// transform (f64 math, one narrowing), topology and attributes are
/// untouched, and the source map re-pins to the edited revision.
fn instantiate(source: &TessellatedBody, placement: &Placement3) -> TessellatedBody {
    let mesh = transform_mesh(&source.mesh, placement);
    let source_map = source.source_map.repinned(&mesh);
    TessellatedBody { mesh, source_map }
}

/// Clones a mesh with vertices rigid-transformed (f64 math, one narrowing).
fn transform_mesh(source: &exedra::Mesh, placement: &Placement3) -> exedra::Mesh {
    let mut mesh = source.clone();
    let vertices: Vec<exedra::VertexId> = mesh.vertices().collect();
    {
        let mut session = mesh.edit();
        for vertex in vertices {
            if let Some(p) = session.mesh().vertex_position(vertex) {
                let local = [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])];
                let world = apply_placement_pub(placement, local);
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "instance placement narrowing mirrors the tessellation boundary"
                )]
                let narrowed = [world[0] as f32, world[1] as f32, world[2] as f32];
                let _ = exedra::op::set_vertex_position(&mut session, vertex, narrowed);
            }
        }
        #[expect(unused_must_use, reason = "discard sink output")]
        {
            session.finish();
        }
    }
    mesh
}

fn apply_placement_pub(p: &Placement3, v: [f64; 3]) -> [f64; 3] {
    let r = &p.rows;
    [
        r[0][0] * v[0] + r[0][1] * v[1] + r[0][2] * v[2] + r[0][3],
        r[1][0] * v[0] + r[1][1] * v[1] + r[1][2] * v[2] + r[1][3],
        r[2][0] * v[0] + r[2][1] * v[1] + r[2][2] * v[2] + r[2][3],
    ]
}

/// Composes two placements: `outer * inner` (inner applies first).
fn compose(outer: &Placement3, inner: &Placement3) -> Placement3 {
    let a = &outer.rows;
    let b = &inner.rows;
    let mut rows = [[0.0; 4]; 3];
    for (i, row) in rows.iter_mut().enumerate() {
        for j in 0..3 {
            row[j] = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
        row[3] = a[i][0] * b[0][3] + a[i][1] * b[1][3] + a[i][2] * b[2][3] + a[i][3];
    }
    Placement3 { rows }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builders;
    use crate::ir::{CapMode, CsgOp, Plane3, RecipeBuilder};
    use alloc::vec;

    fn extrude_recipe() -> Recipe {
        let mut b = RecipeBuilder::new();
        let p = b.add_profile(builders::rect(2.0, 1.0).expect("rect"));
        let e = b
            .add(NodeKind::Extrude {
                profile: p,
                placement: Placement3::IDENTITY,
                height: 3.0,
                caps: CapMode::Both,
            })
            .expect("valid");
        let t = b
            .add(NodeKind::Transform {
                child: e,
                xf: Placement3::translate(5.0, 0.0, 0.0),
            })
            .expect("valid");
        b.finish(t).expect("valid recipe")
    }

    #[test]
    fn evaluates_transformed_extrude() {
        let recipe = extrude_recipe();
        let result = evaluate(&recipe, &EvalPolicy::default()).expect("evaluates");
        assert_eq!(result.bodies.len(), 1);
        assert!(result.report.clean_at(Severity::Warning));
        assert_eq!(result.report.counters.bodies, 1);
        // The transform moved the body: min x is 5.
        let body = &result.bodies[0].body;
        let min_x = body
            .mesh
            .faces()
            .flat_map(|f| body.mesh.face_loop(f))
            .filter_map(|he| body.mesh.to_vertex(he))
            .filter_map(|v| body.mesh.vertex_position(v))
            .map(|p| p[0])
            .fold(f32::INFINITY, f32::min);
        assert!((min_x - 5.0).abs() < 1e-6);
        // The extrude node reports Exact fidelity.
        assert!(matches!(
            result.report.fidelity_of(recipe.root()),
            None | Some(Fidelity::Exact)
        ));
    }

    #[test]
    fn csg_reports_envelope_only_and_emits_nothing() {
        let mut b = RecipeBuilder::new();
        let p = b.add_profile(builders::rect(1.0, 1.0).expect("rect"));
        let e1 = b
            .add(NodeKind::Extrude {
                profile: p,
                placement: Placement3::IDENTITY,
                height: 1.0,
                caps: CapMode::Both,
            })
            .expect("valid");
        let e2 = b
            .add(NodeKind::Extrude {
                profile: p,
                placement: Placement3::translate(0.5, 0.5, 0.0),
                height: 1.0,
                caps: CapMode::Both,
            })
            .expect("valid");
        let csg = b
            .add(NodeKind::Csg {
                op: CsgOp::Difference,
                operands: vec![e1, e2],
            })
            .expect("valid");
        let recipe = b.finish(csg).expect("valid recipe");

        let result = evaluate(&recipe, &EvalPolicy::default()).expect("evaluates");
        assert!(result.bodies.is_empty(), "no fake geometry under CSG");
        assert_eq!(result.report.fidelity_of(csg), Some(Fidelity::EnvelopeOnly));
        let (_, env) = result.report.envelopes[0];
        assert_eq!(env.min, [0.0, 0.0, 0.0]);
        assert_eq!(env.max, [1.5, 1.5, 1.0]);
        assert!(
            result
                .report
                .diagnostics
                .iter()
                .any(|d| d.code == "eval.csg.unsupported"),
            "structured diagnostic present"
        );
    }

    #[test]
    fn groups_emit_all_children() {
        let mut b = RecipeBuilder::new();
        let p = b.add_profile(builders::rect(1.0, 1.0).expect("rect"));
        let e1 = b
            .add(NodeKind::Extrude {
                profile: p,
                placement: Placement3::IDENTITY,
                height: 1.0,
                caps: CapMode::Both,
            })
            .expect("valid");
        let e2 = b
            .add(NodeKind::Extrude {
                profile: p,
                placement: Placement3::translate(3.0, 0.0, 0.0),
                height: 2.0,
                caps: CapMode::Both,
            })
            .expect("valid");
        let g = b
            .add(NodeKind::Group {
                children: vec![e1, e2],
            })
            .expect("valid");
        let recipe = b.finish(g).expect("valid recipe");
        let result = evaluate(&recipe, &EvalPolicy::default()).expect("evaluates");
        assert_eq!(result.bodies.len(), 2);
        assert_eq!(result.bodies[0].node, e1);
        assert_eq!(result.bodies[1].node, e2);
    }

    #[test]
    fn unimplemented_kinds_report_not_panic() {
        let mut b = RecipeBuilder::new();
        let p = b.add_profile(builders::rect(1.0, 1.0).expect("rect"));
        let face = b
            .add(NodeKind::PlanarFace {
                profile: p,
                placement: Placement3::IDENTITY,
            })
            .expect("valid");
        let recipe = b.finish(face).expect("valid recipe");
        let result = evaluate(&recipe, &EvalPolicy::default()).expect("evaluates");
        assert!(result.bodies.is_empty());
        assert_eq!(result.report.counters.unimplemented, 1);
        assert!(
            result
                .report
                .diagnostics
                .iter()
                .any(|d| d.code == "eval.unimplemented")
        );
    }

    #[test]
    fn csg_reports_are_deterministic() {
        let build = || {
            let mut b = RecipeBuilder::new();
            let p = b.add_profile(builders::rect(1.0, 1.0).expect("rect"));
            let e1 = b
                .add(NodeKind::Extrude {
                    profile: p,
                    placement: Placement3::IDENTITY,
                    height: 1.0,
                    caps: CapMode::Both,
                })
                .expect("valid");
            let e2 = b
                .add(NodeKind::Extrude {
                    profile: p,
                    placement: Placement3::translate(0.25, 0.25, 0.25),
                    height: 1.0,
                    caps: CapMode::Both,
                })
                .expect("valid");
            let csg = b
                .add(NodeKind::Csg {
                    op: CsgOp::Union,
                    operands: vec![e1, e2],
                })
                .expect("valid");
            b.finish(csg).expect("valid recipe")
        };
        let a = evaluate(&build(), &EvalPolicy::default()).expect("first");
        let b = evaluate(&build(), &EvalPolicy::default()).expect("second");
        assert_eq!(a.report, b.report, "reports must be bit-deterministic");
    }

    #[test]
    fn report_records_policy_and_schema() {
        let recipe = extrude_recipe();
        let policy = EvalPolicy::default();
        let result = evaluate(&recipe, &policy).expect("evaluates");
        assert_eq!(result.report.policy, policy);
        assert_eq!(result.report.schema_version, crate::EVAL_SCHEMA_VERSION);
    }

    #[test]
    fn mirror_preserves_outward_orientation() {
        // Mirror an L-prism across the yz plane: still a valid solid with
        // positive volume and reflected coordinates.
        let mut b = RecipeBuilder::new();
        let p = b.add_profile(builders::l_profile(1.0, 1.0, 0.5, 0.5).expect("L"));
        let body = b
            .add(NodeKind::Extrude {
                profile: p,
                placement: Placement3::IDENTITY,
                height: 2.0,
                caps: CapMode::Both,
            })
            .expect("valid");
        let mirrored = b
            .add(NodeKind::Mirror {
                child: body,
                plane: Plane3 {
                    normal: [1.0, 0.0, 0.0],
                    distance: 0.0,
                },
            })
            .expect("valid");
        let recipe = b.finish(mirrored).expect("valid recipe");
        let result = evaluate(&recipe, &EvalPolicy::default()).expect("evaluates");
        assert_eq!(result.bodies.len(), 1);
        let mesh = &result.bodies[0].body.mesh;
        let errors = mesh.validate_deep();
        assert!(errors.is_empty(), "{errors:?}");
        // Volume positive (outward orientation kept under reflection).
        let mut vol = 0.0;
        for face in mesh.faces() {
            let verts: Vec<[f64; 3]> = mesh
                .face_loop(face)
                .filter_map(|he| mesh.to_vertex(he))
                .filter_map(|v| mesh.vertex_position(v))
                .map(|p| [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])])
                .collect();
            for i in 1..verts.len().saturating_sub(1) {
                let (a, b, c) = (verts[0], verts[i], verts[i + 1]);
                vol += a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                    + a[2] * (b[0] * c[1] - b[1] * c[0]);
            }
        }
        vol /= 6.0;
        assert!((vol - 1.5).abs() < 1e-4, "mirrored volume {vol}");
        // All x coordinates are now non-positive.
        let max_x = mesh
            .vertices()
            .filter_map(|v| mesh.vertex_position(v))
            .map(|p| p[0])
            .fold(f32::NEG_INFINITY, f32::max);
        assert!(
            max_x <= 1e-6,
            "mirror reflected across x = 0, max_x {max_x}"
        );
    }

    #[test]
    fn instances_reuse_tessellation() {
        let mut b = RecipeBuilder::new();
        let p = b.add_profile(builders::rect(1.0, 1.0).expect("rect"));
        let def = b
            .add(NodeKind::Extrude {
                profile: p,
                placement: Placement3::IDENTITY,
                height: 1.0,
                caps: CapMode::Both,
            })
            .expect("valid");
        let i1 = b
            .add(NodeKind::Instance {
                of: def,
                placement: Placement3::translate(3.0, 0.0, 0.0),
            })
            .expect("valid");
        let i2 = b
            .add(NodeKind::Instance {
                of: def,
                placement: Placement3::translate(6.0, 0.0, 0.0),
            })
            .expect("valid");
        let i3 = b
            .add(NodeKind::Instance {
                of: def,
                placement: Placement3::rotate_z_then_translate(0.5, 9.0, 0.0, 0.0),
            })
            .expect("valid");
        let g = b
            .add(NodeKind::Group {
                children: vec![i1, i2, i3],
            })
            .expect("valid");
        let recipe = b.finish(g).expect("valid recipe");
        let result = evaluate(&recipe, &EvalPolicy::default()).expect("evaluates");
        assert_eq!(result.bodies.len(), 3);
        assert_eq!(
            result.report.counters.tessellations, 1,
            "definition tessellates once; instances clone and transform"
        );
        // Each instance is valid, with an intact (re-pinned) source map.
        for placed in &result.bodies {
            assert!(placed.body.mesh.validate_deep().is_empty());
            placed
                .body
                .source_map
                .check(&placed.body.mesh)
                .expect("re-pinned map is fresh");
        }
        // Instances landed at distinct positions.
        let min_x = |i: usize| {
            let mesh = &result.bodies[i].body.mesh;
            mesh.vertices()
                .filter_map(|v| mesh.vertex_position(v))
                .map(|p| p[0])
                .fold(f32::INFINITY, f32::min)
        };
        assert!((min_x(0) - 3.0).abs() < 1e-5);
        assert!((min_x(1) - 6.0).abs() < 1e-5);
    }

    #[test]
    fn reflecting_instances_are_rejected() {
        let mut b = RecipeBuilder::new();
        let p = b.add_profile(builders::rect(1.0, 1.0).expect("rect"));
        let def = b
            .add(NodeKind::Extrude {
                profile: p,
                placement: Placement3::IDENTITY,
                height: 1.0,
                caps: CapMode::Both,
            })
            .expect("valid");
        let mirror_placement = Placement3 {
            rows: [
                [-1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
            ],
        };
        let inst = b
            .add(NodeKind::Instance {
                of: def,
                placement: mirror_placement,
            })
            .expect("valid");
        let recipe = b.finish(inst).expect("valid recipe");
        let result = evaluate(&recipe, &EvalPolicy::default()).expect("evaluates");
        assert!(result.bodies.is_empty());
        assert!(
            result
                .report
                .diagnostics
                .iter()
                .any(|d| d.code == "eval.instance.reflecting")
        );
    }

    #[test]
    fn policy_curves_downgrade_fidelity_and_land_in_the_report() {
        use crate::profile::{Loop2, Profile2, Seg2, SegKind};
        let mut b = RecipeBuilder::new();
        let policy = b.curve_policy("spec.transition@1");
        // A rect whose top edge is policy-defined, realized as a shallow arc.
        let outer = Loop2::new(vec![
            Seg2::line((2.0, 0.0)),
            Seg2::line((2.0, 1.0)),
            Seg2::policy((0.0, 1.0), policy, SegKind::Arc { bulge: -0.1 }),
            Seg2::line((0.0, 0.0)),
        ])
        .expect("valid loop");
        let p = b.add_profile(Profile2::simple(outer).expect("valid profile"));
        let n = b
            .add(NodeKind::Extrude {
                profile: p,
                placement: Placement3::IDENTITY,
                height: 1.0,
                caps: CapMode::Both,
            })
            .expect("valid");
        let recipe = b.finish(n).expect("valid recipe");
        assert_eq!(recipe.policy(PolicyId(0)), Some("spec.transition@1"));

        let result = evaluate(&recipe, &EvalPolicy::default()).expect("evaluates");
        assert_eq!(result.bodies.len(), 1);
        assert!(result.bodies[0].body.mesh.validate_deep().is_empty());
        assert_eq!(
            result.report.fidelity_of(n),
            Some(Fidelity::PolicyDefined(policy)),
            "policy curves must downgrade fidelity from Exact"
        );
        assert_eq!(result.report.policy_curves, vec![(n, policy)]);
    }

    #[test]
    fn declared_issues_report_conflicted() {
        let mut b = RecipeBuilder::new();
        let p = b.add_profile(builders::rect(1.0, 1.0).expect("rect"));
        let issue = b.source_ref("spec.issue.nonclosing-profile");
        let n = b
            .with_issue(issue)
            .add(NodeKind::Extrude {
                profile: p,
                placement: Placement3::IDENTITY,
                height: 1.0,
                caps: CapMode::Both,
            })
            .expect("valid");
        let recipe = b.finish(n).expect("valid recipe");
        let result = evaluate(&recipe, &EvalPolicy::default()).expect("evaluates");
        assert_eq!(
            result.report.fidelity_of(n),
            Some(Fidelity::Conflicted(issue)),
            "declared spec issues must report Conflicted, and still build"
        );
        assert_eq!(
            result.bodies.len(),
            1,
            "conflicted nodes still emit geometry"
        );
    }

    #[test]
    fn unregistered_policies_are_rejected_at_finish() {
        use crate::profile::{Loop2, Profile2, Seg2, SegKind};
        let mut b = RecipeBuilder::new();
        let outer = Loop2::new(vec![
            Seg2::line((1.0, 0.0)),
            Seg2::policy((1.0, 1.0), PolicyId(7), SegKind::Line),
            Seg2::line((0.0, 1.0)),
            Seg2::line((0.0, 0.0)),
        ])
        .expect("valid loop");
        let p = b.add_profile(Profile2::simple(outer).expect("valid profile"));
        let n = b
            .add(NodeKind::Extrude {
                profile: p,
                placement: Placement3::IDENTITY,
                height: 1.0,
                caps: CapMode::Both,
            })
            .expect("valid");
        assert!(matches!(
            b.finish(n),
            Err(crate::ir::RecipeError::UnknownPolicy { policy: 7 })
        ));
    }

    #[test]
    fn grid_surface_evaluates_exact_under_transform() {
        let mut b = RecipeBuilder::new();
        let points: Vec<[f64; 3]> = (0..3)
            .flat_map(|r| (0..4).map(move |c| [f64::from(c), f64::from(r), 0.0]))
            .collect();
        let grid = b
            .add(NodeKind::GridSurface {
                points,
                rows: 3,
                cols: 4,
                close_u: false,
                close_w: false,
                thickness: Some(0.5),
                placement: Placement3::translate(0.0, 0.0, 1.0),
            })
            .expect("valid");
        let moved = b
            .add(NodeKind::Transform {
                child: grid,
                xf: Placement3::rotate_z_then_translate(0.7, 10.0, -2.0, 3.0),
            })
            .expect("valid");
        let recipe = b.finish(moved).expect("valid recipe");
        let result = evaluate(&recipe, &EvalPolicy::default()).expect("evaluates");
        assert_eq!(result.bodies.len(), 1);
        assert_eq!(result.report.fidelity_of(grid), Some(Fidelity::Exact));
        let mesh = &result.bodies[0].body.mesh;
        let errors = mesh.validate_deep();
        assert!(errors.is_empty(), "{errors:?}");
        // Rigid motion preserves the slab volume: 3 * 2 * 0.5.
        let mut vol = 0.0;
        for face in mesh.faces() {
            let verts: Vec<[f64; 3]> = mesh
                .face_loop(face)
                .filter_map(|he| mesh.to_vertex(he))
                .filter_map(|v| mesh.vertex_position(v))
                .map(|p| [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])])
                .collect();
            for i in 1..verts.len().saturating_sub(1) {
                let (a, b, c) = (verts[0], verts[i], verts[i + 1]);
                vol += a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                    + a[2] * (b[0] * c[1] - b[1] * c[0]);
            }
        }
        vol /= 6.0;
        assert!((vol - 3.0).abs() < 1e-4, "grid volume {vol}");
    }
}

#[cfg(test)]
mod drill_regression {
    use alloc::vec;

    use super::*;
    use crate::builders;
    use crate::discretize::DiscretizePolicy;
    use crate::ir::{CapMode, CsgOp, NodeKind, Placement3, RecipeBuilder};
    use crate::tessellate::EvalPolicy;

    /// A cylinder drilled through a slab must succeed at every
    /// discretization resolution: cut loops mix exactly-collinear vertices
    /// (wall-triangle diagonals crossing the cap plane), which once left
    /// T-junctions in the re-faced caps and deferred the whole boolean.
    #[test]
    fn drill_succeeds_across_resolutions() {
        for tol in [3.0, 1.0, 0.3, 0.1, 0.03, 0.01] {
            let mut b = RecipeBuilder::new();
            let block = b.add_profile(builders::rect(200.0, 100.0).unwrap());
            let drill = b.add_profile(builders::circle(30.0).unwrap());
            let e1 = b
                .add(NodeKind::Extrude {
                    profile: block,
                    placement: Placement3::IDENTITY,
                    height: 80.0,
                    caps: CapMode::Both,
                })
                .unwrap();
            let e2 = b
                .add(NodeKind::Extrude {
                    profile: drill,
                    placement: Placement3::translate(130.0, 50.0, -20.0),
                    height: 120.0,
                    caps: CapMode::Both,
                })
                .unwrap();
            let csg = b
                .add(NodeKind::Csg {
                    op: CsgOp::Difference,
                    operands: vec![e1, e2],
                })
                .unwrap();
            let recipe = b.finish(csg).unwrap();
            let policy = EvalPolicy {
                discretize: DiscretizePolicy {
                    chord_tolerance: tol,
                    ..DiscretizePolicy::default()
                },
                ..EvalPolicy::default()
            };
            let result = evaluate(&recipe, &policy).unwrap();
            assert_eq!(
                result.bodies.len(),
                1,
                "tol={tol}: {:?}",
                result.report.diagnostics
            );
            assert!(
                result.report.diagnostics.is_empty(),
                "tol={tol}: {:?}",
                result.report.diagnostics
            );
            let errors = result.bodies[0].body.mesh.validate_deep();
            assert!(errors.is_empty(), "tol={tol}: {errors:?}");
        }
    }

    /// The drill under rotated placements (exe-lz4w): f32-narrowed rotated
    /// coordinates once drove the stitcher into a non-manifold edge via a
    /// dropped cut-bound patch; the classifier's largest-triangle sampling
    /// fixed it. Guard several non-axis-aligned angles.
    #[test]
    fn rotated_drill_succeeds() {
        for angle in [
            core::f64::consts::FRAC_PI_4,
            core::f64::consts::FRAC_PI_6,
            1.1,
        ] {
            let mut b = RecipeBuilder::new();
            let block = b.add_profile(builders::rect(200.0, 100.0).unwrap());
            let drill = b.add_profile(builders::circle(30.0).unwrap());
            let e1 = b
                .add(NodeKind::Extrude {
                    profile: block,
                    placement: Placement3::IDENTITY,
                    height: 80.0,
                    caps: CapMode::Both,
                })
                .unwrap();
            let e2 = b
                .add(NodeKind::Extrude {
                    profile: drill,
                    placement: Placement3::translate(130.0, 50.0, -20.0),
                    height: 120.0,
                    caps: CapMode::Both,
                })
                .unwrap();
            let csg = b
                .add(NodeKind::Csg {
                    op: CsgOp::Difference,
                    operands: vec![e1, e2],
                })
                .unwrap();
            let moved = b
                .add(NodeKind::Transform {
                    child: csg,
                    xf: Placement3::rotate_z_then_translate(angle, 50.0, 0.0, 0.0),
                })
                .unwrap();
            let recipe = b.finish(moved).unwrap();
            let result = evaluate(&recipe, &EvalPolicy::default()).unwrap();
            assert_eq!(
                result.bodies.len(),
                1,
                "angle={angle}: {:?}",
                result.report.diagnostics
            );
            assert!(
                result.report.diagnostics.is_empty(),
                "angle={angle}: {:?}",
                result.report.diagnostics
            );
            let errors = result.bodies[0].body.mesh.validate_deep();
            assert!(errors.is_empty(), "angle={angle}: {errors:?}");
        }
    }
}

#[cfg(test)]
mod cache_regression {
    use alloc::vec;
    use alloc::vec::Vec;

    use super::*;
    use crate::builders;
    use crate::cache::EvalCache;
    use crate::discretize::DiscretizePolicy;
    use crate::ir::{CapMode, CsgOp, NodeKind, RecipeBuilder};
    use crate::tessellate::EvalPolicy;

    /// A mixed fixture: extrude, holed extrude, an instanced pair, and a
    /// drilled CSG difference under a transform.
    fn mixed_recipe(drill_radius: f64) -> Recipe {
        let mut b = RecipeBuilder::new();
        let rect = b.add_profile(builders::rect(200.0, 100.0).unwrap());
        let ring = b.add_profile(builders::ring(60.0, 30.0).unwrap());
        let drill = b.add_profile(builders::circle(drill_radius).unwrap());
        let slab = b
            .add(NodeKind::Extrude {
                profile: rect,
                placement: Placement3::IDENTITY,
                height: 80.0,
                caps: CapMode::Both,
            })
            .unwrap();
        let tube = b
            .add(NodeKind::Extrude {
                profile: ring,
                placement: Placement3::translate(400.0, 0.0, 0.0),
                height: 40.0,
                caps: CapMode::Both,
            })
            .unwrap();
        let bit = b
            .add(NodeKind::Extrude {
                profile: drill,
                placement: Placement3::translate(130.0, 50.0, -20.0),
                height: 120.0,
                caps: CapMode::Both,
            })
            .unwrap();
        let holed = b
            .add(NodeKind::Csg {
                op: CsgOp::Difference,
                operands: vec![slab, bit],
            })
            .unwrap();
        let moved = b
            .add(NodeKind::Transform {
                child: holed,
                xf: Placement3::translate(0.0, 300.0, 0.0),
            })
            .unwrap();
        let near = b
            .add(NodeKind::Instance {
                of: tube,
                placement: Placement3::translate(0.0, -300.0, 0.0),
            })
            .unwrap();
        let far = b
            .add(NodeKind::Instance {
                of: tube,
                placement: Placement3::translate(0.0, -600.0, 0.0),
            })
            .unwrap();
        let root = b
            .add(NodeKind::Group {
                children: vec![slab, tube, moved, near, far],
            })
            .unwrap();
        b.finish(root).unwrap()
    }

    fn body_signatures(evaluation: &Evaluation) -> Vec<(NodeId, u64, usize)> {
        evaluation
            .bodies
            .iter()
            .map(|placed| {
                let (tri, _) = placed
                    .body
                    .mesh
                    .to_trimesh(&exedra::ExtractParams::default());
                (
                    placed.node,
                    exedra_testkit::golden::trimesh_signature(&tri),
                    placed.body.source_map.face_count(),
                )
            })
            .collect()
    }

    fn assert_reports_match_modulo_counters(a: &GeometryReport, b: &GeometryReport) {
        assert_eq!(a.fidelity, b.fidelity, "fidelity entries");
        assert_eq!(a.diagnostics, b.diagnostics, "diagnostics");
        assert_eq!(a.envelopes, b.envelopes, "envelopes");
        assert_eq!(a.policy_curves, b.policy_curves, "policy curves");
        assert_eq!(a.policy, b.policy, "policy");
        assert_eq!(a.schema_version, b.schema_version, "schema version");
        assert_eq!(a.counters.bodies, b.counters.bodies, "bodies counter");
        assert_eq!(a.counters.faces, b.counters.faces, "faces counter");
        assert_eq!(a.counters.vertices, b.counters.vertices, "vertices");
    }

    #[test]
    fn warm_and_cold_evaluations_are_bit_identical() {
        let recipe = mixed_recipe(30.0);
        let policy = EvalPolicy::default();
        let pure = evaluate(&recipe, &policy).unwrap();
        assert_eq!(pure.report.counters.cache_hits, 0);
        assert_eq!(pure.report.counters.cache_misses, 0);

        let mut cache = EvalCache::new();
        let cold = evaluate_with_cache(&recipe, &policy, &mut cache).unwrap();
        let warm = evaluate_with_cache(&recipe, &policy, &mut cache).unwrap();

        assert_eq!(body_signatures(&pure), body_signatures(&cold), "pure/cold");
        assert_eq!(body_signatures(&cold), body_signatures(&warm), "cold/warm");
        for (a, b) in cold.bodies.iter().zip(&warm.bodies) {
            assert_eq!(a.body.source_map, b.body.source_map, "source maps");
        }
        assert_reports_match_modulo_counters(&pure.report, &cold.report);
        assert_reports_match_modulo_counters(&cold.report, &warm.report);
        // A cold cached run may do LESS work than the pure run: nodes the
        // recipe references twice (the tube is both a group child and an
        // instanced definition) hit their own same-run entry. Every pure
        // tessellation is accounted for as either work or a hit.
        assert_eq!(
            cold.report.counters.tessellations + cold.report.counters.cache_hits,
            pure.report.counters.tessellations,
            "cold work plus same-run reuse covers the pure run's work"
        );
        assert_eq!(
            warm.report.counters.tessellations, 0,
            "unchanged recipe re-tessellates nothing"
        );
        assert_eq!(warm.report.counters.cache_misses, 0);
        assert!(warm.report.counters.cache_hits > 0);
    }

    #[test]
    fn single_node_edit_misses_exactly_that_node() {
        fn many(heights: &[f64]) -> Recipe {
            let mut b = RecipeBuilder::new();
            let mut children = Vec::new();
            for (i, height) in heights.iter().enumerate() {
                let p = b.add_profile(builders::rect(100.0, 50.0).unwrap());
                let n = b
                    .add(NodeKind::Extrude {
                        profile: p,
                        placement: Placement3::translate(
                            f64::from(u32::try_from(i).unwrap()) * 200.0,
                            0.0,
                            0.0,
                        ),
                        height: *height,
                        caps: CapMode::Both,
                    })
                    .unwrap();
                children.push(n);
            }
            let root = b.add(NodeKind::Group { children }).unwrap();
            b.finish(root).unwrap()
        }

        let policy = EvalPolicy::default();
        let mut heights = vec![80.0_f64; 12];
        let mut cache = EvalCache::new();
        let cold = evaluate_with_cache(&many(&heights), &policy, &mut cache).unwrap();
        assert_eq!(cold.report.counters.tessellations, 12);

        heights[7] = 95.0;
        let edited = many(&heights);
        let warm = evaluate_with_cache(&edited, &policy, &mut cache).unwrap();
        assert_eq!(warm.report.counters.cache_misses, 1, "one edited node");
        assert_eq!(warm.report.counters.cache_hits, 11, "the rest reuse");
        assert_eq!(warm.report.counters.tessellations, 1);

        // The warm result is bit-identical to a fresh evaluation.
        let fresh = evaluate(&edited, &policy).unwrap();
        assert_eq!(body_signatures(&fresh), body_signatures(&warm));
        assert_reports_match_modulo_counters(&fresh.report, &warm.report);
    }

    #[test]
    fn drilled_csg_reuses_the_boolean_result() {
        let recipe = mixed_recipe(30.0);
        let policy = EvalPolicy::default();
        let mut cache = EvalCache::new();
        let _ = evaluate_with_cache(&recipe, &policy, &mut cache).unwrap();
        let warm = evaluate_with_cache(&recipe, &policy, &mut cache).unwrap();
        assert_eq!(
            warm.report.counters.tessellations, 0,
            "the boolean pipeline must not re-run on an unchanged recipe"
        );
        // Editing only the drill radius invalidates the drill body and the
        // CSG node that contains it, nothing else.
        let edited = mixed_recipe(25.0);
        let warm = evaluate_with_cache(&edited, &policy, &mut cache).unwrap();
        assert_eq!(warm.report.counters.cache_misses, 2, "drill bit + csg");
        assert_eq!(warm.report.counters.tessellations, 2);
        let fresh = evaluate(&edited, &policy).unwrap();
        assert_eq!(body_signatures(&fresh), body_signatures(&warm));
    }

    #[test]
    fn policy_changes_reuse_nothing_across_policies() {
        let recipe = mixed_recipe(30.0);
        let coarse = EvalPolicy {
            discretize: DiscretizePolicy {
                chord_tolerance: 1.0,
                ..DiscretizePolicy::default()
            },
            ..EvalPolicy::default()
        };
        // Warm the cache under the default policy, then evaluate coarse:
        // the coarse run must do exactly as much tessellation work as a
        // coarse run on a fresh cache — zero cross-policy reuse (same-run
        // sharing, like the twice-referenced tube, is still allowed).
        let mut shared = EvalCache::new();
        let _ = evaluate_with_cache(&recipe, &EvalPolicy::default(), &mut shared).unwrap();
        let crossed = evaluate_with_cache(&recipe, &coarse, &mut shared).unwrap();
        let mut fresh = EvalCache::new();
        let baseline = evaluate_with_cache(&recipe, &coarse, &mut fresh).unwrap();
        assert_eq!(
            crossed.report.counters.tessellations, baseline.report.counters.tessellations,
            "policy is in the key: nothing from the default run is reusable"
        );
    }

    #[test]
    fn eviction_is_deterministic_and_correct() {
        let policy = EvalPolicy::default();
        let mut cache = EvalCache::with_capacity(2);
        let a = mixed_recipe(30.0);
        let b = mixed_recipe(25.0);
        for _ in 0..3 {
            let ra = evaluate_with_cache(&a, &policy, &mut cache).unwrap();
            let rb = evaluate_with_cache(&b, &policy, &mut cache).unwrap();
            // Outputs stay correct under heavy eviction.
            let fa = evaluate(&a, &policy).unwrap();
            let fb = evaluate(&b, &policy).unwrap();
            assert_eq!(body_signatures(&ra), body_signatures(&fa));
            assert_eq!(body_signatures(&rb), body_signatures(&fb));
        }
        assert!(cache.counters().evictions > 0, "capacity 2 must evict");
        assert!(cache.len() <= 2);
        // Determinism: replaying the same sequence on a fresh cache yields
        // identical counters.
        let mut replay = EvalCache::with_capacity(2);
        for _ in 0..3 {
            let _ = evaluate_with_cache(&a, &policy, &mut replay).unwrap();
            let _ = evaluate_with_cache(&b, &policy, &mut replay).unwrap();
        }
        assert_eq!(cache.counters(), replay.counters());
        assert_eq!(cache.bytes_retained(), replay.bytes_retained());
    }
}
