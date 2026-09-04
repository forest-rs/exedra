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

use exedra_triangulate::RefineStats;

use crate::cache::{CacheKey, EvalCache, policy_fingerprint};
use crate::ir::{NodeId, NodeKind, Placement3, PolicyId, ProfileId, Recipe, SourceId};
use crate::tessellate::{
    EvalPolicy, TessellateError, TessellatedBody, tessellate_extrude, tessellate_loft,
    tessellate_planar_face, tessellate_primitive, tessellate_revolve, tessellate_sweep,
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
    /// Only a bounding envelope is available because an operation refused
    /// unsupported or ambiguous topology.
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
#[non_exhaustive]
pub struct Diagnostic {
    /// Severity class.
    pub severity: Severity,
    /// Stable machine-readable code (for example `eval.csg.unsupported`).
    pub code: &'static str,
    /// Human-readable detail.
    pub message: String,
    /// The node this concerns, when there is one.
    pub node: Option<NodeId>,
    /// Resolved opaque source reference owned by this diagnostic.
    ///
    /// This is the source bound to [`Self::node`] when the diagnostic was
    /// emitted. Owning the string keeps a detached [`GeometryReport`]
    /// self-describing without retaining its [`Recipe`].
    pub source: Option<String>,
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
    /// The canonical empty bounds used to begin a union fold.
    pub const EMPTY: Self = Self {
        min: [f64::INFINITY; 3],
        max: [f64::NEG_INFINITY; 3],
    };

    fn include(&mut self, p: [f64; 3]) {
        for (axis, &value) in p.iter().enumerate() {
            self.min[axis] = self.min[axis].min(value);
            self.max[axis] = self.max[axis].max(value);
        }
    }

    /// Expands these bounds to include `other`.
    ///
    /// An empty `other` is the identity element, so callers can fold optional
    /// bounds into [`Self::EMPTY`] without special-casing empty inputs.
    pub fn union(&mut self, other: &Self) {
        if other.is_empty() {
            return;
        }
        self.include(other.min);
        self.include(other.max);
    }

    /// Returns `true` when at least one axis has no covered interval.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.min.iter().zip(self.max).any(|(&min, max)| min > max)
    }
}

