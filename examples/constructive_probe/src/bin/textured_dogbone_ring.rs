// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Controlled shading/UV study over the same complete 21-cut dogbone geometry.

#[path = "../dogbone.rs"]
mod dogbone;

use dogbone::{CUT_COUNT, RING_RADIUS, ring_recipe};
use exedra_assembly::{Assembly, CompilePolicy, PartCompiler};
use exedra_constructive::{
    evaluate::{Severity, evaluate},
    ir::{UvFrame, UvMapping, UvProjection, UvTransform},
    source_map::SourceMap,
    tessellate::EvalPolicy,
};
use exedra_gltf::{GltfExportOptions, MaterialResolver, Texture, export_glb_with_materials};
use exedra_mesh::{Mesh, NormalParams, NormalsSource, op};
use exedra_mesh_ops::uv::{
    UvSet, corner_uv,
    quality::{
        UvQualityExtremum, UvQualityIssue, UvQualityLimits, UvQualityParams, UvQualityReport,
        UvQualityScratch, inspect_uv_quality, visit_uv_quality,
    },
};
use serde_json::{Value, json};
use std::{
    error::Error,
    path::{Path, PathBuf},
};

const GRID_IMAGE: &[u8] = include_bytes!("../../../material_gallery/assets/projection-grid.png");
fn texture_size() -> [u32; 2] {
    [
        u32::from_be_bytes(GRID_IMAGE[16..20].try_into().unwrap()),
        u32::from_be_bytes(GRID_IMAGE[20..24].try_into().unwrap()),
    ]
}
fn issue_code(issue: UvQualityIssue) -> &'static str {
    match issue {
        UvQualityIssue::MissingUv => "missing_uv",
        UvQualityIssue::NonfiniteUv => "nonfinite_uv",
        UvQualityIssue::NonfiniteGeometry => "nonfinite_geometry",
        UvQualityIssue::DegenerateGeometry => "degenerate_geometry",
        UvQualityIssue::DegenerateUv => "degenerate_uv",
        UvQualityIssue::NumericLimit => "numeric_limit",
        UvQualityIssue::LimitExceeded => "limit_exceeded",
        UvQualityIssue::TriangulationFallback => "triangulation_fallback",
    }
}

