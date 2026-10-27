// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::*;
use alloc::{collections::BTreeSet, sync::Arc, vec, vec::Vec};
use exedra_analytic::{
    body::FaceId,
    loft::{Loft, LoftPolicy},
    revolve::Revolution,
    sweep::Sweep,
};
use exedra_constructive::{
    builders,
    chart::{ChartError, ChartTransform, LoftStations, SurfaceChart},
};
use exedra_math::{
    Placement3,
    path::{CurvePathView, PathClosure, PathJoin, PathSegment3},
};
use exedra_profile::{Loop, Profile, ProfilePolicy, Segment};

pub(crate) fn revolution() -> NativeSource {
    let outline = Loop::new(vec![
        Segment::line([6.0, -2.0]),
        Segment::line([6.0, 2.0]),
        Segment::line([4.0, 2.0]),
        Segment::line([4.0, -2.0]),
    ])
    .unwrap();
    Revolution::new(
        Profile::new(outline, vec![], &ProfilePolicy::new(1e-9)).unwrap(),
        Placement3::IDENTITY,
        core::f64::consts::TAU,
    )
    .unwrap()
    .into()
}
fn sweep() -> NativeSource {
    let profile = builders::rect_centered(4.0, 4.0)
        .unwrap()
        .to_line_arc(&ProfilePolicy::new(1e-9))
        .unwrap();
    let spans = [PathSegment3::Line {
        to: [0.0, 0.0, 5.0],
    }];
    let path = CurvePathView {
        start: [0.0; 3],
        segments: &spans,
        section_x: [1.0, 0.0, 0.0],
        section_origin: [0.0; 2],
        closure: PathClosure::Open,
        joins: PathJoin::Smooth,
    };
    Sweep::new(profile, &path, Placement3::IDENTITY)
        .unwrap()
        .into()
}
fn loft() -> NativeSource {
    let policy = ProfilePolicy::new(1e-9);
    let profile = builders::rect_centered(4.0, 4.0)
        .unwrap()
        .to_line_arc(&policy)
        .unwrap();
    Loft::from_profiles(
        [
            (Placement3::IDENTITY, &profile),
            (Placement3::translate(0.0, 0.0, 2.0), &profile),
            (Placement3::translate(0.0, 0.0, 8.0), &profile),
        ],
        Placement3::IDENTITY,
        &LoftPolicy::new(policy),
    )
    .unwrap()
    .into()
}
pub(crate) fn chart() -> SurfaceChart {
    SurfaceChart::Revolve {
        reference_radius: 5.0,
        wall: ChartTransform::IDENTITY,
        caps: ChartTransform::IDENTITY,
    }
}
pub(crate) fn triangle_key(points: [[f32; 3]; 3]) -> [[u32; 3]; 3] {
    let points = points.map(|p| p.map(f32::to_bits));
    [
        points,
        [points[1], points[2], points[0]],
        [points[2], points[0], points[1]],
    ]
    .into_iter()
    .min()
    .unwrap()
}
fn expected_triangles(
    realized: &exedra_analytic::realize::RealizedBody,
) -> BTreeSet<(u32, [[u32; 3]; 3])> {
    realized
        .mesh_faces
        .iter()
        .map(|&(face, native)| {
            let points: Vec<_> = realized
                .mesh
                .face_loop(face)
                .map(|c| {
                    *realized
                        .mesh
                        .vertex_position(realized.mesh.from_vertex(c).unwrap())
                        .unwrap()
                })
                .collect();
            (native.0, triangle_key(points.try_into().unwrap()))
        })
        .collect()
}
#[test]
fn charted_compilation_retains_source_identity_geometry_and_triangle_correspondence() {
    let wall = ChartTransform {
        matrix: [[0.2, 0.0], [0.0, 0.2]],
        offset: [0.0; 2],
    };
    for (source, chart) in [
        (
            revolution(),
            SurfaceChart::Revolve {
                reference_radius: 5.0,
                wall,
                caps: wall,
            },
        ),
        (sweep(), SurfaceChart::Sweep { wall, caps: wall }),
        (
            loft(),
            SurfaceChart::Loft {
                reference_section: 0,
                rest_length: 12.0,
                stations: LoftStations::DatumDistance,
                wall,
                caps: wall,
            },
        ),
    ] {
        source.check_chart(&chart).unwrap();
        let fingerprint = source.fingerprint();
        let selected = source.select_face(FaceId(0)).unwrap();
        let prepared = PreparedSource::new(PartSource::Native(source.clone()));
        let mut compiler = PartCompiler::new();
        let mut policy = CompilePolicy::default();
        let uncharted = compiler
            .compile_snapshot(&[PartInput::prepared("part", &prepared)], &policy)
            .unwrap();
        let raw = source.realize(&policy.native).unwrap();
        let mut expected = expected_triangles(&raw);
        assert_eq!(expected.len(), raw.mesh_faces.len());
        policy.native_chart = Some(chart);
        policy.tangents = Some(exedra_mesh::TangentUv::Primary);
        let charted = compiler
            .compile_snapshot(&[PartInput::prepared("part", &prepared)], &policy)
            .unwrap();
        let body = charted.body(PartId(0), 0).unwrap();
        assert!(body.native().unwrap().same_installation(&source));
        assert_eq!(body.native_realization().unwrap().chart(), Some(&chart));
        assert!(
            uncharted
                .body(PartId(0), 0)
                .unwrap()
                .native_realization()
                .unwrap()
                .chart()
                .is_none()
        );
        assert_eq!(
            body.native().unwrap().resolve_face(&selected),
            Ok(FaceId(0))
        );
        assert_eq!(source.fingerprint(), fingerprint);
        let compiled = &charted.compiled().part(PartId(0)).unwrap().bodies[0];
        assert!(compiled.regions.iter().all(|r| r.has_uvs));
        assert!(compiled.tri.uvs.iter().flatten().all(|v| v.is_finite()));
        assert_eq!(compiled.tri.indices.len() / 3, raw.mesh_faces.len());
        for (ordinal, indices) in compiled.tri.indices.as_chunks::<3>().0.iter().enumerate() {
            let native = body.native_render_face(ordinal).unwrap();
            let points = core::array::from_fn(|i| compiled.tri.positions[indices[i] as usize]);
            assert!(
                expected.remove(&(native.native_id().0, triangle_key(points))),
                "render/native mapping must match actual geometry"
            );
        }
        assert!(expected.is_empty());
        let hit = compiler
            .compile_snapshot(&[PartInput::prepared("part", &prepared)], &policy)
            .unwrap();
        assert!(Arc::ptr_eq(
            charted.compiled().part(PartId(0)).unwrap(),
            hit.compiled().part(PartId(0)).unwrap()
        ));
        assert_eq!(compiler.counters().native_realizations, 2);
        let hit_body = hit.body(PartId(0), 0).unwrap();
        assert!(core::ptr::eq(
            body.native_realization().unwrap().chart().unwrap(),
            hit_body.native_realization().unwrap().chart().unwrap()
        ));
        let fine = compiler
            .compile_snapshot(
                &[PartInput::prepared("part", &prepared)],
                &CompilePolicy {
                    native: exedra_analytic::realize::RealizationPolicy {
                        tolerance: 0.005,
                        ..Default::default()
                    },
                    ..policy.clone()
                },
            )
            .unwrap();
        assert_eq!(
            fine.body(PartId(0), 0)
                .unwrap()
                .native()
                .unwrap()
                .resolve_face(&selected),
            Ok(FaceId(0))
        );
        assert_eq!(
            uncharted
                .body(PartId(0), 0)
                .unwrap()
                .native()
                .unwrap()
                .resolve_face(&selected),
            Ok(FaceId(0))
        );
        assert_eq!(
            uncharted
                .body(PartId(0), 0)
                .unwrap()
                .native_realization()
                .unwrap()
                .report,
            body.native_realization().unwrap().report
        );
    }
}
#[test]
fn revolution_uv_seam_splits_render_vertices_without_splitting_native_identity() {
    let source = revolution();
    let prepared = PreparedSource::new(PartSource::Native(source.clone()));
    let mut compiler = PartCompiler::new();
    let mut policy = CompilePolicy::default();
    let plain = compiler
        .compile_snapshot(&[PartInput::prepared("turned", &prepared)], &policy)
        .unwrap();
    policy.native_chart = Some(chart());
    let charted = compiler
        .compile_snapshot(&[PartInput::prepared("turned", &prepared)], &policy)
        .unwrap();
    let tri = &charted.compiled().part(PartId(0)).unwrap().bodies[0].tri;
    assert!(
        tri.positions.len()
            > plain.compiled().part(PartId(0)).unwrap().bodies[0]
                .tri
                .positions
                .len()
    );
    assert!(
        (0..tri.positions.len()).any(|i| (i + 1..tri.positions.len())
            .any(|j| tri.positions[i] == tri.positions[j]
                && tri.normals[i] == tri.normals[j]
                && tri.uvs[i] != tri.uvs[j])),
        "authored seam needs two UVs at a shared native point"
    );
    let held = charted.body(PartId(0), 0).unwrap();
    compiler.clear_cache();
    assert_eq!(held.native_realization().unwrap().chart(), Some(&chart()));
    let fresh = revolution();
    let fresh_prepared = PreparedSource::new(PartSource::Native(fresh.clone()));
    let shared = compiler
        .compile_snapshot(&[PartInput::prepared("restored", &prepared)], &policy)
        .unwrap();
    let hit = compiler
        .compile_snapshot(&[PartInput::prepared("fresh", &fresh_prepared)], &policy)
        .unwrap();
    assert!(Arc::ptr_eq(
        shared.compiled().part(PartId(0)).unwrap(),
        hit.compiled().part(PartId(0)).unwrap()
    ));
    let selection = source.select_face(FaceId(0)).unwrap();
    assert_eq!(
        hit.body(PartId(0), 0)
            .unwrap()
            .native()
            .unwrap()
            .resolve_face(&selection),
        Err(NativeSelectionError::SourceMismatch)
    );
}
#[test]
fn native_chart_configuration_and_capability_refusals_precede_cache_work() {
    let source = revolution();
    let prepared = PreparedSource::new(PartSource::Native(source.clone()));
    let mut compiler = PartCompiler::new();
    let mut policy = CompilePolicy {
        native_chart: Some(chart()),
        ..Default::default()
    };
    compiler
        .compile_snapshot(&[PartInput::prepared("good", &prepared)], &policy)
        .unwrap();
    let counters = compiler.counters();
    policy.native_chart = Some(SurfaceChart::Sweep {
        wall: ChartTransform::IDENTITY,
        caps: ChartTransform::IDENTITY,
    });
    assert!(
        matches!(compiler.compile_snapshot(&[PartInput::prepared("wrong-kind",&prepared)],&policy),Err(CompileError::NativeChart {part:PartId(0),part_key,error}) if part_key=="wrong-kind"&&matches!(*error,NativeChartError::WrongOperation {family:NativeFamily::Revolution}))
    );
    let mut wall = ChartTransform::IDENTITY;
    wall.offset[0] = f64::NAN;
    policy.native_chart = Some(SurfaceChart::Revolve {
        reference_radius: 5.0,
        wall,
        caps: ChartTransform::IDENTITY,
    });
    assert!(
        matches!(compiler.compile_snapshot(&[PartInput::prepared("invalid",&prepared)],&policy),Err(CompileError::NativeChart {error,..}) if matches!(*error,NativeChartError::Invalid(ChartError::InvalidTransform)))
    );
    assert_eq!(compiler.counters(), counters);
    let shape = builders::rect_centered(40.0, 30.0)
        .unwrap()
        .to_line_arc(&ProfilePolicy::new(1e-9))
        .unwrap();
    let plate =
        exedra_analytic::extrusion::Extrusion::new(shape, Placement3::IDENTITY, 10.0).unwrap();
    let bored = plate
        .with_through_bore(
            exedra_analytic::bore::Bore::new([0.0; 3], [0.2, 0.0, 1.0], 2.0).unwrap(),
            &exedra_analytic::bore::BorePolicy {
                separation: 0.1,
                geometry_gap: 1e-8,
                min_axis_dot: 0.1,
            },
        )
        .unwrap();
    for native in [NativeSource::from(plate), NativeSource::from(bored)] {
        assert_eq!(
            native.check_chart(&chart()),
            Err(NativeChartError::Unavailable {
                family: native.family()
            })
        );
    }
    let native = loft();
    assert_eq!(
        native.check_chart(&SurfaceChart::Loft {
            reference_section: 3,
            rest_length: 12.0,
            stations: LoftStations::SectionIndex,
            wall: ChartTransform::IDENTITY,
            caps: ChartTransform::IDENTITY
        }),
        Err(NativeChartError::Invalid(
            ChartError::InvalidReferenceSection { section: 3 }
        ))
    );
}
#[test]
fn every_native_chart_transform_metric_and_station_mode_changes_policy_identity() {
    let mut policy = CompilePolicy::default();
    let none = policy_fingerprint(&policy);
    let base = chart();
    policy.native_chart = Some(base);
    let original = policy_fingerprint(&policy);
    assert_ne!(none, original);
    let mut fingerprints = BTreeSet::new();
    fingerprints.insert(original);
    for slot in 0..12 {
        let mut wall = ChartTransform::IDENTITY;
        let mut caps = ChartTransform::IDENTITY;
        let transform = if slot < 6 { &mut wall } else { &mut caps };
        let at = slot % 6;
        if at < 4 {
            transform.matrix[at / 2][at % 2] += 0.25;
        } else {
            transform.offset[at - 4] += 0.25;
        }
        policy.native_chart = Some(SurfaceChart::Revolve {
            reference_radius: 5.0,
            wall,
            caps,
        });
        assert!(
            fingerprints.insert(policy_fingerprint(&policy)),
            "each transform word joins the key"
        );
    }
    for (reference_section, rest_length, stations) in [
        (0, 10.0, LoftStations::SectionIndex),
        (1, 10.0, LoftStations::SectionIndex),
        (0, 11.0, LoftStations::SectionIndex),
        (0, 10.0, LoftStations::DatumDistance),
    ] {
        policy.native_chart = Some(SurfaceChart::Loft {
            reference_section,
            rest_length,
            stations,
            wall: ChartTransform::IDENTITY,
            caps: ChartTransform::IDENTITY,
        });
        assert!(
            fingerprints.insert(policy_fingerprint(&policy)),
            "each loft metric joins the key"
        );
    }
    policy.native_chart = Some(SurfaceChart::Revolve {
        reference_radius: 6.0,
        wall: ChartTransform::IDENTITY,
        caps: ChartTransform::IDENTITY,
    });
    assert!(
        fingerprints.insert(policy_fingerprint(&policy)),
        "reference radius joins the key"
    );
}

