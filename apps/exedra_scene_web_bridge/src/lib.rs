// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exedra geometry over a caller-owned official Layerstack Stage in one Wasm memory.
//! Layerstack owns composition, typed edits, undo, asset loading and persistence.
//! This module owns geometry evaluation, display deltas and native correspondence.

mod backend;
mod correspondence;
mod error;
mod packet;
#[cfg(test)]
mod tests;
pub use packet::ScenePacket;

use backend::{Result, State};
use exedra_scene::{Time, composition::*};
use serde_json::json;
use wasm_bindgen::prelude::*;

fn js_native_error(error: Box<dyn std::error::Error>) -> JsValue {
    error::native(error).into()
}

/// Root bytes for a fresh demo stage, opened by the official Layerstack binding.
#[must_use]
#[wasm_bindgen]
pub fn scene_root() -> Vec<u8> {
    backend::ROOT_BYTES.to_vec()
}

/// Retained geometry and extraction state, bound to one caller-owned official Stage.
///
/// Every operation borrows that Stage again. Freeing this object never frees the
/// Stage, and passing another Stage refuses before either document is changed.
#[derive(Debug)]
#[wasm_bindgen]
pub struct SceneKernel {
    state: State,
    realm: String,
}

#[wasm_bindgen]
impl SceneKernel {
    /// Authors a demo assembly into a fresh stage opened from [`scene_root`].
    ///
    /// # Errors
    /// Rejects invalid counts, realms, seeds or native authoring failures.
    #[wasm_bindgen(constructor)]
    pub fn new(
        stage: &mut layerstack_wasm::Stage,
        instances: u32,
        realm: String,
    ) -> std::result::Result<Self, JsValue> {
        Self::new_inner(stage, instances, realm).map_err(js_native_error)
    }

    /// Evaluates changed source geometry and publishes into the official document.
    ///
    /// # Errors
    /// Rejects foreign stages or invalid authored inputs, preserving retained output.
    pub fn sync(
        &mut self,
        stage: &mut layerstack_wasm::Stage,
    ) -> std::result::Result<String, JsValue> {
        self.sync_inner(stage).map_err(js_native_error)
    }

    /// Extracts actual composed polygon and occurrence deltas.
    /// No-op and placement-only edits transfer no polygon buffers.
    ///
    /// # Errors
    /// Refuses foreign stages, unsupported topology or invalid composed values.
    pub fn pack(
        &mut self,
        stage: &mut layerstack_wasm::Stage,
    ) -> std::result::Result<ScenePacket, JsValue> {
        let mut context = self.state.borrow(stage).map_err(js_native_error)?;
        packet::pack(&mut context, &self.realm).map_err(js_native_error)
    }

    /// Sets source-local display tolerance without replacing retained native geometry.
    ///
    /// # Errors
    /// Rejects nonfinite tolerances or values outside 1e-6 through 10 millimetres.
    pub fn set_display_tolerance(&mut self, tolerance: f64) -> std::result::Result<(), JsValue> {
        if !tolerance.is_finite() || !(1e-6..=10.0).contains(&tolerance) {
            return Err(js_native_error(
                "display tolerance must be finite between 1e-6 and 10 mm".into(),
            ));
        }
        self.state.policy.native.tolerance = tolerance;
        Ok(())
    }

    /// Explains a displayed triangle through its current native source and occurrence.
    ///
    /// # Errors
    /// Rejects foreign stages, stale source scopes or overridden correspondence.
    pub fn pick(
        &mut self,
        stage: &mut layerstack_wasm::Stage,
        instance: &str,
        triangle: u32,
        source_scope: &str,
    ) -> std::result::Result<String, JsValue> {
        self.pick_inner(stage, instance, triangle, source_scope)
            .map(|v| v.to_string())
            .map_err(js_native_error)
    }
}

impl SceneKernel {
    fn new_inner(
        stage: &mut layerstack_wasm::Stage,
        instances: u32,
        realm: String,
    ) -> Result<Self> {
        if !(16..=128).contains(&realm.len())
            || !realm
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err("supply a unique host-generated realm, such as crypto.randomUUID()".into());
        }
        Ok(Self {
            state: State::new(stage, instances)?,
            realm,
        })
    }

    fn sync_inner(&mut self, stage: &mut layerstack_wasm::Stage) -> Result<String> {
        let mut context = self.state.borrow(stage)?;
        let b = &mut context;
        let next_source = b
            .state
            .source_revision
            .checked_add(1)
            .ok_or("source revision budget exhausted")?;
        let snapshot = b
            .state
            .evaluator
            .sync(
                b.live,
                b.store,
                &mut b.state.compiler,
                Time::Default,
                &b.state.policy,
            )
            .map_err(|error| error::refusal("scene.geometry", error.to_string()))?;
        let same_source = b
            .state
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
            .or(b.state.source_summary.as_ref())
            .ok_or("missing retained source summary")?;
        let revision = if same_source {
            b.state.source_revision
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
            b.state.source_summary = fresh;
        }
        b.state.source_revision = revision;
        b.state.snapshot = Some(snapshot);
        // Evaluation has already observed input changes; generated publication
        // cannot alter occurrence placements, so do not rediscover them next call.
        b.state.cursor = b.live.change_cursor();
        Ok(response)
    }

    fn pick_inner(
        &mut self,
        stage: &mut layerstack_wasm::Stage,
        instance: &str,
        triangle: u32,
        source_scope: &str,
    ) -> Result<serde_json::Value> {
        let context = self.state.borrow(stage)?;
        let b = &context;
        if source_scope != format!("{}/source/{}", self.realm, b.state.source_revision) {
            return Err(error::refusal(
                "scene.stale_pick",
                "display source is stale; extract current scene before picking",
            ));
        }
        let path = b
            .store
            .paths()
            .lookup(&Path::parse_absolute(instance, b.store.tokens())?)
            .ok_or("unknown instance path")?;
        if !b.state.placements.contains(&path)
            || !b
                .live
                .stage()
                .view(b.store)
                .prim(path)
                .is_some_and(|p| p.is_instance())
        {
            return Err("pick requires a current native instance root".into());
        }
        correspondence::validate_pick(b, path)?;
        let arrays = packet::composed_arrays(b)?;
        if !packet::producer_output(b, &arrays) {
            return Err(
                "composed geometry overrides producer output; native picking is unavailable".into(),
            );
        }
        let triangle = usize::try_from(triangle)?;
        let source = b
            .state
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
            json!({"source":source.source,"sourceScope":source_scope,"instance":instance,"nativeFace":face.0,"tags":native.face_tags(face)?.map(|tag|tag.0).collect::<Vec<_>>(),"planarFrame":native.planar_frame(face).ok().map(|frame|frame.rows),"units":"mm","frameSpace":"source-local"}),
        )
    }
}
