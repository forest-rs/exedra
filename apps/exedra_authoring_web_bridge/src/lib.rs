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
use exedra_analytic::{
    extrusion::{Extrusion, FaceRole},
    geometry::Surface,
};
use exedra_compile::{
    CompilePolicy, NativeSource, PartCompiler, PartId, PartInput, PartSource, PreparedSource,
};
use exedra_math::Placement3;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

mod authoring;
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
    fn solid(&self) -> &Extrusion {
        self.native()
            .as_extrusion()
            .expect("authoring creates extrusion families")
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
            .solid()
            .workplane(exedra_analytic::body::FaceId(face))
            .map_err(|e| Error::new("selection.workplane", "part.workplane", e.to_string()))?
            .frame();
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
        let native = part.solid();
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
        let file = exedra_step::export_body(native.body(), native.frame(), &options)
            .map_err(|e| Error::new("exchange.step", "part.exportStep", e.to_string()))?;
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
            let solid = part.solid();
            let bounds = placed_bounds(solid.local_bounds(), solid.frame());
            if !solid.volume().is_finite()
                || !solid.surface_area().is_finite()
                || bounds.iter().flatten().any(|v| !v.is_finite())
            {
                return Err(Error::new(
                    "measurement.unrepresentable",
                    "document.inspect",
                    "Native measurements exceed finite f64 coordinates",
                ));
            }
            let faces: Vec<_> = solid
                .indexed_faces()
                .map(|(id, face)| {
                    let role = match face.role {
                        FaceRole::Cap(_) => "cap",
                        FaceRole::Wall { .. } => "wall",
                        FaceRole::Floor { .. } => "floor",
                    };
                    let surface = match face.surface {
                        Surface::Plane(_) => "plane",
                        Surface::Cylinder(_) => "cylinder",
                        Surface::Cone(_) => "cone",
                        Surface::Torus(_) => "torus",
                        Surface::Ruled(_) => "ruled",
                    };
                    let mut value = serde_json::json!({"id":id.0,"role":role,"surface":surface});
                    if let Some(feature) = solid.face_tag(id).and_then(|t| self.names.get(&t.0)) {
                        value["feature"] = serde_json::json!(feature);
                    }
                    value
                })
                .collect();
            parts.push(serde_json::json!({"name":part.name,"index":index,"fingerprint":format!("{:032x}",part.source.fingerprint().0),
                "measurements":{"volume":solid.volume(),"surfaceArea":solid.surface_area(),"bounds":bounds,
                    "faces":solid.faces().len(),"edges":solid.edges().len(),"vertices":solid.vertices().len()},"faces":faces}));
        }
        Ok(json(
            &serde_json::json!({"id":id,"units":self.units,"parts":parts}),
        ))
    }
}
fn placed_bounds(bounds: [[f64; 3]; 2], frame: Placement3) -> [[f64; 3]; 2] {
    let mut result = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    for i in 0..8 {
        let p = frame.transform_point(core::array::from_fn(|k| {
            bounds[usize::from(i & (1 << k) != 0)][k]
        }));
        for (k, &value) in p.iter().enumerate() {
            result[0][k] = result[0][k].min(value);
            result[1][k] = result[1][k].max(value);
        }
    }
    result
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
