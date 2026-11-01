// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::*;
use alloc::{sync::Arc, vec};
use exedra_analytic::{
    End,
    body::FaceId,
    extrusion::{Extrusion, Pocket},
};
use exedra_constructive::builders;
use exedra_curves::profile::{ProfilePolicy, SegTag};
use exedra_math::{Placement3, cross, dot, norm, sub};

fn source(tag: SegTag, frame: Placement3) -> NativeSource {
    let policy = ProfilePolicy::new(1e-9);
    let profile = builders::rect_centered(40.0, 30.0)
        .unwrap()
        .to_profile(&policy)
        .unwrap();
    let bore = builders::circle(3.0).unwrap().to_profile(&policy).unwrap();
    Extrusion::new(profile, frame, 12.0)
        .unwrap()
        .with_pocket(
            Pocket::from_region(bore.outer(), End::End, 4.0).tagged(tag),
            &policy,
        )
        .unwrap()
        .into()
}
fn snapshot(
    compiler: &mut PartCompiler,
    native: &NativeSource,
    tolerance: f64,
) -> GeometrySnapshot {
    let input = PreparedSource::new(PartSource::Native(native.clone()));
    let mut policy = CompilePolicy::default();
    policy.native.tolerance = tolerance;
    compiler
        .compile_snapshot(&[PartInput::prepared("panel", &input)], &policy)
        .unwrap()
}
#[test]
fn native_content_is_captured_independently_of_installation_identity() {
    let a = source(SegTag(7), Placement3::IDENTITY);
    let b = source(SegTag(7), Placement3::IDENTITY);
    assert_eq!(a.fingerprint(), b.fingerprint());
    assert!(!a.same_installation(&b));
    assert!(a.same_installation(&a.clone()));
    assert_ne!(
        a.fingerprint(),
        source(SegTag(8), Placement3::IDENTITY).fingerprint()
    );
    assert_ne!(
        a.fingerprint(),
        source(SegTag(7), Placement3::translate(1.0, 0.0, 0.0)).fingerprint()
    );
    let input = PreparedSource::new(PartSource::Native(a.clone()));
    assert_eq!(a.fingerprint(), input.fingerprint());
    assert!((a.volume().unwrap() - (14_400.0 - 36.0 * core::f64::consts::PI)).abs() < 1e-8);
}
#[test]
fn coarse_fine_and_equal_content_cache_hits_keep_correct_native_scopes() {
    let native = source(SegTag(7), Placement3::IDENTITY);
    let floor = native
        .as_extrusion()
        .unwrap()
        .floor_tagged(SegTag(7))
        .unwrap();
    let selection = native.select_face(floor).unwrap();
    let mut compiler = PartCompiler::new();
    let coarse = snapshot(&mut compiler, &native, 0.5);
    let fine = snapshot(&mut compiler, &native, 0.005);
    assert_eq!(compiler.counters().native_realizations, 2);
    assert!(
        fine.compiled().part(PartId(0)).unwrap().triangle_count()
            > coarse.compiled().part(PartId(0)).unwrap().triangle_count()
    );
    let body = fine.body(PartId(0), 0).unwrap();
    assert_eq!(body.native().unwrap().resolve_face(&selection), Ok(floor));
    assert_eq!(body.native().unwrap().volume(), native.volume());
    assert_eq!(compiler.native_storage().sources, 1);
    assert_eq!(compiler.native_storage().realizations, 2);
    assert!(body.native_realization().unwrap().report.within_tolerance());
    let fresh = source(SegTag(7), Placement3::IDENTITY);
    let equal = snapshot(&mut compiler, &fresh, 0.005);
    assert_eq!(
        compiler.counters().native_realizations,
        2,
        "equal content must hit realized buffers"
    );
    assert!(Arc::ptr_eq(
        fine.compiled().part(PartId(0)).unwrap(),
        equal.compiled().part(PartId(0)).unwrap()
    ));
    let equal_body = equal.body(PartId(0), 0).unwrap();
    assert!(equal_body.native().unwrap().same_installation(&fresh));
    assert_eq!(
        equal_body.native().unwrap().resolve_face(&selection),
        Err(NativeSelectionError::SourceMismatch)
    );
    let own = equal_body.native().unwrap().select_face(floor).unwrap();
    compiler.clear_cache();
    drop(compiler);
    drop(equal);
    drop(fresh);
    assert_eq!(own.source().resolve_face(&own), Ok(floor));
    assert_eq!(body.native().unwrap().resolve_face(&selection), Ok(floor));
    assert!(
        body.native()
            .unwrap()
            .as_extrusion()
            .unwrap()
            .workplane(floor)
            .is_ok()
    );
}
#[test]
fn repeated_equal_inputs_share_buffers_without_sharing_native_selection_scopes() {
    let a = source(SegTag(7), Placement3::IDENTITY);
    let b = source(SegTag(7), Placement3::IDENTITY);
    let sources = [PartSource::Native(a.clone()), PartSource::Native(b.clone())];
    let input = [
        PartInput::new("a", &sources[0]),
        PartInput::new("b", &sources[1]),
    ];
    let mut compiler = PartCompiler::with_capacity(1);
    let s = compiler
        .compile_snapshot(&input, &CompilePolicy::default())
        .unwrap();
    assert_eq!(compiler.counters().native_realizations, 1);
    assert!(Arc::ptr_eq(
        s.compiled().part(PartId(0)).unwrap(),
        s.compiled().part(PartId(1)).unwrap()
    ));
    let sa = s.body(PartId(0), 0).unwrap();
    let sb = s.body(PartId(1), 0).unwrap();
    assert!(sa.native().unwrap().same_installation(&a));
    assert!(sb.native().unwrap().same_installation(&b));
    let face = sa.native().unwrap().select_face(FaceId(0)).unwrap();
    assert_eq!(
        sb.native().unwrap().resolve_face(&face),
        Err(NativeSelectionError::SourceMismatch)
    );
}
#[test]
fn render_picking_follows_native_surfaces_without_one_draw_region_per_face() {
    let native = source(SegTag(7), Placement3::IDENTITY);
    let s = snapshot(&mut PartCompiler::new(), &native, 0.05);
    let body = s.body(PartId(0), 0).unwrap();
    let compiled = &s.compiled().part(PartId(0)).unwrap().bodies[0];
    assert_eq!(compiled.regions.len(), 1);
    let correspondence = body.native_realization().unwrap();
    assert_eq!(
        correspondence.render_faces().len(),
        compiled.tri.indices.len() / 3
    );
    let original = native.as_extrusion().unwrap();
    for (index, tri) in compiled.tri.indices.as_chunks::<3>().0.iter().enumerate() {
        let selected = body.native_render_face(index).unwrap();
        let face = selected.native_id();
        assert_eq!(native.resolve_face(&selected), Ok(face));
        let p = core::array::from_fn::<_, 3, _>(|i| {
            compiled.tri.positions[tri[i] as usize].map(f64::from)
        });
        let center = core::array::from_fn(|i| (p[0][i] + p[1][i] + p[2][i]) / 3.0);
        let record = original.body().face(face).unwrap();
        let uv = record.surface.parameters(center).unwrap();
        let e = original.body().evaluate_support(face, uv).unwrap();
        assert!(norm(sub(center, e.point)) <= correspondence.report.max_deviation + 1e-9);
        assert!(dot(cross(sub(p[1], p[0]), sub(p[2], p[0])), e.normal) > 0.0);
    }
    assert!(body.native_render_face(usize::MAX).is_none());
    assert!(
        body.native_face(exedra_mesh::FaceId::new(
            u32::MAX,
            core::num::NonZeroU32::new(1).unwrap()
        ))
        .is_none()
    );
}
#[test]
fn native_accuracy_refusals_and_unmet_emission_bounds_are_visible() {
    let native = source(SegTag(7), Placement3::IDENTITY);
    let input = PartSource::Native(native.clone());
    let mut compiler = PartCompiler::new();
    let mut policy = CompilePolicy::default();
    policy.native.tolerance = f64::NAN;
    assert!(
        matches!(compiler.compile_snapshot(&[PartInput::new("bad-policy",&input)],&policy),Err(CompileError::Native {part:PartId(0),part_key,..}) if part_key=="bad-policy")
    );
    assert_eq!(compiler.counters().native_realizations, 0);
    let p = ProfilePolicy::new(1e-9);
    let shape = builders::rect_centered(4.0, 4.0)
        .unwrap()
        .to_profile(&p)
        .unwrap();
    let shifted: NativeSource =
        Extrusion::new(shape, Placement3::translate(1_000_000.03, 0.0, 0.0), 10.0)
            .unwrap()
            .into();
    let s = snapshot(&mut compiler, &shifted, 0.01);
    assert!(
        !s.body(PartId(0), 0)
            .unwrap()
            .native_realization()
            .unwrap()
            .report
            .within_tolerance()
    );
    assert_eq!(shifted.volume(), Ok(160.0));
}
#[test]
fn loft_measure_unavailability_is_typed_and_all_families_install() {
    let policy = ProfilePolicy::new(1e-9);
    let profile = builders::rect_centered(4.0, 4.0)
        .unwrap()
        .to_profile(&policy)
        .unwrap();
    let loft = exedra_analytic::loft::Loft::from_profiles(
        [
            (Placement3::IDENTITY, &profile),
            (Placement3::translate(0.0, 0.0, 5.0), &profile),
        ],
        Placement3::IDENTITY,
        &exedra_analytic::loft::LoftPolicy::new(policy),
    )
    .unwrap();
    let source: NativeSource = loft.into();
    assert_eq!(
        source.volume(),
        Err(NativeMeasureError::Unavailable {
            family: NativeFamily::Loft,
            measure: NativeMeasure::Volume,
        })
    );
    assert_eq!(
        source.surface_area(),
        Err(NativeMeasureError::Unavailable {
            family: NativeFamily::Loft,
            measure: NativeMeasure::SurfaceArea,
        })
    );
    let result = snapshot(&mut PartCompiler::new(), &source, 0.01);
    assert!(
        result
            .body(PartId(0), 0)
            .unwrap()
            .native_realization()
            .unwrap()
            .report
            .within_tolerance()
    );
    let outline = exedra_curves::profile::Loop::new(vec![
        exedra_curves::profile::Segment::line([6.0, -2.0]),
        exedra_curves::profile::Segment::line([6.0, 2.0]),
        exedra_curves::profile::Segment::line([4.0, 2.0]),
        exedra_curves::profile::Segment::line([4.0, -2.0]),
    ])
    .unwrap();
    let radial = exedra_curves::profile::Profile::new(outline, vec![], &policy).unwrap();
    let revolution: NativeSource = exedra_analytic::revolve::Revolution::new(
        radial,
        Placement3::IDENTITY,
        core::f64::consts::TAU,
    )
    .unwrap()
    .into();
    assert_eq!(revolution.family(), NativeFamily::Revolution);
    let spans = [exedra_curves::path::PathSegment3::Line {
        to: [0.0, 0.0, 5.0],
    }];
    let path = exedra_curves::path::CurvePathView {
        start: [0.0; 3],
        segments: &spans,
        section_x: [1.0, 0.0, 0.0],
        section_origin: [0.0; 2],
        closure: exedra_curves::path::PathClosure::Open,
        joins: exedra_curves::path::PathJoin::Smooth,
    };
    let sweep: NativeSource =
        exedra_analytic::sweep::Sweep::new(profile, &path, Placement3::IDENTITY)
            .unwrap()
            .into();
    assert_eq!(sweep.family(), NativeFamily::Sweep);
    for native in [&revolution, &sweep] {
        let s = snapshot(&mut PartCompiler::new(), native, 0.1);
        assert!(
            s.body(PartId(0), 0)
                .unwrap()
                .native_realization()
                .unwrap()
                .report
                .within_tolerance()
        );
    }
}

