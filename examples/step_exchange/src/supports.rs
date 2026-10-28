// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exact supporting surfaces exercised through native authored revolutions.

use exedra_analytic::revolve::Revolution;
use exedra_math::Placement3;
use exedra_profile::{Loop, Profile, ProfilePolicy, Segment};
use exedra_step::{LengthUnit, StepOptions, export_body};
use std::{error::Error, path::Path};

fn disk_at(major: f64, radius: f64, seam: f64) -> Loop {
    let offset = [radius * seam.cos(), radius * seam.sin()];
    Loop::new(vec![
        Segment::arc([major - offset[0], -offset[1]], 1.0),
        Segment::arc([major + offset[0], offset[1]], 1.0),
    ])
    .unwrap()
}
fn torus(sweep: f64, hole: bool, seam: f64) -> Revolution {
    Revolution::new(
        Profile::new(
            disk_at(10.0, 2.0, seam),
            if hole {
                vec![disk_at(10.0, 1.0, seam).reversed()]
            } else {
                vec![]
            },
            &ProfilePolicy::new(1e-9),
        )
        .unwrap(),
        Placement3::IDENTITY,
        sweep,
    )
    .unwrap()
}
fn torus_radii(major: f64, minor: f64) -> Revolution {
    Revolution::new(
        Profile::new(
            disk_at(major, minor, 0.0),
            vec![],
            &ProfilePolicy::new(1e-9),
        )
        .unwrap(),
        Placement3::IDENTITY,
        std::f64::consts::TAU,
    )
    .unwrap()
}
fn frustum(sweep: f64, decreasing: bool) -> Revolution {
    frustum_height(sweep, decreasing, 5.0)
}
fn frustum_height(sweep: f64, decreasing: bool, height: f64) -> Revolution {
    let (bottom, top) = if decreasing { (12.0, 8.0) } else { (8.0, 12.0) };
    frustum_radii(sweep, bottom, top, height)
}
fn frustum_radii(sweep: f64, bottom: f64, top: f64, height: f64) -> Revolution {
    Revolution::new(
        Profile::new(
            Loop::new(vec![
                Segment::line([bottom, 0.0]),
                Segment::line([top, height]),
                Segment::line([6.0, height]),
                Segment::line([5.0, 0.0]),
            ])
            .unwrap(),
            vec![],
            &ProfilePolicy::new(1e-9),
        )
        .unwrap(),
        Placement3::IDENTITY,
        sweep,
    )
    .unwrap()
}

