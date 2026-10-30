// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use layerstack_wasm::{EditBatch, EditReceipt, Stage as OfficialStage};
use serde_json::Value as Json;

const REALM: &str = "test-native-scene-realm-a";
const FIRST: &str = "/World/Placements/P0000";

fn fresh() -> OfficialStage {
    OfficialStage::open_with_provenance(
        backend::ROOT_NAME,
        scene_root(),
        false,
        false,
        layerstack_wasm::BROWSER_LIMITS,
        true,
    )
    .unwrap()
}

fn kernel(instances: u32) -> (OfficialStage, SceneKernel) {
    let mut stage = fresh();
    let kernel = SceneKernel::new_inner(&mut stage, instances, REALM.into()).unwrap();
    (stage, kernel)
}

fn overrides(stage: &OfficialStage) -> LayerId {
    let document = stage.document();
    document
        .store()
        .layer_ids()
        .into_iter()
        .find(|&id| document.identifier(id) == Some("overrides.usda"))
        .unwrap()
}

fn sync(stage: &mut OfficialStage, kernel: &mut SceneKernel) -> Json {
    serde_json::from_str(&kernel.sync_inner(stage).unwrap()).unwrap()
}

fn packet(stage: &mut OfficialStage, kernel: &mut SceneKernel) -> ScenePacket {
    packet::pack(&mut kernel.state.borrow(stage).unwrap(), REALM).unwrap()
}

fn meta(packet: &ScenePacket) -> Json {
    serde_json::from_str(&packet.metadata()).unwrap()
}

fn number_edit(
    stage: &mut OfficialStage,
    target: LayerId,
    path: &str,
    ty: &str,
    value: Vec<f64>,
) -> EditReceipt {
    let mut batch = EditBatch::new();
    let value = batch.value_numbers(ty.into(), value);
    // Official createAttribute copies the composed declaration into stronger layers.
    batch.create_attribute(path.into(), ty.into(), false, false, Some(value));
    stage.apply(target.0, &mut batch).unwrap()
}