const COLORS: [[f64; 3]; 10] = [
    [0.05, 0.45, 0.26],
    [0.15, 0.64, 0.34],
    [0.48, 0.75, 0.22],
    [0.88, 0.80, 0.12],
    [0.98, 0.52, 0.09],
    [0.89, 0.24, 0.06],
    [0.72, 0.06, 0.05],
    [0.42, 0.02, 0.03],
    [0.28, 0.01, 0.15],
    [0.8, 0.0, 0.8],
];
const SLOTS: [&str; 10] = ["q0", "q1", "q2", "q3", "q4", "q5", "q6", "q7", "q8", "q9"];
struct Materials;
impl MaterialResolver for Materials {
    fn resolve(&self, key: &str) -> Option<Value> {
        if key == "grid" {
            return Some(
                json!({"pbrMetallicRoughness":{"baseColorTexture":{"index":0},"metallicFactor":0.0,"roughnessFactor":0.6}}),
            );
        }
        let color = if key == "solid" {
            [0.65, 0.70, 0.73]
        } else {
            COLORS[*SLOTS.iter().position(|&name| name == key).as_ref()?]
        };
        Some(
            json!({"pbrMetallicRoughness":{"baseColorFactor":[color[0],color[1],color[2],1.0],"metallicFactor":0.0,"roughnessFactor":0.6}}),
        )
    }
    fn resolve_texture(&self, index: u32) -> Option<Texture<'_>> {
        (index == 0).then_some(Texture {
            image: GRID_IMAGE,
            mime_type: "image/png",
            sampler: Some(json!({"wrapS":10497,"wrapT":10497,"magFilter":9729,"minFilter":9987})),
        })
    }
}
fn export(
    mesh: Mesh,
    root: &Path,
    name: &str,
    material: &str,
    baked_normals: bool,
    heatmap: bool,
) -> Result<(), Box<dyn Error>> {
    let mut assembly = Assembly::new();
    let part = assembly.add_baked_part("ring", mesh, if heatmap { &SLOTS } else { &["finish"] })?;
    if heatmap {
        for (region, slot) in SLOTS.iter().enumerate() {
            assembly.bind_region_slot(part, u32::try_from(region)?, slot)?;
            assembly.set_part_material(part, slot, slot)?;
        }
    } else {
        assembly.set_default_slot(part, "finish")?;
        assembly.set_part_material(part, "finish", material)?;
    }
    assembly.add_instance(
        None,
        "ring",
        part,
        exedra_constructive::ir::Placement3::IDENTITY,
    )?;
    let policy = CompilePolicy {
        normals: if baked_normals {
            NormalsSource::CustomOrDerived
        } else {
            NormalsSource::Derived
        },
        ..CompilePolicy::default()
    };
    let compiled = PartCompiler::new().compile_parts(&assembly, &policy)?;
    let output = export_glb_with_materials(
        &assembly,
        &compiled,
        &Materials,
        GltfExportOptions::z_up_to_y_up(),
    )?;
    std::fs::write(root.join(format!("{name}.glb")), output.bytes)?;
    Ok(())
}
fn strip_creases(mesh: &mut Mesh) -> usize {
    let edges: Vec<_> = mesh
        .half_edges()
        .filter(|&edge| {
            mesh.canonical_edge(edge) == Some(edge)
                && mesh.edge_sharpness(edge).is_some_and(|v| v > 0.0)
        })
        .collect();
    let mut edit = mesh.edit();
    for &edge in &edges {
        op::set_edge_sharpness(&mut edit, edge, 0.0).unwrap();
    }
    let _: () = edit.finish();
    edges.len()
}
fn bake_normals(mesh: &mut Mesh) {
    let normals = mesh.derive_corner_normals(&NormalParams::default());
    let corners: Vec<_> = mesh.faces().flat_map(|face| mesh.face_loop(face)).collect();
    let mut edit = mesh.edit();
    for corner in corners {
        op::set_corner_normal_override(&mut edit, corner, normals.get(corner)).unwrap();
    }
    let _: () = edit.finish();
}
fn signature(mesh: &Mesh, uv: bool) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut add = |v: f32| {
        for byte in v.to_bits().to_le_bytes() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3);
        }
    };
    if uv {
        for face in mesh.faces() {
            for corner in mesh.face_loop(face) {
                for value in corner_uv(mesh, UvSet::Primary, corner).expect("complete UVs") {
                    add(value);
                }
            }
        }
    } else {
        for vertex in mesh.vertices() {
            for &value in mesh.vertex_position(vertex).unwrap() {
                add(value);
            }
        }
    }
    hash
}
fn extent(value: Option<UvQualityExtremum>, mesh: &Mesh, source: &SourceMap) -> Value {
    value.map_or(Value::Null, |value| {
        let origin = source.surface_origin(value.face);
        json!({
            "value": value.value,
            "face": value.face.index(),
            "corners": value.corners.map(|corner| corner.index()),
            "positions": value.corners.map(|corner| {
                *mesh.vertex_position(mesh.to_vertex(corner).unwrap()).unwrap()
            }),
            "source": origin.and_then(|origin| origin.source.as_deref()),
        })
    })
}
fn quality_params() -> UvQualityParams {
    UvQualityParams {
        texture_size: texture_size(),
        triangulation: CompilePolicy::default()
            .extraction_params()
            .face_triangulation,
        limits: UvQualityLimits {
            max_anisotropy: Some(8.0),
            ..UvQualityLimits::default()
        },
        ..UvQualityParams::default()
    }
}
fn survey(
    mesh: &Mesh,
    source: &SourceMap,
    scratch: &mut UvQualityScratch,
    buckets: &mut Vec<u8>,
) -> (UvQualityReport, Value) {
    let params = quality_params();
    let slots = mesh
        .faces()
        .map(|face| face.index() as usize + 1)
        .max()
        .unwrap_or(0);
    buckets.resize(slots, 0);
    buckets.fill(0);
    let mut histogram = [0_u64; 10];
    let mut area_histogram = [0.0_f64; 10];
    let started = std::time::Instant::now();
    let report = visit_uv_quality(mesh, &params, scratch, |triangle| {
        let bucket = triangle.metrics.map_or(9, |metrics| {
            [1.05, 1.25, 1.5, 2.0, 3.0, 4.0, 8.0, 16.0]
                .iter()
                .position(|&v| metrics.anisotropy <= v)
                .unwrap_or(8)
        });
        histogram[bucket] += 1;
        if let Ok(metrics) = triangle.metrics {
            area_histogram[bucket] += metrics.surface_area;
        }
        buckets[triangle.face.index() as usize] =
            buckets[triangle.face.index() as usize].max(u8::try_from(bucket).unwrap());
    })
    .unwrap();
    let survey_ns = u64::try_from(started.elapsed().as_nanos()).unwrap();
    let witnesses: Vec<_> = report
        .witnesses
        .iter()
        .map(|witness| {
            json!({
                "face": witness.face.index(),
                "issue": issue_code(witness.issue),
                "source": source.surface_origin(witness.face)
                    .and_then(|origin| origin.source.as_deref()),
            })
        })
        .collect();
    let summary = json!({
        "inspection_with_heatmap_ns": survey_ns,
        "operation_scratch_bytes": report.scratch_bytes,
        "heatmap_buffer_bytes": buckets.capacity(),
        "revision": report.revision.value(),
        "faces": report.faces_examined,
        "triangles": report.triangles_examined,
        "measured": report.triangles_measured,
        "measured_surface_area": report.measured_surface_area,
        "outside_limits_surface_area": report.outside_limits_surface_area,
        "outside_limits_area_fraction": report.outside_limits_area_fraction(),
        "refused": report.triangles_refused,
        "refusals": {
            "missing_uv": report.refusals.missing_uv,
            "nonfinite_uv": report.refusals.nonfinite_uv,
            "nonfinite_geometry": report.refusals.nonfinite_geometry,
            "degenerate_geometry": report.refusals.degenerate_geometry,
            "degenerate_uv": report.refusals.degenerate_uv,
            "numeric_limit": report.refusals.numeric_limit,
        },
        "outside_anisotropy_limit": report.triangles_outside_limits,
        "mirrored": report.mirrored_triangles,
        "max_anisotropy": extent(report.max_anisotropy, mesh, source),
        "min_density": extent(report.min_density, mesh, source),
        "max_density": extent(report.max_density, mesh, source),
        "histogram": histogram,
        "surface_area_histogram": area_histogram,
        "witnesses": witnesses,
        "witnesses_omitted": report.witnesses_omitted,
    });
    (report, summary)
}
fn heatmap(mut mesh: Mesh, buckets: &[u8]) -> Mesh {
    let faces: Vec<_> = mesh.faces().collect();
    let mut edit = mesh.edit();
    for face in faces {
        op::set_face_region(&mut edit, face, u32::from(buckets[face.index() as usize])).unwrap();
    }
    let _: () = edit.finish();
    mesh
}
fn main() -> Result<(), Box<dyn Error>> {
    let root = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/surface_quality"));
    std::fs::create_dir_all(&root)?;
    let mut policy = EvalPolicy::default();
    policy.sweep_path.max_tangent_angle = 0.0125;
    let mut summaries = Vec::new();
    let mut loss_evidence = Value::Null;
    let mut scratch = UvQualityScratch::default();
    let mut buckets = Vec::new();
    for native in [false, true] {
        let mapping = (!native).then(|| UvMapping {
            projection: UvProjection::Toroidal {
                frame: UvFrame::XY,
                major_radius: RING_RADIUS,
            },
            transform: UvTransform::scale([4.0, 1.0]).unwrap(),
        });
        let recipe = ring_recipe(CUT_COUNT, 0, &policy, native, mapping)?;
        let result = evaluate(&recipe, &policy)?;
        assert!(
            result.report.clean_at(Severity::Error),
            "sample evaluation must be complete"
        );
        assert_eq!(result.bodies.len(), 1, "ring stays connected");
        let body = &result.bodies[0].body;
        assert!(
            body.mesh.validate_deep().is_empty(),
            "ring topology remains valid"
        );
        assert!(
            body.mesh.boundary_loops()?.is_empty(),
            "ring remains closed"
        );
        let name = if native { "construction" } else { "projected" };
        let (report, summary) = survey(&body.mesh, &body.source_map, &mut scratch, &mut buckets);
        assert_eq!(
            report.refusals.missing_uv, 0,
            "every surface has an authored mapping"
        );
        println!(
            "{name}: triangles={} refused={} outside_limit={} max_anisotropy={:?}",
            report.triangles_examined,
            report.triangles_refused,
            report.triangles_outside_limits,
            report.max_anisotropy.map(|e| e.value)
        );
        summaries.push(json!({"name":name,"quality":summary}));
        export(body.mesh.clone(), &root, name, "grid", false, false)?;
        export(
            heatmap(body.mesh.clone(), &buckets),
            &root,
            &format!("{name}-quality"),
            "",
            false,
            true,
        )?;
        if !native {
            export(body.mesh.clone(), &root, "solid", "solid", false, false)?;
            let source_normals = body.mesh.derive_corner_normals(&NormalParams::default());
            let mut lossy = body.mesh.clone();
            let lost = strip_creases(&mut lossy);
            assert!(lost > 0, "the reproduction must remove actual cut creases");
            assert_eq!(
                signature(&lossy, false),
                signature(&body.mesh, false),
                "shading experiment must preserve positions"
            );
            assert_eq!(
                signature(&lossy, true),
                signature(&body.mesh, true),
                "shading experiment must preserve UVs"
            );
            let lost_normals = lossy.derive_corner_normals(&NormalParams::default());
            let mut changed = 0;
            for face in lossy.faces() {
                for corner in lossy.face_loop(face) {
                    if source_normals.get(corner) != lost_normals.get(corner) {
                        changed += 1;
                    }
                }
            }
            assert!(changed > 0, "lost sharpness must change derived shading");
            let mut lossy_report = inspect_uv_quality(&lossy, &quality_params(), &mut scratch)?;
            // The crease edit advances revision without changing UV metric inputs.
            lossy_report.revision = report.revision;
            assert_eq!(
                lossy_report, report,
                "unchanged UVs must retain identical distortion measurements"
            );
            export(lossy, &root, "lost-creases", "grid", false, false)?;
            let mut baked = body.mesh.clone();
            bake_normals(&mut baked);
            strip_creases(&mut baked);
            for face in baked.faces() {
                for corner in baked.face_loop(face) {
                    assert_eq!(
                        baked
                            .attrs()
                            .sparse(exedra_mesh::attr::CORNER_NORMAL_OVERRIDE)
                            .unwrap()
                            .get(corner.into())
                            .copied(),
                        source_normals.get(corner),
                        "baked normals must survive the lost edge metadata"
                    );
                }
            }
            export(baked, &root, "baked-normals", "grid", true, false)?;
            loss_evidence = json!({
                "sharp_edges_removed": lost,
                "corners_with_changed_normals": changed,
                "geometry_unchanged": true,
                "uvs_unchanged": true,
                "uv_quality_unchanged": true,
            });
        }
    }
    let manifest = json!({
        "position_unit": "mm",
        "triangulation": "robust",
        "texture_size": texture_size(),
        "construction_repeat_mm": 40,
        "anisotropy_histogram_upper_bounds": [1.05, 1.25, 1.5, 2, 3, 4, 8, 16],
        "quality_colors": COLORS,
        "cases": summaries,
        "lost_crease_experiment": loss_evidence,
    });
    std::fs::write(
        root.join("survey.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_edge_metadata_changes_shading_but_baked_normals_keep_the_source() {
        let mut mesh = exedra_testkit::fixtures::unit_box();
        let original = mesh.derive_corner_normals(&NormalParams::default());
        let mut baked = mesh.clone();
        bake_normals(&mut baked);
        assert!(strip_creases(&mut mesh) > 0);
        assert!(strip_creases(&mut baked) > 0);
        let softened = mesh.derive_corner_normals(&NormalParams::default());
        let mut changed = 0;
        for face in mesh.faces() {
            for corner in mesh.face_loop(face) {
                changed += usize::from(softened.get(corner) != original.get(corner));
                assert_eq!(
                    baked
                        .attrs()
                        .sparse(exedra_mesh::attr::CORNER_NORMAL_OVERRIDE)
                        .unwrap()
                        .get(corner.into())
                        .copied(),
                    original.get(corner)
                );
            }
        }
        assert!(changed > 0);
        let params = exedra_mesh::ExtractParams {
            normals: NormalsSource::CustomOrDerived,
            ..exedra_mesh::ExtractParams::default()
        };
        let (actual, _) = baked.to_trimesh(&params);
        assert!(
            actual
                .normals
                .iter()
                .all(|normal| normal.iter().filter(|&&v| v != 0.0).count() == 1)
        );
    }
}
