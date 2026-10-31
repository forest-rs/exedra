// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Filled-region composition and corner treatment, drawn as SVG and extruded
//! into a GLB with generated transitions highlighted by their authored tag.
//!
//! ```sh
//! cargo run -p analytic_extrusion --example region_studies -- /tmp/region-studies
//! # Writes /tmp/region-studies.svg and /tmp/region-studies.glb.
//! ```

use exedra::analytic::extrusion::{TessellationPolicy, Workplane};
use exedra::compile::{CompilePolicy, PartCompiler, PartId, PartInput, PartSource};
use exedra::constructive::{builders, ir::Placement3};
use exedra::curves::profile::{
    Arc, CornerKind, CornerTreatment, Curve, Loop, Profile, ProfilePolicy, RegionOp, RegionPolicy,
    SegTag,
};
use exedra::gltf::{GltfExportOptions, GltfMesh, GltfNode, GltfScene, export_glb_with_materials};
use std::{error::Error, fmt::Write as _, path::PathBuf};

const VALIDATION: ProfilePolicy = ProfilePolicy::new(1e-7);
const REGION: RegionPolicy = RegionPolicy::new(VALIDATION);
const TRANSITION: SegTag = SegTag(6000);

struct Study {
    name: &'static str,
    title: &'static str,
    profiles: Vec<Profile>,
    evidence: String,
}

fn studies() -> Result<Vec<Study>, Box<dyn Error>> {
    let horizontal = builders::rect_centered(40.0, 10.0)?.to_line_arc(&VALIDATION)?;
    let vertical = builders::rect_centered(10.0, 40.0)?.to_line_arc(&VALIDATION)?;
    let union = horizontal.combine(&vertical, RegionOp::Union, &REGION)?;
    let evidence = format!(
        "{} events; {} boundary sources",
        union.report.events,
        union.segments.len()
    );
    let cross = union.into_single()?;
    let corners = || {
        cross
            .corners()
            .filter(|c| c.kind == CornerKind::Concave)
            .map(|c| c.segment)
    };
    let fillet = cross.with_corners(
        corners(),
        CornerTreatment::fillet(1.0).tagged(TRANSITION),
        &VALIDATION,
    )?;
    let chamfer = cross.with_corners(
        corners(),
        CornerTreatment::chamfer(1.0).tagged(TRANSITION),
        &VALIDATION,
    )?;
    let mut studies = vec![
        Study {
            name: "cross_union",
            title: "Union: two filled rectangles",
            profiles: vec![cross],
            evidence,
        },
        Study {
            name: "concave_fillets",
            title: "Four concave fillets, radius 1",
            profiles: vec![fillet.profile],
            evidence: format!("{} labelled circular transitions", fillet.changed_corners),
        },
        Study {
            name: "concave_chamfers",
            title: "Four concave chamfers, trim 1",
            profiles: vec![chamfer.profile],
            evidence: format!("{} labelled straight transitions", chamfer.changed_corners),
        },
    ];
    let a = builders::circle(12.0)?
        .translated((-6.0, 0.0))?
        .to_line_arc(&VALIDATION)?;
    let b = builders::circle(12.0)?
        .translated((6.0, 0.0))?
        .to_line_arc(&VALIDATION)?;
    for (name, title, op) in [
        (
            "circle_union",
            "Union: circular boundaries",
            RegionOp::Union,
        ),
        (
            "circle_intersection",
            "Intersection: circular lens",
            RegionOp::Intersection,
        ),
        (
            "circle_difference",
            "Difference: circular crescent",
            RegionOp::Difference,
        ),
    ] {
        let result = a.combine(&b, op, &REGION)?;
        studies.push(Study {
            name,
            title,
            profiles: result.profiles,
            evidence: format!(
                "{} events; {} retained arcs",
                result.report.events, result.report.retained_segments
            ),
        });
    }
    let plate = builders::rect_centered(40.0, 20.0)?.to_line_arc(&VALIDATION)?;
    let stripe = builders::rect_centered(8.0, 30.0)?.to_line_arc(&VALIDATION)?;
    let split = plate.combine(&stripe, RegionOp::Difference, &REGION)?;
    studies.push(Study {
        name: "split_difference",
        title: "Difference: disconnected material",
        evidence: format!("{} connected profiles", split.profiles.len()),
        profiles: split.profiles,
    });
    let square = builders::rect_centered(40.0, 40.0)?.to_line_arc(&VALIDATION)?;
    let ring = builders::ring(14.0, 6.0)?.to_line_arc(&VALIDATION)?;
    let island = square.combine(&ring, RegionOp::Difference, &REGION)?;
    studies.push(Study {
        name: "nested_island",
        title: "Difference: a retained island",
        evidence: format!(
            "{} connected profiles; circular hole retained",
            island.profiles.len()
        ),
        profiles: island.profiles,
    });
    Ok(studies)
}