/// Counters exposed for introspection (tenet: if we cannot measure it, we
/// cannot improve it).
///
/// New work counters may be added as evaluators grow. Construct this
/// non-exhaustive ledger through [`Default`] rather than a struct literal.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
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
    /// Stretch nodes evaluated through a constructive rewrite.
    pub stretch_exact: u32,
    /// Stretch nodes evaluated through the general closed-mesh path.
    pub stretch_mesh: u32,
    /// Input mesh faces partitioned by stretch section planes.
    pub stretch_faces_split: u64,
    /// Prismatic band faces emitted by mesh-backed expansion.
    pub stretch_band_faces: u64,
    /// Mapped source faces whose stretched UV extension was underdetermined.
    pub stretch_uv_unmapped_faces: u64,
    /// Stretch nodes refused because their topology was not safely evaluable.
    pub stretch_refusals: u32,
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
#[non_exhaustive]
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
    /// Refinement work and stopping outcomes, in node order. Entries are
    /// present only for bodies whose policy requested planar or cap
    /// refinement; cache hits replay the outcome stored with the tessellated
    /// body so they cannot hide incomplete quality.
    pub refinements: Vec<(NodeId, RefineStats)>,
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

    /// Refinement work and stopping outcome recorded for a node, if any.
    #[must_use]
    pub fn refinement_of(&self, node: NodeId) -> Option<RefineStats> {
        self.refinements
            .iter()
            .find(|(n, _)| *n == node)
            .map(|(_, stats)| *stats)
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
/// Supported nodes include declared box/cylinder primitives, constructive
/// bodies, planar faces, groups, transforms, stretch, and CSG operations
/// handled by the mesh Boolean pipeline. Refused CSG and stretch operations
/// report [`Fidelity::EnvelopeOnly`] with structured diagnostics rather than
/// returning invented geometry.
/// The walk starts at the recipe root and visits children depth-first in
/// operand order.
///
/// # Errors
///
/// Fails when a supported body cannot be tessellated. Bounded CSG refusals and
/// refused operations are recorded in the evaluation report.
pub fn evaluate(recipe: &Recipe, policy: &EvalPolicy) -> Result<Evaluation, EvalError> {
    evaluate_inner(recipe, policy, None)
}

/// Evaluates `recipe` under `policy`, reusing bodies from `cache`.
///
/// Bit-identical to [`evaluate`] by contract: bodies, source maps, and the
/// report agree exactly, except work counters such as
/// [`EvalCounters::tessellations`], [`EvalCounters::stretch_faces_split`],
/// [`EvalCounters::cache_hits`], and [`EvalCounters::cache_misses`], which
/// honestly describe how much work each run actually did. The cache is
/// caller-owned and survives across evaluations; entries key on content
/// fingerprints, so edited recipes re-tessellate exactly their changed nodes.
/// See [`crate::cache`] for the key design.
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
            refinements: Vec::new(),
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
            NodeKind::PlanarFace { profile, placement } => {
                let combined = compose(world, placement);
                let profile = *profile;
                let body = self.body_cached(node_id, world, |cx| {
                    tessellate_planar_face(
                        cx.recipe.profile(profile).expect("validated profile id"),
                        &combined,
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
            NodeKind::Primitive { spec, placement } => {
                let combined = compose(world, placement);
                let spec = *spec;
                let body = self.body_cached(node_id, world, |cx| {
                    tessellate_primitive(spec, &combined, cx.policy).map_err(|error| EvalError {
                        node: node_id,
                        error,
                    })
                })?;
                let fidelity = self.body_fidelity(node_id, &[]);
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
                let reflects = crate::tessellate::det3(&placement) < 0.0;
                let import = *import;
                let body = self.body_cached(node_id, world, |cx| {
                    let source = cx.recipe.import(import).expect("validated import id");
                    // A reflection reverses orientation, so transforming only
                    // positions would leave an outward source inside out. The
                    // rebuild reverses loops and remaps every built-in semantic
                    // attribute; proper placements retain the cheaper clone.
                    let mesh = if reflects {
                        crate::import_mesh::transform_reflecting(source, &placement).map_err(
                            |error| EvalError {
                                node: node_id,
                                error,
                            },
                        )?
                    } else {
                        transform_mesh(source, &placement)
                    };
                    let face_features =
                        alloc::vec![crate::tessellate::Feature::Imported; mesh.faces().count()];
                    let vertex_features =
                        alloc::vec![crate::tessellate::Feature::Imported; mesh.vertices().count()];
                    let source_map =
                        crate::source_map::SourceMap::new(&mesh, face_features, vertex_features);
                    Ok(TessellatedBody {
                        mesh,
                        source_map,
                        refinement: None,
                    })
                })?;
                Ok(self.finish_body(node_id, body, emit, Fidelity::Exact))
            }
            NodeKind::Stretch {
                child,
                plane,
                length,
            } => self.evaluate_stretch(node_id, *child, plane, *length, world, emit),
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
                    self.push_diagnostic(
                        node_id,
                        Severity::Error,
                        "eval.instance.reflecting",
                        String::from(
                            "instance placements must not reflect; put a Mirror \
                             node inside the instanced definition",
                        ),
                    );
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
        }
    }

    fn evaluate_stretch(
        &mut self,
        node_id: NodeId,
        child: NodeId,
        plane: &crate::ir::Plane3,
        length: f64,
        world: &Placement3,
        emit: bool,
    ) -> Result<Aabb3, EvalError> {
        match crate::stretch::exact_plan(self.recipe, child, plane, length, world) {
            Ok(Some(plan)) => {
                let profiles = self.record_exact_stretch_child(child);
                let body = self.body_cached(node_id, world, |cx| {
                    plan.tessellate(cx.policy).map_err(|error| EvalError {
                        node: node_id,
                        error,
                    })
                })?;
                self.report.counters.stretch_exact += plan.stretch_nodes();
                let fidelity = self.body_fidelity(node_id, &profiles);
                Ok(self.finish_body(node_id, body, emit, fidelity))
            }
            Err(refusal) => self.finish_stretch_refusal(
                node_id,
                child,
                world,
                refusal.code(),
                refusal.message(),
            ),
            Ok(None) => {
                // General stretch consumes child bodies just like CSG does:
                // their evaluation work and fidelity stay visible, but their
                // pre-deformation meshes are not emitted alongside the result.
                let taken = core::mem::take(&mut self.bodies);
                let emitted_before = (
                    self.report.counters.bodies,
                    self.report.counters.faces,
                    self.report.counters.vertices,
                    self.report.counters.source_map_bytes,
                );
                self.walk(child, world, true)?;
                let collected: Vec<PlacedBody> = core::mem::replace(&mut self.bodies, taken);
                (
                    self.report.counters.bodies,
                    self.report.counters.faces,
                    self.report.counters.vertices,
                    self.report.counters.source_map_bytes,
                ) = emitted_before;
                let mut bounds = Aabb3::EMPTY;
                for placed in &collected {
                    bounds.union(&mesh_bounds(&placed.body.mesh));
                }
                if collected.is_empty() {
                    return Ok(self.record_stretch_refusal(
                        node_id,
                        bounds,
                        "eval.stretch.empty_child",
                        "the stretch child produced no body",
                    ));
                }

                // UV-extension diagnostics are part of the report while the
                // cache stores only bodies. Keep mapped mesh stretches out of
                // that cache until its value can carry deterministic stats;
                // otherwise a warm run would hide an unmapped-face note.
                let cacheable = collected.len() == 1
                    && collected[0]
                        .body
                        .mesh
                        .attrs()
                        .sparse(exedra_mesh::attr::CORNER_UV)
                        .is_none();
                let key = cacheable.then(|| self.cache_key(node_id, world)).flatten();
                let cached =
                    if let (Some(cache), Some(key)) = (self.cache.as_deref_mut(), key.as_ref()) {
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

                let mut stretched = Vec::with_capacity(collected.len());
                let mut stats = crate::stretch::MeshStretchStats::default();
                if let Some(body) = cached {
                    stretched.push(body);
                } else {
                    for placed in collected {
                        match crate::stretch::stretch_mesh(
                            &placed.body,
                            plane,
                            length,
                            world,
                            self.policy,
                        ) {
                            Ok((body, body_stats)) => {
                                stats.split_faces += body_stats.split_faces;
                                stats.band_faces += body_stats.band_faces;
                                stats.uv_unmapped_faces += body_stats.uv_unmapped_faces;
                                stretched.push(Rc::new(body));
                                self.report.counters.tessellations += 1;
                            }
                            Err(refusal) => {
                                return Ok(self.record_stretch_refusal(
                                    node_id,
                                    bounds,
                                    refusal.code(),
                                    refusal.message(),
                                ));
                            }
                        }
                    }
                    if let (Some(cache), Some(key), [body]) =
                        (self.cache.as_deref_mut(), key, stretched.as_slice())
                    {
                        cache.insert(key, Rc::clone(body));
                    }
                }
                self.report.counters.stretch_mesh += 1;
                self.report.counters.stretch_faces_split += stats.split_faces;
                self.report.counters.stretch_band_faces += stats.band_faces;
                self.report.counters.stretch_uv_unmapped_faces += stats.uv_unmapped_faces;
                if stats.uv_unmapped_faces > 0 {
                    self.push_diagnostic(
                        node_id,
                        Severity::Note,
                        "eval.stretch.uv_unmapped",
                        alloc::format!(
                            "{} stretched face(s) had no determinate planar UV extension",
                            stats.uv_unmapped_faces
                        ),
                    );
                }
                let mut result_bounds = Aabb3::EMPTY;
                for body in stretched {
                    let fidelity = self.body_fidelity(node_id, &[]);
                    result_bounds.union(&self.finish_body(node_id, body, emit, fidelity));
                }
                Ok(result_bounds)
            }
        }
    }

    /// Replays the fidelity footprint that an exact stretch rewrite bypasses.
    /// Transform nodes do not record their own fidelity in the ordinary walk;
    /// the underlying body does, including any profile policy attribution.
    fn record_exact_stretch_child(&mut self, child: NodeId) -> Vec<ProfileId> {
        let kind = self
            .recipe
            .node(child)
            .expect("stretch child is validated")
            .kind
            .clone();
        match kind {
            NodeKind::Transform { child, .. } => self.record_exact_stretch_child(child),
            NodeKind::Stretch {
                child: nested_child,
                ..
            } => {
                let profiles = self.record_exact_stretch_child(nested_child);
                let fidelity = self.body_fidelity(child, &profiles);
                self.report.fidelity.push((child, fidelity));
                profiles
            }
            NodeKind::Extrude { profile, .. } => {
                let profiles = alloc::vec![profile];
                let fidelity = self.body_fidelity(child, &profiles);
                self.report.fidelity.push((child, fidelity));
                profiles
            }
            _ => {
                let fidelity = self.body_fidelity(child, &[]);
                self.report.fidelity.push((child, fidelity));
                Vec::new()
            }
        }
    }

    fn finish_stretch_refusal(
        &mut self,
        node_id: NodeId,
        child: NodeId,
        world: &Placement3,
        code: &'static str,
        message: &'static str,
    ) -> Result<Aabb3, EvalError> {
        let bounds = self.walk(child, world, false)?;
        Ok(self.record_stretch_refusal(node_id, bounds, code, message))
    }

    fn record_stretch_refusal(
        &mut self,
        node_id: NodeId,
        bounds: Aabb3,
        code: &'static str,
        message: &'static str,
    ) -> Aabb3 {
        self.report.counters.envelope_only += 1;
        self.report.counters.stretch_refusals += 1;
        self.report.fidelity.push((node_id, Fidelity::EnvelopeOnly));
        if !bounds.is_empty() {
            self.report.envelopes.push((node_id, bounds));
        }
        self.push_diagnostic(node_id, Severity::Error, code, String::from(message));
        bounds
    }

    /// Evaluates one operand subtree into its bodies (world-placed),
    /// folding multi-body operands into one mesh by union.
    fn collect_operand_mesh(
        &mut self,
        operand: NodeId,
        world: &Placement3,
        scratch: &mut exedra_mesh::boolean::BooleanScratch,
        diagnostics: &mut exedra_mesh::boolean::BooleanDiagnostics,
    ) -> Result<Option<exedra_mesh::Mesh>, EvalError> {
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
            match exedra_mesh::boolean::boolean_mesh(
                &folded,
                &next,
                exedra_mesh::boolean::BooleanOp::Union,
                exedra_mesh::FaceTriangulation::Fan,
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
        use exedra_mesh::boolean::{BooleanOp, BooleanScratch};

        let mut scratch = BooleanScratch::default();
        let mut diagnostics = exedra_mesh::boolean::BooleanDiagnostics::default();

        // The catalog-free difference convention: A op (union of the rest).
        let mut meshes: Vec<exedra_mesh::Mesh> = Vec::with_capacity(operands.len());
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
                match exedra_mesh::boolean::boolean_mesh(
                    &tail,
                    &next,
                    BooleanOp::Union,
                    exedra_mesh::FaceTriangulation::Fan,
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
                exedra_mesh::boolean::boolean_mesh(
                    &first,
                    &tail,
                    boolean_op,
                    exedra_mesh::FaceTriangulation::Fan,
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
                            exedra_mesh::boolean::MeshSide::A => 0,
                            exedra_mesh::boolean::MeshSide::B => 1,
                        },
                    })
                    .collect();
                let vertex_features = alloc::vec![
                    crate::tessellate::Feature::BooleanFace { operand: 0 };
                    mesh.vertices().count()
                ];
                let source_map =
                    crate::source_map::SourceMap::new(&mesh, face_features, vertex_features);
                let body = Rc::new(TessellatedBody {
                    mesh,
                    source_map,
                    refinement: None,
                });
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
                    self.push_diagnostic(
                        node_id,
                        Severity::Warning,
                        "eval.csg.pipeline",
                        alloc::format!("{entry}"),
                    );
                }
                self.push_diagnostic(
                    node_id,
                    Severity::Error,
                    "eval.csg.unsupported",
                    String::from(
                        "CSG evaluation fell back to the operand envelope; \
                         see the pipeline diagnostics",
                    ),
                );
                Ok(bounds)
            }
        }
    }

    fn push_diagnostic(
        &mut self,
        node: NodeId,
        severity: Severity,
        code: &'static str,
        message: String,
    ) {
        let source = self.recipe.source_of(node).map(String::from);
        self.report.diagnostics.push(Diagnostic {
            severity,
            code,
            message,
            node: Some(node),
            source,
        });
    }

    /// Records refinement work and makes every incomplete stopping reason
    /// visible in the honest geometry report. The tessellated body carries
    /// the same value, which makes this replayable on cache hits.
    fn record_refinement(&mut self, node: NodeId, stats: RefineStats) {
        self.report.refinements.push((node, stats));
        if stats.budget_exhausted {
            self.push_diagnostic(
                node,
                Severity::Warning,
                "eval.refinement.budget_exhausted",
                alloc::format!(
                    "refinement exhausted its generated-point or internal-index budget after {} generated point(s)",
                    stats.steiner_points
                ),
            );
        }
        if stats.declined_insertions > 0 {
            self.push_diagnostic(
                node,
                Severity::Warning,
                "eval.refinement.declined_insertions",
                alloc::format!(
                    "{} refinement insertion(s) were declined",
                    stats.declined_insertions
                ),
            );
        }
        if stats.input_limited_triangles > 0 {
            self.push_diagnostic(
                node,
                Severity::Warning,
                "eval.refinement.input_limited",
                alloc::format!(
                    "{} triangle(s) remain limited by input boundary angles",
                    stats.input_limited_triangles
                ),
            );
        }
        if stats.remaining_bad_triangles > 0 {
            self.push_diagnostic(
                node,
                Severity::Warning,
                "eval.refinement.remaining_violations",
                alloc::format!(
                    "{} triangle(s) remain outside the requested quality bound",
                    stats.remaining_bad_triangles
                ),
            );
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
        if let Some(stats) = body.refinement {
            self.record_refinement(node, stats);
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
    // Every Plane3 enters a Recipe through validation. Keeping normalization
    // there makes a bad plane a typed construction error rather than an
    // evaluation-time NaN that contaminates mesh coordinates.
    let (u, d) = plane
        .normalized()
        .expect("mirror planes are validated at recipe construction");
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
pub(crate) fn mesh_bounds(mesh: &exedra_mesh::Mesh) -> Aabb3 {
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
    TessellatedBody {
        mesh,
        source_map,
        refinement: source.refinement,
    }
}

/// Clones a mesh with vertices rigid-transformed (f64 math, one narrowing).
fn transform_mesh(source: &exedra_mesh::Mesh, placement: &Placement3) -> exedra_mesh::Mesh {
    let mut mesh = source.clone();
    let vertices: Vec<exedra_mesh::VertexId> = mesh.vertices().collect();
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
                let _ = exedra_mesh::op::set_vertex_position(&mut session, vertex, narrowed);
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
    use crate::ir::{CapMode, CsgOp, Plane3, PrimitiveSpec, RecipeBuilder};
    use alloc::vec;

    #[test]
    fn public_bounds_fold_treats_empty_as_the_identity() {
        // Consumers start a bounds fold at EMPTY. Union must ignore an empty
        // operand in either order and report an inverted interval on any axis
        // as empty rather than relying on the sentinel's X axis alone.
        let occupied = Aabb3 {
            min: [-2.0, 1.0, 4.0],
            max: [3.0, 5.0, 9.0],
        };
        let mut folded = Aabb3::EMPTY;
        assert!(folded.is_empty());
        folded.union(&occupied);
        folded.union(&Aabb3::EMPTY);
        assert_eq!(folded, occupied);
        assert!(!folded.is_empty());
        assert!(
            Aabb3 {
                min: [0.0, 2.0, 0.0],
                max: [1.0, 1.0, 1.0],
            }
            .is_empty()
        );
    }

    fn imported_unit_box() -> exedra_mesh::Mesh {
        let mut builder = exedra_mesh::MeshBuilder::new();
        for point in [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            [0.0, 1.0, 1.0],
        ] {
            builder.push_vertex(point);
        }
        for face in [
            [0, 3, 2, 1],
            [4, 5, 6, 7],
            [0, 1, 5, 4],
            [1, 2, 6, 5],
            [2, 3, 7, 6],
            [3, 0, 4, 7],
        ] {
            builder.add_face(&face).expect("box face is valid");
        }
        builder.build().expect("box mesh is valid").mesh
    }

    fn attributed_imported_unit_box() -> exedra_mesh::Mesh {
        let mut mesh = imported_unit_box();
        let face = mesh.faces().next().expect("box has a face");
        let corner = mesh
            .face_loop(face)
            .nth(1)
            .expect("box face has a second corner");
        let vertex = mesh
            .to_vertex(corner)
            .expect("box corner has a destination");
        {
            let mut edit = mesh.edit();
            exedra_mesh::op::set_face_region(&mut edit, face, 23).expect("live face");
            exedra_mesh::op::set_edge_seam(&mut edit, corner, true).expect("live edge");
            exedra_mesh::op::set_edge_sharpness(&mut edit, corner, 1.75).expect("live edge");
            exedra_mesh::op::set_vertex_sharpness(&mut edit, vertex, 2.5).expect("live vertex");
            exedra_mesh::op::set_corner_uv(&mut edit, corner, [0.25, 0.75]).expect("live corner");
            exedra_mesh::op::set_corner_normal_override(
                &mut edit,
                corner,
                Some([
                    core::f32::consts::FRAC_1_SQRT_2,
                    core::f32::consts::FRAC_1_SQRT_2,
                    0.0,
                ]),
            )
            .expect("live corner");
            #[expect(unused_must_use, reason = "discard sink output")]
            {
                edit.finish();
            }
        }
        mesh
    }

    fn signed_mesh_volume(mesh: &exedra_mesh::Mesh) -> f64 {
        let mut six_volume = 0.0;
        for face in mesh.faces() {
            for corners in mesh.face_triangles(face, exedra_mesh::FaceTriangulation::Fan) {
                let points = corners.map(|corner| {
                    let vertex = mesh.to_vertex(corner).expect("corner has a vertex");
                    mesh.vertex_position(vertex)
                        .expect("vertex has a position")
                        .map(f64::from)
                });
                six_volume += exedra_math::dot(points[0], exedra_math::cross(points[1], points[2]));
            }
        }
        six_volume / 6.0
    }

    fn mirrored_import_recipe(mesh: exedra_mesh::Mesh) -> Recipe {
        let mut builder = RecipeBuilder::new();
        let import = builder.add_import(mesh).expect("deep-valid import");
        let imported = builder
            .add(NodeKind::MeshImport {
                import,
                placement: Placement3::IDENTITY,
            })
            .expect("import node");
        builder
            .finish(imported)
            .expect("valid recipe")
            .mirrored(Plane3 {
                normal: [1.0, 0.0, 0.0],
                distance: 0.0,
            })
            .expect("valid mirror")
    }

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

    fn face_cross_z(mesh: &exedra_mesh::Mesh, face: exedra_mesh::FaceId) -> f32 {
        let positions: Vec<[f32; 3]> = mesh
            .face_loop(face)
            .filter_map(|half_edge| mesh.to_vertex(half_edge))
            .filter_map(|vertex| mesh.vertex_position(vertex).copied())
            .collect();
        assert_eq!(positions.len(), 3, "planar faces are triangulated");
        let ab = [
            positions[1][0] - positions[0][0],
            positions[1][1] - positions[0][1],
        ];
        let ac = [
            positions[2][0] - positions[0][0],
            positions[2][1] - positions[0][1],
        ];
        ab[0] * ac[1] - ab[1] * ac[0]
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
    fn planar_face_evaluates_as_two_positive_normal_triangles() {
        // PlanarFace is an existing validated recipe node. This trap pins the
        // missing evaluator contract: a square becomes one single-sided open
        // surface, triangulated with winding toward the placement's local +Z.
        let mut builder = RecipeBuilder::new();
        let profile = builder.add_profile(builders::rect(2.0, 1.0).expect("rectangle"));
        let face = builder
            .add(NodeKind::PlanarFace {
                profile,
                placement: Placement3::IDENTITY,
            })
            .expect("the public IR accepts a planar face");
        let recipe = builder.finish(face).expect("valid planar-face recipe");

        let result = evaluate(&recipe, &EvalPolicy::default()).expect("face evaluates");
        assert_eq!(result.bodies.len(), 1);
        assert_eq!(result.report.fidelity_of(face), Some(Fidelity::Exact));
        assert_eq!(result.report.counters.unimplemented, 0);

        let body = &result.bodies[0].body;
        assert_eq!(body.mesh.faces().count(), 2, "a square has two triangles");
        assert_eq!(
            body.mesh.boundary_loops().expect("boundary is valid").len(),
            1,
            "the surface remains single-sided and open"
        );
        for mesh_face in body.mesh.faces() {
            assert!(
                face_cross_z(&body.mesh, mesh_face) > 0.0,
                "triangle winding faces local +Z"
            );
        }
    }

    #[test]
    fn planar_refinement_stats_and_incomplete_quality_reach_report_and_cache() {
        // A long, thin face with a five-point budget cannot finish its
        // requested quality pass. This trap requires the evaluator to retain
        // generated work, expose the remaining violations, and replay both
        // the stats and warnings when the body comes from the cache.
        use exedra_triangulate::RefineParams;

        let mut builder = RecipeBuilder::new();
        let profile = builder.add_profile(builders::rect(10.0, 1.0).expect("rectangle"));
        let face = builder
            .add(NodeKind::PlanarFace {
                profile,
                placement: Placement3::IDENTITY,
            })
            .expect("planar face");
        let recipe = builder.finish(face).expect("valid planar-face recipe");
        let policy = EvalPolicy::default()
            .with_planar_face_refinement(RefineParams::new(1.0).with_max_steiner_points(5));

        let mut cache = EvalCache::new();
        let cold = evaluate_with_cache(&recipe, &policy, &mut cache).expect("cold evaluation");
        let stats = cold
            .report
            .refinement_of(face)
            .expect("refinement stats are reported");
        assert_eq!(
            stats.steiner_points, 5,
            "the configured work budget is visible"
        );
        assert!(stats.budget_exhausted, "pending refinement hits the budget");
        assert!(
            stats.remaining_bad_triangles > 0,
            "the report does not claim an incomplete pass is complete"
        );
        for code in [
            "eval.refinement.budget_exhausted",
            "eval.refinement.remaining_violations",
        ] {
            assert!(
                cold.report
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == code),
                "incomplete refinement explains {code}"
            );
        }
        assert!(!cold.report.clean_at(Severity::Warning));

        let warm = evaluate_with_cache(&recipe, &policy, &mut cache).expect("warm evaluation");
        assert_eq!(warm.report.refinements, cold.report.refinements);
        assert_eq!(warm.report.diagnostics, cold.report.diagnostics);
        assert_eq!(warm.report.counters.tessellations, 0);
        assert_eq!(warm.report.counters.cache_hits, 1);
    }

    #[test]
    fn planar_face_composes_through_transform_mirror_and_instance() {
        // This chain traps all placement-bearing parents at once. The proper
        // instance placement must reuse the reflected definition, while the
        // inner mirror repairs winding and the transform moves the face before
        // reflection; none of those steps may turn it into opaque geometry.
        let mut builder = RecipeBuilder::new();
        let profile = builder.add_profile(builders::rect(2.0, 1.0).expect("rectangle"));
        let face = builder
            .add(NodeKind::PlanarFace {
                profile,
                placement: Placement3::IDENTITY,
            })
            .expect("planar face");
        let transformed = builder
            .add(NodeKind::Transform {
                child: face,
                xf: Placement3::rotate_z_then_translate(
                    core::f64::consts::FRAC_PI_2,
                    3.0,
                    0.0,
                    0.0,
                ),
            })
            .expect("transformed face");
        let mirrored = builder
            .add(NodeKind::Mirror {
                child: transformed,
                plane: Plane3 {
                    normal: [1.0, 0.0, 0.0],
                    distance: 0.0,
                },
            })
            .expect("mirrored face");
        let instance = builder
            .add(NodeKind::Instance {
                of: mirrored,
                placement: Placement3::translate(10.0, 0.0, 0.0),
            })
            .expect("instance");
        let recipe = builder.finish(instance).expect("valid composition");

        let result = evaluate(&recipe, &EvalPolicy::default()).expect("composition evaluates");
        assert_eq!(result.bodies.len(), 1);
        let body = &result.bodies[0].body;
        assert_eq!(
            mesh_bounds(&body.mesh),
            Aabb3 {
                min: [7.0, 0.0, 0.0],
                max: [8.0, 2.0, 0.0],
            }
        );
        for mesh_face in body.mesh.faces() {
            assert!(
                face_cross_z(&body.mesh, mesh_face) > 0.0,
                "mirror repairs the reflected triangle winding"
            );
            assert_eq!(
                body.source_map.face_feature(mesh_face),
                Some(crate::tessellate::Feature::PlanarFace)
            );
        }
    }

    #[test]
    fn stretch_refuses_a_planar_face_crossing_as_an_open_shell() {
        // A surface that crosses the stretch plane has no closed section loop
        // from which to construct a band. PlanarFace must reach the ordinary
        // mesh stretch refusal rather than being mistaken for a solid recipe.
        // Its resolved source must remain available after the recipe is gone.
        let mut builder = RecipeBuilder::new();
        let source = builder.source_ref("o1.o2");
        let profile = builder.add_profile(builders::rect(2.0, 1.0).expect("rectangle"));
        let face = builder
            .add(NodeKind::PlanarFace {
                profile,
                placement: Placement3::IDENTITY,
            })
            .expect("planar face");
        let stretch = builder
            .with_source(source)
            .add(NodeKind::Stretch {
                child: face,
                plane: Plane3 {
                    normal: [1.0, 0.0, 0.0],
                    distance: 1.0,
                },
                length: 1.0,
            })
            .expect("stretch node");
        let recipe = builder.finish(stretch).expect("valid recipe");

        let result = evaluate(&recipe, &EvalPolicy::default()).expect("refusal is reported");
        drop(recipe);
        assert!(result.bodies.is_empty());
        assert_eq!(
            result.report.fidelity_of(stretch),
            Some(Fidelity::EnvelopeOnly)
        );
        let diagnostic = result
            .report
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "eval.stretch.open_shell")
            .expect("typed open-shell refusal");
        assert_eq!(diagnostic.node, Some(stretch));
        assert_eq!(diagnostic.source.as_deref(), Some("o1.o2"));
    }

    #[test]
    fn axis_contact_revolve_evaluates_as_one_exact_body() {
        // This evaluator-level trap complements the topology tests: the
        // supported axis contact must remain an ordinary exact constructive
        // body with placed bounds, not a tessellator-only special case.
        use crate::profile::{Loop2, Profile2, Seg2};

        let outer = Loop2::new(vec![Seg2::arc((0.0, 1.0), 1.0), Seg2::line((0.0, -1.0))])
            .expect("semicircle loop");
        let mut builder = RecipeBuilder::new();
        let profile = builder.add_profile(Profile2::simple(outer).expect("half-disc profile"));
        let revolve = builder
            .add(NodeKind::Revolve {
                profile,
                placement: Placement3::translate(5.0, 7.0, 11.0),
                sweep: core::f64::consts::TAU,
                caps: CapMode::Both,
            })
            .expect("valid axis-contact revolve");
        let recipe = builder.finish(revolve).expect("valid recipe");

        let result = evaluate(&recipe, &EvalPolicy::default()).expect("revolve evaluates");
        assert_eq!(result.bodies.len(), 1);
        assert_eq!(result.report.fidelity_of(revolve), Some(Fidelity::Exact));
        assert!(result.report.clean_at(Severity::Warning));
        assert_eq!(
            mesh_bounds(&result.bodies[0].body.mesh),
            Aabb3 {
                min: [4.0, 6.0, 10.0],
                max: [6.0, 8.0, 12.0],
            }
        );
    }

    #[test]
    fn mirror_over_import_repairs_winding_and_preserves_provenance() {
        // Mirror is the sanctioned recipe-space reflection. Imported geometry
        // must follow the same contract as constructive bodies: positions
        // reflect, loops reverse, and the opaque imported feature survives.
        let mirrored = mirrored_import_recipe(imported_unit_box());

        let result = evaluate(&mirrored, &EvalPolicy::default()).expect("mirror evaluates");
        assert_eq!(result.bodies.len(), 1);
        assert!(result.report.clean_at(Severity::Warning));
        let body = &result.bodies[0].body;
        assert!(body.mesh.validate_deep().is_empty());
        assert_eq!(
            mesh_bounds(&body.mesh),
            Aabb3 {
                min: [-1.0, 0.0, 0.0],
                max: [0.0, 1.0, 1.0],
            }
        );
        assert_eq!(signed_mesh_volume(&body.mesh), 1.0, "winding stays outward");
        assert!(
            body.source_map
                .face_features()
                .iter()
                .all(|feature| *feature == crate::tessellate::Feature::Imported)
        );
        assert!(body.mesh.vertices().all(|vertex| {
            body.source_map.vertex_feature(vertex) == Some(crate::tessellate::Feature::Imported)
        }));
        body.source_map
            .check(&body.mesh)
            .expect("rebuilt provenance is pinned to the output revision");
        assert!(
            body.mesh
                .attrs()
                .sparse(exedra_mesh::attr::CORNER_UV)
                .is_none(),
            "reflection must not invent an unauthored sparse layer"
        );
    }

    #[test]
    fn mirror_over_open_import_preserves_its_boundary() {
        // Reflection repairs orientation without changing dimensionality: an
        // authored single-sided import remains one open face with one boundary
        // loop, rather than being rejected or implicitly solidified.
        let triangle = exedra_mesh::Mesh::from_polygons(
            &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            &[&[0, 1, 2]],
        )
        .expect("open triangle is valid");
        let recipe = mirrored_import_recipe(triangle);

        let result = evaluate(&recipe, &EvalPolicy::default()).expect("mirror evaluates");
        assert_eq!(result.bodies.len(), 1);
        let mesh = &result.bodies[0].body.mesh;
        assert!(mesh.validate_deep().is_empty());
        assert_eq!(mesh.faces().count(), 1);
        assert_eq!(mesh.boundary_loops().expect("valid boundary").len(), 1);
        let face = mesh.faces().next().expect("one output face");
        assert!(
            face_cross_z(mesh, face) > 0.0,
            "reversed loop retains the source face's +Z orientation"
        );
    }

    #[test]
    fn reflecting_non_uniform_import_preserves_built_in_attributes() {
        // This traps the full remapping contract rather than only geometry:
        // attributes must follow their face, undirected edge, vertex, or
        // face-corner owner, while normals use inverse-transpose transport.
        let mut builder = RecipeBuilder::new();
        let import = builder
            .add_import(attributed_imported_unit_box())
            .expect("deep-valid attributed import");
        let imported = builder
            .add(NodeKind::MeshImport {
                import,
                placement: Placement3::from_axes(
                    [-2.0, 0.0, 0.0],
                    [0.0, 3.0, 0.0],
                    [0.0, 0.0, 4.0],
                    [0.0, 0.0, 0.0],
                ),
            })
            .expect("reflecting scaled import");
        let recipe = builder.finish(imported).expect("valid recipe");

        let result = evaluate(&recipe, &EvalPolicy::default()).expect("import evaluates");
        assert_eq!(result.bodies.len(), 1);
        assert!(result.report.clean_at(Severity::Warning));
        let mesh = &result.bodies[0].body.mesh;
        assert!(mesh.validate_deep().is_empty());
        assert_eq!(
            mesh_bounds(mesh),
            Aabb3 {
                min: [-2.0, 0.0, 0.0],
                max: [0.0, 3.0, 4.0],
            }
        );
        assert_eq!(signed_mesh_volume(mesh), 24.0, "winding stays outward");

        let regions = mesh
            .attrs()
            .dense(exedra_mesh::attr::FACE_REGION)
            .expect("built-in regions");
        let tagged_face = mesh
            .faces()
            .find(|face| regions.get((*face).into()) == Some(&23))
            .expect("face region follows its face");
        let uvs = mesh
            .attrs()
            .sparse(exedra_mesh::attr::CORNER_UV)
            .expect("authored UV layer survives");
        let tagged_corner = mesh
            .face_loop(tagged_face)
            .find(|corner| uvs.get((*corner).into()) == Some(&[0.25, 0.75]))
            .expect("UV follows its geometric face corner");
        let tagged_vertex = mesh
            .to_vertex(tagged_corner)
            .expect("tagged corner has a destination");
        assert_eq!(
            mesh.vertex_position(tagged_vertex),
            Some(&[-2.0, 3.0, 0.0]),
            "corner value remains attached to the transformed source vertex"
        );
        assert_eq!(mesh.vertex_sharpness(tagged_vertex), Some(2.5));

        let tagged_edge = mesh
            .faces()
            .flat_map(|face| mesh.face_loop(face))
            .find(|edge| mesh.edge_seam(*edge) == Some(true))
            .expect("seam follows its undirected geometric edge");
        assert_eq!(mesh.edge_sharpness(tagged_edge), Some(1.75));
        let from = mesh
            .from_vertex(tagged_edge)
            .and_then(|vertex| mesh.vertex_position(vertex))
            .copied()
            .expect("tagged edge has an origin");
        let to = mesh
            .to_vertex(tagged_edge)
            .and_then(|vertex| mesh.vertex_position(vertex))
            .copied()
            .expect("tagged edge has a destination");
        assert!(
            [from, to].contains(&[-2.0, 3.0, 0.0]) && [from, to].contains(&[0.0, 3.0, 0.0]),
            "edge values remain attached to the transformed source edge"
        );

        let normals = mesh
            .attrs()
            .sparse(exedra_mesh::attr::CORNER_NORMAL_OVERRIDE)
            .expect("authored normal layer survives");
        let normal = normals
            .get(tagged_corner.into())
            .expect("normal follows its geometric face corner");
        let expected = [-3.0 / 13.0_f32.sqrt(), 2.0 / 13.0_f32.sqrt(), 0.0];
        for axis in 0..3 {
            assert!(
                (normal[axis] - expected[axis]).abs() <= 2.0 * f32::EPSILON,
                "axis {axis}: {normal:?} != {expected:?}"
            );
        }
    }

    #[test]
    fn proper_non_uniform_import_transform_remains_supported() {
        // Supporting sanctioned reflections must not narrow the existing
        // affine-import path: positive-determinant non-uniform transforms
        // still transform the body directly and retain exact fidelity.
        let mut builder = RecipeBuilder::new();
        let import = builder
            .add_import(imported_unit_box())
            .expect("deep-valid import");
        let imported = builder
            .add(NodeKind::MeshImport {
                import,
                placement: Placement3::from_axes(
                    [2.0, 0.0, 0.0],
                    [0.0, 3.0, 0.0],
                    [0.0, 0.0, 4.0],
                    [1.0, -2.0, 5.0],
                ),
            })
            .expect("scaled import");
        let recipe = builder.finish(imported).expect("valid recipe");

        let result = evaluate(&recipe, &EvalPolicy::default()).expect("import evaluates");
        assert_eq!(result.bodies.len(), 1);
        assert_eq!(result.report.fidelity_of(imported), Some(Fidelity::Exact));
        assert_eq!(
            mesh_bounds(&result.bodies[0].body.mesh),
            Aabb3 {
                min: [1.0, -2.0, 5.0],
                max: [3.0, 1.0, 9.0],
            }
        );
    }

    #[test]
    fn mirrored_import_is_bit_identical_across_cache_reuse() {
        // A reflected rebuild is cached under the same content/world/policy
        // contract as other bodies; warm reuse must return identical topology,
        // attributes, and provenance without rebuilding it.
        let recipe = mirrored_import_recipe(attributed_imported_unit_box());
        let policy = EvalPolicy::default();
        let pure = evaluate(&recipe, &policy).expect("pure evaluation");
        let mut cache = EvalCache::new();
        let cold = evaluate_with_cache(&recipe, &policy, &mut cache).expect("cold evaluation");
        let warm = evaluate_with_cache(&recipe, &policy, &mut cache).expect("warm evaluation");

        let pure_body = &pure.bodies[0].body;
        let cold_body = &cold.bodies[0].body;
        let warm_body = &warm.bodies[0].body;
        assert_eq!(
            exedra_testkit::dump_mesh_topology(&pure_body.mesh),
            exedra_testkit::dump_mesh_topology(&cold_body.mesh)
        );
        assert_eq!(
            exedra_testkit::dump_mesh_topology(&cold_body.mesh),
            exedra_testkit::dump_mesh_topology(&warm_body.mesh)
        );
        assert_eq!(
            exedra_testkit::dump_attributes(&pure_body.mesh),
            exedra_testkit::dump_attributes(&warm_body.mesh)
        );
        assert_eq!(pure_body.source_map, warm_body.source_map);
        assert_eq!(cold.report.counters.cache_misses, 1);
        assert_eq!(warm.report.counters.cache_hits, 1);
        assert_eq!(warm.report.counters.tessellations, 0);
    }

    #[test]
    fn declared_box_primitive_evaluates_as_one_exact_body() {
        // PrimitiveSpec::Box is already accepted by the public IR. This test
        // pins the missing evaluator contract: its minimum corner is the
        // local origin, its placement composes normally, and it must not fall
        // through to `eval.unimplemented` after successful recipe validation.
        let mut builder = RecipeBuilder::new();
        let primitive = builder
            .add(NodeKind::Primitive {
                spec: PrimitiveSpec::Box {
                    size: [2.0, 3.0, 4.0],
                },
                placement: Placement3::translate(5.0, 7.0, 11.0),
            })
            .expect("the public IR accepts a positive finite box");
        let recipe = builder.finish(primitive).expect("valid primitive recipe");

        let result = evaluate(&recipe, &EvalPolicy::default()).expect("box evaluates");
        assert_eq!(result.bodies.len(), 1);
        assert_eq!(result.report.fidelity_of(primitive), Some(Fidelity::Exact));
        assert_eq!(result.report.counters.unimplemented, 0);
        assert!(result.report.clean_at(Severity::Warning));
        assert_eq!(
            mesh_bounds(&result.bodies[0].body.mesh),
            Aabb3 {
                min: [5.0, 7.0, 11.0],
                max: [7.0, 10.0, 15.0],
            }
        );
    }

    #[test]
    fn declared_cylinder_primitive_uses_the_documented_z_axis() {
        // The public cylinder contract places its base centre at the origin
        // and grows along +Z. A direct primitive backend may use another
        // native axis internally, but that convention must not leak through
        // constructive evaluation.
        let mut builder = RecipeBuilder::new();
        let primitive = builder
            .add(NodeKind::Primitive {
                spec: PrimitiveSpec::Cylinder {
                    radius: 2.0,
                    height: 5.0,
                    segments: 16,
                },
                placement: Placement3::translate(7.0, 11.0, 13.0),
            })
            .expect("the public IR accepts a positive finite cylinder");
        let recipe = builder.finish(primitive).expect("valid primitive recipe");

        let result = evaluate(&recipe, &EvalPolicy::default()).expect("cylinder evaluates");
        assert_eq!(result.bodies.len(), 1);
        assert_eq!(result.report.fidelity_of(primitive), Some(Fidelity::Exact));
        assert_eq!(result.report.counters.unimplemented, 0);
        assert!(result.report.clean_at(Severity::Warning));
        let bounds = mesh_bounds(&result.bodies[0].body.mesh);
        for (actual, expected) in bounds.min.into_iter().zip([5.0, 9.0, 13.0]) {
            assert!((actual - expected).abs() < 1.0e-5, "min bound {bounds:?}");
        }
        for (actual, expected) in bounds.max.into_iter().zip([9.0, 13.0, 18.0]) {
            assert!((actual - expected).abs() < 1.0e-5, "max bound {bounds:?}");
        }
    }

    #[test]
    fn declared_primitives_participate_in_csg() {
        // Standalone primitive output is insufficient for constructive IR:
        // this through-cut proves primitive meshes reach the ordinary Boolean
        // pipeline with valid closed topology and produce an exact body.
        let mut builder = RecipeBuilder::new();
        let host = builder
            .add(NodeKind::Primitive {
                spec: PrimitiveSpec::Box {
                    size: [4.0, 4.0, 4.0],
                },
                placement: Placement3::IDENTITY,
            })
            .expect("valid host primitive");
        let cutter = builder
            .add(NodeKind::Primitive {
                spec: PrimitiveSpec::Box {
                    size: [1.0, 1.0, 6.0],
                },
                placement: Placement3::translate(1.25, 1.5, -1.0),
            })
            .expect("valid cutter primitive");
        let difference = builder
            .add(NodeKind::Csg {
                op: CsgOp::Difference,
                operands: vec![host, cutter],
            })
            .expect("valid difference");
        let recipe = builder.finish(difference).expect("valid recipe");

        let result = evaluate(&recipe, &EvalPolicy::default()).expect("CSG evaluates");
        assert_eq!(result.bodies.len(), 1);
        assert_eq!(result.report.fidelity_of(difference), Some(Fidelity::Exact));
        assert!(result.report.clean_at(Severity::Warning));
        let mesh = &result.bodies[0].body.mesh;
        assert!(mesh.validate_deep().is_empty());
        assert!(
            mesh.attrs().dense(exedra_mesh::attr::FACE_REGION).is_some(),
            "primitive regions must reach the Boolean result"
        );
    }

    #[test]
    fn declared_primitive_reuses_the_incremental_evaluation_cache() {
        // Primitive nodes use the same content/world/policy cache contract as
        // every other body node: a second identical evaluation must reuse the
        // body without performing another tessellation.
        let mut builder = RecipeBuilder::new();
        let primitive = builder
            .add(NodeKind::Primitive {
                spec: PrimitiveSpec::Cylinder {
                    radius: 2.0,
                    height: 5.0,
                    segments: 16,
                },
                placement: Placement3::translate(3.0, 4.0, 5.0),
            })
            .expect("valid primitive");
        let recipe = builder.finish(primitive).expect("valid recipe");
        let mut cache = EvalCache::new();

        let cold = evaluate_with_cache(&recipe, &EvalPolicy::default(), &mut cache)
            .expect("cold evaluation");
        let warm = evaluate_with_cache(&recipe, &EvalPolicy::default(), &mut cache)
            .expect("warm evaluation");
        assert_eq!(cold.report.counters.tessellations, 1);
        assert_eq!(cold.report.counters.cache_misses, 1);
        assert_eq!(warm.report.counters.tessellations, 0);
        assert_eq!(warm.report.counters.cache_hits, 1);
        assert_eq!(
            mesh_bounds(&cold.bodies[0].body.mesh),
            mesh_bounds(&warm.bodies[0].body.mesh)
        );
    }

    #[test]
    fn csg_difference_emits_exact_geometry() {
        // Two overlapping box extrusions exercise the ordinary CSG path:
        // supported geometry must be emitted as an exact, deeply valid body
        // without the old envelope-only fallback diagnostic.
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
        assert_eq!(result.bodies.len(), 1);
        assert_eq!(result.report.fidelity_of(csg), Some(Fidelity::Exact));
        assert!(result.bodies[0].body.mesh.validate_deep().is_empty());
        assert!(result.report.clean_at(Severity::Warning));
    }

    #[test]
    fn csg_intersects_overlapping_curved_imports() {
        // Reusing one tessellated sphere import at two placements exercises
        // the constructive path that previously surfaced a dangling-cut
        // refusal. The closed result and clean report pin both the mesh
        // Boolean repair and its diagnostic contract at this layer.
        let mut builder = RecipeBuilder::new();
        let sphere = exedra_primitives::uv_sphere(&exedra_primitives::UvSphereParams::default());
        let import = builder
            .add_import(sphere.mesh)
            .expect("sphere import is deep-valid");
        let left = builder
            .add(NodeKind::MeshImport {
                import,
                placement: Placement3::translate(-0.8, 0.0, 0.0),
            })
            .expect("left sphere import");
        let right = builder
            .add(NodeKind::MeshImport {
                import,
                placement: Placement3::translate(0.8, 0.0, 0.0),
            })
            .expect("right sphere import");
        let intersection = builder
            .add(NodeKind::Csg {
                op: CsgOp::Intersection,
                operands: vec![left, right],
            })
            .expect("valid intersection");
        let recipe = builder.finish(intersection).expect("valid recipe");

        let result = evaluate(&recipe, &EvalPolicy::default()).expect("evaluation is total");
        assert_eq!(result.bodies.len(), 1);
        assert_eq!(
            result.report.fidelity_of(intersection),
            Some(Fidelity::Exact)
        );
        assert!(result.report.clean_at(Severity::Warning));
        let mesh = &result.bodies[0].body.mesh;
        assert!(mesh.validate_deep().is_empty());
        assert!(signed_mesh_volume(mesh) > 0.0);
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
    fn composed_mirror_preserves_provenance_and_outward_orientation() {
        // Recipe composition must reflect the L-prism without changing its
        // producer identity/source map or turning its closed solid inside out.
        let mut b = RecipeBuilder::new();
        let source = b.source_ref("test:rafter/body");
        let p = b.add_profile(builders::l_profile(1.0, 1.0, 0.5, 0.5).expect("L"));
        let body = b
            .with_source(source)
            .add(NodeKind::Extrude {
                profile: p,
                placement: Placement3::IDENTITY,
                height: 2.0,
                caps: CapMode::Both,
            })
            .expect("valid");
        let recipe = b.finish(body).expect("valid recipe");
        let original = evaluate(&recipe, &EvalPolicy::default()).expect("evaluates");
        let mirrored = recipe
            .mirrored(Plane3 {
                // Non-unit form of x = 2 also pins normal/distance scaling.
                normal: [2.0, 0.0, 0.0],
                distance: 4.0,
            })
            .expect("valid mirror plane");
        let result = evaluate(&mirrored, &EvalPolicy::default()).expect("evaluates");
        assert_eq!(result.bodies.len(), 1);
        assert_eq!(result.bodies[0].node, body);
        assert_eq!(mirrored.source(source), Some("test:rafter/body"));
        assert_eq!(
            result.bodies[0].body.source_map.dump(),
            original.bodies[0].body.source_map.dump()
        );
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
        // Original x=[0, 1] reflected across x=2 becomes x=[3, 4].
        let min_x = mesh
            .vertices()
            .filter_map(|v| mesh.vertex_position(v))
            .map(|p| p[0])
            .fold(f32::INFINITY, f32::min);
        let max_x = mesh
            .vertices()
            .filter_map(|v| mesh.vertex_position(v))
            .map(|p| p[0])
            .fold(f32::NEG_INFINITY, f32::max);
        assert!(
            (min_x - 3.0).abs() <= 1e-6 && (max_x - 4.0).abs() <= 1e-6,
            "mirror reflected across x = 2, x bounds [{min_x}, {max_x}]"
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
mod multi_cutter_regression {
    use alloc::format;
    use alloc::vec;
    use alloc::vec::Vec;

    use super::*;
    use crate::builders;
    use crate::ir::{CapMode, CsgOp, NodeKind, Placement3, RecipeBuilder};
    use crate::tessellate::EvalPolicy;

    #[derive(Copy, Clone)]
    struct BoxSpec {
        size: [f64; 3],
        origin: [f64; 3],
    }

    #[derive(Copy, Clone, Debug)]
    enum DifferenceForm {
        Nary,
        Chained,
    }

    fn add_box(builder: &mut RecipeBuilder, size: [f64; 3], origin: [f64; 3]) -> NodeId {
        let profile = builder.add_profile(builders::rect(size[0], size[1]).expect("valid box"));
        builder
            .add(NodeKind::Extrude {
                profile,
                placement: Placement3::translate(origin[0], origin[1], origin[2]),
                height: size[2],
                caps: CapMode::Both,
            })
            .expect("valid box")
    }

    fn difference_recipe(cutters: &[BoxSpec], form: DifferenceForm, scale: f64) -> Recipe {
        let mut builder = RecipeBuilder::new();
        let base = add_box(
            &mut builder,
            [6.0 * scale, 0.6 * scale, 4.0 * scale],
            [0.0, 0.0, 0.0],
        );
        let cutters: Vec<NodeId> = cutters
            .iter()
            .map(|spec| add_box(&mut builder, spec.size, spec.origin))
            .collect();
        let root = match form {
            DifferenceForm::Nary => {
                let mut operands = Vec::with_capacity(cutters.len() + 1);
                operands.push(base);
                operands.extend(cutters);
                builder
                    .add(NodeKind::Csg {
                        op: CsgOp::Difference,
                        operands,
                    })
                    .expect("valid n-ary difference")
            }
            DifferenceForm::Chained => cutters.into_iter().fold(base, |left, right| {
                builder
                    .add(NodeKind::Csg {
                        op: CsgOp::Difference,
                        operands: vec![left, right],
                    })
                    .expect("valid chained difference")
            }),
        };
        builder.finish(root).expect("valid recipe")
    }

    /// Computes the exact set-theoretic volume of the axis-aligned fixture by
    /// partitioning at every host/cutter coordinate. This is independent of
    /// both CSG spellings and catches a clean, one-body result with the wrong
    /// retained region.
    fn expected_volume(cutters: &[BoxSpec], scale: f64) -> f64 {
        let host_max = [6.0 * scale, 0.6 * scale, 4.0 * scale];
        let mut coordinates = [
            vec![0.0, host_max[0]],
            vec![0.0, host_max[1]],
            vec![0.0, host_max[2]],
        ];
        for cutter in cutters {
            for axis in 0..3 {
                let min = cutter.origin[axis].clamp(0.0, host_max[axis]);
                let max = (cutter.origin[axis] + cutter.size[axis]).clamp(0.0, host_max[axis]);
                coordinates[axis].extend([min, max]);
            }
        }
        for axis in &mut coordinates {
            axis.sort_by(f64::total_cmp);
            axis.dedup();
        }

        let mut volume = 0.0;
        for x in coordinates[0].windows(2) {
            for y in coordinates[1].windows(2) {
                for z in coordinates[2].windows(2) {
                    let midpoint = [
                        (x[0] + x[1]) * 0.5,
                        (y[0] + y[1]) * 0.5,
                        (z[0] + z[1]) * 0.5,
                    ];
                    let removed = cutters.iter().any(|cutter| {
                        (0..3).all(|axis| {
                            let min = cutter.origin[axis];
                            let max = min + cutter.size[axis];
                            (min..max).contains(&midpoint[axis])
                        })
                    });
                    if !removed {
                        volume += (x[1] - x[0]) * (y[1] - y[0]) * (z[1] - z[0]);
                    }
                }
            }
        }
        volume
    }

    /// Signed volume via the divergence theorem over each planar face fan.
    fn mesh_volume(mesh: &exedra_mesh::Mesh) -> f64 {
        let mut volume = 0.0;
        for face in mesh.faces() {
            let points: Vec<[f64; 3]> = mesh
                .face_loop(face)
                .filter_map(|half_edge| mesh.to_vertex(half_edge))
                .filter_map(|vertex| mesh.vertex_position(vertex))
                .map(|point| point.map(f64::from))
                .collect();
            for index in 1..points.len().saturating_sub(1) {
                let (a, b, c) = (points[0], points[index], points[index + 1]);
                volume += a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                    + a[2] * (b[0] * c[1] - b[1] * c[0]);
            }
        }
        volume / 6.0
    }

    /// Representative unit-scale axis-aligned box cutters remain exact across
    /// two through six cutters, selected operand permutations,
    /// proximity/contact/overlap, and n-ary versus chained CSG spelling.
    #[test]
    fn axis_aligned_multi_cutter_differences_produce_one_body() {
        let well_separated = [
            BoxSpec {
                size: [0.6, 0.8, 1.0],
                origin: [0.3, -0.1, 1.5],
            },
            BoxSpec {
                size: [0.6, 0.8, 1.0],
                origin: [1.5, -0.1, 1.5],
            },
            BoxSpec {
                size: [0.6, 0.8, 1.0],
                origin: [2.7, -0.1, 1.5],
            },
            BoxSpec {
                size: [0.6, 0.8, 1.0],
                origin: [3.9, -0.1, 1.5],
            },
            BoxSpec {
                size: [0.6, 0.8, 1.0],
                origin: [5.1, -0.1, 1.5],
            },
        ];
        let six_well_separated = [
            BoxSpec {
                size: [0.4, 0.8, 1.0],
                origin: [0.2, -0.1, 1.5],
            },
            BoxSpec {
                size: [0.4, 0.8, 1.0],
                origin: [1.1, -0.1, 1.5],
            },
            BoxSpec {
                size: [0.4, 0.8, 1.0],
                origin: [2.0, -0.1, 1.5],
            },
            BoxSpec {
                size: [0.4, 0.8, 1.0],
                origin: [2.9, -0.1, 1.5],
            },
            BoxSpec {
                size: [0.4, 0.8, 1.0],
                origin: [3.8, -0.1, 1.5],
            },
            BoxSpec {
                size: [0.4, 0.8, 1.0],
                origin: [4.7, -0.1, 1.5],
            },
        ];
        let flush = [
            BoxSpec {
                size: [1.2, 0.8, 1.4],
                origin: [2.4, -0.1, 1.8],
            },
            BoxSpec {
                size: [1.6, 0.8, 0.2],
                origin: [2.2, -0.1, 1.6],
            },
        ];
        let interpenetrating = [
            BoxSpec {
                size: [1.2, 0.8, 1.4],
                origin: [2.4, -0.1, 1.8],
            },
            BoxSpec {
                size: [1.6, 0.8, 0.4],
                origin: [2.2, -0.1, 1.6],
            },
        ];
        let close_disjoint = [
            BoxSpec {
                size: [1.2, 0.8, 0.5],
                origin: [2.4, -0.1, 0.8],
            },
            BoxSpec {
                size: [1.4, 0.8, 0.5],
                origin: [2.3, -0.1, 1.31],
            },
            BoxSpec {
                size: [1.6, 0.8, 0.5],
                origin: [2.2, -0.1, 1.82],
            },
        ];
        let mut flush_reversed = flush;
        flush_reversed.reverse();
        let mut interpenetrating_reversed = interpenetrating;
        interpenetrating_reversed.reverse();
        let mut close_disjoint_reversed = close_disjoint;
        close_disjoint_reversed.reverse();
        let cases = [
            ("two-well-separated", &well_separated[..2]),
            ("three-well-separated", &well_separated[..3]),
            ("four-well-separated", &well_separated[..4]),
            ("five-well-separated", &well_separated[..]),
            ("six-well-separated", &six_well_separated[..]),
            ("flush", &flush[..]),
            ("flush-reversed", &flush_reversed[..]),
            ("interpenetrating", &interpenetrating[..]),
            ("interpenetrating-reversed", &interpenetrating_reversed[..]),
            ("close-disjoint", &close_disjoint[..]),
            ("close-disjoint-reversed", &close_disjoint_reversed[..]),
        ];

        let mut failures = Vec::new();
        let scale = 1.0;
        for (name, cutters) in cases {
            let expected_volume = expected_volume(cutters, scale);
            for form in [DifferenceForm::Nary, DifferenceForm::Chained] {
                let recipe = difference_recipe(cutters, form, scale);
                let result = evaluate(&recipe, &EvalPolicy::default()).expect("evaluates");
                let pipeline_clean = result
                    .report
                    .diagnostics
                    .iter()
                    .all(|diagnostic| diagnostic.code != "eval.csg.pipeline");
                let actual_volume = result
                    .bodies
                    .first()
                    .map_or(f64::NAN, |body| mesh_volume(&body.body.mesh));
                let volume_tolerance = expected_volume.abs() * 1.0e-5 + 1.0e-12;
                if result.bodies.len() != 1
                    || !pipeline_clean
                    || (actual_volume - expected_volume).abs() > volume_tolerance
                {
                    failures.push(format!(
                            "scale={scale} {name}/{form:?}: bodies={}, volume={actual_volume}, expected={expected_volume}, diagnostics={:?}",
                        result.bodies.len(),
                        result.report.diagnostics
                    ));
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// Scale extremes may remain outside the supported boolean envelope, but
    /// they must either produce the exact analytic volume or refuse
    /// explicitly—never panic or return a clean, wrong body.
    #[test]
    fn scale_and_flush_order_stress_is_exact_or_typed() {
        let flush = [
            BoxSpec {
                size: [1.2, 0.8, 1.4],
                origin: [2.4, -0.1, 1.8],
            },
            BoxSpec {
                size: [1.6, 0.8, 0.2],
                origin: [2.2, -0.1, 1.6],
            },
        ];
        let mut flush_reversed = flush;
        flush_reversed.reverse();
        let five = [
            BoxSpec {
                size: [0.6, 0.8, 1.0],
                origin: [0.3, -0.1, 1.5],
            },
            BoxSpec {
                size: [0.6, 0.8, 1.0],
                origin: [1.5, -0.1, 1.5],
            },
            BoxSpec {
                size: [0.6, 0.8, 1.0],
                origin: [2.7, -0.1, 1.5],
            },
            BoxSpec {
                size: [0.6, 0.8, 1.0],
                origin: [3.9, -0.1, 1.5],
            },
            BoxSpec {
                size: [0.6, 0.8, 1.0],
                origin: [5.1, -0.1, 1.5],
            },
        ];
        for (name, cutters, scale) in [
            ("milli-flush", &flush[..], 1.0e-3),
            ("milli-flush-reversed", &flush_reversed[..], 1.0e-3),
            ("kilo-five-cutters", &five[..], 1.0e4),
        ] {
            let scaled: Vec<BoxSpec> = cutters
                .iter()
                .map(|cutter| BoxSpec {
                    size: cutter.size.map(|coordinate| coordinate * scale),
                    origin: cutter.origin.map(|coordinate| coordinate * scale),
                })
                .collect();
            let expected_volume = expected_volume(&scaled, scale);
            for form in [DifferenceForm::Nary, DifferenceForm::Chained] {
                let recipe = difference_recipe(&scaled, form, scale);
                let result =
                    evaluate(&recipe, &EvalPolicy::default()).expect("evaluation is total");
                if let [body] = result.bodies.as_slice() {
                    let actual_volume = mesh_volume(&body.body.mesh);
                    let tolerance = expected_volume.abs() * 1.0e-5 + 1.0e-12;
                    assert!(
                        (actual_volume - expected_volume).abs() <= tolerance,
                        "{name}/{form:?}: volume={actual_volume}, expected={expected_volume}"
                    );
                    assert!(
                        result
                            .report
                            .diagnostics
                            .iter()
                            .all(|diagnostic| diagnostic.code != "eval.csg.pipeline"),
                        "{name}/{form:?}: exact geometry carried pipeline diagnostics"
                    );
                } else {
                    assert!(
                        result.bodies.is_empty(),
                        "{name}/{form:?}: partial body set"
                    );
                    assert!(
                        result
                            .report
                            .diagnostics
                            .iter()
                            .any(|diagnostic| diagnostic.code == "eval.csg.unsupported"),
                        "{name}/{form:?}: refusal was not explicit"
                    );
                }
            }
        }
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
                    .to_trimesh(&exedra_mesh::ExtractParams::default());
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
        assert_eq!(a.refinements, b.refinements, "refinement outcomes");
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
