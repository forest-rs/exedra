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
use exedra_math::{Placement3, cross, dot, norm, sub};
use exedra_profile::{ProfilePolicy, SegTag};

fn source(tag: SegTag, frame: Placement3) -> NativeSource {
    let policy = ProfilePolicy::new(1e-9);
    let profile = builders::rect_centered(40.0, 30.0)
        .unwrap()
        .to_line_arc(&policy)
        .unwrap();
    let bore = builders::circle(3.0).unwrap().to_line_arc(&policy).unwrap();
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
        .to_line_arc(&p)
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
        .to_line_arc(&policy)
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
    let outline = exedra_profile::Loop::new(vec![
        exedra_profile::Segment::line([6.0, -2.0]),
        exedra_profile::Segment::line([6.0, 2.0]),
        exedra_profile::Segment::line([4.0, 2.0]),
        exedra_profile::Segment::line([4.0, -2.0]),
    ])
    .unwrap();
    let radial = exedra_profile::Profile::new(outline, vec![], &policy).unwrap();
    let revolution: NativeSource = exedra_analytic::revolve::Revolution::new(
        radial,
        Placement3::IDENTITY,
        core::f64::consts::TAU,
    )
    .unwrap()
    .into();
    assert_eq!(revolution.family(), NativeFamily::Revolution);
    let spans = [exedra_math::path::PathSegment3::Line {
        to: [0.0, 0.0, 5.0],
    }];
    let path = exedra_math::path::CurvePathView {
        start: [0.0; 3],
        segments: &spans,
        section_x: [1.0, 0.0, 0.0],
        section_origin: [0.0; 2],
        closure: exedra_math::path::PathClosure::Open,
        joins: exedra_math::path::PathJoin::Smooth,
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
        .to_line_arc(&policy)
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
        .to_line_arc(&policy)
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