#[test]
fn official_edits_undo_and_native_publication_preserve_incremental_contracts() {
    let (mut stage, mut kernel) = kernel(1000);
    assert!(
        stage
            .document()
            .stage()
            .stage()
            .composition_errors()
            .is_empty()
    );
    let initial = sync(&mut stage, &mut kernel);
    assert_eq!(initial["stats"]["geometryEvaluations"], 1);
    let mut first = packet(&mut stage, &mut kernel);
    assert_eq!(meta(&first)["nativePicking"], true);
    assert_eq!(first.take_matrices().len(), 1000 * 16);
    assert!(!first.take_positions().is_empty());
    assert!(first.take_positions().is_empty());

    let no_op = sync(&mut stage, &mut kernel);
    assert_eq!(no_op["stats"]["geometryEvaluations"], 0);
    let unchanged = meta(&packet(&mut stage, &mut kernel));
    for counter in [
        "geometryBytes",
        "transformBytes",
        "prototypeChecks",
        "instanceChecks",
        "transformQueries",
    ] {
        assert_eq!(unchanged[counter], 0, "{counter}");
    }

    let overrides = overrides(&stage);
    let _move = number_edit(
        &mut stage,
        overrides,
        &format!("{FIRST}.xformOp:translate"),
        "double3",
        vec![220.0, 15.0, 4.0],
    );
    let posed = sync(&mut stage, &mut kernel);
    assert_eq!(posed["stats"]["geometryEvaluations"], 0);
    assert_eq!(posed["source"]["scope"], initial["source"]["scope"]);
    let mut moved = packet(&mut stage, &mut kernel);
    assert_eq!(meta(&moved)["instances"], json!([FIRST]));
    assert_eq!(meta(&moved)["geometryBytes"], 0);
    assert_eq!(&moved.take_matrices()[12..15], &[220.0, 15.0, 4.0]);

    let _material = number_edit(
        &mut stage,
        overrides,
        "/World/PanelAsset.displayOpacity",
        "float",
        vec![0.7],
    );
    let appeared = sync(&mut stage, &mut kernel);
    assert_eq!(appeared["stats"]["geometryEvaluations"], 0);
    let appeared = meta(&packet(&mut stage, &mut kernel));
    assert_eq!(appeared["geometryBytes"], 0);
    assert_eq!(appeared["transformBytes"], 0);

    let mut edit = number_edit(
        &mut stage,
        overrides,
        &format!("/World/PanelAsset.{}", native_scene::PANEL_DEPTH),
        "double",
        vec![6.0],
    );
    let changed = sync(&mut stage, &mut kernel);
    assert_eq!(changed["stats"]["geometryEvaluations"], 1);
    assert_ne!(changed["source"]["scope"], initial["source"]["scope"]);
    let changed = meta(&packet(&mut stage, &mut kernel));
    assert!(changed["geometryBytes"].as_u64().unwrap() > 0);
    assert_eq!(changed["transformBytes"], 0);
    stage.undo(&mut edit).unwrap();
    let restored = sync(&mut stage, &mut kernel);
    assert_eq!(restored["stats"]["geometryEvaluations"], 0);
    assert_eq!(restored["source"]["volume"], initial["source"]["volume"]);

    assert!(stage.js_set_layer_muted(overrides.0, true).unwrap());
    sync(&mut stage, &mut kernel);
    let mut muted = packet(&mut stage, &mut kernel);
    assert_eq!(&muted.take_matrices()[12..15], &[160.0, 0.0, 0.0]);
    assert!(stage.js_set_layer_muted(overrides.0, false).unwrap());
    sync(&mut stage, &mut kernel);
    packet(&mut stage, &mut kernel);

    let mut batch = EditBatch::new();
    let translate = batch.value_numbers("double3".into(), vec![10.0, 0.0, 0.0]);
    batch.create_attribute(
        "/World/Placements.xformOp:translate".into(),
        "double3".into(),
        false,
        false,
        Some(translate),
    );
    let order = batch.value_text("token[]".into(), vec!["xformOp:translate".into()]);
    batch.create_attribute(
        "/World/Placements.xformOpOrder".into(),
        "token[]".into(),
        true,
        false,
        Some(order),
    );
    stage.apply(kernel.state.layers.base.0, &mut batch).unwrap();
    let translated = sync(&mut stage, &mut kernel);
    assert_eq!(translated["stats"]["geometryEvaluations"], 0);
    let mut ancestor = packet(&mut stage, &mut kernel);
    assert_eq!(meta(&ancestor)["geometryBytes"], 0);
    assert_eq!(meta(&ancestor)["instances"].as_array().unwrap().len(), 1000);
    assert_eq!(&ancestor.take_matrices()[12..15], &[230.0, 15.0, 4.0]);
}

#[test]
fn official_save_observes_native_publication_and_reopens_all_named_layers() {
    let (mut stage, mut kernel) = kernel(3);
    sync(&mut stage, &mut kernel);
    let layers = kernel.state.layers;
    let overrides = overrides(&stage);
    assert!(stage.document().is_dirty(layers.base));
    assert!(stage.document().is_dirty(layers.generated));
    let _edit = number_edit(
        &mut stage,
        overrides,
        &format!("/World/PanelAsset.{}", native_scene::PANEL_DEPTH),
        "double",
        vec![6.0],
    );
    let current = sync(&mut stage, &mut kernel);
    assert!(stage.document().is_dirty(overrides));
    stage.document_mut().save();
    for id in [layers.root, layers.base, overrides, layers.generated] {
        assert!(!stage.document().is_dirty(id));
    }
    let storage = stage.document().storage();
    let mut reopened = OfficialStage::open_with_provenance(
        backend::ROOT_NAME,
        storage.bytes(backend::ROOT_NAME).unwrap().to_vec(),
        false,
        false,
        layerstack_wasm::BROWSER_LIMITS,
        true,
    )
    .unwrap();
    for name in ["base.usda", "overrides.usda", "generated.usda"] {
        reopened
            .document_mut()
            .storage_mut()
            .supply(name, storage.bytes(name).unwrap().to_vec());
    }
    reopened.document_mut().load_requested_assets().unwrap();
    let doc = reopened.document();
    assert!(doc.stage().stage().composition_errors().is_empty());
    let source = doc.store().path("/World/PanelAsset");
    let actual = doc
        .stage()
        .stage()
        .view(doc.store())
        .prim(source)
        .unwrap()
        .attribute(native_scene::PANEL_DEPTH)
        .unwrap()
        .get(Time::Default)
        .unwrap()
        .value;
    assert_eq!(actual, Value::Double(6.0));
    // Resolve through the official document rather than a separately composed clone.
    assert!(
        doc.stage()
            .stage()
            .view(doc.store())
            .prim(source)
            .unwrap()
            .children()
            .any(|p| p.type_name() == Some("Mesh"))
    );
    let scope = current["source"]["scope"].as_str().unwrap();
    assert!(kernel.pick_inner(&mut stage, FIRST, 0, scope).is_ok());
    assert!(
        kernel.state.borrow(&mut reopened).is_err(),
        "reopened stores require a new geometry evaluator identity"
    );
}