#[test]
fn mapped_uv_numeric_refusal_retains_source_and_has_input_face_diagnosis() {
    let source = revolution();
    let selected = source.select_face(FaceId(0)).unwrap();
    let fingerprint = source.fingerprint();
    let prepared = PreparedSource::new(PartSource::Native(source.clone()));
    let mut compiler = PartCompiler::new();
    let mut wall = ChartTransform::IDENTITY;
    wall.offset = [1e30; 2];
    let policy = CompilePolicy {
        native_chart: Some(SurfaceChart::Revolve {
            reference_radius: 5.0,
            wall,
            caps: ChartTransform::IDENTITY,
        }),
        ..Default::default()
    };
    let error = compiler
        .compile_snapshot(
            &[PartInput::prepared("collapsed-texture", &prepared)],
            &policy,
        )
        .unwrap_err();
    assert!(
        matches!(error,CompileError::NativeChart {part:PartId(0),part_key,error} if part_key=="collapsed-texture" && matches!(*error,NativeChartError::Revolution(exedra_analytic::revolve::RevolutionUvError::Mapping {face:_,error:ChartError::NumericLimit})))
    );
    assert_eq!(source.fingerprint(), fingerprint);
    assert_eq!(source.resolve_face(&selected), Ok(FaceId(0)));
    assert_eq!(compiler.counters().native_realizations, 0);
    compiler
        .compile_snapshot(
            &[PartInput::prepared("recovered", &prepared)],
            &CompilePolicy {
                native_chart: Some(chart()),
                ..Default::default()
            },
        )
        .unwrap();
}

