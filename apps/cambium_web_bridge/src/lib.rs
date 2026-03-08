// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Wasm bridge for deterministic Cambium scenario execution.
#![no_std]
extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use cambium::{
    BakeFaceNormals, BakeFaceNormalsParams, BridgeBoundaryLoops, BridgeBoundaryLoopsParams,
    CutRectFace, CutRectFaceParams, CylinderAxis, DeleteFaces, DeleteFacesParams, DiagCode,
    DiagLevel, DissolveEdges, DissolveEdgesParams, DissolveVertices, DissolveVerticesParams,
    ExtractParams, ExtrudeFaces, ExtrudeFacesParams, ExtrudeMode, InsetFaces, InsetFacesParams,
    Mesh, NormalParams, NormalsSource, OperatorRunner, PokeFaces, PokeFacesParams,
    SelectBoundaryEdgeLoop, SelectBoundaryEdgeLoopParams, SmoothFaceNormals,
    SmoothFaceNormalsParams, SolidifyFaces, SolidifyFacesParams, SolidifyMode, TagFaceRegion,
    TagFaceRegionParams, UvBox, UvBoxParams, UvCylinder, UvCylinderParams, UvPlanar,
    UvPlanarParams, UvPlane, UvScope, flood_fill_faces_by_region, mesh_signature,
    select_faces_by_region,
};
use exedra::attr;
use exedra::{ChangeSetBuilder, op};
use exedra_primitives::{
    BoxParams, CapFill, ConeParams, CylinderParams, GridParams, IcosphereParams, QuadParams,
    TorusParams, UvSphereParams,
};
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

/// Optional scenario execution settings.
#[derive(Copy, Clone, Debug, Deserialize, Serialize)]
pub struct ScenarioOptions {
    /// Include an initial mesh snapshot before any operator steps.
    pub include_initial: bool,
}

impl Default for ScenarioOptions {
    fn default() -> Self {
        Self {
            include_initial: true,
        }
    }
}

/// Top-level serialized scenario payload.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ScenarioResponse {
    /// Stable scenario name.
    pub scenario: String,
    /// Ordered step snapshots.
    pub steps: Vec<StepSnapshot>,
}

/// One deterministic step snapshot.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StepSnapshot {
    /// Human-readable step label.
    pub label: String,
    /// Operator name for this step when applicable.
    pub operator: Option<String>,
    /// Operator plan fingerprint when applicable.
    pub plan_fingerprint: Option<u64>,
    /// Deterministic mesh signature after this step.
    pub mesh_signature: u64,
    /// Render buffers extracted from current mesh state.
    pub mesh: MeshBuffers,
    /// Compact deterministic stats summary for this step.
    pub stats: StepStats,
    /// Diagnostics emitted during this step.
    pub diagnostics: Vec<StepDiagnostic>,
}

/// Flattened renderable mesh buffers.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct MeshBuffers {
    /// Triangle indices.
    pub indices: Vec<u32>,
    /// Flattened xyz positions.
    pub positions: Vec<f32>,
    /// Flattened uv coordinates.
    pub uvs: Vec<f32>,
    /// Flattened xyz normals.
    pub normals: Vec<f32>,
    /// Flattened topology edge segments as xyzxyz line endpoints.
    pub topology_lines: Vec<f32>,
    /// Per-triangle face region ID (one entry per triangle, parallel to index triples).
    pub region_ids: Vec<u32>,
}

/// Deterministic operator-step counters.
#[derive(Copy, Clone, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
pub struct StepStats {
    /// Faces processed by the operator.
    pub faces_processed: u64,
    /// Created vertex count.
    pub created_vertices: u64,
    /// Created face count.
    pub created_faces: u64,
    /// Deleted face count.
    pub deleted_faces: u64,
    /// Deleted vertex count.
    pub deleted_vertices: u64,
    /// Corner write count.
    pub corners_written: u64,
    /// Edge write count.
    pub edges_written: u64,
}

/// One diagnostic item from operator execution.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct StepDiagnostic {
    /// Severity level as a stable string.
    pub level: String,
    /// Diagnostic code as a stable string.
    pub code: String,
    /// Human-readable message.
    pub message: String,
}

/// Returns the current named scenarios as JSON array.
#[wasm_bindgen]
pub fn list_scenarios_json() -> String {
    serde_json::to_string(&[
        "boxy_hat",
        "stepped_tower",
        "pedestal",
        "wall_openings",
        "poked_grid",
        "bridge_loops",
        "cylinder_normals",
        "region_select_flow",
        "uv_projection_gallery",
        "topology_dissolve_repair",
        "primitive_gallery",
    ])
    .expect("scenario list should serialize")
}

/// Runs one named scenario and returns JSON snapshots.
///
/// `options_json` may be empty for defaults.
#[wasm_bindgen]
pub fn run_scenario_json(name: &str, options_json: &str) -> Result<String, JsValue> {
    let options = parse_options(options_json)?;
    let response = run_scenario_impl(name, &options).map_err(|err| JsValue::from_str(&err))?;
    serde_json::to_string(&response)
        .map_err(|err| JsValue::from_str(&format!("failed to serialize scenario response: {err}")))
}

fn parse_options(input: &str) -> Result<ScenarioOptions, JsValue> {
    if input.trim().is_empty() {
        return Ok(ScenarioOptions::default());
    }
    serde_json::from_str(input)
        .map_err(|err| JsValue::from_str(&format!("invalid scenario options JSON: {err}")))
}

fn run_scenario_impl(name: &str, options: &ScenarioOptions) -> Result<ScenarioResponse, String> {
    match name {
        "boxy_hat" => run_boxy_hat(options),
        "stepped_tower" => run_stepped_tower(options),
        "pedestal" => run_pedestal(options),
        "wall_openings" => run_wall_openings(options),
        "poked_grid" => run_poked_grid(options),
        "bridge_loops" => run_bridge_loops(options),
        "cylinder_normals" => run_cylinder_normals(options),
        "region_select_flow" => run_region_select_flow(options),
        "uv_projection_gallery" => run_uv_projection_gallery(options),
        "topology_dissolve_repair" => run_topology_dissolve_repair(options),
        "primitive_gallery" => run_primitive_gallery(options),
        _ => Err(format!("unknown scenario `{name}`")),
    }
}