#[test]
fn available_native_measures_must_be_finite() {
    let policy = ProfilePolicy::new(1e-9);
    let profile = builders::rect_centered(1e100, 1e100)
        .unwrap()
        .to_profile(&policy)
        .unwrap();
    let native = NativeSource::from(Extrusion::new(profile, Placement3::IDENTITY, 1e200).unwrap());
    assert_eq!(
        native.volume(),
        Err(NativeMeasureError::NumericLimit {
            family: NativeFamily::Extrusion,
            measure: NativeMeasure::Volume,
        })
    );
    assert!(native.surface_area().unwrap().is_finite());
    assert!(
        native
            .as_extrusion()
            .unwrap()
            .vertices()
            .iter()
            .flatten()
            .all(|v| v.is_finite())
    );
    assert!(native.as_extrusion().unwrap().is_closed());
}

#[test]
fn positive_native_measures_must_not_silently_underflow() {
    let policy = ProfilePolicy::new(1e-110);
    let profile = builders::rect_centered(1e-100, 1e-100)
        .unwrap()
        .to_profile(&policy)
        .unwrap();
    let native = NativeSource::from(Extrusion::new(profile, Placement3::IDENTITY, 1e-200).unwrap());
    assert_eq!(
        native.volume(),
        Err(NativeMeasureError::NumericLimit {
            family: NativeFamily::Extrusion,
            measure: NativeMeasure::Volume,
        })
    );
    assert!(native.surface_area().unwrap() > 0.0);
}

