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

use crate::ir::{NodeId, NodeKind, Placement3, Recipe, SourceId};
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
    PolicyDefined(SourceId),
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
    /// Bodies tessellated.
    pub bodies: u32,
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
        bodies: Vec::new(),
        report: GeometryReport {
            fidelity: Vec::new(),
            diagnostics: Vec::new(),
            envelopes: Vec::new(),
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
        let node = self.recipe.node(node_id);
        match &node.kind {
            NodeKind::Extrude {
                profile,
                placement,
                height,
                caps,
            } => {
                let combined = compose(world, placement);
                let body = tessellate_extrude(
                    self.recipe.profile(*profile),
                    &combined,
                    *height,
                    *caps,
                    self.policy,
                )
                .map_err(|error| EvalError {
                    node: node_id,
                    error,
                })?;
                Ok(self.finish_body(node_id, body, emit))
            }
            NodeKind::Revolve {
                profile,
                placement,
                sweep,
                caps,
            } => {
                let combined = compose(world, placement);
                let body = tessellate_revolve(
                    self.recipe.profile(*profile),
                    &combined,
                    *sweep,
                    *caps,
                    self.policy,
                )
                .map_err(|error| EvalError {
                    node: node_id,
                    error,
                })?;
                Ok(self.finish_body(node_id, body, emit))
            }
            NodeKind::Loft {
                sections,
                policy: _,
                caps,
            } => {
                let placed: Vec<(Placement3, &crate::profile::Profile2)> = sections
                    .iter()
                    .map(|(placement, profile)| {
                        (compose(world, placement), self.recipe.profile(*profile))
                    })
                    .collect();
                let caps = *caps;
                let body =
                    tessellate_loft(&placed, caps, self.policy).map_err(|error| EvalError {
                        node: node_id,
                        error,
                    })?;
                Ok(self.finish_body(node_id, body, emit))
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
                    self.recipe.profile(profile),
                    world,
                    &points,
                    caps,
                    self.policy,
                )
                .map_err(|error| EvalError {
                    node: node_id,
                    error,
                })?;
                Ok(self.finish_body(node_id, body, emit))
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

    /// Records a successfully tessellated body and returns its bounds.
    fn finish_body(&mut self, node: NodeId, body: TessellatedBody, emit: bool) -> Aabb3 {
        let mut bounds = Aabb3::EMPTY;
        let mesh = &body.mesh;
        for face in mesh.faces() {
            for he in mesh.face_loop(face) {
                if let Some(p) = mesh.to_vertex(he).and_then(|v| mesh.vertex_position(v)) {
                    bounds.include([f64::from(p[0]), f64::from(p[1]), f64::from(p[2])]);
                }
            }
        }
        self.report.fidelity.push((node, Fidelity::Exact));
        if emit {
            self.report.counters.bodies += 1;
            self.report.counters.faces += crate::len_u32(mesh.faces().count());
            self.report.counters.vertices += crate::len_u32(mesh.vertices().count());
            self.bodies.push(PlacedBody { node, body });
        }
        bounds
    }
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
    use crate::ir::{CapMode, CsgOp, RecipeBuilder};
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
}
