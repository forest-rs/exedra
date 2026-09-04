// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Provenance inspection payloads (`exedra-ops-inspect-v1`).
//!
//! An inspection payload lets an external viewer resolve any picked
//! triangle to its full construction chain — node, feature, source
//! reference, fidelity verdict, and policy/issue citations — without
//! linking Rust. The resolution path is:
//!
//! `triangle t` → `bodies[b].tri_face[t]` → `bodies[b].faces[f]` (the
//! feature and region) → the node named by `bodies[b].node` (scoped by
//! `bodies[b].part` when the scenario is an assembly) → the `nodes` table
//! entry (kind, fingerprint, source ref, material slot, issue citation).
//!
//! Every list is emitted in deterministic order: nodes ascend by
//! `(part, id)`, faces follow `mesh.faces()` order, triangles follow the
//! extraction order (`tri_face` and `region_ids` are parallel to the
//! index triples), and diagnostics keep report emission order. Running
//! the same scenario twice serializes byte-identically.
//!
//! Format evolution is additive-only within `exedra-ops-inspect-v1`, the
//! same policy as the bridge's other payloads.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use exedra_constructive::evaluate::{Evaluation, Fidelity, Severity, evaluate};
use exedra_constructive::ir::{
    CapMode, CsgOp, NodeId, NodeKind, Placement3, Recipe, RecipeBuilder,
};
use exedra_constructive::tessellate::{EvalPolicy, Feature};
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

use crate::{MeshBuffers, extract_mesh_buffers, matrix16, panel_trio_assembly};

/// The inspection payload format identifier.
pub const INSPECT_FORMAT: &str = "exedra-ops-inspect-v1";

/// Top-level inspection payload.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct InspectionResponse {
    /// Format identifier (`exedra-ops-inspect-v1`).
    pub format: String,
    /// Stable scenario name.
    pub scenario: String,
    /// Node tables of every inspected recipe, ascending by `(part, id)`.
    pub nodes: Vec<InspectionNode>,
    /// Tessellated bodies with per-face provenance.
    pub bodies: Vec<InspectionBody>,
    /// Placed instances referencing `bodies` (a single identity instance
    /// per body for plain recipe scenarios).
    pub instances: Vec<InspectionInstance>,
    /// Per-node fidelity verdicts, in report order.
    pub fidelity: Vec<InspectionFidelity>,
    /// Policy-defined curve usage, in report order.
    pub policy_curves: Vec<InspectionPolicyUse>,
    /// The diagnostics ledger, in report emission order.
    pub diagnostics: Vec<InspectionDiagnostic>,
    /// Aggregated evaluation counters (summed across inspected recipes).
    pub counters: InspectionCounters,
}

/// One recipe node, identified by `(part, id)`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct InspectionNode {
    /// Owning part key for assembly scenarios; `null` for plain recipes.
    pub part: Option<String>,
    /// Node id within its recipe.
    pub id: u32,
    /// Stable node-kind name (`extrude`, `csg`, `transform`, ...).
    pub kind: String,
    /// Content fingerprint as 32 hex digits.
    pub fingerprint: String,
    /// Opaque frontend source reference, when attached.
    pub source: Option<String>,
    /// Material slot name, when bound.
    pub material: Option<String>,
    /// Opaque spec-issue citation, when attached.
    pub issue: Option<String>,
}

/// Structured feature attribution for one face.
///
/// `kind` is always present; the numeric fields are `null` unless the
/// feature carries them.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct InspectionFeature {
    /// Feature kind (`cap_start`, `cap_end`, `wall`, `loft_wall`,
    /// `sweep_wall`, `imported`, `boolean_face`, `grid_patch`,
    /// `unknown`).
    pub kind: String,
    /// Profile loop: 0 = outer, `1 + i` = hole `i`.
    pub loop_index: Option<u32>,
    /// Source segment index within the loop.
    pub seg: Option<u32>,
    /// Band index for loft/sweep walls.
    pub band: Option<u32>,
    /// CSG operand index for boolean faces.
    pub operand: Option<u32>,
    /// Grid patch row.
    pub row: Option<u32>,
    /// Grid patch column.
    pub col: Option<u32>,
}