#[test]
fn native_support_bounds_and_planar_frames_are_independent_of_display_quality() {
    let tag = SegTag(73);
    let frame = Placement3::rotate_y_then_translate(0.35, 30.0, -7.0, 9.0);
    let native = source(tag, frame);
    let bounds = native.conservative_bounds().unwrap();
    let extrusion = native.as_extrusion().unwrap();
    let floor = extrusion.floor_tagged(tag).unwrap();
    assert_eq!(
        native.face_tags(floor).unwrap().collect::<vec::Vec<_>>(),
        vec![tag]
    );
    assert!(matches!(
        native.face_tags(FaceId(u32::MAX)),
        Err(NativeSelectionError::UnknownFace { .. })
    ));
    let plane = native.planar_frame(floor).unwrap();
    assert!(
        norm(sub(
            plane.rows.map(|r| r[3]),
            frame.transform_point([0.0, 0.0, 8.0])
        )) < 1e-10
    );
    let curved = extrusion
        .indexed_faces()
        .find(|(_, f)| matches!(f.surface, exedra_analytic::geometry::Surface::Cylinder(_)))
        .unwrap()
        .0;
    assert_eq!(
        native.planar_frame(curved),
        Err(NativeSelectionError::NotPlanar { face: curved })
    );
    assert_eq!(
        native.planar_frame(FaceId(u32::MAX)),
        Err(NativeSelectionError::UnknownFace {
            face: FaceId(u32::MAX)
        })
    );
    let mut compiler = PartCompiler::new();
    snapshot(&mut compiler, &native, 0.4);
    snapshot(&mut compiler, &native, 0.03);
    assert_eq!(native.conservative_bounds().unwrap(), bounds);
}