#[test]
fn foreign_stages_and_invalid_seeds_are_refused_before_mutation() {
    let mut no_provenance = OfficialStage::open(
        backend::ROOT_NAME,
        scene_root(),
        false,
        false,
        layerstack_wasm::BROWSER_LIMITS,
    )
    .unwrap();
    let before = no_provenance.document().store().layer_ids();
    let refused = SceneKernel::new_inner(&mut no_provenance, 1, REALM.into()).unwrap_err();
    assert_eq!(
        refused
            .downcast_ref::<layerstack_wasm::Error>()
            .unwrap()
            .code,
        "scene.provenance"
    );
    assert_eq!(no_provenance.document().store().layer_ids(), before);
    let (mut stage, mut kernel) = kernel(1);
    let initial = sync(&mut stage, &mut kernel);
    let mut foreign = fresh();
    let before = foreign.document().store().layer_ids();
    let error = kernel.sync_inner(&mut foreign).unwrap_err();
    assert_eq!(
        error.downcast_ref::<layerstack_wasm::Error>().unwrap().code,
        "scene.stage_affinity"
    );
    assert_eq!(foreign.document().store().layer_ids(), before);
    let stale = kernel.pick_inner(&mut stage, FIRST, 0, "foreign/source/1");
    assert_eq!(error::native(stale.unwrap_err()).code, "scene.stale_pick");
    assert_eq!(
        sync(&mut stage, &mut kernel)["source"]["scope"],
        initial["source"]["scope"]
    );
    assert!(SceneKernel::new_inner(&mut stage, 1, REALM.into()).is_err());
    drop(kernel);
    assert!(
        stage
            .document()
            .stage()
            .stage()
            .view(stage.document().store())
            .prim(stage.document().store().path(FIRST))
            .is_some(),
        "dropping geometry state does not dispose the official stage"
    );
}

#[test]
fn invalid_drafts_preserve_native_output_and_quality_reuses_the_source() {
    let (mut stage, mut kernel) = kernel(1);
    let initial = sync(&mut stage, &mut kernel);
    packet(&mut stage, &mut kernel);
    let overrides = overrides(&stage);
    let mut invalid = number_edit(
        &mut stage,
        overrides,
        &format!("/World/PanelAsset.{}", native_scene::PANEL_DEPTH),
        "double",
        vec![-1.0],
    );
    assert_eq!(
        error::native(kernel.sync_inner(&mut stage).unwrap_err()).code,
        "scene.geometry"
    );
    let held = meta(&packet(&mut stage, &mut kernel));
    assert_eq!(held["sourceScope"], initial["source"]["scope"]);
    assert_eq!(held["geometryBytes"], 0);
    stage.undo(&mut invalid).unwrap();
    let recovered = sync(&mut stage, &mut kernel);
    kernel.state.policy.native.tolerance = 0.1;
    let refined = sync(&mut stage, &mut kernel);
    assert_eq!(refined["stats"]["sourceEvaluations"], 0);
    assert_eq!(refined["stats"]["geometryEvaluations"], 1);
    assert_eq!(refined["source"]["scope"], recovered["source"]["scope"]);
    assert_eq!(refined["accuracy"]["tolerance"], 0.1);
}

