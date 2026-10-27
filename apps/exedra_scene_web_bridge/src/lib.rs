// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Lazy browser application combining a native stage with Exedra construction.
//!
//! The application owns one store and stage. The geometry adapter borrows them;
//! no geometry handles cross independent Wasm memories. Host clocks measure
//! transactions, synchronization and transfer separately.

mod backend;
mod correspondence;
mod packet;
mod protocol;
#[cfg(test)]
mod tests;
pub use packet::ScenePacket;

use backend::{Backend, Result};
use exedra_scene::{Time, composition::*};
use serde_json::json;
use wasm_bindgen::prelude::*;

fn js_error(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&json!({"code":"scene.operation","message":error.to_string()}).to_string())
}
fn js_native_error(error: Box<dyn std::error::Error>) -> JsValue {
    let code = error
        .downcast_ref::<protocol::Refusal>()
        .map_or("scene.operation", |r| r.code);
    JsValue::from_str(&json!({"code":code,"message":error.to_string()}).to_string())
}

/// Internal demo application backend; its stage transport is supplied separately.
#[derive(Debug)]
#[wasm_bindgen]
pub struct SceneKernel {
    backend: Option<Backend>,
    realm: String,
}

#[wasm_bindgen]
impl SceneKernel {
    /// Creates real native prototype references on a caller-owned application stage.
    ///
    /// # Errors
    /// Rejects invalid instance budgets or native stage/schema authoring failures.
    #[wasm_bindgen(constructor)]
    pub fn new(instances: u32, realm: String) -> std::result::Result<Self, JsValue> {
        if !(16..=128).contains(&realm.len())
            || !realm
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(js_error(
                "supply a unique host-generated backend realm, such as crypto.randomUUID()",
            ));
        }
        Backend::new(instances)
            .map(|backend| Self {
                backend: Some(backend),
                realm,
            })
            .map_err(js_native_error)
    }

    /// Extracts actual composed USD values and changed native instance transforms.
    /// No-op and placement-only edits transfer no polygon buffers.
    ///
    /// # Errors
    /// Refuses unsupported topology or invalid composed values explicitly.
    pub fn pack(&mut self) -> std::result::Result<ScenePacket, JsValue> {
        packet::pack(
            self.backend
                .as_mut()
                .ok_or_else(|| js_error("stage is disposed"))?,
            &self.realm,
        )
        .map_err(js_native_error)
    }

    /// Dispatches the versioned generic Layerstack transport on this same stage.
    /// Errors are structured envelopes; no Rust or Wasm handles cross the boundary.
    pub fn request(&mut self, raw: &str) -> String {
        let request: Result<serde_json::Value> = if raw.len() > 1 << 20 {
            Err(protocol::refusal(
                "request.budget",
                "stage request exceeds one MiB",
            ))
        } else {
            serde_json::from_str(raw).map_err(Into::into)
        };
        let namespace = request
            .as_ref()
            .ok()
            .and_then(|v| {
                v.get("protocol")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| "layerstack".into());
        let context=request.as_ref().ok().and_then(|v|v.get("op")).map(|op|json!({"operation":op.get("type"),"path":op.get("path"),"target":op.get("target")}));
        let result = request.and_then(|request| self.request_inner(&request));
        let mut response = json!({"protocol":namespace,"version":1,"stage":protocol::stage_id(&self.realm),"store":protocol::store_id(&self.realm),"ok":result.is_ok()});
        match result {
            Ok(value) => response["result"] = value,
            Err(error) => {
                let code = if let Some(refusal) = error.downcast_ref::<protocol::Refusal>() {
                    refusal.code
                } else if error.is::<serde_json::Error>() {
                    "request.invalidJson"
                } else {
                    "stage.nativeRefused"
                };
                response["error"] =
                    json!({"code":code,"message":error.to_string(),"context":context});
            }
        }
        response.to_string()
    }
}

impl SceneKernel {
    fn sync_inner(&mut self) -> Result<String> {
        let b = self.backend.as_mut().ok_or("stage is disposed")?;
        let next_source = b
            .source_revision
            .checked_add(1)
            .ok_or("source revision budget exhausted")?;
        let snapshot = b.evaluator.sync(
            &mut b.live,
            &mut b.store,
            &mut b.compiler,
            Time::Default,
            &b.policy,
        )?;
        let same_source = b
            .snapshot
            .as_ref()
            .and_then(|s| s.sources.first())
            .and_then(|s| s.native())
            .zip(snapshot.sources.first().and_then(|s| s.native()))
            .is_some_and(|(old, new)| old.same_installation(new));
        let s = snapshot.stats;
        let native = snapshot
            .sources
            .first()
            .and_then(|source| source.native())
            .ok_or("missing native source")?;
        let fresh = if same_source {
            None
        } else {
            Some(backend::SourceSummary {
                volume: native.volume()?,
                area: native.surface_area()?,
                bounds: native.conservative_bounds()?,
                fingerprint: format!("{:032x}", snapshot.sources[0].input().fingerprint().0),
            })
        };
        let summary = fresh
            .as_ref()
            .or(b.source_summary.as_ref())
            .ok_or("missing retained source summary")?;
        let revision = if same_source {
            b.source_revision
        } else {
            next_source
        };
        let body = snapshot.sources[0].body(0).ok_or("missing source body")?;
        let report = &body
            .native_realization()
            .ok_or("missing native realization")?
            .report;
        let response = json!({
            "stats":{
                "discoveredPrims":s.discovered_prims,"sourceEvaluations":s.source_evaluations,
                "inputResolutions":s.input_resolutions,"inputReused":s.input_reused,
                "geometryEvaluations":s.geometry_evaluations,"geometryReused":s.geometry_reused,
                "polygonEncodings":s.polygon_encodings,"polygonValidations":s.polygon_validations,
                "publicationProperties":s.publication_properties,"meshesWritten":s.meshes_written,
                "pointsWritten":s.points_written,"sourceBoundsWritten":s.source_bounds_written,
            },
            "source":{"volume":summary.volume,"surfaceArea":summary.area,"bounds":summary.bounds,"scope":format!("{}/source/{}",self.realm,revision),"fingerprint":summary.fingerprint},
            "accuracy":{"tolerance":report.tolerance,"maxDeviation":report.max_deviation,"withinTolerance":report.within_tolerance()},
            "units":"mm","accuracySpace":"source-local",
        }).to_string();
        if fresh.is_some() {
            b.source_summary = fresh;
        }
        b.source_revision = revision;
        b.snapshot = Some(snapshot);
        Ok(response)
    }