fn path(loop_: &Loop) -> String {
    let mut path = String::new();
    let start = loop_.segments().last().expect("validated loop").to;
    write!(path, "M{},{}", start[0], start[1]).unwrap();
    for (from, segment) in loop_.iter_with_starts() {
        match segment.curve {
            Curve::Line => write!(path, "L{},{}", segment.to[0], segment.to[1]).unwrap(),
            curve => {
                let arc = Arc::resolve(from, segment.to, curve)
                    .expect("validated arc")
                    .expect("non-line curve");
                write!(
                    path,
                    "A{r},{r} 0 {large} {positive} {},{}",
                    segment.to[0],
                    segment.to[1],
                    r = arc.radius,
                    large = u8::from(arc.sweep.abs() > std::f64::consts::PI),
                    positive = u8::from(arc.sweep > 0.0)
                )
                .unwrap();
            }
        }
    }
    path.push('Z');
    path
}

fn svg(studies: &[Study]) -> String {
    let rows = (studies.len() + 1).div_ceil(3);
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"960\" height=\"{}\" viewBox=\"0 0 960 {}\"><rect width=\"100%\" height=\"100%\" fill=\"#edf2f5\"/><g font-family=\"sans-serif\" fill=\"#17384a\">",
        rows * 250,
        rows * 250
    );
    for (i, study) in studies.iter().enumerate() {
        let x = (i % 3) * 320 + 8;
        let y = (i / 3) * 250 + 8;
        write!(svg,"<rect x=\"{x}\" y=\"{y}\" width=\"304\" height=\"234\" rx=\"8\" fill=\"white\"/><text x=\"{}\" y=\"{}\" font-size=\"15\">{}</text><text x=\"{}\" y=\"{}\" font-size=\"10\">{}</text><g transform=\"translate({},{}) scale(4,-4)\" fill=\"#8fc6d8\" stroke=\"#17475a\" stroke-width=\"0.22\" fill-rule=\"evenodd\">",x+12,y+24,study.title,x+12,y+43,study.evidence,x+152,y+130).unwrap();
        for profile in &study.profiles {
            let mut outline = path(profile.outer());
            for hole in profile.holes() {
                outline.push_str(&path(hole));
            }
            write!(svg, "<path d=\"{outline}\"/>").unwrap();
            for loop_ in std::iter::once(profile.outer()).chain(profile.holes()) {
                for (from, segment) in loop_
                    .iter_with_starts()
                    .filter(|(_, s)| s.tag == Some(TRANSITION))
                {
                    let mut d = format!("M{},{}", from[0], from[1]);
                    match segment.curve {
                        Curve::Line => write!(d, "L{},{}", segment.to[0], segment.to[1]).unwrap(),
                        curve => {
                            let arc = Arc::resolve(from, segment.to, curve)
                                .unwrap()
                                .expect("non-line curve");
                            write!(
                                d,
                                "A{r},{r} 0 0 {} {},{}",
                                u8::from(arc.sweep > 0.0),
                                segment.to[0],
                                segment.to[1],
                                r = arc.radius
                            )
                            .unwrap();
                        }
                    }
                    write!(
                        svg,
                        "<path d=\"{d}\" fill=\"none\" stroke=\"#e97c25\" stroke-width=\"0.7\"/>"
                    )
                    .unwrap();
                }
            }
        }
        let area: f64 = study.profiles.iter().map(Profile::area).sum();
        write!(
            svg,
            "</g><text x=\"{}\" y=\"{}\" font-size=\"12\">Filled area: {area:.4} mm²</text>",
            x + 12,
            y + 222
        )
        .unwrap();
    }
    if let Some(fillet) = studies.iter().find(|s| s.name == "concave_fillets")
        && let Some(original) = studies.iter().find(|s| s.name == "cross_union")
        && let Some((from, segment)) = fillet.profiles[0]
            .outer()
            .iter_with_starts()
            .find(|(p, s)| s.tag == Some(TRANSITION) && p[0] > 0.0 && p[1] > 0.0)
        && let Some(arc) = Arc::resolve(from, segment.to, segment.curve).unwrap()
    {
        let index = studies.len();
        let x = (index % 3) * 320 + 8;
        let y = (index / 3) * 250 + 8;
        let center = [
            arc.center[0] - 0.5 * arc.radius,
            arc.center[1] - 0.5 * arc.radius,
        ];
        write!(svg,"<defs><clipPath id=\"corner-detail\"><rect x=\"{}\" y=\"{}\" width=\"272\" height=\"160\"/></clipPath></defs><rect x=\"{x}\" y=\"{y}\" width=\"304\" height=\"234\" rx=\"8\" fill=\"white\"/><text x=\"{}\" y=\"{}\" font-size=\"15\">Concave fillet: 11× detail</text><text x=\"{}\" y=\"{}\" font-size=\"10\">Same profile; original boundary dashed</text><g clip-path=\"url(#corner-detail)\"><g transform=\"translate({},{}) scale(44,-44) translate({},{})\"><path d=\"{}\" fill=\"#8fc6d8\" stroke=\"#17475a\" stroke-width=\"0.022\"/><path d=\"{}\" fill=\"none\" stroke=\"#456372\" stroke-width=\"0.023\" stroke-dasharray=\"0.10 0.08\"/>",x+16,y+50,x+12,y+24,x+12,y+43,x+152,y+130,-center[0],-center[1],path(fillet.profiles[0].outer()),path(original.profiles[0].outer())).unwrap();
        write!(svg,"<path d=\"M{},{} A{r},{r} 0 0 {} {},{}\" fill=\"none\" stroke=\"#e97c25\" stroke-width=\"0.065\"/><circle cx=\"{}\" cy=\"{}\" r=\"0.05\" fill=\"#17384a\"/><circle cx=\"{}\" cy=\"{}\" r=\"0.05\" fill=\"#17384a\"/></g></g><text x=\"{}\" y=\"{}\" font-size=\"12\">Radius {:.4} mm; tangent joins marked</text>",from[0],from[1],u8::from(arc.sweep>0.0),segment.to[0],segment.to[1],from[0],from[1],segment.to[0],segment.to[1],x+12,y+222,arc.radius,r=arc.radius).unwrap();
    }
    svg.push_str("</g></svg>");
    svg
}

