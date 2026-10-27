// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use serde_json::{Value, json};

const REALM: &str = "test-native-scene-realm-a";
fn kernel(instances: u32) -> SceneKernel {
    SceneKernel::new(instances, REALM.into()).unwrap()
}
fn call(kernel: &mut SceneKernel, namespace: &str, op: Value) -> Value {
    let request = json!({"protocol":namespace,"version":1,"stage":protocol::stage_id(&kernel.realm),"store":protocol::store_id(&kernel.realm),"op":op});
    serde_json::from_str(&kernel.request(&request.to_string())).unwrap()
}
fn stage(kernel: &mut SceneKernel, op: Value) -> Value {
    let response = call(kernel, "layerstack", op);
    assert_eq!(response["ok"], true, "{response}");
    response["result"].clone()
}
fn sync(kernel: &mut SceneKernel) -> Value {
    let response = call(kernel, "exedra.scene", json!({"type":"sync"}));
    assert_eq!(response["ok"], true, "{response}");
    response["result"].clone()
}
fn packet(kernel: &mut SceneKernel) -> ScenePacket {
    packet::pack(kernel.backend.as_mut().unwrap(), &kernel.realm).unwrap()
}
fn meta(packet: &ScenePacket) -> Value {
    serde_json::from_str(&packet.metadata()).unwrap()
}
fn create_attribute(path: &str, name: &str, value: Value) -> Value {
    json!({"type":"createAttribute","path":path,"name":name,"typeName":value["type"],"value":value})
}

#[test]
fn native_stage_edits_transfer_only_changed_prototype_or_instance_data() {
    let mut kernel = kernel(1000);
    let info = stage(&mut kernel, json!({"type":"inspect"}));
    assert!(
        info["schemaTypes"]
            .as_array()
            .unwrap()
            .contains(&json!(native_scene::PANEL_SCHEMA))
    );
    assert_eq!(info["compositionErrors"], json!([]));
    let initial = sync(&mut kernel);
    assert_eq!(initial["stats"]["geometryEvaluations"], 1);
    let mut first = packet(&mut kernel);
    assert!(meta(&first)["nativePicking"].as_bool().unwrap());
    assert_eq!(first.take_matrices().len(), 1000 * 16);
    assert!(!first.take_positions().is_empty());
    assert!(
        first.take_positions().is_empty(),
        "transfers consume their buffers exactly once"
    );
    let noop = sync(&mut kernel);
    assert_eq!(noop["stats"]["geometryEvaluations"], 0);
    let mut no_change = packet(&mut kernel);
    assert_eq!(meta(&no_change)["geometryBytes"], 0);
    assert_eq!(meta(&no_change)["transformBytes"], 0);
    assert_eq!(meta(&no_change)["prototypeChecks"], 0);
    assert_eq!(meta(&no_change)["instanceChecks"], 0);
    assert!(no_change.take_positions().is_empty());
    let moved = stage(
        &mut kernel,
        json!({"type":"transaction","target":protocol::layer_handle(REALM,backend::OVERRIDES),"edits":[{"type":"setAttribute","path":"/World/Placements/P0000","name":"xformOp:translate","value":{"type":"double3","value":[220,15,4]}}]}),
    );
    let posed = sync(&mut kernel);
    assert_eq!(posed["stats"]["geometryEvaluations"], 0);
    assert_eq!(posed["source"]["scope"], initial["source"]["scope"]);
    let mut placement = packet(&mut kernel);
    assert_eq!(
        meta(&placement)["instances"].as_array().unwrap().len(),
        1,
        "one changed native instance"
    );
    assert_eq!(meta(&placement)["instances"][0], "/World/Placements/P0000");
    assert_eq!(meta(&placement)["geometryBytes"], 0);
    let matrices = placement.take_matrices();
    assert_eq!(matrices.len(), 16);
    assert_eq!(&matrices[12..15], &[220.0, 15.0, 4.0]);
    let edit = stage(
        &mut kernel,
        json!({"type":"transaction","target":protocol::layer_handle(REALM,backend::OVERRIDES),"edits":[create_attribute("/World/PanelAsset",native_scene::PANEL_DEPTH,json!({"type":"double","value":6}))]}),
    );
    let changed = sync(&mut kernel);
    assert_eq!(changed["stats"]["geometryEvaluations"], 1);
    assert_ne!(changed["source"]["scope"], initial["source"]["scope"]);
    let changed_packet = packet(&mut kernel);
    assert!(meta(&changed_packet)["geometryBytes"].as_u64().unwrap() > 0);
    assert_eq!(meta(&changed_packet)["transformBytes"], 0);
    stage(
        &mut kernel,
        json!({"type":"undo","receipt":edit["receipt"]}),
    );
    let restored = sync(&mut kernel);
    assert_eq!(restored["stats"]["geometryEvaluations"], 0);
    assert_eq!(restored["source"]["volume"], initial["source"]["volume"]);
    stage(
        &mut kernel,
        json!({"type":"releaseReceipt","receipt":moved["receipt"]}),
    );
    let muted = stage(
        &mut kernel,
        json!({"type":"mute","layer":protocol::layer_handle(REALM,backend::OVERRIDES),"muted":true}),
    );
    assert!(muted["changes"].is_object());
    sync(&mut kernel);
    let queried = stage(
        &mut kernel,
        json!({"type":"query","path":"/World/Placements/P0000","attributes":["xformOp:translate"]}),
    );
    assert_eq!(queried["worldTransform"][12], 160.0);
    assert_eq!(queried["isInstance"], true);
    stage(
        &mut kernel,
        json!({"type":"mute","layer":protocol::layer_handle(REALM,backend::OVERRIDES),"muted":false}),
    );
    sync(&mut kernel);
    packet(&mut kernel);
    // The first stronger edit had incomplete ancestor inventories; conservative
    // queries must still emit only changed values. A real ancestor transform,
    // however, must affect every descendant native instance.
    stage(
        &mut kernel,
        json!({"type":"transaction","target":protocol::layer_handle(REALM,backend::BASE),"edits":[
            create_attribute("/World/Placements","xformOp:translate",json!({"type":"double3","value":[10,0,0]})),
            {"type":"createAttribute","path":"/World/Placements","name":"xformOpOrder","typeName":"token[]","variability":"uniform","value":{"type":"token[]","value":["xformOp:translate"]}}
        ]}),
    );
    let translated = sync(&mut kernel);
    assert_eq!(translated["stats"]["geometryEvaluations"], 0);
    let mut ancestor = packet(&mut kernel);
    assert_eq!(meta(&ancestor)["geometryBytes"], 0);
    assert_eq!(meta(&ancestor)["instances"].as_array().unwrap().len(), 1000);
    let matrices = ancestor.take_matrices();
    assert_eq!(&matrices[12..15], &[230.0, 15.0, 4.0]);
}