/// Per-face provenance, in `mesh.faces()` order.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct InspectionFace {
    /// The producing feature of the body's node.
    pub feature: InspectionFeature,
    /// The face's `FACE_REGION` value.
    pub region: u32,
}

/// One tessellated body with render buffers and provenance.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct InspectionBody {
    /// Owning part key for assembly scenarios; `null` for plain recipes.
    pub part: Option<String>,
    /// The producing node id (scoped by `part`).
    pub node: u32,
    /// Flattened render buffers (fan triangulation, face order).
    pub mesh: MeshBuffers,
    /// Per-face provenance, in extraction face order.
    pub faces: Vec<InspectionFace>,
    /// Triangle → face-ordinal mapping, parallel to the index triples.
    pub tri_face: Vec<u32>,
}

/// One placed instance of a body.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct InspectionInstance {
    /// Stable instance path (`root/child/leaf`), or the scenario name for
    /// plain recipe scenarios.
    pub path: String,
    /// Index into the response's `bodies`.
    pub body: u32,
    /// World matrix as 16 column-major values (glTF/Three.js layout).
    pub matrix: Vec<f64>,
}

/// One node's fidelity verdict.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct InspectionFidelity {
    /// Owning part key for assembly scenarios; `null` for plain recipes.
    pub part: Option<String>,
    /// The node id (scoped by `part`).
    pub node: u32,
    /// Verdict (`exact`, `policy_defined`, `conflicted`,
    /// `envelope_only`, `unknown`).
    pub verdict: String,
    /// Named curve policy for `policy_defined` verdicts.
    pub policy: Option<String>,
    /// Cited spec issue for `conflicted` verdicts.
    pub issue: Option<String>,
}

/// One policy-defined curve usage record.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct InspectionPolicyUse {
    /// Owning part key for assembly scenarios; `null` for plain recipes.
    pub part: Option<String>,
    /// The node that realized a curve under the policy.
    pub node: u32,
    /// The opaque policy name.
    pub policy: String,
}

/// One diagnostics-ledger entry.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct InspectionDiagnostic {
    /// Owning part key for assembly scenarios; `null` for plain recipes.
    pub part: Option<String>,
    /// Severity (`note`, `warning`, `error`).
    pub severity: String,
    /// Stable machine-readable code.
    pub code: String,
    /// Human-readable detail.
    pub message: String,
    /// The node concerned, when there is one.
    pub node: Option<u32>,
}

/// Aggregated evaluation counters.
#[derive(Copy, Clone, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
pub struct InspectionCounters {
    /// Bodies emitted.
    pub bodies: u32,
    /// Distinct tessellations performed.
    pub tessellations: u32,
    /// Total faces emitted.
    pub faces: u32,
    /// Total vertices emitted.
    pub vertices: u32,
    /// Nodes reported envelope-only.
    pub envelope_only: u32,
    /// Nodes skipped as unimplemented.
    pub unimplemented: u32,
    /// Source-map bytes retained.
    pub source_map_bytes: u64,
}

/// Returns the inspection scenario names as a JSON array.
#[wasm_bindgen]
#[must_use]
pub fn list_inspection_scenarios_json() -> String {
    serde_json::to_string(&["drilled_block", "policy_curve", "panel_trio"])
        .expect("scenario list should serialize")
}

/// Runs one inspection scenario and returns its `exedra-ops-inspect-v1`
/// payload as JSON.
///
/// # Errors
///
/// Returns a `JsValue` error string when the scenario is unknown, fails
/// to evaluate, or fails to serialize.
#[wasm_bindgen]
pub fn run_inspection_scenario_json(name: &str) -> Result<String, JsValue> {
    let response = run_inspection_impl(name).map_err(|err| JsValue::from_str(&err))?;
    serde_json::to_string(&response).map_err(|err| {
        JsValue::from_str(&format!("failed to serialize inspection response: {err}"))
    })
}

fn run_inspection_impl(name: &str) -> Result<InspectionResponse, String> {
    match name {
        "drilled_block" => inspect_recipe(name, &drilled_block_recipe()?),
        "policy_curve" => inspect_recipe(name, &policy_curve_recipe()?),
        "panel_trio" => inspect_panel_trio(),
        _ => Err(format!("unknown inspection scenario `{name}`")),
    }
}