#[test]
fn bored_plate_retains_new_scope_provenance_and_native_picking_across_display_policies() {
    use exedra_analytic::bore::{Bore, BorePolicy, FaceRole};
    let validation = ProfilePolicy::new(1e-9);
    let profile = builders::rect_centered(40.0, 30.0)
        .unwrap()
        .to_profile(&validation)
        .unwrap();
    let plate = Extrusion::new(profile, Placement3::IDENTITY, 10.0).unwrap();
    let old = NativeSource::from(plate.clone());
    let old_cap = old.select_face(plate.cap(End::End)).unwrap();
    let policy = BorePolicy {
        separation: 1e-6,
        geometry_gap: 1e-9,
        min_axis_dot: 0.05,
    };
    let bore = Bore::new([0.0; 3], [0.5, 0.25, 1.0], 2.0)
        .unwrap()
        .tagged(SegTag(77));
    let result = plate.with_through_bore(bore.clone(), &policy).unwrap();
    let wall = result.report().wall;
    assert!(
        matches!(result.body().face(plate.cap(End::End)).unwrap().role,FaceRole::Plate{source,trimmed:true,..} if source==plate.cap(End::End))
    );
    let native = NativeSource::from(result);
    assert_eq!(native.family(), NativeFamily::BoredPlate);
    assert_eq!(
        native.resolve_face(&old_cap),
        Err(NativeSelectionError::SourceMismatch)
    );
    let selected = native.select_face(wall).unwrap();
    let mut compiler = PartCompiler::new();
    let coarse = snapshot(&mut compiler, &native, 0.2);
    let fine = snapshot(&mut compiler, &native, 0.01);
    assert_eq!(compiler.counters().native_realizations, 2);
    assert_eq!(
        fine.body(PartId(0), 0)
            .unwrap()
            .native()
            .unwrap()
            .resolve_face(&selected),
        Ok(wall)
    );
    let mut picked_wall = false;
    let body = fine.body(PartId(0), 0).unwrap();
    for i in 0..body.native_realization().unwrap().render_faces().len() {
        let selection = body.native_render_face(i).unwrap();
        assert_eq!(native.resolve_face(&selection), Ok(selection.native_id()));
        picked_wall |= selection.native_id() == wall;
    }
    assert!(picked_wall);
    let fresh = NativeSource::from(plate.with_through_bore(bore, &policy).unwrap());
    assert_eq!(fresh.fingerprint(), native.fingerprint());
    let hit = snapshot(&mut compiler, &fresh, 0.01);
    assert_eq!(compiler.counters().native_realizations, 2);
    assert!(Arc::ptr_eq(
        fine.compiled().part(PartId(0)).unwrap(),
        hit.compiled().part(PartId(0)).unwrap()
    ));
    assert_eq!(
        hit.body(PartId(0), 0)
            .unwrap()
            .native()
            .unwrap()
            .resolve_face(&selected),
        Err(NativeSelectionError::SourceMismatch)
    );
    assert_eq!(
        coarse
            .body(PartId(0), 0)
            .unwrap()
            .native()
            .unwrap()
            .volume(),
        native.volume()
    );
}