#[test]
fn generic_native_transport_checks_affinity_exact_values_and_atomic_failure() {
    let mut kernel = kernel(2);
    let mut other = SceneKernel::new(2, "test-native-scene-realm-b".into()).unwrap();
    let foreign = json!({"protocol":"layerstack","version":1,"stage":protocol::stage_id(REALM),"store":protocol::store_id(REALM),"op":{"type":"transaction","target":protocol::layer_handle(REALM,backend::BASE),"edits":[]}});
    let rejected: Value = serde_json::from_str(&other.request(&foreign.to_string())).unwrap();
    assert_eq!(rejected["ok"], false);
    assert_eq!(rejected["error"]["code"], "stage.affinity");
    let rejected = call(
        &mut other,
        "layerstack",
        json!({"type":"transaction","target":protocol::layer_handle(REALM,backend::BASE),"edits":[]}),
    );
    assert_eq!(
        rejected["ok"], false,
        "foreign layer handle refused despite correct stage envelope"
    );
    assert_eq!(rejected["error"]["code"], "handle.affinity");
    let edit = stage(
        &mut kernel,
        json!({"type":"transaction","target":protocol::layer_handle(REALM,backend::OVERRIDES),"edits":[create_attribute("/World/PanelAsset","serial",json!({"type":"uint64","value":"18446744073709551615"}))]}),
    );
    let queried = stage(
        &mut kernel,
        json!({"type":"query","path":"/World/PanelAsset","attributes":["serial"]}),
    );
    assert_eq!(
        queried["attributes"]["serial"]["value"]["value"],
        "18446744073709551615"
    );
    let rejected = call(
        &mut kernel,
        "layerstack",
        json!({"type":"transaction","target":protocol::layer_handle(REALM,backend::OVERRIDES),"edits":[create_attribute("/World/PanelAsset","mustNotAppear",json!({"type":"double","value":2})),{"type":"addReference","path":"/World/Bad","layer":"other/layer/1","primPath":"/World/PanelAsset"}]}),
    );
    assert_eq!(rejected["ok"], false);
    let queried = stage(
        &mut kernel,
        json!({"type":"query","path":"/World/PanelAsset","attributes":["mustNotAppear"]}),
    );
    assert!(queried["attributes"]["mustNotAppear"].is_null());
    stage(
        &mut kernel,
        json!({"type":"undo","receipt":edit["receipt"]}),
    );
    assert_eq!(
        call(
            &mut kernel,
            "layerstack",
            json!({"type":"undo","receipt":edit["receipt"]})
        )["ok"],
        false
    );
    let text = stage(
        &mut kernel,
        json!({"type":"layerText","layer":protocol::layer_handle(REALM,backend::ROOT)}),
    );
    assert!(
        layerstack_usda::parser::parse(text["text"].as_str().unwrap())
            .diagnostics
            .is_empty()
    );
    stage(&mut kernel, json!({"type":"dispose"}));
    assert!(
        kernel.backend.is_none(),
        "stage and geometry owners released"
    );
    assert_eq!(
        call(&mut kernel, "layerstack", json!({"type":"inspect"}))["ok"],
        false
    );
    let oversized: Value =
        serde_json::from_str(&kernel.request(&"x".repeat((1 << 20) + 1))).unwrap();
    assert_eq!(oversized["error"]["code"], "request.budget");
}