/// A cylinder drilled clean through a slab, moved by a translation: the
/// through-hole boolean exercised end to end (genus-1 result, boolean
/// provenance on every face).
fn drilled_block_recipe() -> Result<Recipe, String> {
    use exedra_constructive::builders;

    let mut b = RecipeBuilder::new();
    let block = b.add_profile(builders::rect(200.0, 100.0).map_err(|e| format!("{e}"))?);
    let drill = b.add_profile(builders::circle(30.0).map_err(|e| format!("{e}"))?);
    let slab_src = b.source_ref("demo:drilled_block/slab");
    let e1 = b
        .with_source(slab_src)
        .add(NodeKind::Extrude {
            profile: block,
            placement: Placement3::IDENTITY,
            height: 80.0,
            caps: CapMode::Both,
        })
        .map_err(|e| format!("{e}"))?;
    let drill_src = b.source_ref("demo:drilled_block/drill");
    let e2 = b
        .with_source(drill_src)
        .add(NodeKind::Extrude {
            profile: drill,
            placement: Placement3::translate(130.0, 50.0, -20.0),
            height: 120.0,
            caps: CapMode::Both,
        })
        .map_err(|e| format!("{e}"))?;
    let csg_src = b.source_ref("demo:drilled_block/difference");
    let csg = b
        .with_source(csg_src)
        .add(NodeKind::Csg {
            op: CsgOp::Difference,
            operands: alloc::vec![e1, e2],
        })
        .map_err(|e| format!("{e}"))?;
    let moved = b
        .add(NodeKind::Transform {
            child: csg,
            xf: Placement3::translate(50.0, 0.0, 0.0),
        })
        .map_err(|e| format!("{e}"))?;
    b.finish(moved).map_err(|e| format!("{e}"))
}

/// An underspecified front edge realized as an arc under a named policy
/// with a cited spec issue: exercises the `Conflicted` fidelity channel
/// and policy attribution.
fn policy_curve_recipe() -> Result<Recipe, String> {
    use exedra_constructive::profile::{Loop2, Profile2, Seg2, SegKind, SegTag};

    let mut b = RecipeBuilder::new();
    let policy = b.curve_policy("demo.front-transition@1");
    let issue = b.source_ref("demo.issue.front-profile-nonclosing");
    let src = b.source_ref("demo:policy_curve");
    let outer = Loop2::new(alloc::vec![
        Seg2::line((500.0, 0.0)).tagged(SegTag(0)),
        Seg2::line((500.0, 280.0)).tagged(SegTag(1)),
        Seg2::policy((0.0, 300.0), policy, SegKind::Arc { bulge: -0.15 }).tagged(SegTag(2)),
        Seg2::line((0.0, 0.0)).tagged(SegTag(3)),
    ])
    .map_err(|e| format!("{e}"))?;
    let profile = Profile2::simple(outer).map_err(|e| format!("{e}"))?;
    let p = b.add_profile(profile);
    let n = b
        .with_source(src)
        .with_issue(issue)
        .add(NodeKind::Extrude {
            profile: p,
            placement: Placement3::IDENTITY,
            height: 18.0,
            caps: CapMode::Both,
        })
        .map_err(|e| format!("{e}"))?;
    b.finish(n).map_err(|e| format!("{e}"))
}

/// Inspects a plain recipe scenario: one identity instance per body.
fn inspect_recipe(name: &str, recipe: &Recipe) -> Result<InspectionResponse, String> {
    let evaluation =
        evaluate(recipe, &EvalPolicy::default()).map_err(|e| format!("evaluation failed: {e}"))?;
    let mut response = empty_response(name);
    append_recipe(&mut response, None, recipe, &evaluation);
    for (index, _) in response.bodies.iter().enumerate() {
        response.instances.push(InspectionInstance {
            path: name.to_string(),
            body: u32::try_from(index).unwrap_or(u32::MAX),
            matrix: matrix16(&Placement3::IDENTITY),
        });
    }
    Ok(response)
}