fn main() -> Result<(), Box<dyn Error>> {
    let prefix = std::env::args().nth(1).map(PathBuf::from);
    let studies = studies()?;
    if let Some(prefix) = &prefix {
        std::fs::write(prefix.with_extension("svg"), svg(&studies))?;
    }
    let mut names = Vec::new();
    let mut sources = Vec::new();
    for (index, study) in studies.into_iter().enumerate() {
        let xy = [
            60.0 * f64::from(u32::try_from(index % 3)?),
            -60.0 * f64::from(u32::try_from(index / 3)?),
        ];
        for (part, profile) in study.profiles.into_iter().enumerate() {
            let name = format!("{}_{part}", study.name);
            let solid = Workplane::XY.translated(xy)?.extrude(profile, 3.0)?;
            let realized = solid.tessellate_with_regions(
                &TessellationPolicy {
                    chord_tolerance: 0.02,
                    ..Default::default()
                },
                |id, _| u32::from(solid.face_tag(id) == Some(TRANSITION)),
            )?;
            assert!(
                realized.mesh.boundary_loops()?.is_empty(),
                "{name} must be closed"
            );
            assert!(
                realized.report.within_tolerance(),
                "{name} must meet its mesh deviation"
            );
            println!(
                "{name}: volume={:.6} mm3 native_faces={} mesh_faces={} max_deviation={:.6} mm",
                solid.volume(),
                solid.faces().len(),
                realized.mesh.faces().count(),
                realized.report.max_deviation
            );
            names.push(name);
            sources.push(PartSource::Baked(realized.mesh));
        }
    }
    let Some(prefix) = prefix else {
        return Ok(());
    };
    let inputs: Vec<_> = names
        .iter()
        .zip(&sources)
        .map(|(name, source)| PartInput::new(name, source))
        .collect();
    let compiled = PartCompiler::new().compile_parts(&inputs, &CompilePolicy::default())?;
    let mut root = GltfNode::new("profile_region_studies");
    root.transform = Placement3::from_axes(
        [1e-3, 0.0, 0.0],
        [0.0, 1e-3, 0.0],
        [0.0, 0.0, 1e-3],
        [0.0; 3],
    );
    let mut nodes = vec![root];
    for (i, name) in names.iter().enumerate() {
        let id = PartId(u32::try_from(i)?);
        let mut mesh = GltfMesh::new(id, name, &compiled)?;
        mesh.materials = compiled
            .part(id)
            .unwrap()
            .bodies
            .iter()
            .map(|b| {
                b.regions
                    .iter()
                    .map(|r| Some(if r.region == 1 { "transition" } else { "base" }.to_owned()))
                    .collect()
            })
            .collect();
        let mut node = GltfNode::new(name);
        node.parent = Some(0);
        node.geometry = Some(mesh.into());
        nodes.push(node);
    }
    let scene = GltfScene {
        nodes,
        ..Default::default()
    };
    let resolve = |key: &str| {
        Some(
            serde_json::json!({"name":key,"pbrMetallicRoughness":{"baseColorFactor":if key=="transition" {[0.95,0.35,0.06,1.0]}else{[0.22,0.58,0.73,1.0]},"metallicFactor":0.2,"roughnessFactor":0.4}}),
        )
    };
    let export = export_glb_with_materials(
        &scene,
        &compiled,
        &resolve,
        GltfExportOptions::z_up_to_y_up(),
    )?;
    std::fs::write(prefix.with_extension("glb"), &export.bytes)?;
    println!(
        "wrote {}.svg and {}.glb ({} meshes, {} materials)",
        prefix.display(),
        prefix.display(),
        export.stats.meshes,
        export.stats.materials
    );
    Ok(())
}