#[test]
fn loft_station_modes_change_uv_metric_without_changing_native_geometry() {
    let source = loft();
    let prepared = PreparedSource::new(PartSource::Native(source.clone()));
    let mut compiler = PartCompiler::new();
    for (stations, middle) in [
        (LoftStations::SectionIndex, 6.0),
        (LoftStations::DatumDistance, 3.0),
    ] {
        let chart = SurfaceChart::Loft {
            reference_section: 0,
            rest_length: 12.0,
            stations,
            wall: ChartTransform::IDENTITY,
            caps: ChartTransform::IDENTITY,
        };
        let output = compiler
            .compile_snapshot(
                &[PartInput::prepared("loft", &prepared)],
                &CompilePolicy {
                    native_chart: Some(chart),
                    ..Default::default()
                },
            )
            .unwrap();
        let tri = &output.compiled().part(PartId(0)).unwrap().bodies[0].tri;
        let mut checked = 0;
        for (position, uv) in tri.positions.iter().zip(&tri.uvs) {
            if position[2] == 2.0 {
                assert_eq!(uv[1], middle);
                checked += 1;
            }
        }
        assert!(checked > 0);
        assert_eq!(
            output
                .body(PartId(0), 0)
                .unwrap()
                .native_realization()
                .unwrap()
                .chart(),
            Some(&chart)
        );
    }
}