/// Inspects the shared `panel_trio` assembly: bodies come from evaluating
/// the part recipe (full provenance), instances from the assembly's
/// flattened render list.
fn inspect_panel_trio() -> Result<InspectionResponse, String> {
    use exedra_assembly::{PartCompiler, PartSource, flatten};

    let asm = panel_trio_assembly()?;
    let mut compiler = PartCompiler::new();
    let compiled = compiler
        .compile_parts(&asm, &EvalPolicy::default())
        .map_err(|e| format!("{e}"))?;
    let list = flatten(&asm, &compiled);

    let mut response = empty_response("panel_trio");
    // Evaluate each distinct part's recipe once for provenance; the body
    // table indexes (part, body) pairs in first-use order.
    let mut body_lookup: alloc::collections::BTreeMap<(u32, u32), u32> =
        alloc::collections::BTreeMap::new();
    let mut evaluated: alloc::collections::BTreeMap<u32, usize> =
        alloc::collections::BTreeMap::new();
    for item in &list.items {
        let key = (item.part.0, item.body);
        let body = match body_lookup.get(&key).copied() {
            Some(body) => body,
            None => {
                let def = asm
                    .part(item.part)
                    .ok_or_else(|| "flatten referenced an unknown part".to_string())?;
                let part_key = def.key().to_string();
                let first_body = match evaluated.get(&item.part.0) {
                    Some(&first_body) => first_body,
                    None => {
                        let PartSource::Recipe(recipe) = def.source() else {
                            return Err("panel_trio parts are recipes".to_string());
                        };
                        let evaluation = evaluate(recipe, &EvalPolicy::default())
                            .map_err(|e| format!("evaluation failed: {e}"))?;
                        let first_body = response.bodies.len();
                        append_recipe(&mut response, Some(&part_key), recipe, &evaluation);
                        evaluated.insert(item.part.0, first_body);
                        first_body
                    }
                };
                let body_index = u32::try_from(first_body + item.body as usize)
                    .map_err(|_| "body index overflow".to_string())?;
                body_lookup.insert(key, body_index);
                body_index
            }
        };
        response.instances.push(InspectionInstance {
            path: item.path.to_string(),
            body,
            matrix: matrix16(&item.world),
        });
    }
    Ok(response)
}

fn empty_response(name: &str) -> InspectionResponse {
    InspectionResponse {
        format: INSPECT_FORMAT.to_string(),
        scenario: name.to_string(),
        nodes: Vec::new(),
        bodies: Vec::new(),
        instances: Vec::new(),
        fidelity: Vec::new(),
        policy_curves: Vec::new(),
        diagnostics: Vec::new(),
        counters: InspectionCounters::default(),
    }
}