#[test]
fn native_pose_range_is_distinct_from_gpu_range_and_failed_packets_keep_last_sent_state() {
    let mut kernel = kernel(1);
    sync(&mut kernel);
    let _held = packet(&mut kernel);
    let edit = stage(
        &mut kernel,
        json!({"type":"transaction","target":protocol::layer_handle(REALM,backend::BASE),"edits":[{"type":"setAttribute","path":"/World/Placements/P0000","name":"xformOp:translate","value":{"type":"double3","value":[1e300,0,0]}}]}),
    );
    let result = sync(&mut kernel);
    assert_eq!(result["stats"]["geometryEvaluations"], 0);
    let query = stage(
        &mut kernel,
        json!({"type":"query","path":"/World/Placements/P0000","attributes":[]}),
    );
    assert_eq!(
        query["worldTransform"][12],
        json!(1e300),
        "native stage retains f64 placement"
    );
    assert!(
        packet::pack(kernel.backend.as_mut().unwrap(), REALM).is_err(),
        "GPU range must be refused before narrowing"
    );
    stage(
        &mut kernel,
        json!({"type":"undo","receipt":edit["receipt"]}),
    );
    sync(&mut kernel);
    let restored = packet(&mut kernel);
    assert_eq!(
        meta(&restored)["transformBytes"],
        0,
        "failed transfer did not advance last-sent matrices"
    );
    assert_eq!(meta(&restored)["geometryBytes"], 0);
}