#[test]
fn native_f64_pose_refuses_gpu_overflow_without_advancing_transfer_state() {
    let (mut stage, mut kernel) = kernel(1);
    sync(&mut stage, &mut kernel);
    packet(&mut stage, &mut kernel);
    let mut edit = number_edit(
        &mut stage,
        kernel.state.layers.base,
        &format!("{FIRST}.xformOp:translate"),
        "double3",
        vec![1e300, 0.0, 0.0],
    );
    assert_eq!(
        sync(&mut stage, &mut kernel)["stats"]["geometryEvaluations"],
        0
    );
    assert!(packet::pack(&mut kernel.state.borrow(&mut stage).unwrap(), REALM).is_err());
    stage.undo(&mut edit).unwrap();
    sync(&mut stage, &mut kernel);
    let restored = meta(&packet(&mut stage, &mut kernel));
    assert_eq!(restored["geometryBytes"], 0);
    assert_eq!(restored["transformBytes"], 0);
}

fn add_other_panel(stage: &mut OfficialStage, kernel: &SceneKernel, width: f64, depth: f64) {
    let mut batch = EditBatch::new();
    batch.define_prim(
        "/World/OtherPanel".into(),
        "def".into(),
        Some(native_scene::PANEL_SCHEMA.into()),
    );
    let system = batch.value_text("token".into(), vec![native_scene::PANEL_SYSTEM.into()]);
    batch.create_attribute(
        "/World/OtherPanel.proceduralSystem".into(),
        "token".into(),
        false,
        false,
        Some(system),
    );
    for (name, value) in [
        (native_scene::PANEL_WIDTH, width),
        (native_scene::PANEL_DEPTH, depth),
    ] {
        let value = batch.value_numbers("double".into(), vec![value]);
        batch.create_attribute(
            format!("/World/OtherPanel.{name}"),
            "double".into(),
            false,
            false,
            Some(value),
        );
    }
    stage.apply(kernel.state.layers.base.0, &mut batch).unwrap();
    let mut batch = EditBatch::new();
    batch.add_arc(
        false,
        FIRST.into(),
        None,
        Some("/World/OtherPanel".into()),
        "backOfPrepend".into(),
        0.0,
        1.0,
    );
    stage.apply(overrides(stage).0, &mut batch).unwrap();
}

#[test]
fn stronger_reference_cannot_borrow_another_native_sources_identity() {
    for (width, depth) in [(160.0, 3.0), (120.0, 5.0)] {
        let (mut stage, mut kernel) = kernel(2);
        sync(&mut stage, &mut kernel);
        packet(&mut stage, &mut kernel);
        add_other_panel(&mut stage, &kernel, width, depth);
        let current = sync(&mut stage, &mut kernel);
        let sources = &kernel.state.snapshot.as_ref().unwrap().sources;
        assert_eq!(sources.len(), 2);
        if width == 120.0 {
            assert!(sources[0].shares_geometry(&sources[1]));
            assert!(
                !sources[0]
                    .native()
                    .unwrap()
                    .same_installation(sources[1].native().unwrap())
            );
        }
        let instance = if sources[0].source == "/World/PanelAsset" {
            FIRST
        } else {
            "/World/Placements/P0001"
        };
        let picked = kernel
            .pick_inner(
                &mut stage,
                instance,
                0,
                current["source"]["scope"].as_str().unwrap(),
            )
            .unwrap_err();
        assert_eq!(
            picked
                .downcast_ref::<layerstack_wasm::Error>()
                .unwrap()
                .code,
            "scene.instanceCorrespondence"
        );
        assert!(packet::pack(&mut kernel.state.borrow(&mut stage).unwrap(), REALM).is_err());
    }
}

#[test]
fn native_error_records_preserve_extension_codes_and_path_context() {
    assert_eq!(layerstack_wasm::ABI, 3);
    let original = layerstack_wasm::Error::new("scene.stale_pick", "extract again").at_path(FIRST);
    assert_eq!(error::native(Box::new(original.clone())), original);
    let generic = error::native("missing source".into());
    assert_eq!(generic.code, "scene.operation");
    assert_eq!(generic.message, "missing source");
}