#[test]
fn borrowed_constructors_expose_caps_planes_and_structural_chart_kind() {
    use exedra_analytic::End;
    use exedra_constructive::chart::SurfaceChartKind;
    for (source, kind, has_caps) in [
        (revolution(), Some(SurfaceChartKind::Revolve), false),
        (sweep(), Some(SurfaceChartKind::Sweep), true),
        (loft(), Some(SurfaceChartKind::Loft), true),
    ] {
        let view = source.geometry();
        assert_eq!(view.family(), source.family());
        assert_eq!(view.frame(), source.frame());
        assert_eq!(view.chart_kind(), kind);
        assert_eq!(source.chart_kind(), kind);
        for end in [End::Start, End::End] {
            assert_eq!(source.cap(end), view.cap(end));
            assert_eq!(source.cap(end).is_some(), has_caps);
            if let Some(cap) = view.cap(end) {
                assert_eq!(view.planar_frame(cap), source.planar_frame(cap));
                let plane = view.planar_frame(cap).unwrap();
                let sign = if end == End::Start { -1.0 } else { 1.0 };
                assert!((plane.rows[2][2] - sign).abs() < 1e-12);
            }
        }
        assert_eq!(
            view.planar_frame(FaceId(u32::MAX)),
            Err(NativeSelectionError::UnknownFace {
                face: FaceId(u32::MAX)
            })
        );
    }
    let shape = builders::rect_centered(40.0, 30.0)
        .unwrap()
        .to_line_arc(&ProfilePolicy::new(1e-9))
        .unwrap();
    let plate = exedra_analytic::extrusion::Extrusion::new(
        shape,
        Placement3::rotate_y_then_translate(0.35, 8.0, -2.0, 5.0),
        10.0,
    )
    .unwrap();
    // A transient constructor is queried before any NativeSource installation.
    let view = NativeGeometryRef::Extrusion(&plate);
    assert_eq!(view.chart_kind(), None);
    let cap = view.cap(End::End).unwrap();
    let plane = view.planar_frame(cap).unwrap();
    let bored = plate
        .with_through_bore(
            exedra_analytic::bore::Bore::new([0.0; 3], [0.2, 0.0, 1.0], 2.0).unwrap(),
            &exedra_analytic::bore::BorePolicy {
                separation: 0.1,
                geometry_gap: 1e-8,
                min_axis_dot: 0.1,
            },
        )
        .unwrap();
    let view = NativeGeometryRef::BoredPlate(&bored);
    assert_eq!(view.chart_kind(), None);
    assert_eq!(view.cap(End::End), Some(cap));
    assert_eq!(view.planar_frame(cap), Ok(plane));
    assert_eq!(
        view.planar_frame(bored.report().wall),
        Err(NativeSelectionError::NotPlanar {
            face: bored.report().wall
        })
    );
}