pub(super) fn export_cases(
    output: &Path,
    expected: &mut Vec<serde_json::Value>,
) -> Result<(), Box<dyn Error>> {
    let full = std::f64::consts::TAU;
    let partial = 1.8_f64;
    let placement = Placement3::from_axes(
        [0.0, 0.0, -1.0],
        [0.0, 1.0, 0.0],
        [1.0, 0.0, 0.0],
        [100.0, -30.0, 7.0],
    );
    let cases = [
        (
            "cone-full",
            frustum(full, false),
            Placement3::IDENTITY,
            LengthUnit::Millimetre,
            [[-12.0, 0.0, -12.0], [12.0, 5.0, 12.0]],
        ),
        (
            "cone-negative",
            frustum(full, true),
            placement,
            LengthUnit::Metre,
            [[88.0, -30.0, -5.0], [112.0, -25.0, 19.0]],
        ),
        (
            "cone-partial",
            frustum(partial, false),
            Placement3::IDENTITY,
            LengthUnit::Millimetre,
            [[12.0 * partial.cos(), 0.0, -12.0], [12.0, 5.0, 0.0]],
        ),
        (
            "cone-near-cylinder",
            frustum_radii(full, 8.0, 8.0 + 5e-9, 5.0),
            Placement3::IDENTITY,
            LengthUnit::Millimetre,
            [
                [-8.000000005, 0.0, -8.000000005],
                [8.000000005, 5.0, 8.000000005],
            ],
        ),
        (
            "cone-near-frame",
            frustum(full, false),
            Placement3::from_axes(
                [1.0 + 1e-14, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
                [0.0; 3],
            ),
            LengthUnit::Millimetre,
            [[-12.0, 0.0, -12.0], [12.0, 5.0, 12.0]],
        ),
        (
            "cone-negative-partial",
            frustum(partial, true),
            Placement3::IDENTITY,
            LengthUnit::Millimetre,
            [[12.0 * partial.cos(), 0.0, -12.0], [12.0, 5.0, 0.0]],
        ),
        (
            "torus-near-horn",
            torus_radii(2.0001, 2.0),
            Placement3::IDENTITY,
            LengthUnit::Millimetre,
            [[-4.0001, -2.0, -4.0001], [4.0001, 2.0, 4.0001]],
        ),
        (
            "torus-small",
            torus_radii(1e-3, 2e-4),
            Placement3::IDENTITY,
            LengthUnit::Millimetre,
            [[-1.2e-3, -2e-4, -1.2e-3], [1.2e-3, 2e-4, 1.2e-3]],
        ),
        (
            "torus-full",
            torus(full, false, 0.0),
            Placement3::IDENTITY,
            LengthUnit::Millimetre,
            [[-12.0, -2.0, -12.0], [12.0, 2.0, 12.0]],
        ),
        (
            "torus-partial-hole",
            torus(partial, true, 0.0),
            placement,
            LengthUnit::Metre,
            [
                [88.0, -32.0, -5.0],
                [100.0, -28.0, 7.0 - 12.0 * partial.cos()],
            ],
        ),
        (
            "torus-near-seam",
            torus(full - 1e-6, false, 1e-8),
            Placement3::IDENTITY,
            LengthUnit::Millimetre,
            [[-12.0, -2.0, -12.0], [12.0, 2.0, 12.0]],
        ),
        (
            "torus-near-frame",
            torus(full, false, 0.0),
            Placement3::from_axes(
                [1.0 + 1e-14, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
                [0.0; 3],
            ),
            LengthUnit::Millimetre,
            [[-12.0, -2.0, -12.0], [12.0, 2.0, 12.0]],
        ),
    ];
    for (name, solid, placement, units, bounds) in cases {
        let doc = export_body(
            solid.body(),
            placement,
            &StepOptions {
                name,
                units,
                ..StepOptions::default()
            },
        )?;
        super::save(
            output,
            expected,
            name,
            &doc,
            solid.volume(),
            solid.surface_area(),
            bounds,
        )?;
        let mut counts = serde_json::Map::new();
        for face in solid.body().faces() {
            let name = match &face.surface {
                exedra_analytic::geometry::Surface::Plane(_) => "plane",
                exedra_analytic::geometry::Surface::Cylinder(_) => "cylinder",
                exedra_analytic::geometry::Surface::Cone(_) => "cone",
                exedra_analytic::geometry::Surface::Torus(_) => "torus",
                exedra_analytic::geometry::Surface::Ruled(_) => "ruled",
                exedra_analytic::geometry::Surface::Sphere(_) => "sphere",
            };
            let entry = counts.entry(name).or_insert(serde_json::json!(0));
            *entry = serde_json::json!(entry.as_u64().unwrap() + 1);
        }
        expected.last_mut().unwrap()["surface_counts"] = serde_json::Value::Object(counts);
        let witness = if name.starts_with("cone") {
            [7.0, 2.5, 0.0]
        } else if name == "torus-near-horn" {
            [3.5001, 0.0, 0.0]
        } else if name == "torus-small" {
            [1.15e-3, 0.0, 0.0]
        } else {
            [11.5, 0.0, 0.0]
        };
        // For partial turns choose an interior angular position rather than a cap.
        let angle = if solid.sweep() < full {
            solid.sweep() * 0.5
        } else {
            0.4
        };
        let point = [
            witness[0] * angle.cos(),
            witness[1],
            -witness[0] * angle.sin(),
        ];
        expected.last_mut().unwrap()["witnesses"] = serde_json::json!([
            {"point":placement.transform_point(point),"state":"inside"},
            {"point":placement.transform_point([0.0,0.0,0.0]),"state":"outside"}
        ]);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use exedra_analytic::{
        body::{Body, ValidationPolicy},
        geometry::ParameterRange,
    };
    use exedra_step::{StepError, check_geometry_support};

    #[test]
    fn native_support_capabilities_and_separate_conversion_allowance() {
        let cone = frustum(std::f64::consts::TAU, false);
        let ring = torus(std::f64::consts::TAU, false, 0.0);
        for body in [cone.body(), ring.body()] {
            assert_eq!(check_geometry_support(body), Ok(()));
            let doc = export_body(body, Placement3::IDENTITY, &StepOptions::default()).unwrap();
            assert_eq!(
                doc.report.max_deviation,
                doc.report.max_native_support_gap
                    + doc.report.frame_roundoff_bound
                    + doc.report.parameter_roundoff_bound
            );
        }
        let cone = export_body(cone.body(), Placement3::IDENTITY, &StepOptions::default()).unwrap();
        assert!(cone.text.contains("CONICAL_SURFACE"));
        assert!(cone.report.parameter_roundoff_bound > 0.0);
        let ring = export_body(ring.body(), Placement3::IDENTITY, &StepOptions::default()).unwrap();
        assert!(ring.text.contains("TOROIDAL_SURFACE"));
        assert_eq!(ring.report.parameter_roundoff_bound, 0.0);
    }
    #[test]
    fn periodic_surface_domains_refuse_multiple_turns() {
        for parameter in ["u", "v"] {
            let mut parts = torus(std::f64::consts::TAU, false, 0.0)
                .body()
                .clone()
                .into_parts();
            let range = ParameterRange::new(0.0, 2.0 * std::f64::consts::TAU).unwrap();
            if parameter == "u" {
                parts.faces[0].domain.u = range;
            } else {
                parts.faces[0].domain.v = range;
            }
            let body = Body::try_from_parts(parts, &ValidationPolicy { max_gap: 1e-9 }).unwrap();
            assert!(
                matches!(export_body(&body,Placement3::IDENTITY,&StepOptions::default()),Err(StepError::PeriodicSurfaceDomain{parameter:p,..}) if p==parameter)
            );
        }
    }

    #[test]
    fn zero_slope_uses_exact_cylinder_and_steep_cone_obeys_conversion_budget() {
        let mut parts = super::super::periodic::cylinder().into_parts();
        parts.faces[0].surface =
            exedra_analytic::geometry::Surface::cone(Placement3::IDENTITY, 2.0, 0.0).unwrap();
        let body = Body::try_from_parts(parts, &ValidationPolicy { max_gap: 1e-9 }).unwrap();
        let doc = export_body(&body, Placement3::IDENTITY, &StepOptions::default()).unwrap();
        assert!(doc.text.contains("CYLINDRICAL_SURFACE"));
        assert!(!doc.text.contains("CONICAL_SURFACE"));
        assert_eq!(doc.report.parameter_roundoff_bound, 0.0);
        let steep = frustum_height(std::f64::consts::TAU, false, 1e-8);
        assert_eq!(check_geometry_support(steep.body()), Ok(()));
        assert!(matches!(
            export_body(steep.body(), Placement3::IDENTITY, &StepOptions::default()),
            Err(StepError::Accuracy { .. })
        ));
    }
    #[test]
    fn positive_and_negative_tiny_slopes_refuse_the_observed_reader_clamp() {
        for slope in [1e-13, -1e-13, 1e-14, -1e-14] {
            let radius = 0.01;
            let height = 1e5;
            let profile = Profile::new(
                Loop::new(vec![
                    Segment::line([radius, 0.0]),
                    Segment::line([radius + slope * height, height]),
                    Segment::line([0.005, height]),
                    Segment::line([0.005, 0.0]),
                ])
                .unwrap(),
                vec![],
                &ProfilePolicy::new(1e-10),
            )
            .unwrap();
            let native =
                Revolution::new(profile, Placement3::IDENTITY, std::f64::consts::TAU).unwrap();
            assert!(matches!(
                check_geometry_support(native.body()),
                Err(StepError::ConeAngle { .. })
            ));
            assert!(matches!(
                export_body(
                    native.body(),
                    Placement3::IDENTITY,
                    &StepOptions {
                        tolerance: 0.01,
                        ..StepOptions::default()
                    }
                ),
                Err(StepError::ConeAngle { .. })
            ));
        }
    }
    #[test]
    fn understated_face_domains_do_not_understate_cone_conversion_error() {
        let native = frustum(std::f64::consts::TAU, false);
        let baseline =
            export_body(native.body(), Placement3::IDENTITY, &StepOptions::default()).unwrap();
        let mut parts = native.body().clone().into_parts();
        for face in &mut parts.faces {
            if matches!(face.surface, exedra_analytic::geometry::Surface::Cone(_)) {
                face.domain.v = ParameterRange::new(0.0, 1e-10).unwrap();
            }
        }
        // Native structural validation does not certify domain/trim containment.
        let body = Body::try_from_parts(parts, &ValidationPolicy { max_gap: 1e-9 }).unwrap();
        let exported = export_body(&body, Placement3::IDENTITY, &StepOptions::default()).unwrap();
        assert!(
            exported.report.parameter_roundoff_bound >= baseline.report.parameter_roundoff_bound
        );
    }
}
