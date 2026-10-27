// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exact prepared sections shared by native and mesh loft measurements.

use exedra_analytic::{
    loft::{Loft, LoftPolicy},
    realize::{RealizationPolicy, RealizedBody},
};
use exedra_constructive::{
    builders,
    ir::{CapMode, LoftPolicy as Interpolation, Placement3},
    loft::{BoundaryPoint, CorrespondencePolicy, CorrespondenceProfile, correspond_profiles},
    profile::Profile2,
    tessellate::{EvalPolicy, TessellatedBody, tessellate_loft},
};
use exedra_profile::{LoopRef, ProfilePolicy};

pub(crate) fn sections(hollow: bool) -> Vec<(Placement3, Profile2)> {
    let a = if hollow {
        builders::ring(35.0, 25.0).unwrap()
    } else {
        builders::circle(35.0).unwrap()
    };
    let b = builders::rect_centered(80.0, 56.0).unwrap();
    let b = if hollow {
        b.with_cutouts_at(&builders::rect_centered(64.0, 40.0).unwrap(), [(0.0, 0.0)])
            .unwrap()
    } else {
        b
    };
    let circle = [
        BoundaryPoint::new(0, 0.25),
        BoundaryPoint::new(0, 0.75),
        BoundaryPoint::new(1, 0.25),
        BoundaryPoint::new(1, 0.75),
    ];
    let rectangle = [
        BoundaryPoint::new(0, 0.0),
        BoundaryPoint::new(1, 0.0),
        BoundaryPoint::new(2, 0.0),
        BoundaryPoint::new(3, 0.0),
    ];
    let mut first = CorrespondenceProfile::new(&a).with_landmarks(LoopRef::Outer, circle);
    let mut last = CorrespondenceProfile::new(&b).with_landmarks(LoopRef::Outer, rectangle);
    if hollow {
        first = first.with_landmarks(
            LoopRef::Hole(0),
            [
                BoundaryPoint::new(1, 0.75),
                BoundaryPoint::new(0, 0.25),
                BoundaryPoint::new(0, 0.75),
                BoundaryPoint::new(1, 0.25),
            ],
        );
        last = last.with_landmarks(LoopRef::Hole(0), rectangle);
    }
    correspond_profiles(
        &[first, last],
        &CorrespondencePolicy::new(ProfilePolicy::new(1e-7)),
    )
    .unwrap()
    .profiles
    .into_iter()
    .enumerate()
    .map(|(i, p)| (Placement3::translate(0.0, 0.0, 100.0 * i as f64), p.profile))
    .collect()
}
pub(crate) fn native(sections: &[(Placement3, Profile2)]) -> Loft {
    Loft::from_profiles(
        sections.iter().map(|(f, p)| (*f, p)),
        Placement3::IDENTITY,
        &LoftPolicy::new(ProfilePolicy::new(1e-7)),
    )
    .unwrap()
}
pub(crate) fn native_mesh(loft: &Loft) -> RealizedBody {
    loft.realize(&RealizationPolicy {
        tolerance: 0.05,
        ..RealizationPolicy::default()
    })
    .unwrap()
}
pub(crate) fn charted_mesh(loft: &Loft) -> RealizedBody {
    let chart = exedra_constructive::chart::SurfaceChart::Loft {
        reference_section: 0,
        rest_length: 100.0,
        stations: exedra_constructive::chart::LoftStations::SectionIndex,
        wall: exedra_constructive::chart::ChartTransform {
            matrix: [[0.05, 0.0], [0.0, 0.05]],
            offset: [0.0; 2],
        },
        caps: exedra_constructive::chart::ChartTransform {
            matrix: [[0.05, 0.0], [0.0, 0.05]],
            offset: [0.0; 2],
        },
    };
    loft.realize_with_uvs(
        &RealizationPolicy {
            tolerance: 0.05,
            ..RealizationPolicy::default()
        },
        0,
        &[0.0, 100.0],
        |_, _| 0,
        |role, p| match role {
            exedra_analytic::loft::FaceRole::Wall { .. } => chart.map_loft_wall_triangle(p),
            exedra_analytic::loft::FaceRole::Cap { .. } => chart.map_cap_triangle(p),
        },
    )
    .unwrap()
}
pub(crate) fn mesh_loft(sections: &[(Placement3, Profile2)]) -> TessellatedBody {
    let refs: Vec<_> = sections.iter().map(|(f, p)| (*f, p)).collect();
    let mut policy = EvalPolicy::default();
    policy.discretize.chord_tolerance = 0.025;
    policy.loft.chord_tolerance = 0.025;
    tessellate_loft(&refs, Interpolation::Ruled, CapMode::Both, &policy).unwrap()
}
