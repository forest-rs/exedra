// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Browser-host adapter for typed profile authoring and retained native parts.
//!
//! Rust owns geometry, validation and content-addressed realizations. Hosts own
//! units, document lifetimes, presentation, materials and display scheduling.
#![no_std]
extern crate alloc;

use alloc::{
    boxed::Box,
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
mod body;
mod chart;
mod inspect;
mod validation;
use authoring::{Request, Units};

#[derive(Debug, Serialize)]
struct Error {
    code: &'static str,
    operation: &'static str,
    message: String,
    #[serde(flatten)]
    context: Option<Box<ErrorContext>>,
}
#[derive(Debug, Serialize)]
struct CornerLocation {
    boundary: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    hole: Option<usize>,
    segment: usize,
    point: [f64; 2],
}
#[derive(Debug, Default, Serialize)]
struct ErrorContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    edge: Option<u32>,
    #[serde(rename = "loop", skip_serializing_if = "Option::is_none")]
    boundary_loop: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool: Option<usize>,
    #[serde(rename = "leftTool", skip_serializing_if = "Option::is_none")]
    left_tool: Option<usize>,
    #[serde(rename = "rightTool", skip_serializing_if = "Option::is_none")]
    right_tool: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    step: Option<usize>,
    #[serde(rename = "entryDistance", skip_serializing_if = "Option::is_none")]
    entry_distance: Option<f64>,
    #[serde(
        rename = "allowedEntryDistance",
        skip_serializing_if = "Option::is_none"
    )]
    allowed_entry_distance: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    operand: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vertex: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    axis: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cell: Option<[usize; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    required: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    maximum: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    node: Option<usize>,
    #[serde(rename = "cornerPoint", skip_serializing_if = "Option::is_none")]
    corner_point: Option<[f64; 2]>,
    #[serde(rename = "cornerTolerance", skip_serializing_if = "Option::is_none")]
    corner_tolerance: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    corner: Option<CornerLocation>,
    #[serde(rename = "cornerCandidates", skip_serializing_if = "Option::is_none")]
    corner_candidates: Option<Vec<CornerLocation>>,
    #[serde(rename = "requestedSize", skip_serializing_if = "Option::is_none")]
    requested_size: Option<f64>,
    #[serde(rename = "segmentLength", skip_serializing_if = "Option::is_none")]
    segment_length: Option<f64>,
    #[serde(rename = "requestedTrim", skip_serializing_if = "Option::is_none")]
    requested_trim: Option<f64>,
    #[serde(rename = "minimumSeparation", skip_serializing_if = "Option::is_none")]
    minimum_separation: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    profile: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    segment: Option<usize>,
    #[serde(rename = "startRadius", skip_serializing_if = "Option::is_none")]
    start_radius: Option<f64>,
    #[serde(rename = "endRadius", skip_serializing_if = "Option::is_none")]
    end_radius: Option<f64>,
    #[serde(
        rename = "radiusRoundoffBound",
        skip_serializing_if = "Option::is_none"
    )]
    radius_roundoff_bound: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    radius: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    height: Option<f64>,
    #[serde(rename = "axisOffset", skip_serializing_if = "Option::is_none")]
    axis_offset: Option<f64>,
    #[serde(rename = "rimRadius", skip_serializing_if = "Option::is_none")]
    rim_radius: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    face: Option<u32>,
    #[serde(rename = "geometryFamily", skip_serializing_if = "Option::is_none")]
    geometry_family: Option<&'static str>,
    #[serde(rename = "semiAngle", skip_serializing_if = "Option::is_none")]
    semi_angle: Option<f64>,
    #[serde(rename = "semiAngleAbsRange", skip_serializing_if = "Option::is_none")]
    semi_angle_abs_range: Option<[f64; 2]>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_family"
    )]
    family: Option<exedra_compile::NativeFamily>,
    #[serde(skip_serializing_if = "Option::is_none")]
    feature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    clearance: Option<f64>,
    #[serde(rename = "requiredClearance", skip_serializing_if = "Option::is_none")]
    required_clearance: Option<f64>,
    #[serde(rename = "axisDot", skip_serializing_if = "Option::is_none")]
    axis_dot: Option<f64>,
    #[serde(rename = "minimumAxisDot", skip_serializing_if = "Option::is_none")]
    minimum_axis_dot: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    end: Option<&'static str>,
}
fn serialize_family<S: serde::Serializer>(
    family: &Option<exedra_compile::NativeFamily>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    family.map(inspect::family).serialize(serializer)
}
impl Error {
    fn new(code: &'static str, operation: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            operation,
            message: message.into(),
            context: None,
        }
    }
    fn context_mut(&mut self) -> &mut ErrorContext {
        self.context.get_or_insert_with(Box::default)
    }
    fn node(mut self, node: usize) -> Self {
        self.context_mut().node = Some(node);
        self
    }
    fn profile(mut self, profile: usize) -> Self {
        self.context_mut().profile = Some(profile);
        self
    }
    fn step(operation: &'static str, error: exedra_step::StepError) -> Self {
        let code = match &error {
            exedra_step::StepError::UnsupportedSurface { .. } => "exchange.unsupportedSurface",
            exedra_step::StepError::Numeric | exedra_step::StepError::ConeAngle { .. } => {
                "exchange.numericLimit"
            }
            _ => "exchange.step",
        };
        let mut diagnostic = Self::new(code, operation, error.to_string());
        match &error {
            exedra_step::StepError::UnsupportedSurface { face, family } => {
                diagnostic.context_mut().face = Some(face.0);
                diagnostic.context_mut().geometry_family = Some(family);
            }
            exedra_step::StepError::ConeAngle {
                face,
                semi_angle,
                angle_range,
                ..
            } => {
                diagnostic.context_mut().face = Some(face.0);
                diagnostic.context_mut().geometry_family = Some("cone");
                diagnostic.context_mut().semi_angle = Some(*semi_angle);
                diagnostic.context_mut().semi_angle_abs_range = Some(*angle_range);
            }
            exedra_step::StepError::PeriodicSurfaceDomain { face, .. }
            | exedra_step::StepError::CylinderDomain { face } => {
                diagnostic.context_mut().face = Some(face.0);
            }
            _ => {}
        }
        diagnostic
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
    machining: Option<authoring::MachiningEvidence>,
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
        let mut built = request.build()?;
        let document = Document {
            parts: built
                .roots
                .into_iter()
                .map(|(name, source)| InstalledPart {
                    machining: built.machining.remove(&name),
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
        let policy = CompilePolicy {
            normals: exedra_compile::NormalsSource::CustomOrDerived,
            max_render_vertices: Some(options.max_vertices),
            native_chart: options.chart.map(Into::into),
            native: exedra_analytic::realize::RealizationPolicy {
                tolerance: options.tolerance,
                geometry_gap: doc.separation,
                max_vertices: options.max_vertices,
                ..exedra_analytic::realize::RealizationPolicy::default()
            },
            ..CompilePolicy::default()
        };
        let before = self.compiler.counters();
        let snapshot = self
            .compiler
            .compile_snapshot(&[PartInput::prepared(&part.name, &part.source)], &policy)
            .map_err(|error| {
                let message = error.to_string();
                match &error {
                    exedra_compile::CompileError::RenderVertices { .. } => {
                        Error::new("budget.renderVertices", "part.realize", message)
                    }
                    exedra_compile::CompileError::NativeChart { error, .. } => {
                        chart::diagnostic(error, message)
                    }
                    _ => Error::new("realization.invalid", "part.realize", message),
                }
            })?;
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
        if tri.positions.len() > options.max_vertices as usize {
            return Err(Error::new(
                "budget.renderVertices",
                "part.realize",
                format!(
                    "Display needs {} render vertices after attribute seams; maxVertices is {}",
                    tri.positions.len(),
                    options.max_vertices
                ),
            ));
        }
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
        let curve_refinement=report.curve_refinement.as_ref().map(|refinement| {
            use exedra_analytic::realize::CurveRefinementOutcome;
            let (outcome,diagnostic)=match &refinement.outcome {
                CurveRefinementOutcome::Improved=>("improved",None),
                CurveRefinementOutcome::NotImproved=>("notImproved",None),
                CurveRefinementOutcome::Refused(error)=>("refused",Some(Error::new("refinement.refused","part.realize",error.to_string()))),
            };
            serde_json::json!({"initialDeviation":refinement.initial_deviation,"curveTolerance":refinement.curve_tolerance,"outcome":outcome,"diagnostic":diagnostic})
        });
        if report
            .curve_refinement
            .as_ref()
            .is_some_and(|r| !r.initial_deviation.is_finite() || !r.curve_tolerance.is_finite())
        {
            return Err(Error::new(
                "realization.numericLimit",
                "part.realize",
                "Curve refinement evidence exceeds finite f64 values",
            ));
        }
        let applied_chart = realization
            .chart()
            .map(chart::InputChart::try_from)
            .transpose()?;
        if applied_chart.is_some() && tri.uvs.len() != tri.positions.len() {
            return Err(Error::new(
                "realization.uvs",
                "part.realize",
                "Construction UV/render vertex correspondence is incomplete",
            ));
        }
        let data = serde_json::json!({"tolerance":report.tolerance,"maxDeviation":report.max_deviation,
            "maxCurveDeviation":report.max_curve_deviation,"maxSurfaceDeviation":report.max_surface_deviation,
            "maxVertexDisplacement":report.max_vertex_displacement,"withinTolerance":report.within_tolerance(),
            "triangles":tri.indices.len()/3,"vertices":tri.positions.len(),
            "cacheHits":u32::try_from(after.cache_hits-before.cache_hits).expect("one input"),
            "nativeRealizations":u32::try_from(after.native_realizations-before.native_realizations).expect("one input"),
            "hasUvs":applied_chart.is_some(),"chart":applied_chart,
            "work":{"attempts":report.work.attempts,"completedBoundarySamples":report.work.completed_boundary_samples,"completedTriangles":report.work.completed_triangles,"trimRefinementBatches":report.work.trim_refinement_batches,"trimTriangleTests":report.work.trim_triangle_tests,"trimGeneratedVertices":report.work.trim_generated_vertices,"trimNormalCorners":report.work.trim_normal_corners,"trimNormalScratchBytes":report.work.trim_normal_scratch_bytes},
            "curveRefinement":curve_refinement});
        Ok(NativeBuffers {
            positions: tri.positions.iter().flatten().copied().collect(),
            normals: tri.normals.iter().flatten().copied().collect(),
            uvs: if applied_chart.is_some() {
                tri.uvs.iter().flatten().copied().collect()
            } else {
                Vec::new()
            },
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
            NativeGeometryRef::Body { body, frame } => {
                exedra_step::export_body(body, frame, &options)
            }
            NativeGeometryRef::Extrusion(v) => {
                exedra_step::export_body(v.body(), v.frame(), &options)
            }
            NativeGeometryRef::Revolution(v) => {
                exedra_step::export_body(v.body(), v.frame(), &options)
            }
            NativeGeometryRef::Sweep(v) => exedra_step::export_body(v.body(), v.frame(), &options),
            NativeGeometryRef::Loft(v) => exedra_step::export_body(v.body(), v.frame(), &options),
            NativeGeometryRef::BoredPlate(v) => {
                exedra_step::export_body(v.body(), v.frame(), &options)
            }
        }
        .map_err(|error| Error::step("part.exportStep", error))?;
        let report = serde_json::json!({"schema":file.report.schema,"units":doc.units,"declaredUncertainty":file.report.declared_uncertainty,
        "maxDeviation":file.report.max_deviation,"maxNativeSupportGap":file.report.max_native_support_gap,
        "frameRoundoffBound":file.report.frame_roundoff_bound,"parameterRoundoffBound":file.report.parameter_roundoff_bound,
        "entities":file.report.counts.entities,"faces":file.report.counts.faces,"edges":file.report.counts.edges,
        "shells":file.report.counts.shells,"solids":file.report.counts.solids,
        "materialValidation":file.report.material_validation.as_ref().map(|m|serde_json::json!({
            "family":"rectilinear","outerShell":m.outer_shell,"voidShells":m.void_shells,
            "shellFaces":m.boundary.shells.iter().map(|s|s.faces.iter().map(|f|f.0).collect::<Vec<_>>()).collect::<Vec<_>>(),
            "planes":m.planes,"cells":m.cells,"faceTests":m.face_tests,"faceTestBound":m.face_test_bound,
            "volume":m.volume,"volumeBounds":m.volume_bounds,"surfaceArea":m.surface_area,"surfaceAreaBounds":m.surface_area_bounds,
            "maxNativeSupportGap":m.boundary.geometry.max_gap,"arithmeticAllowance":m.arithmetic_allowance,
            "planeVertices":m.plane_vertices,"linkBytes":m.link_bytes,"arrangementScratchBytes":m.arrangement_scratch_bytes,
        })),
        "correspondence":{
            "vertices":file.report.correspondence.vertices.iter().map(|id|id.map(|v|v.0)).collect::<Vec<_>>(),
            "edges":file.report.correspondence.edges.iter().map(|entry|entry.map(|edge|match edge {
                exedra_step::StepEdgeEntity::Curve(id)=>serde_json::json!({"kind":"curve","entity":id.0}),
                chain@exedra_step::StepEdgeEntity::CurveChain{..}=>serde_json::json!({"kind":"curveChain","entities":chain.curve_entities().map(|id|id.0).collect::<Vec<_>>()}),
                exedra_step::StepEdgeEntity::CollapsedVertex(id)=>serde_json::json!({"kind":"collapsedVertex","entity":id.0})
            })).collect::<Vec<_>>(),
            "loops":file.report.correspondence.loops.iter().map(|id|id.map(|v|v.0)).collect::<Vec<_>>(),
            "faces":file.report.correspondence.faces.iter().map(|id|id.map(|v|v.0)).collect::<Vec<_>>(),
            "reversedFaces":file.report.correspondence.reversed_faces.iter().map(|face|face.0).collect::<Vec<_>>()
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
    #[serde(default)]
    chart: Option<chart::InputChart>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExchangeOptions {
    name: Option<String>,
    timestamp: String,
    tolerance: Option<f64>,
}
fn solid_operation_metadata(operation: &authoring::AuthoredSolidOperation) -> serde_json::Value {
    use authoring::AuthoredSolidOperation;
    use exedra_analytic::{
        halfspace::ClipOutcome,
        rectilinear::{BooleanOperation, Operand},
    };
    match operation {
        AuthoredSolidOperation::Passages {
            node,
            base,
            tools,
            feature,
            report,
        } => {
            use exedra_analytic::machining::FaceChangeKind;
            serde_json::json!({"kind":"subtractParallelPassages","node":node,"base":base,"feature":feature,
                "tools":tools.iter().map(|tool|serde_json::json!({"point":tool.point,"direction":tool.direction,"radius":tool.radius})).collect::<Vec<_>>(),
                "trimmedFaces":report.trimmed_faces.map(|id|id.0),
                "walls":report.walls.iter().map(|id|id.0).collect::<Vec<_>>(),"wallTools":report.wall_tools,
                "openingComponents":report.opening_components,"pairTests":report.pair_tests,
                "intersectionVertices":report.intersection_vertices,"removedSectionArea":report.removed_section_area,
                "removedVolume":report.removed_volume,"surfaceAreaDelta":report.surface_area_delta,
                "minimumClearance":report.min_clearance,"maxGap":report.max_gap,
                "faces":report.faces.iter().map(|face|serde_json::json!({"source":face.source.0,"result":face.result.0,
                    "change":match face.kind{FaceChangeKind::Retained=>"retained",FaceChangeKind::Trimmed=>"trimmed"}})).collect::<Vec<_>>()
            })
        }
        AuthoredSolidOperation::Bore {
            node,
            base,
            entry,
            direction,
            steps,
            feature,
            report,
        } => {
            use exedra_analytic::machining::{BoreDepth, FaceChangeKind};
            serde_json::json!({
                "kind":"subtractBore", "node":node, "base":base,
                "entry":entry, "direction":direction, "feature":feature,
                "steps":steps.iter().map(|step| serde_json::json!({"radius":step.radius,
                    "depth":match step.end {BoreDepth::ToDepth(depth)=>serde_json::json!(depth), BoreDepth::Through=>serde_json::json!("through")}})).collect::<Vec<_>>(),
                "entryFace":report.entry.0,"exitFace":report.exit.map(|id|id.0),
                "rims":report.rims.iter().map(|id|id.0).collect::<Vec<_>>(),
                "walls":report.walls.iter().map(|id|id.0).collect::<Vec<_>>(),
                "shoulders":report.shoulders.iter().map(|id|id.0).collect::<Vec<_>>(),
                "floor":report.floor.map(|id|id.0),"minimumClearance":report.min_clearance,
                "maxGap":report.max_gap,"removedVolume":report.removed_volume,
                "surfaceAreaDelta":report.surface_area_delta,
                "faces":report.faces.iter().map(|face|serde_json::json!({"source":face.source.0,"result":face.result.0,
                    "change":match face.kind{FaceChangeKind::Retained=>"retained",FaceChangeKind::Trimmed=>"trimmed"}})).collect::<Vec<_>>()
            })
        }
        AuthoredSolidOperation::Rectilinear {
            node,
            left,
            right,
            report,
        } => serde_json::json!({
            "kind":match report.operation {BooleanOperation::Union=>"union",BooleanOperation::Intersection=>"intersect",BooleanOperation::Difference=>"subtract"},
            "node":node,"left":left,"right":right,"planes":report.planes,"cells":report.cells,"faceTests":report.face_tests,"faceTestBound":report.face_test_bound,
            "materialCells":report.material_cells,"materialComponents":report.material_components,"cavities":report.cavities,"boundaryShells":report.boundary_shells,
            "faceSources":report.face_sources.iter().map(|sources|sources.iter().map(|source|serde_json::json!({"operand":match source.operand{Operand::Left=>"left",Operand::Right=>"right"},"face":source.face.0,"reversed":source.reversed})).collect::<Vec<_>>()).collect::<Vec<_>>(),
            "leftFaces":report.left_faces.iter().map(|ids|ids.iter().map(|id|id.0).collect::<Vec<_>>()).collect::<Vec<_>>(),
            "rightFaces":report.right_faces.iter().map(|ids|ids.iter().map(|id|id.0).collect::<Vec<_>>()).collect::<Vec<_>>(),
            "leftEdges":report.left_edges.iter().map(|ids|ids.iter().map(|id|id.0).collect::<Vec<_>>()).collect::<Vec<_>>(),
            "rightEdges":report.right_edges.iter().map(|ids|ids.iter().map(|id|id.0).collect::<Vec<_>>()).collect::<Vec<_>>(),
            "leftVertices":report.left_vertices.iter().map(|id|id.map(|id|id.0)).collect::<Vec<_>>(),
            "rightVertices":report.right_vertices.iter().map(|id|id.map(|id|id.0)).collect::<Vec<_>>(),
            "volume":report.volume,"volumeBounds":report.volume_bounds,"surfaceArea":report.surface_area,"surfaceAreaBounds":report.surface_area_bounds,"maxGap":report.max_gap,
        }),
        AuthoredSolidOperation::HalfSpace {
            node,
            base,
            point,
            normal,
            feature,
            report,
        } => {
            let mut value = serde_json::json!({
                "kind":"intersectHalfSpace","node":node,"base":base,"point":point,"normal":normal,
                "outcome":match report.outcome {ClipOutcome::Unchanged=>"unchanged",ClipOutcome::Empty=>"empty",ClipOutcome::Clipped=>"clipped"},
                "faces":report.faces.iter().map(|face|serde_json::json!({"source":face.source.0,"result":face.result.map(|id|id.0),"trimmed":face.trimmed})).collect::<Vec<_>>(),
                "edgeImages":report.edge_images.iter().map(|ids|ids.iter().map(|id|id.0).collect::<Vec<_>>()).collect::<Vec<_>>(),
                "splits":report.splits.iter().map(|split|serde_json::json!({"source":split.source.0,"vertex":split.vertex.0,"fraction":split.fraction})).collect::<Vec<_>>(),
                "cap":report.cap.map(|id|id.0),"capEdges":report.cap_edges.iter().map(|id|id.0).collect::<Vec<_>>(),"maxGap":report.max_gap,
            });
            if let Some(feature) = feature {
                value["feature"] = serde_json::json!(feature);
            }
            value
        }
    }
}
impl Document {
    fn metadata(&self, id: u32) -> Result<String, Error> {
        let mut parts = Vec::with_capacity(self.parts.len());
        for (index, part) in self.parts.iter().enumerate() {
            let mut value = inspect::metadata(part.native(), &self.names)?;
            if let Some(evidence) = &part.machining {
                let operations:Vec<_>=evidence.reports.iter().map(|operation| {
                    let report=&operation.report;
                    let faces:Vec<_>=report.faces.iter().map(|face|serde_json::json!({"source":face.source.0,"result":face.result.0,"change":match face.kind {exedra_analytic::machining::FaceChangeKind::Retained=>"retained",exedra_analytic::machining::FaceChangeKind::Trimmed=>"trimmed"}})).collect();
                    serde_json::json!({"point":operation.point,"direction":operation.direction,"radius":operation.radius,"feature":operation.feature,
                        "trimmedFaces":report.trimmed_faces.map(|id|id.0),"rims":report.rims.map(|id|id.0),"wallFace":report.wall.0,"seamEdge":report.seam.0,
                        "minimumClearance":report.min_clearance,"maxNativeGap":report.max_gap,"removedVolume":report.removed_volume,"surfaceAreaDelta":report.surface_area_delta,"faces":faces})
                }).collect();
                if !evidence.reports.is_empty() {
                    value["machining"] = serde_json::json!({"coordinateSpace":"construction","operations":operations});
                }
                if !evidence.operations.is_empty() {
                    value["solidOperations"] = serde_json::json!({"coordinateSpace":"construction","operations":evidence.operations.iter().map(solid_operation_metadata).collect::<Vec<_>>()});
                }
                if let Some([volume, area]) = evidence.integrals {
                    let has_boolean = evidence.operations.iter().any(|operation| {
                        matches!(
                            operation,
                            authoring::AuthoredSolidOperation::Rectilinear { .. }
                        )
                    });
                    let has_machining = !evidence.reports.is_empty()
                        || evidence.operations.iter().any(|operation| {
                            matches!(
                                operation,
                                authoring::AuthoredSolidOperation::Bore { .. }
                                    | authoring::AuthoredSolidOperation::Passages { .. }
                            )
                        });
                    let basis = match (has_boolean, !has_machining) {
                        (true, true) => "nativeBoolean",
                        (true, false) => "nativeBooleanAndAnalyticMachining",
                        (false, _) => "constructorAndAnalyticMachining",
                    };
                    value["measurements"]["volume"] =
                        serde_json::json!({"status":"available","value":volume,"basis":basis});
                    value["measurements"]["surfaceArea"] =
                        serde_json::json!({"status":"available","value":area,"basis":basis});
                }
            }
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
    /// Checks oriented boundary topology and supported continuous trim arrangements.
    /// Does not certify global self-intersection, shell containment or material.
    /// # Errors
    /// Returns scoped geometry/topology diagnoses or explicit work refusals.
    pub fn validate_boundary_json(
        &self,
        document: u32,
        part: u32,
        options: &str,
    ) -> Result<String, JsValue> {
        let (document, part) = self.session.part(document, part).map_err(Error::js)?;
        validation::inspect(
            part.native(),
            document.separation,
            decode(options).map_err(Error::js)?,
        )
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
    uvs: Vec<f32>,
    indices: Vec<u32>,
    native_faces: Vec<u32>,
    report: String,
}
#[wasm_bindgen]
impl NativeBuffers {
    /// Takes primary construction UVs, or an empty buffer for uncharted output.
    pub fn take_uvs(&mut self) -> Vec<f32> {
        core::mem::take(&mut self.uvs)
    }
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
                    chart: None,
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
        assert_eq!(collar["stepSupport"]["status"], "supported");
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
                        max_vertices: 1000,
                        chart: None,
                    }
                )
                .unwrap_err()
                .code,
            "lifecycle.documentDisposed"
        );
        session.release(19);
        assert_eq!(session.documents.len(), 0);
    }
    #[test]
    fn pointed_cone_display_keeps_directional_native_pole_normals() {
        let request=serde_json::from_value(serde_json::json!({"units":"mm","separation":1e-9,"profiles":[{"kind":"polygon","points":[[0,0],[2,0],[0,3]]}],"nodes":[{"kind":"revolve","profile":0,"angle":core::f64::consts::TAU}],"roots":[{"name":"cone","node":0}]})).unwrap();
        let mut session = Session::new();
        session.build(request).unwrap();
        let output = display(&mut session, 1, 0.01);
        let mut count = 0;
        for (position, normal) in output
            .positions
            .as_chunks::<3>()
            .0
            .iter()
            .zip(output.normals.as_chunks::<3>().0)
        {
            if *position == [0.0, 3.0, 0.0] {
                count += 1;
                assert!(
                    normal[1] > 0.4 && normal[1] < 0.9,
                    "cone tip must preserve the side's directional limit, not a vertical average"
                );
                let length = normal.iter().map(|n| n * n).sum::<f32>();
                assert!((length - 1.0).abs() < 1e-6);
            }
        }
        assert!(
            count >= 8,
            "display vertices must split the native welded apex by corner normal"
        );
    }

    #[test]
    fn construction_uvs_are_real_typed_buffers_with_native_identity_and_policy_reports() {
        let transform = serde_json::json!({"matrix":[[0.05,0],[0,0.05]],"offset":[0,0]});
        for kind in ["revolve", "sweep", "loft"] {
            let profile = if kind == "revolve" {
                serde_json::json!({"kind":"translated","profile":0,"offset":[20,3]})
            } else {
                serde_json::json!({"kind":"circle","radius":3})
            };
            let node = match kind {
                "revolve" => {
                    serde_json::json!({"kind":"revolve","profile":1,"angle":core::f64::consts::TAU})
                }
                "sweep" => serde_json::json!({"kind":"sweep","profile":1,"path":0}),
                _ => {
                    serde_json::json!({"kind":"loft","sections":[{"profile":1,"placement":{}},{"profile":1,"placement":{"translation":[3,0,30]}}]})
                }
            };
            let request=serde_json::from_value(serde_json::json!({"units":"mm","separation":1e-6,"profiles":[{"kind":"circle","radius":3},profile],"paths":[{"start":[0,0,0],"tangent":[0,1,0],"sectionX":[1,0,0],"steps":[{"kind":"lineBy","offset":[0,20,0]},{"kind":"tangentArc","radius":40,"angle":core::f64::consts::FRAC_PI_2,"normal":[0,0,1]}]}],"nodes":[node],"roots":[{"name":"part","node":0}]})).unwrap();
            let mut session = Session::new();
            let metadata = session.build(request).unwrap();
            assert_eq!(value(&metadata)["parts"][0]["chartKind"], kind);
            let support = &value(&metadata)["parts"][0]["stepSupport"]["status"];
            assert_eq!(
                support,
                if kind == "loft" {
                    "unsupported"
                } else {
                    "supported"
                }
            );
            if kind != "loft" {
                let file = session
                    .step(
                        1,
                        0,
                        ExchangeOptions {
                            name: None,
                            timestamp: "2026-10-28T00:00:00Z".into(),
                            tolerance: None,
                        },
                    )
                    .unwrap();
                assert!(file.text.contains("TOROIDAL_SURFACE"));
                assert_eq!(value(&file.report)["parameterRoundoffBound"], 0.0);
            }
            let chart_value = match kind {
                "revolve" => {
                    serde_json::json!({"kind":kind,"referenceRadius":20,"wall":transform,"caps":transform})
                }
                "loft" => {
                    serde_json::json!({"kind":kind,"referenceSection":0,"restLength":30,"stations":"datumDistance","wall":transform,"caps":transform})
                }
                _ => serde_json::json!({"kind":kind,"wall":transform,"caps":transform}),
            };
            let chart: chart::InputChart = serde_json::from_value(chart_value.clone()).unwrap();
            let source_faces = value(&metadata)["parts"][0]["faces"]
                .as_array()
                .unwrap()
                .clone();
            if kind == "revolve" {
                let native = session.documents[&1].parts[0].native();
                let NativeGeometryRef::Revolution(source) = native.geometry() else {
                    unreachable!()
                };
                let native_output = source
                    .realize(&exedra_analytic::realize::RealizationPolicy {
                        tolerance: 0.12,
                        geometry_gap: 1e-6,
                        ..exedra_analytic::realize::RealizationPolicy::default()
                    })
                    .unwrap();
                let native_limit = u32::try_from(native_output.mesh.vertices().count()).unwrap();
                let error = session
                    .realize(
                        1,
                        0,
                        DisplayOptions {
                            tolerance: 0.12,
                            max_vertices: native_limit,
                            chart: Some(chart),
                        },
                    )
                    .unwrap_err();
                assert_eq!(error.code, "budget.renderVertices");
            }
            let mut original = session
                .realize(
                    1,
                    0,
                    DisplayOptions {
                        tolerance: 0.12,
                        max_vertices: 250_000,
                        chart: Some(chart),
                    },
                )
                .unwrap();
            let report = value(&original.report);
            assert_eq!(report["hasUvs"], true);
            assert_eq!(report["chart"], serde_json::to_value(chart).unwrap());
            assert_eq!(report["withinTolerance"], true);
            assert!(report["work"]["attempts"].as_u64().unwrap() >= 1);
            assert_eq!(original.uvs.len() * 3, original.positions.len() * 2);
            assert!(original.uvs.iter().all(|v| v.is_finite()));
            assert_eq!(original.native_faces.len() * 3, original.indices.len());
            assert!(original.native_faces.iter().all(|face| {
                source_faces
                    .iter()
                    .any(|f| f["id"].as_u64() == Some(u64::from(*face)))
            }));
            let repeated = session
                .realize(
                    1,
                    0,
                    DisplayOptions {
                        tolerance: 0.12,
                        max_vertices: 250_000,
                        chart: Some(chart),
                    },
                )
                .unwrap();
            assert_eq!(value(&repeated.report)["cacheHits"], 1);
            assert_eq!(repeated.positions, original.positions);
            assert_eq!(repeated.uvs, original.uvs);
            assert_eq!(repeated.native_faces, original.native_faces);
            let taken = original.take_uvs();
            assert!(!taken.is_empty());
            assert!(original.take_uvs().is_empty());
            session
                .realize(
                    1,
                    0,
                    DisplayOptions {
                        tolerance: 0.035,
                        max_vertices: 250_000,
                        chart: Some(chart),
                    },
                )
                .unwrap();
            assert_eq!(session.documents[&1].metadata(1).unwrap(), metadata);
            let uncharted = session
                .realize(
                    1,
                    0,
                    DisplayOptions {
                        tolerance: 0.12,
                        max_vertices: 250_000,
                        chart: None,
                    },
                )
                .unwrap();
            assert!(uncharted.uvs.is_empty());
            assert_eq!(value(&uncharted.report)["hasUvs"], false);
            // Build an invalid transform without changing the source or relying on a cache miss.
            let mut invalid: serde_json::Value = serde_json::to_value(chart).unwrap();
            invalid["wall"]["matrix"] = serde_json::json!([[0, 0], [0, 0]]);
            let invalid: chart::InputChart = serde_json::from_value(invalid).unwrap();
            assert_eq!(
                session
                    .realize(
                        1,
                        0,
                        DisplayOptions {
                            tolerance: 0.12,
                            max_vertices: 250_000,
                            chart: Some(invalid)
                        }
                    )
                    .unwrap_err()
                    .code,
                "chart.invalid"
            );
            let mut collapsed: serde_json::Value = serde_json::to_value(chart).unwrap();
            collapsed["wall"]["offset"] = serde_json::json!([1e20, 1e20]);
            let collapsed: chart::InputChart = serde_json::from_value(collapsed).unwrap();
            let error = session
                .realize(
                    1,
                    0,
                    DisplayOptions {
                        tolerance: 0.12,
                        max_vertices: 250_000,
                        chart: Some(collapsed),
                    },
                )
                .unwrap_err();
            assert_eq!(error.code, "chart.numericLimit");
            assert!(error.context.as_ref().is_some_and(|c| c.face.is_some()));
            session.release(1);
            assert_eq!(value(&session.stats())["documents"], 0);
            assert_eq!(value(&session.stats())["cacheEntries"], 0);
        }
    }
    fn bore_request(direction: [f64; 3], radius: f64, separation: f64, placed: bool) -> Request {
        let placement = if placed {
            serde_json::json!({"translation":[100,200,300],"rotationXYZ":[0.2,0.3,0.4]})
        } else {
            serde_json::json!({})
        };
        serde_json::from_value(serde_json::json!({"units":"mm","separation":1e-6,"profiles":[{"kind":"rectangle","width":100,"height":70},{"kind":"tagged","profile":0,"name":"plate"}],
            "nodes":[{"kind":"extrude","profile":1,"height":20,"placement":placement},{"kind":"throughBore","base":0,"point":[0,0,10],"direction":direction,"radius":radius,"separation":separation,"feature":"bore"}],"roots":[{"name":"plate","node":1}]
        })).unwrap()
    }
    #[test]
    fn exact_bore_metadata_step_and_placed_local_axis_are_end_to_end() {
        for placed in [false, true] {
            let mut session = Session::new();
            let metadata = value(
                &session
                    .build(bore_request([1.0, 0.0, 1.0], 8.0, 2.0, placed))
                    .unwrap(),
            );
            let part = &metadata["parts"][0];
            assert_eq!(part["family"], "boredPlate");
            assert_eq!(part["stepSupport"]["status"], "supported");
            assert!(part["chartKind"].is_null());
            let expected_volume =
                140_000.0 - core::f64::consts::PI * 64.0 * 20.0 * core::f64::consts::SQRT_2;
            assert!(
                (part["measurements"]["volume"]["value"].as_f64().unwrap() - expected_volume).abs()
                    < 1e-8
            );
            assert_eq!(part["bore"]["coordinateSpace"], "construction");
            assert_eq!(part["bore"]["point"], serde_json::json!([0.0, 0.0, 10.0]));
            for rim in part["bore"]["rims"].as_array().unwrap() {
                assert!(
                    (rim["semiMajor"].as_f64().unwrap() - 8.0 * core::f64::consts::SQRT_2).abs()
                        < 1e-10
                );
                assert!((rim["semiMinor"].as_f64().unwrap() - 8.0).abs() < 1e-10);
            }
            let expected = if placed {
                exedra_math::Placement3::euler_extrinsic_xyz_then_translate(
                    0.2,
                    0.3,
                    0.4,
                    [100.0, 200.0, 300.0],
                )
                .transform_point([0.0, 0.0, 10.0])
            } else {
                [0.0, 0.0, 10.0]
            };
            for (actual, expected) in part["bore"]["documentPoint"]
                .as_array()
                .unwrap()
                .iter()
                .zip(expected)
            {
                assert!((actual.as_f64().unwrap() - expected).abs() < 1e-10);
            }
            let mesh = session
                .realize(
                    1,
                    0,
                    DisplayOptions {
                        tolerance: 0.12,
                        max_vertices: 250_000,
                        chart: None,
                    },
                )
                .unwrap();
            assert_eq!(value(&mesh.report)["withinTolerance"], true);
            assert!(
                mesh.native_faces
                    .contains(&u32::try_from(part["bore"]["wallFace"].as_u64().unwrap()).unwrap())
            );
            let file = session
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
            assert!(file.text.contains("ELLIPSE("));
            assert_eq!(value(&file.report)["faces"], 7);
            session.release(1);
            assert_eq!(value(&session.stats())["documents"], 0);
        }
        let mut session = Session::new();
        let good = session
            .build(bore_request([1.0, 0.0, 1.0], 8.0, 2.0, false))
            .unwrap();
        let clearance = session
            .build(bore_request([1.0, 0.0, 1.0], 8.0, 40.0, false))
            .unwrap_err();
        assert_eq!(clearance.code, "bore.clearance");
        assert_eq!(
            clearance.context.as_ref().unwrap().required_clearance,
            Some(40.0)
        );
        let parallel = session
            .build(bore_request([1.0, 0.0, 0.01], 8.0, 2.0, false))
            .unwrap_err();
        assert_eq!(parallel.code, "bore.nearParallel");
        assert!(parallel.context.as_ref().unwrap().axis_dot.unwrap() < 0.05);
        assert_eq!(session.documents[&1].metadata(1).unwrap(), good);
        assert_eq!(session.documents.len(), 1);
    }
    fn housing_request(port_y: f64) -> Request {
        let bores = [
            ([-38.0, -33.0, 9.0], [0.0, 0.0, 1.0], 5.0, "left mount"),
            ([38.0, -33.0, 9.0], [0.0, 0.0, 1.0], 5.0, "right mount"),
            ([0.0, 15.0, 9.0], [0.364, 0.0, 1.0], 12.0, "main passage"),
            ([0.0, port_y, 9.0], [0.0, 0.0, 1.0], 5.0, "service port"),
        ];
        let mut nodes = alloc::vec![serde_json::json!({"kind":"extrude","profile":1,"height":18})];
        for (base, (point, direction, radius, feature)) in bores.into_iter().enumerate() {
            nodes.push(serde_json::json!({"kind":"subtractThroughCylinder","base":base,"point":point,"direction":direction,"radius":radius,"separation":1,"feature":feature}));
        }
        decode(&json(&serde_json::json!({"units":"mm","separation":1e-6,"profiles":[{"kind":"polygon","points":[[-52,-45],[52,-45],[52,-21],[32,-21],[32,45],[-32,45],[-32,-21],[-52,-21]]},{"kind":"tagged","profile":0,"name":"stock"}],"nodes":nodes,"roots":[{"name":"housing","node":4}]}))).unwrap()
    }
    #[test]
    fn retained_housing_edits_have_exact_evidence_selections_and_atomic_refusals() {
        let mut session = Session::new();
        let metadata = session.build(housing_request(-10.0)).unwrap();
        let data = value(&metadata);
        let part = &data["parts"][0];
        assert_eq!(part["family"], "body");
        assert_eq!(part["machining"]["operations"].as_array().unwrap().len(), 4);
        assert_eq!(
            part["measurements"]["volume"]["basis"],
            "constructorAndAnalyticMachining"
        );
        assert_eq!(part["stepSupport"]["status"], "supported");
        let native = session.documents[&1].parts[0].native();
        assert!(matches!(native.geometry(),NativeGeometryRef::Body{body,..} if body.is_closed()));
        assert!(native.to_body().is_closed());
        assert!(
            matches!(
                native.volume(),
                Err(exedra_compile::NativeMeasureError::Unavailable { .. })
            ),
            "generic body query must not claim a constructor integral"
        );
        for feature in ["left mount", "right mount", "main passage", "service port"] {
            assert_eq!(
                part["faces"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|face| face["features"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|value| value == feature))
                    .count(),
                1
            );
        }
        for tolerance in [0.45, 0.12, 0.035] {
            let display = session
                .realize(
                    1,
                    0,
                    DisplayOptions {
                        tolerance,
                        max_vertices: 250000,
                        chart: None,
                    },
                )
                .unwrap();
            assert_eq!(value(&display.report)["withinTolerance"], true);
            assert_eq!(display.native_faces.len() * 3, display.indices.len());
        }
        let file = session
            .step(
                1,
                0,
                ExchangeOptions {
                    name: None,
                    timestamp: "2026-10-29T00:00:00Z".to_string(),
                    tolerance: None,
                },
            )
            .unwrap();
        assert!(file.text.contains("MANIFOLD_SOLID_BREP("));
        assert_eq!(value(&file.report)["faces"], 14);
        let error = session.build(housing_request(15.0)).unwrap_err();
        assert_eq!(error.code, "machining.clearance");
        assert_eq!(
            error.context.unwrap().feature.as_deref(),
            Some("service port")
        );
        assert_eq!(session.documents[&1].metadata(1).unwrap(), metadata);
        assert_eq!(session.documents.len(), 1);
        session.release(1);
        assert_eq!(session.compiler.native_storage().sources, 0);
    }
    #[test]
    fn fused_lug_housing_serializes_native_union_history_and_correct_integral_basis() {
        let mut nodes = alloc::vec![
            serde_json::json!({"kind":"extrude","profile":1,"height":18}),
            serde_json::json!({"kind":"extrude","profile":4,"height":18}),
            serde_json::json!({"kind":"union","base":0,"other":1}),
            serde_json::json!({"kind":"extrude","profile":6,"height":18}),
            serde_json::json!({"kind":"union","base":2,"other":3})
        ];
        for (point, radius, feature) in [
            ([-38.0, -33.0, 9.0], 5.0, "left mount"),
            ([38.0, -33.0, 9.0], 5.0, "right mount"),
            ([0.0, 15.0, 9.0], 12.0, "main passage"),
            ([0.0, -10.0, 9.0], 5.0, "service port"),
        ] {
            let base = nodes.len() - 1;
            nodes.push(serde_json::json!({"kind":"subtractThroughCylinder","base":base,"point":point,"direction":if feature=="main passage" {[0.364,0.0,1.0]} else {[0.0,0.0,1.0]},"radius":radius,"separation":1,"feature":feature}));
        }
        let request=decode(&json(&serde_json::json!({"units":"mm","separation":1e-6,"profiles":[{"kind":"rectangle","width":64,"height":90},{"kind":"tagged","profile":0,"name":"stock"},{"kind":"rectangle","width":20,"height":24},{"kind":"translated","profile":2,"offset":[-42,-33]},{"kind":"tagged","profile":3,"name":"left lug"},{"kind":"translated","profile":2,"offset":[42,-33]},{"kind":"tagged","profile":5,"name":"right lug"}],"nodes":nodes,"roots":[{"name":"blank","node":4},{"name":"housing","node":8}]}))).unwrap();
        let mut session = Session::new();
        let metadata = value(&session.build(request).unwrap());
        let blank = &metadata["parts"][0];
        let housing = &metadata["parts"][1];
        assert_eq!(blank["measurements"]["volume"]["basis"], "nativeBoolean");
        assert!(blank["machining"].is_null());
        assert_eq!(
            housing["measurements"]["volume"]["basis"],
            "nativeBooleanAndAnalyticMachining"
        );
        assert_eq!(
            housing["machining"]["operations"].as_array().unwrap().len(),
            4
        );
        let operations = housing["solidOperations"]["operations"].as_array().unwrap();
        assert_eq!(operations.len(), 2);
        for operation in operations {
            assert_eq!(operation["kind"], "union");
            assert_eq!(operation["boundaryShells"], 1);
            assert_eq!(operation["materialComponents"], 1);
            assert!(operation["faceTests"].as_u64().unwrap() > 0);
            assert!(operation["faceTests"].as_u64() <= operation["faceTestBound"].as_u64());
            assert!(!operation["rightFaces"].as_array().unwrap().is_empty());
        }
        let features = housing["faces"].as_array().unwrap();
        for feature in [
            "left lug",
            "right lug",
            "left mount",
            "right mount",
            "main passage",
            "service port",
        ] {
            assert!(
                features.iter().any(|face| face["features"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|value| value == feature)),
                "feature {feature} must survive native union/machining"
            );
        }
        assert!(
            session
                .realize(
                    1,
                    1,
                    DisplayOptions {
                        tolerance: 0.035,
                        max_vertices: 250000,
                        chart: None
                    }
                )
                .is_ok()
        );
        let file = session
            .step(
                1,
                1,
                ExchangeOptions {
                    name: None,
                    timestamp: "2026-10-29T00:00:00Z".to_string(),
                    tolerance: None,
                },
            )
            .unwrap();
        assert!(file.text.contains("MANIFOLD_SOLID_BREP("));
    }
    #[test]
    fn unlabeled_halfspace_metadata_omits_optional_feature_instead_of_returning_null() {
        let request=decode(&json(&serde_json::json!({"units":"mm","separation":1e-6,"profiles":[{"kind":"rectangle","width":10,"height":8}],"nodes":[{"kind":"extrude","profile":0,"height":6},{"kind":"intersectHalfSpace","base":0,"point":[1,0,3],"normal":[1,0,1]}],"roots":[{"name":"clipped","node":1}]}))).unwrap();
        let metadata = value(&Session::new().build(request).unwrap());
        let operation = &metadata["parts"][0]["solidOperations"]["operations"][0];
        assert_eq!(operation["kind"], "intersectHalfSpace");
        assert_eq!(operation["outcome"], "clipped");
        assert!(operation.get("feature").is_none());
    }
    fn stepped_bore_request(depth: f64, radius: f64) -> Request {
        decode(&json(&serde_json::json!({"units":"mm","separation":1e-9,
            "profiles":[{"kind":"rectangle","width":60,"height":40}],
            "nodes":[{"kind":"extrude","profile":0,"height":30},
              {"kind":"subtractBore","base":0,"entry":[0,0,30],"direction":[0,0,-1],
               "steps":[{"radius":8,"depth":6},{"radius":4,"depth":"through"}],"separation":0.5,"feature":"counterbore"},
              {"kind":"subtractBore","base":1,"entry":[-20,0,30],"direction":[0,0,-1],
               "steps":[{"radius":radius,"depth":depth}],"separation":0.5,"feature":"fastener"}],
            "roots":[{"name":"block","node":2}]}))).unwrap()
    }
    #[test]
    fn stepped_bore_authoring_retains_shoulders_floors_and_actionable_refusals() {
        let mut session = Session::new();
        let good = session.build(stepped_bore_request(12.0, 3.0)).unwrap();
        let metadata = value(&good);
        let block = &metadata["parts"][0];
        let operations = block["solidOperations"]["operations"].as_array().unwrap();
        assert_eq!(operations.len(), 2);
        assert_eq!(operations[0]["kind"], "subtractBore");
        assert_eq!(operations[0]["walls"].as_array().unwrap().len(), 2);
        assert_eq!(operations[0]["shoulders"].as_array().unwrap().len(), 1);
        assert!(operations[0]["floor"].is_null());
        assert!(operations[0]["exitFace"].is_number());
        assert!(operations[1]["exitFace"].is_null());
        assert!(operations[1]["floor"].is_number());
        assert_eq!(
            operations[1]["direction"],
            serde_json::json!([0.0, 0.0, -1.0])
        );
        assert_eq!(
            block["measurements"]["volume"]["basis"],
            "constructorAndAnalyticMachining"
        );
        let expected = 72000.0 - core::f64::consts::PI * (64.0 * 6.0 + 16.0 * 24.0 + 9.0 * 12.0);
        assert!(
            (block["measurements"]["volume"]["value"].as_f64().unwrap() - expected).abs() < 1e-8
        );
        assert_eq!(
            block["faces"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|face| face["features"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|f| f == "counterbore"))
                .count(),
            3
        );
        assert_eq!(
            block["faces"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|face| face["features"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|f| f == "fastener"))
                .count(),
            2
        );
        let checked = validation::inspect(
            session.documents[&1].parts[0].native(),
            1e-9,
            validation::Options::default(),
        )
        .unwrap();
        assert_eq!(value(&checked)["scope"], "localBoundary");
        assert_eq!(value(&checked)["orientedManifold"], true);
        assert_eq!(value(&checked)["globalSolidValidity"], "notChecked");
        let display = session
            .realize(
                1,
                0,
                DisplayOptions {
                    tolerance: 0.12,
                    max_vertices: 250_000,
                    chart: None,
                },
            )
            .unwrap();
        assert_eq!(value(&display.report)["withinTolerance"], true);
        let floor = u32::try_from(operations[1]["floor"].as_u64().unwrap()).unwrap();
        assert!(display.native_faces.contains(&floor));
        let step = session
            .step(
                1,
                0,
                ExchangeOptions {
                    name: None,
                    timestamp: "2026-10-29T00:00:00Z".into(),
                    tolerance: Some(1e-6),
                },
            )
            .unwrap();
        assert!(step.text.contains("MANIFOLD_SOLID_BREP("));
        for (depth, radius, code) in [
            (30.0, 3.0, "bore.depthClearance"),
            (12.0, -1.0, "bore.invalidStep"),
        ] {
            let error = session
                .build(stepped_bore_request(depth, radius))
                .unwrap_err();
            assert_eq!(error.code, code);
            let context = error.context.as_ref().unwrap();
            assert_eq!(context.step, Some(0));
            assert_eq!(context.feature.as_deref(), Some("fastener"));
            assert_eq!(session.documents.len(), 1);
            assert_eq!(session.documents[&1].metadata(1).unwrap(), good);
        }
    }
    #[test]
    fn cavity_step_report_exposes_material_ownership_work_and_reversed_faces() {
        let mut session = Session::new();
        let request = decode(&json(&serde_json::json!({
            "units":"mm","separation":1e-9,
            "profiles":[{"kind":"rectangle","width":10,"height":10},{"kind":"rectangle","width":2,"height":2}],
            "nodes":[{"kind":"extrude","profile":0,"height":10},{"kind":"extrude","profile":1,"height":2,"placement":{"translation":[0,0,4]}},{"kind":"subtract","base":0,"other":1}],
            "roots":[{"name":"housing","node":2}]
        }))).unwrap();
        session.build(request).unwrap();
        let file = session
            .step(
                1,
                0,
                ExchangeOptions {
                    name: None,
                    timestamp: "2026-10-29T00:00:00Z".into(),
                    tolerance: Some(1e-7),
                },
            )
            .unwrap();
        assert!(file.text.contains("BREP_WITH_VOIDS("));
        let report = value(&file.report);
        assert_eq!(report["shells"], 2);
        assert_eq!(report["solids"], 1);
        let material = &report["materialValidation"];
        assert_eq!(material["family"], "rectilinear");
        assert_eq!(material["volume"], 992.0);
        assert_eq!(material["voidShells"].as_array().unwrap().len(), 1);
        assert!(
            material["faceTests"].as_u64().unwrap() <= material["faceTestBound"].as_u64().unwrap()
        );
        assert!(material["linkBytes"].as_u64().unwrap() > 0);
        assert!(
            material["arrangementScratchBytes"].as_u64().unwrap()
                >= material["linkBytes"].as_u64().unwrap()
        );
        assert_eq!(
            report["correspondence"]["reversedFaces"]
                .as_array()
                .unwrap()
                .len(),
            6
        );
    }
}
