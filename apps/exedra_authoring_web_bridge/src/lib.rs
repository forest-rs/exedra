// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Browser-host adapter for typed profile authoring and retained native parts.
//!
//! Rust owns geometry, validation and content-addressed realizations. Hosts own
//! units, document lifetimes, presentation, materials and display scheduling.
#![no_std]
extern crate alloc;

use alloc::{
    collections::BTreeMap,
    format,
    string::{String, ToString},
    vec::Vec,
};
use exedra_compile::NativeGeometryRef;
use exedra_compile::{
    CompilePolicy, NativeSource, PartCompiler, PartId, PartInput, PartSource, PreparedSource,
};
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

mod authoring;
mod inspect;
use authoring::{Request, Units};

#[derive(Debug, Serialize)]
struct Error {
    code: &'static str,
    operation: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    node: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    profile: Option<usize>,
}
impl Error {
    fn new(code: &'static str, operation: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            operation,
            message: message.into(),
            node: None,
            profile: None,
        }
    }
    fn node(mut self, node: usize) -> Self {
        self.node = Some(node);
        self
    }
    fn profile(mut self, profile: usize) -> Self {
        self.profile = Some(profile);
        self
    }
    fn js(self) -> JsValue {
        JsValue::from_str(&json(&self))
    }
}
fn json(value: &impl Serialize) -> String {
    serde_json::to_string(value).expect("bridge metadata has checked finite numbers")
}
fn decode<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, Error> {
    if value.len() > 1 << 20 {
        return Err(Error::new(
            "budget.request",
            "bridge.decode",
            "Authoring requests must fit in one MiB",
        ));
    }
    serde_json::from_str(value)
        .map_err(|e| Error::new("request.invalid", "bridge.decode", e.to_string()))
}
#[derive(Debug)]
struct InstalledPart {
    name: String,
    source: PreparedSource,
}
impl InstalledPart {
    fn native(&self) -> &NativeSource {
        let PartSource::Native(native) = self.source.source() else {
            unreachable!("authoring installs native sources only")
        };
        native
    }
}
#[derive(Debug)]
struct Document {
    parts: Vec<InstalledPart>,
    units: Units,
    separation: f64,
    names: BTreeMap<u32, String>,
}
#[derive(Debug)]
struct Session {
    documents: BTreeMap<u32, Document>,
    compiler: PartCompiler,
    next: u32,
}
impl Session {
    fn new() -> Self {
        Self {
            documents: BTreeMap::new(),
            compiler: PartCompiler::with_capacity(8),
            next: 1,
        }
    }
    fn build(&mut self, request: Request) -> Result<String, Error> {
        if self.documents.len() >= 8 {
            return Err(Error::new(
                "budget.documents",
                "document.build",
                "Dispose an older document before retaining more than eight documents",
            ));
        }
        let next = self.next.checked_add(1).ok_or_else(|| {
            Error::new(
                "budget.revisions",
                "document.build",
                "The native document ID space is exhausted",
            )
        })?;
        let built = request.build()?;
        let document = Document {
            parts: built
                .roots
                .into_iter()
                .map(|(name, source)| InstalledPart {
                    name,
                    source: PreparedSource::new(PartSource::Native(source)),
                })
                .collect(),
            units: built.units,
            separation: built.separation,
            names: built.names,
        };
        let metadata = document.metadata(self.next)?;
        self.documents.insert(self.next, document);
        self.next = next;
        Ok(metadata)
    }
    fn part(&self, document: u32, part: u32) -> Result<(&Document, &InstalledPart), Error> {
        let doc = self.documents.get(&document).ok_or_else(|| {
            Error::new(
                "lifecycle.documentDisposed",
                "document.lookup",
                "The native document is not retained",
            )
        })?;
        let part = doc.parts.get(part as usize).ok_or_else(|| {
            Error::new(
                "selection.part",
                "document.lookup",
                "The native part is outside this document",
            )
        })?;
        Ok((doc, part))
    }
    fn release(&mut self, document: u32) {
        self.documents.remove(&document);
        if self.documents.is_empty() {
            self.compiler.clear_cache();
        }
    }
    fn realize(
        &mut self,
        document: u32,
        part: u32,
        options: DisplayOptions,
    ) -> Result<NativeBuffers, Error> {
        let doc = self.documents.get(&document).ok_or_else(|| {
            Error::new(
                "lifecycle.documentDisposed",
                "part.realize",
                "The native document is not retained",
            )
        })?;
        let part = doc.parts.get(part as usize).ok_or_else(|| {
            Error::new(
                "selection.part",
                "part.realize",
                "The native part is outside this document",
            )
        })?;
        if !options.tolerance.is_finite() || options.tolerance <= 0.0 {
            return Err(Error::new(
                "policy.tolerance",
                "part.realize",
                "Display tolerance must be finite and positive",
            ));
        }
        if options.max_vertices == 0 || options.max_vertices > 250000 {
            return Err(Error::new(
                "budget.vertices",
                "part.realize",
                "Display maxVertices must be in 1..=250000",
            ));
        }
        let mut policy = CompilePolicy::default();
        policy.native.tolerance = options.tolerance;
        policy.native.geometry_gap = doc.separation;
        policy.native.max_vertices = options.max_vertices;
        let before = self.compiler.counters();
        let snapshot = self
            .compiler
            .compile_snapshot(&[PartInput::prepared(&part.name, &part.source)], &policy)
            .map_err(|e| Error::new("realization.invalid", "part.realize", e.to_string()))?;
        let after = self.compiler.counters();
        let body = snapshot
            .body(PartId(0), 0)
            .expect("native source emits one body");
        let realization = body
            .native_realization()
            .expect("native realization correspondence retained");
        let tri = &snapshot
            .compiled()
            .part(PartId(0))
            .expect("one input")
            .bodies[0]
            .tri;
        let report = &realization.report;
        if [
            report.tolerance,
            report.max_deviation,
            report.max_curve_deviation,
            report.max_surface_deviation,
            report.max_vertex_displacement,
        ]
        .iter()
        .any(|v| !v.is_finite())
        {
            return Err(Error::new(
                "realization.numericLimit",
                "part.realize",
                "Display accuracy evidence exceeds finite f64 values",
            ));
        }
        let data = serde_json::json!({"tolerance":report.tolerance,"maxDeviation":report.max_deviation,
            "maxCurveDeviation":report.max_curve_deviation,"maxSurfaceDeviation":report.max_surface_deviation,
            "maxVertexDisplacement":report.max_vertex_displacement,"withinTolerance":report.within_tolerance(),
            "triangles":tri.indices.len()/3,"vertices":tri.positions.len(),
            "cacheHits":u32::try_from(after.cache_hits-before.cache_hits).expect("one input"),
            "nativeRealizations":u32::try_from(after.native_realizations-before.native_realizations).expect("one input")});
        Ok(NativeBuffers {
            positions: tri.positions.iter().flatten().copied().collect(),
            normals: tri.normals.iter().flatten().copied().collect(),
            indices: tri.indices.clone(),
            native_faces: realization.render_faces().iter().map(|f| f.0).collect(),
            report: json(&data),
        })
    }
    fn workplane(&self, document: u32, part: u32, face: u32) -> Result<String, Error> {
        let (_, part) = self.part(document, part)?;
        let frame = part
            .native()
            .planar_frame(exedra_analytic::body::FaceId(face))
            .map_err(|e| {
                Error::new(
                    match e {
                        exedra_compile::NativeSelectionError::NotPlanar { .. } => {
                            "selection.notPlanar"
                        }
                        exedra_compile::NativeSelectionError::UnknownFace { .. } => {
                            "selection.face"
                        }
                        exedra_compile::NativeSelectionError::PlaneNumericLimit { .. } => {
                            "selection.numericLimit"
                        }
                        exedra_compile::NativeSelectionError::SourceMismatch => "selection.stale",
                    },
                    "part.workplane",
                    e.to_string(),
                )
            })?;
        Ok(json(
            &serde_json::json!({"origin":frame.rows.map(|r|r[3]),"xAxis":frame.rows.map(|r|r[0]),"yAxis":frame.rows.map(|r|r[1]),"zAxis":frame.rows.map(|r|r[2])}),
        ))
    }
    fn step(
        &self,
        document: u32,
        part: u32,
        options: ExchangeOptions,
    ) -> Result<NativeStepFile, Error> {
        let (doc, part) = self.part(document, part)?;
        let native = part.native();
        let units = match doc.units {
            Units::Millimetre => exedra_step::LengthUnit::Millimetre,
            Units::Metre => exedra_step::LengthUnit::Metre,
        };
        let name = options.name.as_deref().unwrap_or(&part.name);
        let options = exedra_step::StepOptions {
            name,
            timestamp: &options.timestamp,
            units,
            tolerance: options.tolerance.unwrap_or(doc.separation),
            ..exedra_step::StepOptions::default()
        };
        let file = match native.geometry() {
            NativeGeometryRef::Extrusion(v) => {
                exedra_step::export_body(v.body(), v.frame(), &options)
            }
            NativeGeometryRef::Revolution(v) => {
                exedra_step::export_body(v.body(), v.frame(), &options)
            }
            NativeGeometryRef::Sweep(v) => exedra_step::export_body(v.body(), v.frame(), &options),
            NativeGeometryRef::Loft(v) => exedra_step::export_body(v.body(), v.frame(), &options),
        }
        .map_err(|e| {
            Error::new(
                match e {
                    exedra_step::StepError::UnsupportedSurface { .. } => {
                        "exchange.unsupportedSurface"
                    }
                    exedra_step::StepError::Numeric => "exchange.numericLimit",
                    _ => "exchange.step",
                },
                "part.exportStep",
                e.to_string(),
            )
        })?;
        let report = serde_json::json!({"schema":file.report.schema,"units":doc.units,"declaredUncertainty":file.report.declared_uncertainty,
        "maxDeviation":file.report.max_deviation,"maxNativeSupportGap":file.report.max_native_support_gap,
        "frameRoundoffBound":file.report.frame_roundoff_bound,
        "entities":file.report.counts.entities,"faces":file.report.counts.faces,"edges":file.report.counts.edges,
        "correspondence":{
            "vertices":file.report.correspondence.vertices.iter().map(|id|id.map(|v|v.0)).collect::<Vec<_>>(),
            "edges":file.report.correspondence.edges.iter().map(|id|id.map(|v|v.0)).collect::<Vec<_>>(),
            "loops":file.report.correspondence.loops.iter().map(|id|id.map(|v|v.0)).collect::<Vec<_>>(),
            "faces":file.report.correspondence.faces.iter().map(|id|id.map(|v|v.0)).collect::<Vec<_>>()
        }});
        Ok(NativeStepFile {
            text: file.text,
            report: json(&report),
        })
    }
    fn stats(&self) -> String {
        let mut distinct: Vec<&NativeSource> = Vec::new();
        let mut roots = 0;
        for document in self.documents.values() {
            for part in &document.parts {
                roots += 1;
                if !distinct.iter().any(|n| n.same_installation(part.native())) {
                    distinct.push(part.native());
                }
            }
        }
        let cache = self.compiler.native_storage();
        json(
            &serde_json::json!({"documents":self.documents.len(),"nativeParts":roots,"cacheEntries":self.compiler.cached_entries(),
            "documentNativeSources":distinct.len(),"documentBoundaryBytes":distinct.iter().map(|n|n.boundary_storage().payload_bytes).sum::<usize>(),
            "cacheNativeSources":cache.sources,"cacheBoundaryBytes":cache.boundary_payload_bytes,
            "maxDocuments":8,"cacheCapacity":8}),
        )
    }
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DisplayOptions {
    tolerance: f64,
    max_vertices: u32,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExchangeOptions {
    name: Option<String>,
    timestamp: String,
    tolerance: Option<f64>,
}
impl Document {
    fn metadata(&self, id: u32) -> Result<String, Error> {
        let mut parts = Vec::with_capacity(self.parts.len());
        for (index, part) in self.parts.iter().enumerate() {
            let mut value = inspect::metadata(part.native(), &self.names)?;
            value["name"] = serde_json::json!(part.name);
            value["index"] = serde_json::json!(index);
            value["fingerprint"] =
                serde_json::json!(format!("{:032x}", part.source.fingerprint().0));
            parts.push(value);
        }
        Ok(json(
            &serde_json::json!({"id":id,"units":self.units,"parts":parts}),
        ))
    }
}

/// Explicit native-document and compiler-cache owner for a browser host.
#[wasm_bindgen]
#[derive(Debug)]
pub struct AuthoringKernel {
    session: Session,
}
impl Default for AuthoringKernel {
    fn default() -> Self {
        Self::new()
    }
}
#[wasm_bindgen]
impl AuthoringKernel {
    /// Creates an empty session with at most eight documents and eight cached realizations.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            session: Session::new(),
        }
    }
    /// Builds a complete acyclic native document before installing it.
    /// # Errors
    /// Returns a JSON diagnostic string for request, policy or native construction failures.
    pub fn build_document(&mut self, request: &str) -> Result<String, JsValue> {
        self.session
            .build(decode(request).map_err(Error::js)?)
            .map_err(Error::js)
    }
    /// Releases document ownership; releasing the final document also clears the cache.
    pub fn release_document(&mut self, document: u32) {
        self.session.release(document);
    }
    /// Compiles one installed native part with explicit display accuracy and vertex budgets.
    /// # Errors
    /// Returns a JSON diagnostic string. The installed source is unchanged.
    pub fn realize(
        &mut self,
        document: u32,
        part: u32,
        options: &str,
    ) -> Result<NativeBuffers, JsValue> {
        self.session
            .realize(document, part, decode(options).map_err(Error::js)?)
            .map_err(Error::js)
    }
    /// Reads a native supporting workplane independently of display triangles.
    /// # Errors
    /// Refuses disposed documents, unknown parts/faces and curved supports.
    pub fn workplane_json(&self, document: u32, part: u32, face: u32) -> Result<String, JsValue> {
        self.session
            .workplane(document, part, face)
            .map_err(Error::js)
    }
    /// Exports one installed native part with document units and caller timestamp.
    /// # Errors
    /// Returns the native STEP export's typed diagnosis as a JSON bridge error.
    pub fn export_step(
        &self,
        document: u32,
        part: u32,
        options: &str,
    ) -> Result<NativeStepFile, JsValue> {
        self.session
            .step(document, part, decode(options).map_err(Error::js)?)
            .map_err(Error::js)
    }
    /// Document-owned and cache-owned native table payloads are reported separately.
    pub fn stats_json(&self) -> String {
        self.session.stats()
    }
}