/// Appends one evaluated recipe's nodes, bodies, report entries, and
/// counters to the response under the given part scope.
fn append_recipe(
    response: &mut InspectionResponse,
    part: Option<&str>,
    recipe: &Recipe,
    evaluation: &Evaluation,
) {
    let part_owned = |response: &InspectionResponse| -> Option<String> {
        let _ = response;
        part.map(ToString::to_string)
    };

    // Node table, ascending by id.
    let mut id = 0_u32;
    while let Some(node) = recipe.node(NodeId(id)) {
        response.nodes.push(InspectionNode {
            part: part_owned(response),
            id,
            kind: kind_name(&node.kind).to_string(),
            fingerprint: recipe
                .fingerprint(NodeId(id))
                .map(|f| format!("{:032x}", f.0))
                .unwrap_or_default(),
            source: node
                .source
                .and_then(|s| recipe.source(s))
                .map(ToString::to_string),
            material: node
                .material
                .and_then(|s| recipe.slot(s))
                .map(ToString::to_string),
            issue: node
                .issue
                .and_then(|s| recipe.source(s))
                .map(ToString::to_string),
        });
        id += 1;
    }

    for placed in &evaluation.bodies {
        let (mesh, tri_face) = extract_mesh_buffers(&placed.body.mesh);
        let region_layer = placed.body.mesh.attrs().dense(exedra::attr::FACE_REGION);
        let faces: Vec<InspectionFace> = placed
            .body
            .mesh
            .faces()
            .map(|face| InspectionFace {
                feature: placed
                    .body
                    .source_map
                    .face_feature(face)
                    .map_or_else(unknown_feature, feature_dto),
                region: region_layer
                    .and_then(|layer| layer.get(face.as_id()).copied())
                    .unwrap_or(0),
            })
            .collect();
        response.bodies.push(InspectionBody {
            part: part_owned(response),
            node: placed.node.0,
            mesh,
            faces,
            tri_face,
        });
    }

    for (node, fidelity) in &evaluation.report.fidelity {
        let (verdict, policy, issue) = match fidelity {
            Fidelity::Exact => ("exact", None, None),
            Fidelity::PolicyDefined(p) => (
                "policy_defined",
                recipe.policy(*p).map(ToString::to_string),
                None,
            ),
            Fidelity::Conflicted(s) => (
                "conflicted",
                None,
                recipe.source(*s).map(ToString::to_string),
            ),
            Fidelity::EnvelopeOnly => ("envelope_only", None, None),
            _ => ("unknown", None, None),
        };
        response.fidelity.push(InspectionFidelity {
            part: part_owned(response),
            node: node.0,
            verdict: verdict.to_string(),
            policy,
            issue,
        });
    }

    for (node, policy) in &evaluation.report.policy_curves {
        response.policy_curves.push(InspectionPolicyUse {
            part: part_owned(response),
            node: node.0,
            policy: recipe.policy(*policy).unwrap_or_default().to_string(),
        });
    }

    for diagnostic in &evaluation.report.diagnostics {
        response.diagnostics.push(InspectionDiagnostic {
            part: part_owned(response),
            severity: match diagnostic.severity {
                Severity::Note => "note",
                Severity::Warning => "warning",
                Severity::Error => "error",
            }
            .to_string(),
            code: diagnostic.code.to_string(),
            message: diagnostic.message.clone(),
            node: diagnostic.node.map(|n| n.0),
        });
    }

    let c = &evaluation.report.counters;
    response.counters.bodies += c.bodies;
    response.counters.tessellations += c.tessellations;
    response.counters.faces += c.faces;
    response.counters.vertices += c.vertices;
    response.counters.envelope_only += c.envelope_only;
    response.counters.unimplemented += c.unimplemented;
    response.counters.source_map_bytes += c.source_map_bytes;
}

fn kind_name(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::Extrude { .. } => "extrude",
        NodeKind::Revolve { .. } => "revolve",
        NodeKind::Loft { .. } => "loft",
        NodeKind::Sweep { .. } => "sweep",
        NodeKind::PlanarFace { .. } => "planar_face",
        NodeKind::Primitive { .. } => "primitive",
        NodeKind::Csg { .. } => "csg",
        NodeKind::Transform { .. } => "transform",
        NodeKind::Mirror { .. } => "mirror",
        NodeKind::Instance { .. } => "instance",
        NodeKind::Group { .. } => "group",
        NodeKind::MeshImport { .. } => "mesh_import",
        NodeKind::Stretch { .. } => "stretch",
        NodeKind::GridSurface { .. } => "grid_surface",
        _ => "unknown",
    }
}

fn unknown_feature() -> InspectionFeature {
    InspectionFeature {
        kind: "unknown".to_string(),
        loop_index: None,
        seg: None,
        band: None,
        operand: None,
        row: None,
        col: None,
    }
}

fn feature_dto(feature: Feature) -> InspectionFeature {
    let mut dto = unknown_feature();
    match feature {
        Feature::CapStart => dto.kind = "cap_start".to_string(),
        Feature::CapEnd => dto.kind = "cap_end".to_string(),
        Feature::Wall { loop_index, seg } => {
            dto.kind = "wall".to_string();
            dto.loop_index = Some(u32::from(loop_index));
            dto.seg = Some(seg);
        }
        Feature::LoftWall {
            band,
            loop_index,
            seg,
        } => {
            dto.kind = "loft_wall".to_string();
            dto.band = Some(u32::from(band));
            dto.loop_index = Some(u32::from(loop_index));
            dto.seg = Some(seg);
        }
        Feature::SweepWall {
            band,
            loop_index,
            seg,
        } => {
            dto.kind = "sweep_wall".to_string();
            dto.band = Some(u32::from(band));
            dto.loop_index = Some(u32::from(loop_index));
            dto.seg = Some(seg);
        }
        Feature::Imported => dto.kind = "imported".to_string(),
        Feature::BooleanFace { operand } => {
            dto.kind = "boolean_face".to_string();
            dto.operand = Some(u32::from(operand));
        }
        Feature::GridPatch { row, col } => {
            dto.kind = "grid_patch".to_string();
            dto.row = Some(u32::from(row));
            dto.col = Some(u32::from(col));
        }
        _ => {}
    }
    dto
}

