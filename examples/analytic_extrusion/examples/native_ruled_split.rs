// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! One original ruled support, partitioned into shared native faces.
//! `cargo run -p analytic_extrusion --example native_ruled_split -- /tmp/ruled-split`
mod support;
use exedra::{
    analytic::{
        body::{Body, FaceId},
        geometry::{Curve, CurveSpan, NurbsCurve, ParameterRange, Surface},
        planar_extrusion::{PlanarExtrusion, PlanarExtrusionPolicy, PlanarSegment},
        query::{ProjectionPolicy, RuledProjection},
        realize::{RealizationPolicy, RealizedBody, TriangleQuality},
        split::{RuledSplitPolicy, RuledSplitReport, split_ruled_face_at_u},
    },
    constructive::ir::Placement3,
    curves::profile::SegTag,
    step::{StepOptions, export_body},
};
use serde_json::{Value, json};
use std::{error::Error, path::PathBuf, time::Instant};
type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn source() -> Result<(Body<(), u8>, FaceId)> {
    let curve = Curve::nurbs(NurbsCurve::new(
        4,
        vec![2., 2., 2., 2., 5., 5., 5., 5.],
        vec![[0., 0., 0.], [1., 2., 0.], [3., 2., 0.], [4., 0., 0.]],
        Some(vec![1., 0.7, 1.4, 1.]),
    )?)?;
    let outer = vec![
        PlanarSegment::line([0., 0.], [4., 0.])?,
        PlanarSegment::new(CurveSpan::new(curve, ParameterRange::new(2., 5.)?, true)?)
            .tagged(SegTag(42)),
    ];
    let points = [[1.5, 0.3], [1.5, 0.6], [2.5, 0.6], [2.5, 0.3]];
    let hole = (0..4)
        .map(|i| PlanarSegment::line(points[i], points[(i + 1) % 4]).map_err(Into::into))
        .collect::<Result<Vec<_>>>()?;
    let solid = PlanarExtrusion::new(
        outer,
        vec![hole],
        Placement3::IDENTITY,
        2.,
        &PlanarExtrusionPolicy::default(),
    )?;
    let face = solid
        .faces_tagged(SegTag(42))
        .next()
        .expect("named crown")
        .0;
    Ok((solid.into_body().map_roles(|_| (), |_| 0_u8), face))
}
fn report(report: &RuledSplitReport) -> Value {
    json!({"source_face":report.source.0,"source_u":report.source_domain.u.bounds(),"u":report.u,
        "children":report.faces.map(|c|json!({"face":c.face.0,"native_u":c.domain.u.bounds()})),
        "rims":report.rims.map(|r|json!({"source":r.source.0,"source_range":r.source_range.bounds(),
            "children":r.children.map(|id|id.0),"ranges":r.ranges.map(ParameterRange::bounds),
            "parameter":r.parameter,"vertex":r.vertex.0,"neighbor":r.neighbor.0,"neighbor_loop":r.neighbor_loop.0})),
        "source_proof":{"controls":report.work.source_controls,"metadata_work":report.work.source_metadata_work,"preparation_work_bound":report.work.source_preparation_work,"enclosure_work":report.work.source_enclosure_work},
        "new_ruling":report.ruling.0,"input_coedges":report.work.input_coedges,"output_coedges":report.validation.coedges,
        "local_support_gap":report.validation.geometry.max_gap,"global_solid_validity":"notChecked"})
}
fn realize(body: &Body<(), u8>, highlight: Option<FaceId>) -> Result<RealizedBody> {
    let output = body.realize_with_regions(
        Placement3::IDENTITY,
        &RealizationPolicy {
            tolerance: 0.01,
            quality: Some(TriangleQuality::new(0.2)),
            ..Default::default()
        },
        |face, _| if Some(face) == highlight { 42 } else { 0 },
    )?;
    assert!(
        output.report.within_tolerance(),
        "emitted geometry must meet tolerance"
    );
    assert!(
        output.mesh.boundary_loops()?.is_empty(),
        "emitted solid must be watertight"
    );
    assert!(
        output.mesh.validate_deep().is_empty(),
        "emitted topology must remain valid"
    );
    Ok(output)
}
fn stats(body: &Body<(), u8>, display: &RealizedBody) -> Value {
    json!({"native_vertices":body.vertices().len(),"native_edges":body.edges().len(),"native_faces":body.faces().len(),
        "mesh_vertices":display.mesh.vertices().count(),"mesh_triangles":display.mesh_faces.len(),
        "max_deviation":display.report.max_deviation,"within_tolerance":display.report.within_tolerance(),
        "quality":display.report.quality.as_ref().map(|q|json!({"within_target":q.within_target(),"bad":q.remaining_bad_triangles,"max_ratio":q.max_radius_edge_ratio}))})
}
fn topology(
    name: &str,
    display: &RealizedBody,
    translation: f64,
    rulings: &[exedra::analytic::body::EdgeId],
) -> Value {
    let positions: Vec<_> = display
        .mesh
        .vertices()
        .map(|v| {
            let p = display.mesh.vertex_position(v).expect("emitted vertex");
            [
                f64::from(p[0]) + translation,
                f64::from(p[1]),
                f64::from(p[2]),
            ]
        })
        .collect();
    let edges: Vec<_> = display
        .mesh
        .half_edges()
        .filter(|&h| display.mesh.canonical_edge(h) == Some(h))
        .map(|h| {
            [
                display.mesh.from_vertex(h).expect("edge start").index(),
                display.mesh.to_vertex(h).expect("edge end").index(),
            ]
        })
        .collect();
    let native_rulings: Vec<_> = rulings
        .iter()
        .map(|&edge| {
            let vertices: Vec<_> = display
                .edge_chains
                .get(edge)
                .expect("shared ruling")
                .iter()
                .map(|v| v.index())
                .collect();
            json!({"edge":edge.0,"vertices":vertices})
        })
        .collect();
    json!({"name":name,"positions_mm":positions,"edges":edges,"native_rulings":native_rulings,
        "polygon_faces":display.mesh_faces.len(),"triangles":display.mesh_faces.len()})
}
fn main() -> Result<()> {
    let directory = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "native-ruled-split".into()),
    );
    std::fs::create_dir_all(&directory)?;
    let (source, face) = source()?;
    // A synthetic model-space pick on the crown: only the point enters the
    // global query; no UV hint or stationarity/uniqueness assumption is supplied.
    let pick = source
        .evaluate_support(face, [0.37, 0.43])
        .expect("source pick")
        .point;
    let Surface::Ruled(support) = &source.face(face).expect("source face").surface else {
        unreachable!("crown wall")
    };
    let query = RuledProjection::prepare(
        support,
        source.face(face).expect("source face").domain,
        &ProjectionPolicy::default(),
    )?;
    let mut scratch = query.scratch()?;
    let projected = query.closest(pick, &mut scratch)?;
    assert!(
        projected.within_tolerance(),
        "complete native distance bracket required"
    );
    let start = Instant::now();
    let split = split_ruled_face_at_u(
        &source,
        face,
        projected.witness.uv[0],
        99,
        &RuledSplitPolicy::default(),
    )?;
    let split_ms = start.elapsed().as_secs_f64() * 1000.;
    let repeat = split_ruled_face_at_u(
        &split.body,
        split.report.faces[1].face,
        0.73,
        99,
        &RuledSplitPolicy::default(),
    )?;
    let original_support = &source.faces()[face.index()].surface;
    for result in [&split, &repeat] {
        assert!(
            result
                .report
                .faces
                .iter()
                .all(|c| &result.body.faces()[c.face.index()].surface == original_support),
            "subdivision must retain the original supporting surface"
        );
    }
    let before = realize(&source, None)?;
    let after = realize(&split.body, Some(split.report.faces[1].face))?;
    let twice = realize(&repeat.body, Some(repeat.report.faces[1].face))?;
    let evidence = json!({"scope":"native body split includes input/output local validation, cloned result and correspondence; source construction, projection query, meshes, STEP, file IO and result destruction excluded from split_ms",
        "split_ms":split_ms,"before":stats(&source,&before),"after":stats(&split.body,&after),"repeat":stats(&repeat.body,&twice),
        "projection":{"target":pick,"native_uv":projected.witness.uv,"distance_lower":projected.distance_lower,"distance_upper":projected.witness.distance_upper,"point_error_upper":projected.witness.point_error_upper,"status":format!("{:?}",projected.status),"work":projected.work.total(),"scope":"global distance bracket; no UV uniqueness or UV error bound"},
        "operation":report(&split.report),"repeated_operation":report(&repeat.report),
        "support_unchanged":true,"fixture_units":"mm","glb_layout_model_units":[[0,0,0],[6,0,0],[12,0,0]],
        "independent_reference_volume":7.757848488677006_f64,
        "material_witnesses":[{"point":[0.2,0.1,1.0],"inside":true},{"point":[3.8,0.1,1.0],"inside":true},{"point":[2.0,1.0,1.0],"inside":true},{"point":[2.0,0.45,1.0],"inside":false},{"point":[2.0,2.0,1.0],"inside":false}]});
    for (name, body) in [
        ("before", &source),
        ("after", &split.body),
        ("repeat", &repeat.body),
    ] {
        let document = export_body(
            body,
            Placement3::IDENTITY,
            &StepOptions {
                name,
                ..Default::default()
            },
        )?;
        std::fs::write(directory.join(format!("{name}.step")), document.text)?;
    }
    std::fs::write(
        directory.join("topology.json"),
        serde_json::to_vec(&[
            topology("Before", &before, 0., &[]),
            topology("One shared ruling", &after, 6., &[split.report.ruling]),
            topology(
                "Repeated subdivision",
                &twice,
                12.,
                &[split.report.ruling, repeat.report.ruling],
            ),
        ])?,
    )?;
    support::export(
        directory
            .join("comparison.glb")
            .to_str()
            .expect("UTF8 output directory"),
        "Ruled face subdivision",
        vec![
            support::Exhibit {
                name: "Before",
                mesh: before.mesh,
                placement: Placement3::IDENTITY,
                color: [0.35, 0.62, 0.8, 1.],
                grid: false,
                cut_regions: &[],
            },
            support::Exhibit {
                name: "One shared ruling",
                mesh: after.mesh,
                placement: Placement3::translate(6., 0., 0.),
                color: [0.35, 0.62, 0.8, 1.],
                grid: false,
                cut_regions: &[42],
            },
            support::Exhibit {
                name: "Repeated subdivision",
                mesh: twice.mesh,
                placement: Placement3::translate(12., 0., 0.),
                color: [0.35, 0.62, 0.8, 1.],
                grid: false,
                cut_regions: &[42],
            },
        ],
    )?;
    std::fs::write(
        directory.join("report.json"),
        serde_json::to_string_pretty(&evidence)?,
    )?;
    println!("{}", serde_json::to_string_pretty(&evidence)?);
    // Ruled supports remain native values; no constructor chart is inferred.
    assert!(
        matches!(original_support, Surface::Ruled(_)),
        "native surface family retained"
    );
    Ok(())
}