#[test]
fn generic_body_installation_retains_tags_geometry_and_independent_selection_scope() {
    use exedra_analytic::body::{Body, BodyParts, Edge, Face, ValidationPolicy};
    let original = source(SegTag(7), Placement3::IDENTITY);
    let parts = original.as_extrusion().unwrap().body().clone().into_parts();
    let tagged = Body::try_from_parts(
        BodyParts {
            trims: parts.trims,
            vertices: parts.vertices,
            edges: parts
                .edges
                .into_iter()
                .map(|edge| Edge {
                    geometry: edge.geometry,
                    start: edge.start,
                    end: edge.end,
                    role: (),
                })
                .collect(),
            loops: parts.loops,
            faces: parts
                .faces
                .into_iter()
                .map(|face| Face {
                    surface: face.surface,
                    domain: face.domain,
                    sense: face.sense,
                    loops: face.loops,
                    role: vec![SegTag(7), SegTag(9)],
                })
                .collect(),
        },
        &ValidationPolicy { max_gap: 1e-9 },
    )
    .unwrap();
    let native =
        NativeSource::from_body(tagged.clone(), Placement3::translate(2.0, 3.0, 4.0)).unwrap();
    let equal = NativeSource::from_body(tagged, Placement3::translate(2.0, 3.0, 4.0)).unwrap();
    assert_eq!(native.family(), NativeFamily::Body);
    assert_eq!(native.chart_kind(), None);
    assert_eq!(native.cap(End::Start), None);
    assert_eq!(
        native
            .face_tags(FaceId(0))
            .unwrap()
            .collect::<vec::Vec<_>>(),
        vec![SegTag(7), SegTag(9)]
    );
    assert_eq!(native.fingerprint(), equal.fingerprint());
    assert!(!native.same_installation(&equal));
    assert!(matches!(
        native.volume(),
        Err(NativeMeasureError::Unavailable {
            family: NativeFamily::Body,
            ..
        })
    ));
    let mut compiler = PartCompiler::new();
    let first = snapshot(&mut compiler, &native, 0.05);
    let selection = native.select_face(FaceId(0)).unwrap();
    let repeated = snapshot(&mut compiler, &equal, 0.05);
    assert_eq!(compiler.counters().native_realizations, 1);
    assert_eq!(
        first
            .body(PartId(0), 0)
            .unwrap()
            .native()
            .unwrap()
            .resolve_face(&selection),
        Ok(FaceId(0))
    );
    assert_eq!(
        repeated
            .body(PartId(0), 0)
            .unwrap()
            .native()
            .unwrap()
            .resolve_face(&selection),
        Err(NativeSelectionError::SourceMismatch)
    );
    assert!(native.planar_frame(FaceId(0)).is_ok());
}

#[test]
fn generic_body_tags_are_distinct_in_authored_order_and_content_identity() {
    use alloc::vec::Vec;
    let original = source(SegTag(7), Placement3::IDENTITY).to_body();
    let tagged = |tags: Vec<SegTag>| original.clone().map_roles(|_| tags.clone(), |_| ());
    let native = NativeSource::from_body(
        tagged(vec![SegTag(9), SegTag(7), SegTag(9), SegTag(7)]),
        Placement3::IDENTITY,
    )
    .unwrap();
    let distinct =
        NativeSource::from_body(tagged(vec![SegTag(9), SegTag(7)]), Placement3::IDENTITY).unwrap();
    assert_eq!(
        native.face_tags(FaceId(0)).unwrap().collect::<Vec<_>>(),
        vec![SegTag(9), SegTag(7)]
    );
    assert_eq!(native.to_body(), distinct.to_body());
    assert_eq!(native.fingerprint(), distinct.fingerprint());
}