/// Single-transfer display buffers. Taking a buffer clears its Rust ownership.
#[wasm_bindgen]
#[derive(Debug)]
pub struct NativeBuffers {
    positions: Vec<f32>,
    normals: Vec<f32>,
    indices: Vec<u32>,
    native_faces: Vec<u32>,
    report: String,
}
#[wasm_bindgen]
impl NativeBuffers {
    /// Takes packed xyz positions; repeated calls return an empty buffer.
    pub fn take_positions(&mut self) -> Vec<f32> {
        core::mem::take(&mut self.positions)
    }
    /// Takes packed xyz normals.
    pub fn take_normals(&mut self) -> Vec<f32> {
        core::mem::take(&mut self.normals)
    }
    /// Takes triangle indices.
    pub fn take_indices(&mut self) -> Vec<u32> {
        core::mem::take(&mut self.indices)
    }
    /// Takes native face IDs parallel to render triangle ordinals.
    pub fn take_native_faces(&mut self) -> Vec<u32> {
        core::mem::take(&mut self.native_faces)
    }
    /// Accuracy, work and cache evidence for this realization.
    pub fn report_json(&self) -> String {
        self.report.clone()
    }
}
/// One independently exported native STEP part and its exchange evidence.
#[wasm_bindgen]
#[derive(Debug)]
pub struct NativeStepFile {
    text: String,
    report: String,
}
#[wasm_bindgen]
impl NativeStepFile {
    /// Takes exchange text without routing it through a display mesh or JSON array.
    pub fn take_text(&mut self) -> String {
        core::mem::take(&mut self.text)
    }
    /// Units, declared uncertainty and native exchange work evidence.
    pub fn report_json(&self) -> String {
        self.report.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn panel_request(width: f64, depth: f64) -> Request {
        decode(&json(&serde_json::json!({
            "units":"mm","separation":1e-6,
            "profiles":[
                {"kind":"roundedRectangle","width":width,"height":80,"radius":6},
                {"kind":"tagged","profile":0,"name":"panel"},
                {"kind":"slot","length":22,"radius":3},
                {"kind":"tagged","profile":2,"name":"vents"},
                {"kind":"cutouts","profile":1,"template":3,"positions":[[-36,22],[-12,22],[12,22],[36,22]]},
                {"kind":"slot","length":48,"radius":8},
                {"kind":"tagged","profile":5,"name":"recess"},
                {"kind":"circle","radius":3},
                {"kind":"tagged","profile":7,"name":"pin"}
            ],
            "nodes":[
                {"kind":"extrude","profile":4,"height":12},
                {"kind":"pocket","base":0,"profile":6,"end":"end","depth":depth,"at":[0,-10]},
                {"kind":"onFace","host":1,"face":{"kind":"floor","feature":"recess"},"profile":8,"height":7,"offset":0.5,"at":[0,-10]}
            ],"roots":[{"name":"panel","node":1},{"name":"pin","node":2}]
        }))).unwrap()
    }
    fn value(text: &str) -> serde_json::Value {
        serde_json::from_str(text).unwrap()
    }
    fn display(session: &mut Session, id: u32, tolerance: f64) -> NativeBuffers {
        session
            .realize(
                id,
                0,
                DisplayOptions {
                    tolerance,
                    max_vertices: 250000,
                },
            )
            .unwrap()
    }
    #[test]
    fn native_panel_attachment_identity_and_realization_are_end_to_end() {
        let mut session = Session::new();
        let metadata = value(&session.build(panel_request(120.0, 5.0)).unwrap());
        let panel = &metadata["parts"][0];
        for (actual, expected) in panel["measurements"]["bounds"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|v| v.as_array().unwrap())
            .zip([[-60.0, -40.0, 0.0], [60.0, 40.0, 12.0]].iter().flatten())
        {
            assert!((actual.as_f64().unwrap() - expected).abs() < 1e-8);
        }
        assert!(
            (metadata["parts"][1]["measurements"]["bounds"][0][2]
                .as_f64()
                .unwrap()
                - 7.5)
                .abs()
                < 1e-8
        );
        let volume = panel["measurements"]["volume"]["value"].as_f64().unwrap();
        let expected = 108864.0 - 5.0 * (512.0 + 64.0 * core::f64::consts::PI);
        assert!((volume - expected).abs() < 1e-8);
        let coarse = display(&mut session, 1, 0.45);
        let fine = display(&mut session, 1, 0.035);
        assert!(fine.indices.len() > coarse.indices.len());
        assert_eq!(fine.native_faces.len() * 3, fine.indices.len());
        let faces = panel["faces"].as_array().unwrap();
        for face in fine.native_faces {
            assert!(
                faces
                    .iter()
                    .any(|entry| entry["id"].as_u64() == Some(u64::from(face)))
            );
        }
        let floor = u32::try_from(
            faces
                .iter()
                .find(|f| f["feature"] == "recess" && f["role"] == "floor")
                .unwrap()["id"]
                .as_u64()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            value(&session.workplane(1, 0, floor).unwrap())["origin"][2],
            7.0
        );
        let again = value(&session.documents[&1].metadata(1).unwrap());
        assert_eq!(again["parts"][0]["fingerprint"], panel["fingerprint"]);
        assert_eq!(again["parts"][0]["measurements"]["volume"]["value"], volume);
        let step = session
            .step(
                1,
                0,
                ExchangeOptions {
                    name: Some("panel".into()),
                    timestamp: "2026-10-27T00:00:00Z".into(),
                    tolerance: None,
                },
            )
            .unwrap();
        let report = value(&step.report);
        assert_eq!(report["units"], "mm");
        assert!(report["correspondence"]["faces"][floor as usize].is_u64());
        assert!(step.text.contains("ADVANCED_FACE"));
    }
    #[test]
    fn failed_edits_are_atomic_and_repeated_release_bounds_native_owners() {
        let mut session = Session::new();
        let baseline = session.build(panel_request(120.0, 5.0)).unwrap();
        let initial = value(&session.stats());
        let before = session.next;
        let error = session.build(panel_request(120.0, 18.0)).unwrap_err();
        assert_eq!(error.code, "pocket.invalid");
        assert_eq!(session.next, before);
        assert_eq!(session.documents[&1].metadata(1).unwrap(), baseline);
        for i in 0..12 {
            let metadata = value(
                &session
                    .build(panel_request(120.01 + f64::from(i) * 0.01, 5.0))
                    .unwrap(),
            );
            let id = u32::try_from(metadata["id"].as_u64().unwrap()).unwrap();
            display(&mut session, id, 0.12);
            session.release(id);
            let stats = value(&session.stats());
            for key in [
                "documents",
                "nativeParts",
                "documentNativeSources",
                "documentBoundaryBytes",
            ] {
                assert_eq!(stats[key], initial[key]);
            }
            assert!(stats["cacheEntries"].as_u64().unwrap() <= 8);
        }
        session.release(1);
        let stats = value(&session.stats());
        assert_eq!(stats["documents"], 0);
        assert_eq!(stats["cacheEntries"], 0);
        assert_eq!(stats["cacheNativeSources"], 0);
    }
    #[test]
    fn native_revolution_installs_realizes_and_exports_with_scoped_inspection() {
        let request=decode(&json(&serde_json::json!({"units":"mm","separation":1e-6,
            "profiles":[{"kind":"rectangle","width":7,"height":30},{"kind":"translated","profile":0,"offset":[15.5,15]},{"kind":"tagged","profile":1,"name":"collar"}],
            "nodes":[{"kind":"revolve","profile":2,"angle":1.5*core::f64::consts::PI,"placement":{"rotationXYZ":[core::f64::consts::FRAC_PI_2,0,0]}}],
            "roots":[{"name":"collar","node":0}]
        }))).unwrap();
        let mut session = Session::new();
        let metadata = value(&session.build(request).unwrap());
        let collar = &metadata["parts"][0];
        assert_eq!(collar["family"], "revolution");
        assert_eq!(collar["measurements"]["volume"]["status"], "available");
        let expected = 0.75 * core::f64::consts::PI * (19.0 * 19.0 - 12.0 * 12.0) * 30.0;
        assert!(
            (collar["measurements"]["volume"]["value"].as_f64().unwrap() - expected).abs() < 1e-8
        );
        let coarse = display(&mut session, 1, 0.45);
        let fine = display(&mut session, 1, 0.035);
        assert!(fine.indices.len() > coarse.indices.len());
        let faces = collar["faces"].as_array().unwrap();
        let plane = faces.iter().find(|f| f["surface"] == "plane").unwrap()["id"]
            .as_u64()
            .unwrap();
        assert!(
            session
                .workplane(1, 0, u32::try_from(plane).unwrap())
                .is_ok()
        );
        let cylinder = faces.iter().find(|f| f["surface"] == "cylinder").unwrap()["id"]
            .as_u64()
            .unwrap();
        assert_eq!(
            session
                .workplane(1, 0, u32::try_from(cylinder).unwrap())
                .unwrap_err()
                .code,
            "selection.notPlanar"
        );
        let step = session
            .step(
                1,
                0,
                ExchangeOptions {
                    name: None,
                    timestamp: "2026-10-27T00:00:00Z".into(),
                    tolerance: None,
                },
            )
            .unwrap();
        assert!(step.text.contains("CYLINDRICAL_SURFACE"));
        session.release(1);
        assert_eq!(value(&session.stats())["cacheEntries"], 0);
    }
    #[test]
    fn unknown_documents_fail_without_work() {
        let mut session = Session::new();
        assert_eq!(
            session
                .realize(
                    19,
                    0,
                    DisplayOptions {
                        tolerance: 0.1,
                        max_vertices: 1000
                    }
                )
                .unwrap_err()
                .code,
            "lifecycle.documentDisposed"
        );
        session.release(19);
        assert_eq!(session.documents.len(), 0);
    }
}
