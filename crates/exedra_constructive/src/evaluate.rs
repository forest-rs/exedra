// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Recipe evaluation: walking a frozen [`Recipe`] into tessellated bodies
//! plus an honest [`GeometryReport`].
//!
//! Evaluation is a pure function of `(recipe, policy)`: nodes are visited
//! deterministically, placements compose in f64, and every outcome —
//! including what could *not* be evaluated — lands in the report as typed
//! fidelity and diagnostics rather than silent approximation.

use alloc::string::String;
use alloc::vec::Vec;

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
#[derive(Debug)]
pub struct PlacedBody {
    /// The producing node.
    pub node: NodeId,
    /// The tessellated body (already placed in world space).
    pub body: TessellatedBody,
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
    let mut cx = EvalCx {
        recipe,
        policy,
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
    bodies: Vec<PlacedBody>,
    report: GeometryReport,
    /// Local-space evaluations of instanced definitions, keyed by node.
    instance_cache: hashbrown::HashMap<NodeId, alloc::rc::Rc<Vec<PlacedBody>>>,
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
                let body = tessellate_extrude(
                    self.recipe.profile(*profile).expect("validated profile id"),
                    &combined,
                    *height,
                    *caps,
                    self.policy,
                )
                .map_err(|error| EvalError {
                    node: node_id,
                    error,
                })?;
                let fidelity = self.body_fidelity(node_id, &[*profile]);
                Ok(self.finish_body(node_id, body, emit, fidelity))
            }
            NodeKind::Revolve {
                profile,
                placement,
                sweep,
                caps,
            } => {
                let combined = compose(world, placement);
                let body = tessellate_revolve(
                    self.recipe.profile(*profile).expect("validated profile id"),
                    &combined,
                    *sweep,
                    *caps,
                    self.policy,
                )
                .map_err(|error| EvalError {
                    node: node_id,
                    error,
                })?;
                let fidelity = self.body_fidelity(node_id, &[*profile]);
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
                let body =
                    tessellate_loft(&placed, caps, self.policy).map_err(|error| EvalError {
                        node: node_id,
                        error,
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
                let body = tessellate_sweep(
                    self.recipe.profile(profile).expect("validated profile id"),
                    world,
                    &points,
                    caps,
                    self.policy,
                )
                .map_err(|error| EvalError {
                    node: node_id,
                    error,
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
            NodeKind::Csg { op: _, operands } => {
                // Operands are evaluated for their envelopes only; no
                // operand geometry escapes an unevaluable CSG node, and no
                // fake combined geometry is emitted.
                let operands = operands.clone();
                let mut bounds = Aabb3::EMPTY;
                for operand in operands {
                    let b = self.walk(operand, world, false)?;
                    bounds.union(&b);
                }
                self.report.counters.envelope_only += 1;
                self.report.fidelity.push((node_id, Fidelity::EnvelopeOnly));
                if !bounds.is_empty() {
                    self.report.envelopes.push((node_id, bounds));
                }
                self.report.diagnostics.push(Diagnostic {
                    severity: Severity::Error,
                    code: "eval.csg.unsupported",
                    message: String::from(
                        "CSG evaluation requires the mesh boolean pipeline; \
                         only the operand envelope is reported",
                    ),
                    node: Some(node_id),
                });
                Ok(bounds)
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
                let source = self.recipe.import(*import).expect("validated import id");
                let mesh = transform_mesh(source, &placement);
                let face_features =
                    alloc::vec![crate::tessellate::Feature::Imported; mesh.faces().count()];
                let vertex_features =
                    alloc::vec![crate::tessellate::Feature::Imported; mesh.vertices().count()];
                let source_map =
                    crate::source_map::SourceMap::new(&mesh, face_features, vertex_features);
                let body = TessellatedBody { mesh, source_map };
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
                    alloc::rc::Rc::clone(cached)
                } else {
                    // Evaluate the definition once in local space.
                    let taken = core::mem::take(&mut self.bodies);
                    self.walk(of, &Placement3::IDENTITY, true)?;
                    let local: Vec<PlacedBody> = core::mem::replace(&mut self.bodies, taken);
                    let rc = alloc::rc::Rc::new(local);
                    self.instance_cache.insert(of, alloc::rc::Rc::clone(&rc));
                    rc
                };
                let mut bounds = Aabb3::EMPTY;
                for source in local.iter() {
                    let body = instantiate(&source.body, &placement);
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

    /// Records a successfully tessellated body and returns its bounds.
    fn finish_body(
        &mut self,
        node: NodeId,
        body: TessellatedBody,
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
        self.report.counters.tessellations += 1;
        if emit {
            self.report.counters.bodies += 1;
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
}