#[test]
fn refined_generic_spherical_faces_keep_render_normals_correspondence_and_cache_scopes() {
    use exedra_analytic::body::{
        Body, BodyParts, BoundaryLoop, Coedge, Edge, EdgeId, Face, LoopId, LoopTrims, Sense,
        TrimCurve, ValidationPolicy, VertexId,
    };
    use exedra_analytic::geometry::{Curve, ParameterDomain, ParameterRange, Surface};
    let surface = Surface::sphere(Placement3::IDENTITY, 2.0).unwrap();
    let mut parts = BodyParts::default();
    for (patch, [umin, umax]) in [[0.2_f64, 1.0], [1.5, 2.3]].into_iter().enumerate() {
        let u = ParameterRange::new(umin, umax).unwrap();
        let v = ParameterRange::new(-0.3, 0.4).unwrap();
        let corners = [[umin, -0.3], [umax, -0.3], [umax, 0.4], [umin, 0.4]];
        parts
            .vertices
            .extend(corners.map(|uv| surface.evaluate(uv).unwrap().point));
        let latitude = |lat: f64| {
            Curve::circle(
                Placement3::translate(0.0, 0.0, 2.0 * lat.sin()),
                2.0 * lat.cos(),
            )
            .unwrap()
        };
        let meridian = |angle: f64| {
            Curve::circle(
                Placement3::from_axes(
                    [angle.cos(), angle.sin(), 0.0],
                    [0.0, 0.0, 1.0],
                    [angle.sin(), -angle.cos(), 0.0],
                    [0.0; 3],
                ),
                2.0,
            )
            .unwrap()
        };
        let base = u32::try_from(patch * 4).unwrap();
        for (curve, range, start, end) in [
            (latitude(-0.3), u, 0, 1),
            (meridian(umax), v, 1, 2),
            (latitude(0.4), u, 3, 2),
            (meridian(umin), v, 0, 3),
        ] {
            parts.edges.push(Edge::new(
                curve,
                range,
                VertexId(base + start),
                VertexId(base + end),
                (),
            ));
        }
        let loop_id = LoopId(u32::try_from(patch).unwrap());
        parts.loops.push(BoundaryLoop {
            coedges: (0..4)
                .map(|i| Coedge {
                    edge: EdgeId(base + i),
                    reversed: i >= 2,
                })
                .collect(),
        });
        parts.faces.push(Face {
            surface: surface.clone(),
            domain: ParameterDomain { u, v },
            sense: Sense::Forward,
            loops: vec![loop_id],
            role: vec![SegTag(17 + loop_id.0)],
        });
        parts.trims.push(LoopTrims {
            loop_id,
            curves: [(0, 1), (1, 2), (3, 2), (0, 3)]
                .map(|(a, b)| TrimCurve::Line {
                    start: corners[a],
                    end: corners[b],
                })
                .into(),
        });
    }
    let body = Body::try_from_parts(parts, &ValidationPolicy { max_gap: 1e-9 }).unwrap();
    let first = NativeSource::from_body(body.clone(), Placement3::IDENTITY).unwrap();
    let equal = NativeSource::from_body(body, Placement3::IDENTITY).unwrap();
    let sources = [
        PartSource::Native(first.clone()),
        PartSource::Native(equal.clone()),
    ];
    let inputs = [
        PartInput::new("first", &sources[0]),
        PartInput::new("equal", &sources[1]),
    ];
    let mut policy = CompilePolicy {
        normals: NormalsSource::CustomOrDerived,
        ..Default::default()
    };
    let mut compiler = PartCompiler::new();
    for (attempt, tolerance) in [0.025, 0.005].into_iter().enumerate() {
        policy.native.tolerance = tolerance;
        let snapshot = compiler.compile_snapshot(&inputs, &policy).unwrap();
        assert_eq!(
            compiler.counters().native_realizations,
            u64::try_from(attempt + 1).unwrap()
        );
        assert!(Arc::ptr_eq(
            snapshot.compiled().part(PartId(0)).unwrap(),
            snapshot.compiled().part(PartId(1)).unwrap()
        ));
        let installed = snapshot.body(PartId(0), 0).unwrap();
        let native = installed.native_realization().unwrap();
        assert!(native.report.work.trim_generated_vertices > 0);
        assert!(native.report.work.trim_refinement_batches > 0);
        assert!(native.report.within_tolerance());
        let tri = &snapshot.compiled().part(PartId(0)).unwrap().bodies[0].tri;
        assert_eq!(tri.positions.len(), tri.normals.len());
        assert_eq!(native.report.work.trim_normal_corners, tri.indices.len());
        assert!(native.report.work.trim_normal_scratch_bytes > 0);
        let mut max_normal_error = 0.0_f64;
        for (p, n) in tri.positions.iter().zip(&tri.normals) {
            let p = p.map(f64::from);
            let n = n.map(f64::from);
            assert!((norm(n) - 1.0).abs() < 1e-6);
            let radial = p.map(|v| v / norm(p));
            max_normal_error = max_normal_error.max(norm(sub(n, radial)));
        }
        assert!(
            max_normal_error < 2e-6,
            "max normal error: {max_normal_error}"
        );
        assert_eq!(native.render_faces().len(), tri.indices.len() / 3);
        let mut seen = [false; 2];
        for (ordinal, indices) in tri.indices.as_chunks::<3>().0.iter().enumerate() {
            let center_x = indices
                .iter()
                .map(|&i| f64::from(tri.positions[i as usize][0]) / 3.0)
                .sum::<f64>();
            let expected = usize::from(center_x < 0.7);
            assert_eq!(
                native.render_face(ordinal),
                Some(FaceId(u32::try_from(expected).unwrap()))
            );
            seen[expected] = true;
        }
        assert_eq!(seen, [true; 2]);
        let selection = first.select_face(FaceId(0)).unwrap();
        assert_eq!(
            installed.native().unwrap().resolve_face(&selection),
            Ok(FaceId(0))
        );
        assert!(
            snapshot
                .body(PartId(1), 0)
                .unwrap()
                .native()
                .unwrap()
                .same_installation(&equal)
        );
        assert_eq!(
            snapshot
                .body(PartId(1), 0)
                .unwrap()
                .native()
                .unwrap()
                .resolve_face(&selection),
            Err(NativeSelectionError::SourceMismatch)
        );
    }
}