#[test]
fn stronger_instance_reference_cannot_borrow_another_sources_render_or_pick_identity() {
    let mut kernel = kernel(2);
    sync(&mut kernel);
    packet(&mut kernel);
    add_other_panel(&mut kernel, 160.0, 3.0);
    let current = sync(&mut kernel);
    for (path, width) in [
        ("/World/Placements/P0000", 160.0),
        ("/World/Placements/P0001", 120.0),
    ] {
        let query = stage(
            &mut kernel,
            json!({"type":"query","path":path,"attributes":[native_scene::PANEL_WIDTH]}),
        );
        assert_eq!(query["isInstance"], true);
        assert_eq!(
            query["attributes"][native_scene::PANEL_WIDTH]["value"]["value"],
            width
        );
    }
    let (different, proxy_widths) = {
        let b = kernel.backend.as_mut().unwrap();
        let different = mismatched_instance(b);
        let mut widths = Vec::new();
        for parent in ["/World/Placements/P0000", "/World/Placements/P0001"] {
            let child = b
                .store
                .path(&format!("{parent}/{}", exedra_scene::GENERATED_CHILD));
            let points = b
                .live
                .stage()
                .view(&b.store)
                .prim(child)
                .unwrap()
                .attribute("points")
                .unwrap()
                .get(Time::Default)
                .unwrap()
                .value;
            let exedra_scene::composition::Value::TypedArray(TypedArray::Vec3f(points)) = points
            else {
                panic!("native proxy points")
            };
            widths.push(
                points
                    .iter()
                    .map(|p| p[0])
                    .fold(f32::NEG_INFINITY, f32::max)
                    - points.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min),
            );
        }
        (different, widths)
    };
    assert_eq!(
        proxy_widths,
        vec![160.0, 120.0],
        "actual native instance proxy geometry differs"
    );
    let rendered = packet::pack(kernel.backend.as_mut().unwrap(), REALM);
    let picked = call(
        &mut kernel,
        "exedra.scene",
        json!({"type":"pick","instance":different,"triangle":0,"sourceScope":current["source"]["scope"]}),
    );
    assert!(
        rendered.is_err(),
        "one-prototype renderer must refuse differing native proxy geometry"
    );
    assert_eq!(
        picked["ok"], false,
        "native pick cannot borrow the wrong retained source: {picked}"
    );
    assert_eq!(picked["error"]["code"], "scene.instanceCorrespondence");
}

fn add_other_panel(kernel: &mut SceneKernel, width: f64, depth: f64) {
    stage(
        kernel,
        json!({"type":"transaction","target":protocol::layer_handle(REALM,backend::BASE),"edits":[
            {"type":"definePrim","path":"/World/OtherPanel","typeName":native_scene::PANEL_SCHEMA},
            create_attribute("/World/OtherPanel",native_scene::PANEL_WIDTH,json!({"type":"double","value":width})),
            create_attribute("/World/OtherPanel",native_scene::PANEL_DEPTH,json!({"type":"double","value":depth}))
        ]}),
    );
    stage(
        kernel,
        json!({"type":"transaction","target":protocol::layer_handle(REALM,backend::OVERRIDES),"edits":[
            {"type":"addReference","path":"/World/Placements/P0000","layer":protocol::layer_handle(REALM,backend::ROOT),"primPath":"/World/OtherPanel"}
        ]}),
    );
}
fn mismatched_instance(b: &Backend) -> &'static str {
    if b.snapshot.as_ref().unwrap().sources[0].source == "/World/PanelAsset" {
        "/World/Placements/P0000"
    } else {
        "/World/Placements/P0001"
    }
}

#[test]
fn shared_equal_geometry_buffers_do_not_erase_native_source_provenance() {
    let mut kernel = kernel(2);
    sync(&mut kernel);
    packet(&mut kernel);
    add_other_panel(&mut kernel, 120.0, 5.0);
    let current = sync(&mut kernel);
    let b = kernel.backend.as_mut().unwrap();
    let sources = &b.snapshot.as_ref().unwrap().sources;
    assert_eq!(sources.len(), 2);
    assert!(
        sources[0].shares_geometry(&sources[1]),
        "equal geometry really shares compiler storage"
    );
    assert!(
        !sources[0]
            .native()
            .unwrap()
            .same_installation(sources[1].native().unwrap()),
        "retained native identities remain independent"
    );
    let instance = mismatched_instance(b);
    let proxy = b
        .store
        .path(&format!("{instance}/{}", exedra_scene::GENERATED_CHILD));
    let source = packet::composed_arrays(b).unwrap();
    let actual = packet::mesh_values_at(b, proxy).unwrap();
    assert!(
        source
            .iter()
            .zip(&actual)
            .all(|(a, (b, _))| a.shares_storage(b)),
        "numeric buffers really are shared; provenance must distinguish them"
    );
    let result = call(
        &mut kernel,
        "exedra.scene",
        json!({"type":"pick","instance":instance,"triangle":0,"sourceScope":current["source"]["scope"]}),
    );
    assert_eq!(result["error"]["code"], "scene.instanceCorrespondence");
    assert!(packet::pack(kernel.backend.as_mut().unwrap(), REALM).is_err());
}