fn run_boxy_hat(options: &ScenarioOptions) -> Result<ScenarioResponse, String> {
    let mut mesh = Mesh::from_polygons(
        &[
            [0.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [2.0, 2.0, 0.0],
            [0.0, 2.0, 0.0],
        ],
        &[&[0, 1, 2, 3]],
    )
    .map_err(|err| format!("boxy_hat setup failed: {err}"))?;
    let face = mesh
        .faces()
        .next()
        .ok_or("boxy_hat setup produced no faces".to_string())?;
    let mut steps = Vec::<StepSnapshot>::new();
    let mut runner = OperatorRunner::new();

    if options.include_initial {
        steps.push(snapshot_from_mesh(
            &mesh,
            "initial",
            None,
            None,
            StepStats::default(),
            &[],
        ));
    }

    let inset_plan = runner
        .compile(
            &mesh,
            &InsetFaces,
            &InsetFacesParams {
                faces: vec![face],
                factor: 0.25,
            },
        )
        .map_err(format_op_error)?;
    let inset_fingerprint = inset_plan.fingerprint.value();
    let inset_result = runner
        .apply_in_place(&mut mesh, &InsetFaces, &inset_plan)
        .map_err(format_op_error)?;
    steps.push(snapshot_from_report(
        &mesh,
        "inset",
        "edit.face.inset",
        inset_fingerprint,
        &inset_result.report,
        &runner,
    ));

    // Tag frame faces as region 1.
    let tag_frame = runner
        .compile(
            &mesh,
            &TagFaceRegion,
            &TagFaceRegionParams {
                region_id: 1,
                faces: inset_result.output.frame_faces.clone(),
            },
        )
        .map_err(format_op_error)?;
    let tag_frame_fp = tag_frame.fingerprint.value();
    let tag_frame_result = runner
        .apply_in_place(&mut mesh, &TagFaceRegion, &tag_frame)
        .map_err(format_op_error)?;
    steps.push(snapshot_from_report(
        &mesh,
        "tag.frame",
        "tag.face.region",
        tag_frame_fp,
        &tag_frame_result.report,
        &runner,
    ));

    let extrude_plan = runner
        .compile(
            &mesh,
            &ExtrudeFaces,
            &ExtrudeFacesParams {
                faces: inset_result.output.inner_faces.clone(),
                mode: ExtrudeMode::ShellOpen,
                distance: 0.6,
            },
        )
        .map_err(format_op_error)?;
    let extrude_fingerprint = extrude_plan.fingerprint.value();
    let extrude_result = runner
        .apply_in_place(&mut mesh, &ExtrudeFaces, &extrude_plan)
        .map_err(format_op_error)?;
    steps.push(snapshot_from_report(
        &mesh,
        "extrude",
        "edit.face.extrude",
        extrude_fingerprint,
        &extrude_result.report,
        &runner,
    ));

    // Tag wall and cap faces from extrude.
    let tag_walls = runner
        .compile(
            &mesh,
            &TagFaceRegion,
            &TagFaceRegionParams {
                region_id: 2,
                faces: extrude_result.output.wall_faces.clone(),
            },
        )
        .map_err(format_op_error)?;
    let tag_walls_fp = tag_walls.fingerprint.value();
    let tag_walls_result = runner
        .apply_in_place(&mut mesh, &TagFaceRegion, &tag_walls)
        .map_err(format_op_error)?;
    steps.push(snapshot_from_report(
        &mesh,
        "tag.walls",
        "tag.face.region",
        tag_walls_fp,
        &tag_walls_result.report,
        &runner,
    ));

    let tag_cap = runner
        .compile(
            &mesh,
            &TagFaceRegion,
            &TagFaceRegionParams {
                region_id: 3,
                faces: extrude_result.output.cap_faces.clone(),
            },
        )
        .map_err(format_op_error)?;
    let tag_cap_fp = tag_cap.fingerprint.value();
    let tag_cap_result = runner
        .apply_in_place(&mut mesh, &TagFaceRegion, &tag_cap)
        .map_err(format_op_error)?;
    steps.push(snapshot_from_report(
        &mesh,
        "tag.cap",
        "tag.face.region",
        tag_cap_fp,
        &tag_cap_result.report,
        &runner,
    ));

    Ok(ScenarioResponse {
        scenario: "boxy_hat".to_string(),
        steps,
    })
}

fn apply_primitive_regions(mesh: &mut Mesh, face_region: &exedra_primitives::FaceRegionLayer) {
    let faces: Vec<_> = mesh.faces().collect();
    let mut txn = mesh.edit();
    for face in faces {
        let region = face_region.get(face);
        let _ = op::set_face_region(&mut txn, face, region.0);
    }
    let _: () = txn.finish();
}

fn tag_faces(
    mesh: &mut Mesh,
    runner: &mut OperatorRunner,
    steps: &mut Vec<StepSnapshot>,
    label: &str,
    region_id: u32,
    faces: Vec<cambium::FaceId>,
) -> Result<(), String> {
    let plan = runner
        .compile(
            mesh,
            &TagFaceRegion,
            &TagFaceRegionParams { region_id, faces },
        )
        .map_err(format_op_error)?;
    let fp = plan.fingerprint.value();
    let result = runner
        .apply_in_place(mesh, &TagFaceRegion, &plan)
        .map_err(format_op_error)?;
    steps.push(snapshot_from_report(
        mesh,
        label,
        "tag.face.region",
        fp,
        &result.report,
        runner,
    ));
    Ok(())
}

fn run_stepped_tower(options: &ScenarioOptions) -> Result<ScenarioResponse, String> {
    let primitive = exedra_primitives::cylinder(&CylinderParams {
        radius: 1.0,
        height: 0.6,
        segments: 8,
        cap_fill: CapFill::Ngon,
        centered: false,
    });
    let face_region = primitive.face_region;
    let mut mesh = primitive.mesh;
    apply_primitive_regions(&mut mesh, &face_region);
    let mut steps = Vec::<StepSnapshot>::new();
    let mut runner = OperatorRunner::new();

    if options.include_initial {
        steps.push(snapshot_from_mesh(
            &mesh,
            "initial",
            None,
            None,
            StepStats::default(),
            &[],
        ));
    }

    // Tier 1: inset the top cap, extrude up.
    let top_face = mesh
        .faces()
        .max_by(|a, b| face_centroid_y(&mesh, *a).total_cmp(&face_centroid_y(&mesh, *b)))
        .ok_or("stepped_tower: no faces".to_string())?;

    let inset1 = runner
        .compile(
            &mesh,
            &InsetFaces,
            &InsetFacesParams {
                faces: vec![top_face],
                factor: 0.35,
            },
        )
        .map_err(format_op_error)?;
    let inset1_fp = inset1.fingerprint.value();
    let inset1_result = runner
        .apply_in_place(&mut mesh, &InsetFaces, &inset1)
        .map_err(format_op_error)?;
    steps.push(snapshot_from_report(
        &mesh,
        "inset.tier1",
        "edit.face.inset",
        inset1_fp,
        &inset1_result.report,
        &runner,
    ));

    tag_faces(
        &mut mesh,
        &mut runner,
        &mut steps,
        "tag.ring1",
        10,
        inset1_result.output.frame_faces.clone(),
    )?;

    let extrude1 = runner
        .compile(
            &mesh,
            &ExtrudeFaces,
            &ExtrudeFacesParams {
                faces: inset1_result.output.inner_faces.clone(),
                mode: ExtrudeMode::ShellOpen,
                distance: 0.8,
            },
        )
        .map_err(format_op_error)?;
    let extrude1_fp = extrude1.fingerprint.value();
    let extrude1_result = runner
        .apply_in_place(&mut mesh, &ExtrudeFaces, &extrude1)
        .map_err(format_op_error)?;
    steps.push(snapshot_from_report(
        &mesh,
        "extrude.tier1",
        "edit.face.extrude",
        extrude1_fp,
        &extrude1_result.report,
        &runner,
    ));

    tag_faces(
        &mut mesh,
        &mut runner,
        &mut steps,
        "tag.walls1",
        11,
        extrude1_result.output.wall_faces.clone(),
    )?;

    // Tier 2: inset the new cap, extrude again.
    let cap1 = extrude1_result
        .output
        .cap_faces
        .first()
        .copied()
        .ok_or("stepped_tower: extrude produced no cap".to_string())?;

    let inset2 = runner
        .compile(
            &mesh,
            &InsetFaces,
            &InsetFacesParams {
                faces: vec![cap1],
                factor: 0.30,
            },
        )
        .map_err(format_op_error)?;
    let inset2_fp = inset2.fingerprint.value();
    let inset2_result = runner
        .apply_in_place(&mut mesh, &InsetFaces, &inset2)
        .map_err(format_op_error)?;
    steps.push(snapshot_from_report(
        &mesh,
        "inset.tier2",
        "edit.face.inset",
        inset2_fp,
        &inset2_result.report,
        &runner,
    ));

    tag_faces(
        &mut mesh,
        &mut runner,
        &mut steps,
        "tag.ring2",
        20,
        inset2_result.output.frame_faces.clone(),
    )?;

    let extrude2 = runner
        .compile(
            &mesh,
            &ExtrudeFaces,
            &ExtrudeFacesParams {
                faces: inset2_result.output.inner_faces.clone(),
                mode: ExtrudeMode::ShellOpen,
                distance: 1.0,
            },
        )
        .map_err(format_op_error)?;
    let extrude2_fp = extrude2.fingerprint.value();
    let extrude2_result = runner
        .apply_in_place(&mut mesh, &ExtrudeFaces, &extrude2)
        .map_err(format_op_error)?;
    steps.push(snapshot_from_report(
        &mesh,
        "extrude.tier2",
        "edit.face.extrude",
        extrude2_fp,
        &extrude2_result.report,
        &runner,
    ));

    tag_faces(
        &mut mesh,
        &mut runner,
        &mut steps,
        "tag.walls2",
        21,
        extrude2_result.output.wall_faces.clone(),
    )?;
    tag_faces(
        &mut mesh,
        &mut runner,
        &mut steps,
        "tag.cap",
        30,
        extrude2_result.output.cap_faces.clone(),
    )?;

    Ok(ScenarioResponse {
        scenario: "stepped_tower".to_string(),
        steps,
    })
}

/// Telescoping box: inset+extrude the top face twice for a stepped pedestal.
fn run_pedestal(options: &ScenarioOptions) -> Result<ScenarioResponse, String> {
    let primitive = exedra_primitives::box_primitive(&BoxParams {
        size: [2.0, 0.5, 2.0],
        centered: true,
        segments: [1, 1, 1],
    });
    let face_region = primitive.face_region;
    let mut mesh = primitive.mesh;
    apply_primitive_regions(&mut mesh, &face_region);
    let mut steps = Vec::<StepSnapshot>::new();
    let mut runner = OperatorRunner::new();

    if options.include_initial {
        steps.push(snapshot_from_mesh(
            &mesh,
            "initial",
            None,
            None,
            StepStats::default(),
            &[],
        ));
    }

    // Tier 1: inset top face, extrude up.
    let top_face = mesh
        .faces()
        .max_by(|a, b| face_centroid_y(&mesh, *a).total_cmp(&face_centroid_y(&mesh, *b)))
        .ok_or("pedestal: no faces".to_string())?;

    let inset1 = runner
        .compile(
            &mesh,
            &InsetFaces,
            &InsetFacesParams {
                faces: vec![top_face],
                factor: 0.25,
            },
        )
        .map_err(format_op_error)?;
    let inset1_fp = inset1.fingerprint.value();
    let inset1_result = runner
        .apply_in_place(&mut mesh, &InsetFaces, &inset1)
        .map_err(format_op_error)?;
    steps.push(snapshot_from_report(
        &mesh,
        "inset.tier1",
        "edit.face.inset",
        inset1_fp,
        &inset1_result.report,
        &runner,
    ));

    tag_faces(
        &mut mesh,
        &mut runner,
        &mut steps,
        "tag.base",
        1,
        inset1_result.output.frame_faces.clone(),
    )?;

    let extrude1 = runner
        .compile(
            &mesh,
            &ExtrudeFaces,
            &ExtrudeFacesParams {
                faces: inset1_result.output.inner_faces.clone(),
                mode: ExtrudeMode::ShellOpen,
                distance: 0.8,
            },
        )
        .map_err(format_op_error)?;
    let extrude1_fp = extrude1.fingerprint.value();
    let extrude1_result = runner
        .apply_in_place(&mut mesh, &ExtrudeFaces, &extrude1)
        .map_err(format_op_error)?;
    steps.push(snapshot_from_report(
        &mesh,
        "extrude.tier1",
        "edit.face.extrude",
        extrude1_fp,
        &extrude1_result.report,
        &runner,
    ));

    tag_faces(
        &mut mesh,
        &mut runner,
        &mut steps,
        "tag.walls1",
        2,
        extrude1_result.output.wall_faces.clone(),
    )?;

    // Tier 2: inset the cap, extrude again (narrower, taller).
    let cap1 = extrude1_result
        .output
        .cap_faces
        .first()
        .copied()
        .ok_or("pedestal: extrude produced no cap".to_string())?;

    let inset2 = runner
        .compile(
            &mesh,
            &InsetFaces,
            &InsetFacesParams {
                faces: vec![cap1],
                factor: 0.3,
            },
        )
        .map_err(format_op_error)?;
    let inset2_fp = inset2.fingerprint.value();
    let inset2_result = runner
        .apply_in_place(&mut mesh, &InsetFaces, &inset2)
        .map_err(format_op_error)?;
    steps.push(snapshot_from_report(
        &mesh,
        "inset.tier2",
        "edit.face.inset",
        inset2_fp,
        &inset2_result.report,
        &runner,
    ));

    tag_faces(
        &mut mesh,
        &mut runner,
        &mut steps,
        "tag.ledge",
        3,
        inset2_result.output.frame_faces.clone(),
    )?;

    let extrude2 = runner
        .compile(
            &mesh,
            &ExtrudeFaces,
            &ExtrudeFacesParams {
                faces: inset2_result.output.inner_faces.clone(),
                mode: ExtrudeMode::ShellOpen,
                distance: 1.2,
            },
        )
        .map_err(format_op_error)?;
    let extrude2_fp = extrude2.fingerprint.value();
    let extrude2_result = runner
        .apply_in_place(&mut mesh, &ExtrudeFaces, &extrude2)
        .map_err(format_op_error)?;
    steps.push(snapshot_from_report(
        &mesh,
        "extrude.tier2",
        "edit.face.extrude",
        extrude2_fp,
        &extrude2_result.report,
        &runner,
    ));

    tag_faces(
        &mut mesh,
        &mut runner,
        &mut steps,
        "tag.walls2",
        4,
        extrude2_result.output.wall_faces.clone(),
    )?;
    tag_faces(
        &mut mesh,
        &mut runner,
        &mut steps,
        "tag.cap",
        5,
        extrude2_result.output.cap_faces.clone(),
    )?;

    Ok(ScenarioResponse {
        scenario: "pedestal".to_string(),
        steps,
    })
}

fn run_wall_openings(options: &ScenarioOptions) -> Result<ScenarioResponse, String> {
    let mut mesh = Mesh::from_polygons(
        &[
            [0.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [0.0, 3.0, 0.0],
            [2.0, 3.0, 0.0],
            [3.0, 0.0, 0.0],
            [5.0, 0.0, 0.0],
            [3.0, 3.0, 0.0],
            [5.0, 3.0, 0.0],
        ],
        &[&[0, 1, 3, 2], &[4, 5, 7, 6]],
    )
    .map_err(|err| format!("wall_openings setup failed: {err}"))?;
    let mut faces = mesh.faces().collect::<Vec<_>>();
    faces
        .sort_unstable_by(|a, b| face_centroid_x(&mesh, *a).total_cmp(&face_centroid_x(&mesh, *b)));
    if faces.len() != 2 {
        return Err("wall_openings requires exactly two source faces".to_string());
    }
    let left_face = faces[0];
    let right_face = faces[1];

    let mut steps = Vec::<StepSnapshot>::new();
    let mut runner = OperatorRunner::new();

    if options.include_initial {
        steps.push(snapshot_from_mesh(
            &mesh,
            "initial",
            None,
            None,
            StepStats::default(),
            &[],
        ));
    }

    let door_cut = runner
        .compile(
            &mesh,
            &CutRectFace,
            &CutRectFaceParams {
                face: left_face,
                frame_origin: [0.0, 0.0, 0.0],
                frame_u: [2.0, 0.0, 0.0],
                frame_v: [0.0, 3.0, 0.0],
                rect_min: [0.35, 0.05],
                rect_max: [0.65, 0.75],
            },
        )
        .map_err(format_op_error)?;
    let door_cut_fingerprint = door_cut.fingerprint.value();
    let door_cut_result = runner
        .apply_in_place(&mut mesh, &CutRectFace, &door_cut)
        .map_err(format_op_error)?;
    steps.push(snapshot_from_report(
        &mesh,
        "door.cut",
        "edit.face.cut.rect",
        door_cut_fingerprint,
        &door_cut_result.report,
        &runner,
    ));

    let door_delete = runner
        .compile(
            &mesh,
            &DeleteFaces,
            &DeleteFacesParams {
                faces: door_cut_result.output.inner_faces.clone(),
                policy: cambium::DeletePolicy::KeepIsolated,
            },
        )
        .map_err(format_op_error)?;
    let door_delete_fingerprint = door_delete.fingerprint.value();
    let door_delete_result = runner
        .apply_in_place(&mut mesh, &DeleteFaces, &door_delete)
        .map_err(format_op_error)?;
    steps.push(snapshot_from_report(
        &mesh,
        "door.open",
        "edit.delete.faces",
        door_delete_fingerprint,
        &door_delete_result.report,
        &runner,
    ));

    let window_cut = runner
        .compile(
            &mesh,
            &CutRectFace,
            &CutRectFaceParams {
                face: right_face,
                frame_origin: [3.0, 0.0, 0.0],
                frame_u: [2.0, 0.0, 0.0],
                frame_v: [0.0, 3.0, 0.0],
                rect_min: [0.25, 0.45],
                rect_max: [0.75, 0.85],
            },
        )
        .map_err(format_op_error)?;
    let window_cut_fingerprint = window_cut.fingerprint.value();
    let window_cut_result = runner
        .apply_in_place(&mut mesh, &CutRectFace, &window_cut)
        .map_err(format_op_error)?;
    steps.push(snapshot_from_report(
        &mesh,
        "window.cut",
        "edit.face.cut.rect",
        window_cut_fingerprint,
        &window_cut_result.report,
        &runner,
    ));

    let window_delete = runner
        .compile(
            &mesh,
            &DeleteFaces,
            &DeleteFacesParams {
                faces: window_cut_result.output.inner_faces.clone(),
                policy: cambium::DeletePolicy::KeepIsolated,
            },
        )
        .map_err(format_op_error)?;
    let window_delete_fingerprint = window_delete.fingerprint.value();
    let window_delete_result = runner
        .apply_in_place(&mut mesh, &DeleteFaces, &window_delete)
        .map_err(format_op_error)?;
    steps.push(snapshot_from_report(
        &mesh,
        "window.open",
        "edit.delete.faces",
        window_delete_fingerprint,
        &window_delete_result.report,
        &runner,
    ));

    let all_faces = mesh.faces().collect::<Vec<_>>();
    let solidify_plan = runner
        .compile(
            &mesh,
            &SolidifyFaces,
            &SolidifyFacesParams {
                faces: all_faces,
                mode: SolidifyMode::KeepSource,
                thickness: 0.15,
            },
        )
        .map_err(format_op_error)?;
    let solidify_fingerprint = solidify_plan.fingerprint.value();
    let solidify_result = runner
        .apply_in_place(&mut mesh, &SolidifyFaces, &solidify_plan)
        .map_err(format_op_error)?;
    steps.push(snapshot_from_report(
        &mesh,
        "solidify",
        "edit.face.solidify",
        solidify_fingerprint,
        &solidify_result.report,
        &runner,
    ));

    Ok(ScenarioResponse {
        scenario: "wall_openings".to_string(),
        steps,
    })
}

fn run_region_select_flow(options: &ScenarioOptions) -> Result<ScenarioResponse, String> {
    let mut mesh = Mesh::from_polygons(
        &[
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [3.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
            [2.0, 1.0, 0.0],
            [3.0, 1.0, 0.0],
        ],
        &[&[0, 1, 5, 4], &[1, 2, 6, 5], &[2, 3, 7, 6]],
    )
    .map_err(|err| format!("region_select_flow setup failed: {err}"))?;
    let mut steps = Vec::<StepSnapshot>::new();
    let mut runner = OperatorRunner::new();
    let mut faces = mesh.faces().collect::<Vec<_>>();
    faces
        .sort_unstable_by(|a, b| face_centroid_x(&mesh, *a).total_cmp(&face_centroid_x(&mesh, *b)));

    if options.include_initial {
        steps.push(snapshot_from_mesh(
            &mesh,
            "initial",
            None,
            None,
            StepStats::default(),
            &[],
        ));
    }

    let tag_plan = runner
        .compile(
            &mesh,
            &TagFaceRegion,
            &TagFaceRegionParams {
                region_id: 7,
                faces: vec![faces[0], faces[1]],
            },
        )
        .map_err(format_op_error)?;
    let tag_fingerprint = tag_plan.fingerprint.value();
    let tag_result = runner
        .apply_in_place(&mut mesh, &TagFaceRegion, &tag_plan)
        .map_err(format_op_error)?;
    steps.push(snapshot_from_report(
        &mesh,
        "tag.region.7",
        "tag.face.region",
        tag_fingerprint,
        &tag_result.report,
        &runner,
    ));

    let selected = select_faces_by_region(&mesh, 7).map_err(format_op_error)?;
    steps.push(snapshot_from_mesh(
        &mesh,
        "select.region.7",
        Some("select.faces.by_region"),
        None,
        StepStats {
            faces_processed: selected.counters.faces_processed,
            created_vertices: 0,
            created_faces: 0,
            deleted_faces: 0,
            deleted_vertices: 0,
            corners_written: 0,
            edges_written: 0,
        },
        &[],
    ));

    let flood = flood_fill_faces_by_region(&mesh, faces[0]).map_err(format_op_error)?;
    steps.push(snapshot_from_mesh(
        &mesh,
        "flood.region.seed_left",
        Some("select.faces.flood.region"),
        None,
        StepStats {
            faces_processed: flood.counters.faces_processed,
            created_vertices: 0,
            created_faces: 0,
            deleted_faces: 0,
            deleted_vertices: 0,
            corners_written: 0,
            edges_written: 0,
        },
        &[],
    ));

    Ok(ScenarioResponse {
        scenario: "region_select_flow".to_string(),
        steps,
    })
}

fn run_uv_projection_gallery(options: &ScenarioOptions) -> Result<ScenarioResponse, String> {
    let mut mesh = exedra_primitives::box_primitive(&BoxParams::default()).mesh;
    let mut steps = Vec::<StepSnapshot>::new();
    let mut runner = OperatorRunner::new();

    if options.include_initial {
        steps.push(snapshot_from_mesh(
            &mesh,
            "initial",
            None,
            None,
            StepStats::default(),
            &[],
        ));
    }

    let planar = runner
        .compile(
            &mesh,
            &UvPlanar,
            &UvPlanarParams {
                scope: UvScope::WholeMesh,
                plane: UvPlane::WorldXY,
                scale: 1.0,
                offset: [0.0, 0.0],
                write_missing_only: false,
                normal_epsilon: 1.0e-6,
            },
        )
        .map_err(format_op_error)?;
    let planar_fp = planar.fingerprint.value();
    let planar_result = runner
        .apply_in_place(&mut mesh, &UvPlanar, &planar)
        .map_err(format_op_error)?;
    steps.push(snapshot_from_report(
        &mesh,
        "uv.planar",
        "uv.planar",
        planar_fp,
        &planar_result.report,
        &runner,
    ));

    let box_plan = runner
        .compile(
            &mesh,
            &UvBox,
            &UvBoxParams {
                scope: UvScope::WholeMesh,
                scale: 1.0,
                offset: [0.0, 0.0],
                write_missing_only: false,
                normal_epsilon: 1.0e-6,
            },
        )
        .map_err(format_op_error)?;
    let box_fp = box_plan.fingerprint.value();
    let box_result = runner
        .apply_in_place(&mut mesh, &UvBox, &box_plan)
        .map_err(format_op_error)?;
    steps.push(snapshot_from_report(
        &mesh,
        "uv.box",
        "uv.box",
        box_fp,
        &box_result.report,
        &runner,
    ));

    let cyl_plan = runner
        .compile(
            &mesh,
            &UvCylinder,
            &UvCylinderParams {
                scope: UvScope::WholeMesh,
                axis: CylinderAxis::Y,
                seam_offset_radians: 0.25,
                scale: [1.0, 1.0],
                offset: [0.0, 0.0],
                write_missing_only: false,
            },
        )
        .map_err(format_op_error)?;
    let cyl_fp = cyl_plan.fingerprint.value();
    let cyl_result = runner
        .apply_in_place(&mut mesh, &UvCylinder, &cyl_plan)
        .map_err(format_op_error)?;
    steps.push(snapshot_from_report(
        &mesh,
        "uv.cylinder",
        "uv.cylinder",
        cyl_fp,
        &cyl_result.report,
        &runner,
    ));

    Ok(ScenarioResponse {
        scenario: "uv_projection_gallery".to_string(),
        steps,
    })
}

fn run_topology_dissolve_repair(options: &ScenarioOptions) -> Result<ScenarioResponse, String> {
    let mut mesh = exedra_primitives::grid(&GridParams {
        size: [3.0, 1.0],
        segments: [3, 1],
        centered: true,
    })
    .mesh;
    let mut steps = Vec::<StepSnapshot>::new();
    let mut runner = OperatorRunner::new();

    if options.include_initial {
        steps.push(snapshot_from_mesh(
            &mesh,
            "initial",
            None,
            None,
            StepStats::default(),
            &[],
        ));
    }

    let edge = find_canonical_interior_edge(&mesh)
        .ok_or("topology_dissolve_repair expected one canonical interior edge".to_string())?;
    let split_stats = {
        let mut edit = mesh.edit_with(ChangeSetBuilder::new());
        let inserted = op::split_edge(&mut edit, edge, &exedra::PropagatePolicy::default())
            .map_err(|err| err.to_string())?;
        let changes = edit.finish();
        steps.push(snapshot_from_mesh(
            &mesh,
            "split.edge.center",
            Some("kernel.edge.split"),
            None,
            StepStats {
                faces_processed: 0,
                created_vertices: u64::try_from(changes.created_vertices.len())
                    .expect("count should fit u64"),
                created_faces: u64::try_from(changes.created_faces.len())
                    .expect("count should fit u64"),
                deleted_faces: u64::try_from(changes.deleted_faces.len())
                    .expect("count should fit u64"),
                deleted_vertices: u64::try_from(changes.deleted_vertices.len())
                    .expect("count should fit u64"),
                corners_written: 0,
                edges_written: 0,
            },
            &[],
        ));
        inserted
    };

    let dissolve_vertex_plan = runner
        .compile(
            &mesh,
            &DissolveVertices,
            &DissolveVerticesParams {
                vertices: vec![split_stats],
            },
        )
        .map_err(format_op_error)?;
    let dissolve_vertex_fp = dissolve_vertex_plan.fingerprint.value();
    let dissolve_vertex_result = runner
        .apply_in_place(&mut mesh, &DissolveVertices, &dissolve_vertex_plan)
        .map_err(format_op_error)?;
    steps.push(snapshot_from_report(
        &mesh,
        "dissolve.vertex.center",
        "edit.dissolve.vertices",
        dissolve_vertex_fp,
        &dissolve_vertex_result.report,
        &runner,
    ));

    let edge = find_canonical_interior_edge(&mesh)
        .ok_or("topology_dissolve_repair expected one canonical interior edge".to_string())?;
    let first_dissolve_plan = runner
        .compile(
            &mesh,
            &DissolveEdges,
            &DissolveEdgesParams { edges: vec![edge] },
        )
        .map_err(format_op_error)?;
    let first_dissolve_fp = first_dissolve_plan.fingerprint.value();
    let first_dissolve_result = runner
        .apply_in_place(&mut mesh, &DissolveEdges, &first_dissolve_plan)
        .map_err(format_op_error)?;
    steps.push(snapshot_from_report(
        &mesh,
        "dissolve.edge.left",
        "edit.dissolve.edges",
        first_dissolve_fp,
        &first_dissolve_result.report,
        &runner,
    ));

    let edge = find_canonical_interior_edge(&mesh)
        .ok_or("topology_dissolve_repair expected a second canonical interior edge".to_string())?;
    let second_dissolve_plan = runner
        .compile(
            &mesh,
            &DissolveEdges,
            &DissolveEdgesParams { edges: vec![edge] },
        )
        .map_err(format_op_error)?;
    let second_dissolve_fp = second_dissolve_plan.fingerprint.value();
    let second_dissolve_result = runner
        .apply_in_place(&mut mesh, &DissolveEdges, &second_dissolve_plan)
        .map_err(format_op_error)?;
    steps.push(snapshot_from_report(
        &mesh,
        "dissolve.edge.right",
        "edit.dissolve.edges",
        second_dissolve_fp,
        &second_dissolve_result.report,
        &runner,
    ));

    Ok(ScenarioResponse {
        scenario: "topology_dissolve_repair".to_string(),
        steps,
    })
}

fn run_poked_grid(options: &ScenarioOptions) -> Result<ScenarioResponse, String> {
    let mut mesh = exedra_primitives::grid(&GridParams {
        size: [4.0, 3.0],
        segments: [5, 5],
        centered: true,
    })
    .mesh;
    let mut steps = Vec::<StepSnapshot>::new();
    let mut runner = OperatorRunner::new();

    if options.include_initial {
        steps.push(snapshot_from_mesh(
            &mesh,
            "initial",
            None,
            None,
            StepStats::default(),
            &[],
        ));
    }

    let faces = mesh.faces().collect::<Vec<_>>();
    if faces.len() < 25 {
        return Err("poked_grid expected at least twenty-five grid faces".to_string());
    }
    for (label, face, height) in [
        ("poke.face.upper_left", faces[6], 0.35_f32),
        ("poke.face.center", faces[12], 0.7_f32),
        ("poke.face.lower_right", faces[18], 0.45_f32),
    ] {
        let plan = runner
            .compile(&mesh, &PokeFaces, &PokeFacesParams { faces: vec![face] })
            .map_err(format_op_error)?;
        let fingerprint = plan.fingerprint.value();
        let result = runner
            .apply_in_place(&mut mesh, &PokeFaces, &plan)
            .map_err(format_op_error)?;
        let center = result
            .output
            .center_vertices
            .first()
            .copied()
            .ok_or("poked_grid poke produced no center vertex".to_string())?;
        let position = mesh
            .vertex_position(center)
            .ok_or("poked_grid center vertex should stay live".to_string())?;
        let raised = [position[0], position[1], position[2] + height];
        let mut edit = mesh.edit();
        op::set_vertex_position(&mut edit, center, raised)
            .map_err(|err| format!("poked_grid center raise failed unexpectedly: {err}"))?;
        let _: () = edit.finish();
        steps.push(snapshot_from_report(
            &mesh,
            label,
            "edit.face.poke + kernel.vertex.set_position",
            fingerprint,
            &result.report,
            &runner,
        ));
    }

    Ok(ScenarioResponse {
        scenario: "poked_grid".to_string(),
        steps,
    })
}

fn run_bridge_loops(options: &ScenarioOptions) -> Result<ScenarioResponse, String> {
    let mut mesh = Mesh::from_polygons(
        &[
            [-1.20, 0.00, 0.00],
            [-1.41, 0.49, 0.00],
            [-1.90, 0.70, 0.00],
            [-2.39, 0.49, 0.00],
            [-2.60, 0.00, 0.00],
            [-2.39, -0.49, 0.00],
            [-1.90, -0.70, 0.00],
            [-1.41, -0.49, 0.00],
            [2.00, 0.20, 0.80],
            [1.80, 0.69, 0.80],
            [1.31, 0.90, 0.80],
            [0.82, 0.69, 0.80],
            [0.60, 0.20, 0.80],
            [0.82, -0.29, 0.80],
            [1.31, -0.50, 0.80],
            [1.80, -0.29, 0.80],
        ],
        &[&[0, 1, 2, 3, 4, 5, 6, 7], &[8, 15, 14, 13, 12, 11, 10, 9]],
    )
    .map_err(|err| format!("bridge_loops setup failed: {err}"))?;
    let mut steps = Vec::<StepSnapshot>::new();
    let mut runner = OperatorRunner::new();

    if options.include_initial {
        steps.push(snapshot_from_mesh(
            &mesh,
            "initial",
            None,
            None,
            StepStats::default(),
            &[],
        ));
    }

    let faces = mesh.faces().collect::<Vec<_>>();
    if faces.len() != 2 {
        return Err("bridge_loops requires exactly two source faces".to_string());
    }
    let left_seed = mesh
        .face_edge(faces[0])
        .ok_or("bridge_loops missing left face edge")?;
    let right_seed = mesh
        .face_edge(faces[1])
        .ok_or("bridge_loops missing right face edge")?;

    let select_plan = runner
        .compile(
            &mesh,
            &SelectBoundaryEdgeLoop,
            &SelectBoundaryEdgeLoopParams {
                seed_edge: left_seed,
            },
        )
        .map_err(format_op_error)?;
    let select_fp = select_plan.fingerprint.value();
    let select_result = runner
        .apply_in_place(&mut mesh, &SelectBoundaryEdgeLoop, &select_plan)
        .map_err(format_op_error)?;
    let loop_a = select_result.output;
    steps.push(snapshot_from_report(
        &mesh,
        "select.loop.left",
        "select.edgeloop.boundary",
        select_fp,
        &select_result.report,
        &runner,
    ));

    let loop_b = runner
        .compile(
            &mesh,
            &SelectBoundaryEdgeLoop,
            &SelectBoundaryEdgeLoopParams {
                seed_edge: right_seed,
            },
        )
        .map_err(format_op_error)
        .and_then(|plan| {
            runner
                .apply_in_place(&mut mesh, &SelectBoundaryEdgeLoop, &plan)
                .map_err(format_op_error)
        })?
        .output;

    let bridge_plan = runner
        .compile(
            &mesh,
            &BridgeBoundaryLoops,
            &BridgeBoundaryLoopsParams { loop_a, loop_b },
        )
        .map_err(format_op_error)?;
    let bridge_fp = bridge_plan.fingerprint.value();
    let bridge_result = runner
        .apply_in_place(&mut mesh, &BridgeBoundaryLoops, &bridge_plan)
        .map_err(format_op_error)?;
    let bridge_faces = bridge_result.output.bridge_faces.clone();
    steps.push(snapshot_from_report(
        &mesh,
        "bridge.loops",
        "edit.bridge.loops",
        bridge_fp,
        &bridge_result.report,
        &runner,
    ));

    let tag_plan = runner
        .compile(
            &mesh,
            &TagFaceRegion,
            &TagFaceRegionParams {
                region_id: 42,
                faces: bridge_faces,
            },
        )
        .map_err(format_op_error)?;
    let tag_fp = tag_plan.fingerprint.value();
    let tag_result = runner
        .apply_in_place(&mut mesh, &TagFaceRegion, &tag_plan)
        .map_err(format_op_error)?;
    steps.push(snapshot_from_report(
        &mesh,
        "tag.bridge",
        "tag.face.region",
        tag_fp,
        &tag_result.report,
        &runner,
    ));

    Ok(ScenarioResponse {
        scenario: "bridge_loops".to_string(),
        steps,
    })
}

fn run_cylinder_normals(options: &ScenarioOptions) -> Result<ScenarioResponse, String> {
    let primitive = exedra_primitives::cylinder(&CylinderParams::default());
    let mut mesh = primitive.mesh;
    apply_primitive_regions(&mut mesh, &primitive.face_region);
    let mut steps = Vec::<StepSnapshot>::new();
    let mut runner = OperatorRunner::new();

    if options.include_initial {
        steps.push(snapshot_from_mesh(
            &mesh,
            "initial",
            None,
            None,
            StepStats::default(),
            &[],
        ));
    }

    let side_faces =
        select_faces_by_region(&mesh, exedra_primitives::REGION_SIDE.0).map_err(format_op_error)?;

    let flat_plan = runner
        .compile(
            &mesh,
            &BakeFaceNormals,
            &BakeFaceNormalsParams {
                faces: side_faces.faces.clone(),
            },
        )
        .map_err(format_op_error)?;
    let flat_fingerprint = flat_plan.fingerprint.value();
    let flat_result = runner
        .apply_in_place(&mut mesh, &BakeFaceNormals, &flat_plan)
        .map_err(format_op_error)?;
    steps.push(snapshot_from_report(
        &mesh,
        "normal.face.side",
        "edit.normal.face",
        flat_fingerprint,
        &flat_result.report,
        &runner,
    ));

    let smooth_plan = runner
        .compile(
            &mesh,
            &SmoothFaceNormals,
            &SmoothFaceNormalsParams {
                faces: side_faces.faces,
                normal_params: NormalParams::default(),
            },
        )
        .map_err(format_op_error)?;
    let smooth_fingerprint = smooth_plan.fingerprint.value();
    let smooth_result = runner
        .apply_in_place(&mut mesh, &SmoothFaceNormals, &smooth_plan)
        .map_err(format_op_error)?;
    steps.push(snapshot_from_report(
        &mesh,
        "normal.smooth.side",
        "edit.normal.smooth",
        smooth_fingerprint,
        &smooth_result.report,
        &runner,
    ));

    Ok(ScenarioResponse {
        scenario: "cylinder_normals".to_string(),
        steps,
    })
}

fn run_primitive_gallery(_options: &ScenarioOptions) -> Result<ScenarioResponse, String> {
    let mut steps = Vec::<StepSnapshot>::new();
    let quad_primitive = exedra_primitives::quad(&QuadParams::default());
    let mut quad_mesh = quad_primitive.mesh;
    apply_primitive_regions(&mut quad_mesh, &quad_primitive.face_region);
    steps.push(snapshot_from_mesh(
        &quad_mesh,
        "primitive.quad",
        Some("primitive.quad"),
        None,
        StepStats::default(),
        &[],
    ));

    let box_primitive = exedra_primitives::box_primitive(&BoxParams::default());
    let mut box_mesh = box_primitive.mesh;
    apply_primitive_regions(&mut box_mesh, &box_primitive.face_region);
    steps.push(snapshot_from_mesh(
        &box_mesh,
        "primitive.box",
        Some("primitive.box"),
        None,
        StepStats::default(),
        &[],
    ));

    let cylinder_primitive = exedra_primitives::cylinder(&CylinderParams::default());
    let mut cylinder_mesh = cylinder_primitive.mesh;
    apply_primitive_regions(&mut cylinder_mesh, &cylinder_primitive.face_region);
    steps.push(snapshot_from_mesh(
        &cylinder_mesh,
        "primitive.cylinder",
        Some("primitive.cylinder"),
        None,
        StepStats::default(),
        &[],
    ));

    let grid_primitive = exedra_primitives::grid(&GridParams {
        size: [2.0, 1.5],
        segments: [4, 3],
        centered: true,
    });
    let mut grid_mesh = grid_primitive.mesh;
    apply_primitive_regions(&mut grid_mesh, &grid_primitive.face_region);
    steps.push(snapshot_from_mesh(
        &grid_mesh,
        "primitive.grid",
        Some("primitive.grid"),
        None,
        StepStats::default(),
        &[],
    ));

    let cone_primitive = exedra_primitives::cone(&ConeParams {
        radius: 0.9,
        height: 1.8,
        segments: 24,
        cap_fill: CapFill::TriangleFan,
        centered: true,
    });
    let mut cone_mesh = cone_primitive.mesh;
    apply_primitive_regions(&mut cone_mesh, &cone_primitive.face_region);
    steps.push(snapshot_from_mesh(
        &cone_mesh,
        "primitive.cone",
        Some("primitive.cone"),
        None,
        StepStats::default(),
        &[],
    ));

    let torus_primitive = exedra_primitives::torus(&TorusParams {
        major_radius: 1.1,
        minor_radius: 0.35,
        major_segments: 28,
        minor_segments: 16,
    });
    let mut torus_mesh = torus_primitive.mesh;
    apply_primitive_regions(&mut torus_mesh, &torus_primitive.face_region);
    steps.push(snapshot_from_mesh(
        &torus_mesh,
        "primitive.torus",
        Some("primitive.torus"),
        None,
        StepStats::default(),
        &[],
    ));

    let sphere_primitive = exedra_primitives::uv_sphere(&UvSphereParams::default());
    let mut sphere_mesh = sphere_primitive.mesh;
    apply_primitive_regions(&mut sphere_mesh, &sphere_primitive.face_region);
    steps.push(snapshot_from_mesh(
        &sphere_mesh,
        "primitive.uv_sphere",
        Some("primitive.uv_sphere"),
        None,
        StepStats::default(),
        &[],
    ));

    let icosphere_primitive = exedra_primitives::icosphere(&IcosphereParams {
        radius: 1.0,
        subdivisions: 2,
    });
    let mut icosphere_mesh = icosphere_primitive.mesh;
    apply_primitive_regions(&mut icosphere_mesh, &icosphere_primitive.face_region);
    steps.push(snapshot_from_mesh(
        &icosphere_mesh,
        "primitive.icosphere",
        Some("primitive.icosphere"),
        None,
        StepStats::default(),
        &[],
    ));

    Ok(ScenarioResponse {
        scenario: "primitive_gallery".to_string(),
        steps,
    })
}

fn snapshot_from_report(
    mesh: &Mesh,
    label: &str,
    operator: &str,
    plan_fingerprint: u64,
    report: &cambium::OpReport,
    runner: &OperatorRunner,
) -> StepSnapshot {
    let diagnostics = runner
        .ctx
        .diagnostics
        .iter()
        .map(diagnostic_to_step)
        .collect::<Vec<_>>();
    snapshot_from_mesh(
        mesh,
        label,
        Some(operator),
        Some(plan_fingerprint),
        StepStats {
            faces_processed: report.stats.counters.faces_processed,
            created_vertices: report.stats.elements_created.vertices,
            created_faces: report.stats.elements_created.faces,
            deleted_faces: report.stats.elements_deleted.faces,
            deleted_vertices: report.stats.counters.deleted_vertices,
            corners_written: report.stats.counters.corners_written,
            edges_written: report.stats.counters.edges_written,
        },
        &diagnostics,
    )
}

fn snapshot_from_mesh(
    mesh: &Mesh,
    label: &str,
    operator: Option<&str>,
    plan_fingerprint: Option<u64>,
    stats: StepStats,
    diagnostics: &[StepDiagnostic],
) -> StepSnapshot {
    let (tri, _) = mesh.to_trimesh(&ExtractParams {
        normals: NormalsSource::CustomOrDerived,
        ..ExtractParams::default()
    });
    let positions = tri
        .positions
        .into_iter()
        .flat_map(|p| p.into_iter())
        .collect::<Vec<_>>();
    let uvs = tri
        .uvs
        .into_iter()
        .flat_map(|uv| uv.into_iter())
        .collect::<Vec<_>>();
    let normals = tri
        .normals
        .into_iter()
        .flat_map(|n| n.into_iter())
        .collect::<Vec<_>>();
    // Build per-triangle region IDs in the same face order as to_trimesh.
    let region_layer = mesh.attrs().dense(attr::FACE_REGION);
    let mut region_ids = Vec::new();
    for face in mesh.faces() {
        let region = region_layer
            .and_then(|layer| layer.get(face.as_id()).copied())
            .unwrap_or(0);
        let tri_count = mesh.triangulate_face_fan(face).len();
        for _ in 0..tri_count {
            region_ids.push(region);
        }
    }
    let mut topo_edges = mesh
        .faces()
        .flat_map(|face| mesh.face_loop(face))
        .filter_map(|corner| mesh.canonical_edge(corner))
        .collect::<Vec<_>>();
    topo_edges.sort_unstable();
    topo_edges.dedup();
    let mut topology_lines = Vec::with_capacity(topo_edges.len().saturating_mul(6));
    for half_edge in topo_edges {
        let Some(from) = mesh.from_vertex(half_edge) else {
            continue;
        };
        let Some(to) = mesh.to_vertex(half_edge) else {
            continue;
        };
        let Some(from_pos) = mesh.vertex_position(from) else {
            continue;
        };
        let Some(to_pos) = mesh.vertex_position(to) else {
            continue;
        };
        topology_lines.extend_from_slice(from_pos);
        topology_lines.extend_from_slice(to_pos);
    }
    StepSnapshot {
        label: label.to_string(),
        operator: operator.map(ToString::to_string),
        plan_fingerprint,
        mesh_signature: mesh_signature(mesh),
        mesh: MeshBuffers {
            indices: tri.indices,
            positions,
            uvs,
            normals,
            topology_lines,
            region_ids,
        },
        stats,
        diagnostics: diagnostics.to_vec(),
    }
}

fn face_centroid_x(mesh: &Mesh, face: cambium::FaceId) -> f32 {
    let mut sum = 0.0_f32;
    let mut count = 0_u32;
    for corner in mesh.face_loop(face) {
        let Some(vertex) = mesh.to_vertex(corner) else {
            continue;
        };
        let Some(position) = mesh.vertex_position(vertex) else {
            continue;
        };
        sum += position[0];
        count = count.saturating_add(1);
    }
    if count == 0 { 0.0 } else { sum / count as f32 }
}

fn face_centroid_y(mesh: &Mesh, face: cambium::FaceId) -> f32 {
    let mut sum = 0.0_f32;
    let mut count = 0_u32;
    for corner in mesh.face_loop(face) {
        let Some(vertex) = mesh.to_vertex(corner) else {
            continue;
        };
        let Some(position) = mesh.vertex_position(vertex) else {
            continue;
        };
        sum += position[1];
        count = count.saturating_add(1);
    }
    if count == 0 { 0.0 } else { sum / count as f32 }
}

fn find_canonical_interior_edge(mesh: &Mesh) -> Option<cambium::HalfEdgeId> {
    mesh.faces()
        .flat_map(|face| mesh.face_loop(face))
        .find(|&half_edge| {
            let Some(twin) = mesh.twin(half_edge) else {
                return false;
            };
            if core::cmp::min(half_edge, twin) != half_edge {
                return false;
            }
            let Some(face) = mesh.face(half_edge) else {
                return false;
            };
            let Some(twin_face) = mesh.face(twin) else {
                return false;
            };
            face != cambium::FaceId::OUTSIDE && twin_face != cambium::FaceId::OUTSIDE
        })
}

fn diagnostic_to_step(diag: &cambium::Diagnostic) -> StepDiagnostic {
    StepDiagnostic {
        level: level_name(diag.level).to_string(),
        code: code_name(diag.code).to_string(),
        message: diag.message.clone(),
    }
}

fn level_name(level: DiagLevel) -> &'static str {
    match level {
        DiagLevel::Error => "error",
        DiagLevel::Warn => "warn",
        DiagLevel::Note => "note",
    }
}

fn code_name(code: DiagCode) -> &'static str {
    match code {
        DiagCode::PreconditionFailed => "precondition_failed",
        DiagCode::NonManifoldInput => "non_manifold_input",
        DiagCode::MissingRequiredAttribute => "missing_required_attribute",
        DiagCode::NumericToleranceIssue => "numeric_tolerance_issue",
        DiagCode::InternalInvariantViolation => "internal_invariant_violation",
        DiagCode::Cancelled => "cancelled",
        DiagCode::BudgetExceeded => "budget_exceeded",
    }
}

fn format_op_error(error: cambium::OpError) -> String {
    let mut out = format!("operator failed ({:?})", error.kind);
    if !error.diagnostics.is_empty() {
        out.push_str(": ");
        for (i, diag) in error.diagnostics.iter().enumerate() {
            if i > 0 {
                out.push_str(" | ");
            }
            out.push_str(level_name(diag.level));
            out.push(':');
            out.push_str(code_name(diag.code));
            out.push(':');
            out.push_str(&diag.message);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{ScenarioOptions, list_scenarios_json, run_scenario_impl};

    const SCENARIOS: &[&str] = &[
        "boxy_hat",
        "stepped_tower",
        "pedestal",
        "wall_openings",
        "poked_grid",
        "bridge_loops",
        "cylinder_normals",
        "region_select_flow",
        "uv_projection_gallery",
        "topology_dissolve_repair",
        "primitive_gallery",
    ];

    #[test]
    fn lists_scenarios() {
        let list = list_scenarios_json();
        for scenario in SCENARIOS {
            assert!(
                list.contains(scenario),
                "missing scenario in list: {scenario}"
            );
        }
    }

    #[test]
    fn scenario_replay_is_deterministic() {
        let options = ScenarioOptions::default();
        for scenario in SCENARIOS {
            let first = run_scenario_impl(scenario, &options).expect("scenario should run");
            let second = run_scenario_impl(scenario, &options).expect("scenario should run");
            assert_eq!(first.steps.len(), second.steps.len());
            for (a, b) in first.steps.iter().zip(second.steps.iter()) {
                assert_eq!(a.label, b.label);
                assert_eq!(a.operator, b.operator);
                assert_eq!(a.plan_fingerprint, b.plan_fingerprint);
                assert_eq!(a.mesh_signature, b.mesh_signature);
                assert_eq!(a.mesh.indices, b.mesh.indices);
                assert_eq!(a.mesh.positions, b.mesh.positions);
                assert_eq!(a.mesh.uvs, b.mesh.uvs);
                assert_eq!(a.mesh.topology_lines, b.mesh.topology_lines);
                assert_eq!(a.mesh.region_ids, b.mesh.region_ids);
                assert_eq!(a.stats, b.stats);
                assert_eq!(a.diagnostics, b.diagnostics);
            }
        }
    }

    #[test]
    fn unknown_scenario_returns_error() {
        let options = ScenarioOptions::default();
        let err = run_scenario_impl("missing", &options).expect_err("scenario should fail");
        assert!(err.contains("unknown scenario"));
    }

    #[test]
    fn all_scenarios_have_at_least_three_steps() {
        let options = ScenarioOptions::default();
        for scenario in SCENARIOS {
            let result = run_scenario_impl(scenario, &options).expect("scenario should run");
            assert!(
                result.steps.len() >= 3,
                "scenario {scenario} should have at least 3 steps"
            );
        }
    }

    #[test]
    fn primitive_gallery_carries_region_ids() {
        let response = run_scenario_impl("primitive_gallery", &ScenarioOptions::default())
            .expect("primitive gallery should run");
        for step in response.steps {
            assert!(
                !step.mesh.region_ids.is_empty(),
                "primitive gallery step {} should expose region ids",
                step.label
            );
            assert!(
                step.mesh.region_ids.iter().any(|&region| region != 0),
                "primitive gallery step {} should contain non-default regions",
                step.label
            );
        }
    }

    #[test]
    fn primitive_gallery_ignores_include_initial_flag_for_contents() {
        let response = run_scenario_impl(
            "primitive_gallery",
            &ScenarioOptions {
                include_initial: false,
            },
        )
        .expect("primitive gallery should run");
        let labels = response
            .steps
            .iter()
            .map(|step| step.label.as_str())
            .collect::<alloc::vec::Vec<_>>();
        assert_eq!(
            labels,
            alloc::vec![
                "primitive.quad",
                "primitive.box",
                "primitive.cylinder",
                "primitive.grid",
                "primitive.cone",
                "primitive.torus",
                "primitive.uv_sphere",
                "primitive.icosphere",
            ]
        );
    }

    #[test]
    fn poked_grid_raises_center_vertices() {
        let response =
            run_scenario_impl("poked_grid", &ScenarioOptions::default()).expect("scenario runs");
        assert_eq!(response.scenario, "poked_grid");
        assert!(
            response.steps.len() >= 4,
            "poked_grid should include initial plus three poke steps"
        );

        let final_step = response.steps.last().expect("final step should exist");
        let max_z = final_step.mesh.positions[2..]
            .iter()
            .step_by(3)
            .copied()
            .fold(f32::NEG_INFINITY, f32::max);
        assert!(
            max_z > 0.0,
            "poked_grid should raise center vertices above the plane"
        );
    }

    #[test]
    fn cylinder_normals_scenario_shows_flat_then_smooth_steps() {
        let response = run_scenario_impl("cylinder_normals", &ScenarioOptions::default())
            .expect("scenario runs");
        assert_eq!(response.scenario, "cylinder_normals");
        let labels = response
            .steps
            .iter()
            .map(|step| step.label.as_str())
            .collect::<alloc::vec::Vec<_>>();
        assert_eq!(
            labels,
            alloc::vec!["initial", "normal.face.side", "normal.smooth.side"]
        );
        assert_eq!(
            response.steps[1].operator.as_deref(),
            Some("edit.normal.face")
        );
        assert_eq!(
            response.steps[2].operator.as_deref(),
            Some("edit.normal.smooth")
        );
        assert_ne!(
            response.steps[1].mesh.normals, response.steps[2].mesh.normals,
            "flat and smooth steps should produce different normals"
        );
    }

    #[test]
    fn bridge_loops_scenario_selects_then_bridges_then_tags() {
        let response =
            run_scenario_impl("bridge_loops", &ScenarioOptions::default()).expect("scenario runs");
        assert_eq!(response.scenario, "bridge_loops");
        let labels = response
            .steps
            .iter()
            .map(|step| step.label.as_str())
            .collect::<alloc::vec::Vec<_>>();
        assert_eq!(
            labels,
            alloc::vec!["initial", "select.loop.left", "bridge.loops", "tag.bridge"]
        );
        assert_eq!(
            response.steps[1].operator.as_deref(),
            Some("select.edgeloop.boundary")
        );
        assert_eq!(
            response.steps[2].operator.as_deref(),
            Some("edit.bridge.loops")
        );
        assert_eq!(
            response.steps[3].operator.as_deref(),
            Some("tag.face.region")
        );
    }
}