#[test]
fn explicit_periodic_chart_trims_participate_in_identity_and_survive_role_mapping() {
    use exedra_analytic::body::{
        Body, BodyParts, BoundaryLoop, Coedge, Edge, EdgeId, Face, LoopId, LoopTrims, Sense,
        TrimCurve, ValidationPolicy, VertexId,
    };
    use exedra_analytic::geometry::{Curve, ParameterDomain, ParameterRange, Surface};
    let tau = core::f64::consts::TAU;
    let range = |a, b| ParameterRange::new(a, b).unwrap();
    let parts = BodyParts {
        vertices: vec![[2.0, 0.0, 0.0], [2.0, 0.0, 3.0]],
        edges: vec![
            Edge::new(
                Curve::circle(Placement3::IDENTITY, 2.0).unwrap(),
                range(0.0, tau),
                VertexId(0),
                VertexId(0),
                (),
            ),
            Edge::new(
                Curve::circle(Placement3::translate(0.0, 0.0, 3.0), 2.0).unwrap(),
                range(0.0, tau),
                VertexId(1),
                VertexId(1),
                (),
            ),
            Edge::new(
                Curve::line([2.0, 0.0, 0.0], [0.0, 0.0, 1.0]).unwrap(),
                range(0.0, 3.0),
                VertexId(0),
                VertexId(1),
                (),
            ),
        ],
        loops: vec![
            BoundaryLoop {
                coedges: vec![
                    Coedge {
                        edge: EdgeId(0),
                        reversed: false,
                    },
                    Coedge {
                        edge: EdgeId(2),
                        reversed: false,
                    },
                    Coedge {
                        edge: EdgeId(1),
                        reversed: true,
                    },
                    Coedge {
                        edge: EdgeId(2),
                        reversed: true,
                    },
                ],
            },
            BoundaryLoop {
                coedges: vec![Coedge {
                    edge: EdgeId(0),
                    reversed: true,
                }],
            },
            BoundaryLoop {
                coedges: vec![Coedge {
                    edge: EdgeId(1),
                    reversed: false,
                }],
            },
        ],
        faces: vec![
            Face {
                surface: Surface::cylinder(Placement3::IDENTITY, 2.0).unwrap(),
                domain: ParameterDomain {
                    u: range(0.0, 2.0 * tau),
                    v: range(0.0, 3.0),
                },
                sense: Sense::Forward,
                loops: vec![LoopId(0)],
                role: vec![SegTag(7)],
            },
            Face {
                surface: Surface::plane(Placement3::IDENTITY).unwrap(),
                domain: ParameterDomain {
                    u: range(-2.0, 2.0),
                    v: range(-2.0, 2.0),
                },
                sense: Sense::Reverse,
                loops: vec![LoopId(1)],
                role: vec![],
            },
            Face {
                surface: Surface::plane(Placement3::translate(0.0, 0.0, 3.0)).unwrap(),
                domain: ParameterDomain {
                    u: range(-2.0, 2.0),
                    v: range(-2.0, 2.0),
                },
                sense: Sense::Forward,
                loops: vec![LoopId(2)],
                role: vec![],
            },
        ],
        trims: vec![LoopTrims {
            loop_id: LoopId(0),
            curves: vec![
                TrimCurve::Line {
                    start: [0.0, 0.0],
                    end: [tau, 0.0],
                },
                TrimCurve::Line {
                    start: [tau, 0.0],
                    end: [tau, 3.0],
                },
                TrimCurve::Line {
                    start: [0.0, 3.0],
                    end: [tau, 3.0],
                },
                TrimCurve::Line {
                    start: [0.0, 0.0],
                    end: [0.0, 3.0],
                },
            ],
        }],
    };
    let first = Body::try_from_parts(parts.clone(), &ValidationPolicy { max_gap: 1e-10 }).unwrap();
    let mut shifted = parts;
    for curve in &mut shifted.trims[0].curves {
        let TrimCurve::Line { start, end } = curve else {
            unreachable!()
        };
        start[0] += tau;
        end[0] += tau;
    }
    let shifted = Body::try_from_parts(shifted, &ValidationPolicy { max_gap: 1e-10 }).unwrap();
    assert_eq!(first.vertices(), shifted.vertices());
    assert_eq!(first.edges(), shifted.edges());
    assert_eq!(first.faces(), shifted.faces());
    assert_eq!(
        first.clone().map_roles(|tags| tags, |()| ()).trims(),
        first.trims()
    );
    let a = NativeSource::from_body(first, Placement3::IDENTITY).unwrap();
    let b = NativeSource::from_body(shifted, Placement3::IDENTITY).unwrap();
    assert_ne!(
        a.fingerprint(),
        b.fingerprint(),
        "different valid pcurve charts must not alias realized cache identity"
    );
    assert_eq!(a.to_body().trims(), a.geometry().to_body().trims());
    let mut compiler = PartCompiler::new();
    snapshot(&mut compiler, &a, 0.05);
    snapshot(&mut compiler, &b, 0.05);
    assert_eq!(compiler.counters().native_realizations, 2);
}