    fn request_inner(&mut self, request: &serde_json::Value) -> Result<serde_json::Value> {
        let namespace = request
            .get("protocol")
            .and_then(serde_json::Value::as_str)
            .ok_or("missing protocol namespace")?;
        if !["layerstack", "exedra.scene"].contains(&namespace)
            || request.get("version").and_then(serde_json::Value::as_u64) != Some(1)
        {
            return Err(protocol::refusal(
                "protocol.unsupported",
                "unsupported stage protocol/version",
            ));
        }
        let op = request.get("op").ok_or("missing operation")?;
        let kind = protocol::text(op, "type")?;
        if (namespace != "layerstack" || kind != "describe")
            && (protocol::text(request, "stage")? != protocol::stage_id(&self.realm)
                || protocol::text(request, "store")? != protocol::store_id(&self.realm))
        {
            return Err(protocol::refusal(
                "stage.affinity",
                "foreign stage/store affinity",
            ));
        }
        if namespace == "exedra.scene" {
            return self.scene_operation(op);
        }
        if kind == "dispose" {
            self.backend
                .take()
                .ok_or_else(|| protocol::refusal("stage.disposed", "stage is disposed"))?;
            return Ok(json!({}));
        }
        protocol::dispatch(
            self.backend
                .as_mut()
                .ok_or_else(|| protocol::refusal("stage.disposed", "stage is disposed"))?,
            &self.realm,
            op,
        )
    }

    fn scene_operation(&mut self, op: &serde_json::Value) -> Result<serde_json::Value> {
        match protocol::text(op, "type")? {
            "sync" => Ok(serde_json::from_str(&self.sync_inner()?)?),
            "displayPolicy" => {
                let tolerance = op
                    .get("tolerance")
                    .and_then(serde_json::Value::as_f64)
                    .filter(|v| v.is_finite() && *v >= 1e-6 && *v <= 10.0)
                    .ok_or("display tolerance must be finite between1e-6and10mm")?;
                self.backend
                    .as_mut()
                    .ok_or("stage is disposed")?
                    .policy
                    .native
                    .tolerance = tolerance;
                Ok(json!({}))
            }
            "pick" => {
                let b = self.backend.as_mut().ok_or("stage is disposed")?;
                b.live.synchronize(&mut b.store);
                if protocol::text(op, "sourceScope")?
                    != format!("{}/source/{}", self.realm, b.source_revision)
                {
                    return Err(
                        "display source is stale; extract current scene before picking".into(),
                    );
                }
                let instance = protocol::text(op, "instance")?;
                let path = b
                    .store
                    .paths
                    .lookup(&Path::parse_absolute(instance, &b.store.tokens)?)
                    .ok_or("unknown instance path")?;
                if !b.placements.contains(&path)
                    || !b
                        .live
                        .stage()
                        .view(&b.store)
                        .prim(path)
                        .is_some_and(|p| p.is_instance())
                {
                    return Err("pick requires a current native instance root".into());
                }
                correspondence::validate_pick(b, path)?;
                let arrays = packet::composed_arrays(b)?;
                if !packet::producer_output(b, &arrays) {
                    return Err("composed geometry overrides producer output; native picking is unavailable".into());
                }
                let triangle = usize::try_from(
                    op.get("triangle")
                        .and_then(serde_json::Value::as_u64)
                        .ok_or("expected triangle ordinal")?,
                )?;
                let source = b
                    .snapshot
                    .as_ref()
                    .and_then(|s| s.sources.first())
                    .ok_or("sync before picking")?;
                let selected = source
                    .published_native_face(triangle)
                    .ok_or("unknown generated triangle")?;
                let native = source.native().ok_or("missing native source")?;
                let face = selected.native_id();
                Ok(
                    json!({"source":source.source,"sourceScope":protocol::text(op,"sourceScope")?,"instance":instance,"nativeFace":face.0,"tags":native.face_tags(face)?.map(|tag|tag.0).collect::<Vec<_>>(),"planarFrame":native.planar_frame(face).ok().map(|frame|frame.rows),"units":"mm","frameSpace":"source-local"}),
                )
            }
            unknown => Err(format!("unsupported Exedra scene operation {unknown}").into()),
        }
    }
}