#[cfg(test)]
mod tests {
    use super::*;

    extern crate std;

    fn run(name: &str) -> InspectionResponse {
        run_inspection_impl(name).expect("scenario runs")
    }

    fn json(name: &str) -> String {
        serde_json::to_string(&run(name)).expect("serializes")
    }

    #[test]
    fn drilled_block_resolves_full_chains() {
        let response = run("drilled_block");
        assert_eq!(response.format, INSPECT_FORMAT);
        assert_eq!(response.bodies.len(), 1);
        assert_eq!(response.instances.len(), 1);
        assert!(
            response.diagnostics.is_empty(),
            "{:?}",
            response.diagnostics
        );

        let body = &response.bodies[0];
        assert_eq!(body.tri_face.len() * 3, body.mesh.indices.len());
        assert_eq!(body.tri_face.len(), body.mesh.region_ids.len());
        assert!(!body.faces.is_empty());
        // Every triangle resolves to a face, and every face to boolean
        // provenance with both operands represented.
        let mut operands = std::collections::BTreeSet::new();
        for &face in &body.tri_face {
            let face = &body.faces[face as usize];
            assert_eq!(face.feature.kind, "boolean_face");
            operands.insert(face.feature.operand.expect("operand attribution"));
        }
        assert_eq!(operands.into_iter().collect::<Vec<_>>(), alloc::vec![0, 1]);
        // The body's producing node exists in the node table and the
        // recipe's sources ride along.
        let node = response
            .nodes
            .iter()
            .find(|n| n.part.is_none() && n.id == body.node)
            .expect("body node in table");
        assert!(!node.fingerprint.is_empty());
        assert!(
            response
                .nodes
                .iter()
                .any(|n| n.source.as_deref() == Some("demo:drilled_block/drill"))
        );
        // The csg node evaluated exact.
        let csg = response
            .nodes
            .iter()
            .find(|n| n.kind == "csg")
            .expect("csg node");
        assert!(
            response
                .fidelity
                .iter()
                .any(|f| f.node == csg.id && f.verdict == "exact")
        );
    }

    #[test]
    fn policy_curve_reports_conflicted_fidelity() {
        let response = run("policy_curve");
        let conflicted = response
            .fidelity
            .iter()
            .find(|f| f.verdict == "conflicted")
            .expect("conflicted verdict");
        assert_eq!(
            conflicted.issue.as_deref(),
            Some("demo.issue.front-profile-nonclosing")
        );
        assert!(
            response
                .policy_curves
                .iter()
                .any(|p| p.policy == "demo.front-transition@1")
        );
        // The policy segment's wall faces attribute to segment 2.
        let body = &response.bodies[0];
        assert!(
            body.faces
                .iter()
                .any(|f| f.feature.kind == "wall" && f.feature.seg == Some(2))
        );
    }

    #[test]
    fn panel_trio_instances_share_one_provenant_body() {
        let response = run("panel_trio");
        assert_eq!(response.instances.len(), 3);
        assert!(response.instances.iter().all(|i| i.body == 0));
        assert_eq!(response.bodies.len(), 1);
        let body = &response.bodies[0];
        assert_eq!(body.part.as_deref(), Some("panel"));
        let kinds: std::collections::BTreeSet<&str> =
            body.faces.iter().map(|f| f.feature.kind.as_str()).collect();
        assert!(kinds.contains("cap_start"));
        assert!(kinds.contains("cap_end"));
        assert!(kinds.contains("wall"));
        // Instance placements differ (stacked shelves).
        let z = |i: usize| response.instances[i].matrix[14];
        assert_eq!(z(0), 0.0);
        assert_eq!(z(1), 250.0);
        assert_eq!(z(2), 500.0);
        // Node table is part-scoped.
        assert!(
            response
                .nodes
                .iter()
                .all(|n| n.part.as_deref() == Some("panel"))
        );
    }

    #[test]
    fn payloads_are_deterministic() {
        for name in ["drilled_block", "policy_curve", "panel_trio"] {
            assert_eq!(json(name), json(name), "{name}: byte-identical reruns");
        }
    }
}