#[test]
fn estimated_extrusion_measures_are_unavailable() {
    use exedra_curves::profile::{Bezier, Loop, Profile, Segment};
    let arch = |weights| {
        let outline = Loop::new(vec![
            Segment::line([4.0, 0.0]),
            Segment::bezier(
                [0.0, 0.0],
                Bezier::new(vec![[3.0, 2.0], [1.0, 2.0]], weights),
            ),
        ])
        .unwrap();
        let profile = Profile::new(outline, vec![], &ProfilePolicy::new(1e-9)).unwrap();
        NativeSource::from(Extrusion::new(profile, Placement3::IDENTITY, 2.0).unwrap())
    };
    let unavailable = |measure| NativeMeasureError::Unavailable {
        family: NativeFamily::Extrusion,
        measure,
    };
    // A polynomial crown's area is exact, 18 (1/20 + 4/30 + 1/20) under
    // y = 6t(1 - t), and its length an estimate.
    let polynomial = arch(None);
    assert!((polynomial.volume().unwrap() - 2.0 * 4.2).abs() < 1e-12);
    assert_eq!(
        polynomial.surface_area(),
        Err(unavailable(NativeMeasure::SurfaceArea))
    );
    // A rational one's area is an estimate too.
    let rational = arch(Some(vec![1.0, 2.0, 2.0, 1.0]));
    assert_eq!(rational.volume(), Err(unavailable(NativeMeasure::Volume)));
}
