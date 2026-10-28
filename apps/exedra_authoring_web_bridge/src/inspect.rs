// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Small wire metadata adapted from native queries, independent of display meshes.
use crate::Error;
use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
    vec::Vec,
};
use exedra_analytic::{body::Body, geometry::Surface};
use exedra_compile::{NativeFamily, NativeGeometryRef, NativeMeasureError, NativeSource};
use serde_json::{Value, json};

pub(crate) fn family(value: NativeFamily) -> &'static str {
    match value {
        NativeFamily::Extrusion => "extrusion",
        NativeFamily::Revolution => "revolution",
        NativeFamily::Sweep => "sweep",
        NativeFamily::Loft => "loft",
        NativeFamily::BoredPlate => "boredPlate",
        NativeFamily::Body => "body",
    }
}
fn chart_kind(kind: exedra_constructive::chart::SurfaceChartKind) -> &'static str {
    use exedra_constructive::chart::SurfaceChartKind;
    match kind {
        SurfaceChartKind::Extrude => "extrude",
        SurfaceChartKind::Revolve => "revolve",
        SurfaceChartKind::Sweep => "sweep",
        SurfaceChartKind::Loft => "loft",
    }
}
fn measure(value: Result<f64, NativeMeasureError>, measure: &'static str) -> Value {
    match value {
        Ok(value) => json!({"status":"available","value":value}),
        Err(error) => {
            let (status, code, family_name) = match error {
                NativeMeasureError::Unavailable { family, .. } => {
                    ("unavailable", "measurement.unavailable", family)
                }
                NativeMeasureError::NumericLimit { family, .. } => {
                    ("numericLimit", "measurement.numericLimit", family)
                }
            };
            json!({"status":status,"diagnostic":{"code":code,"operation":"part.measure","message":error.to_string(),"family":family(family_name),"measure":measure}})
        }
    }
}
fn faces<F, E>(
    body: &Body<F, E>,
    names: &BTreeMap<u32, String>,
    native: &NativeSource,
    role: impl Fn(&F) -> &'static str,
) -> Vec<Value> {
    let caps = [
        native.cap(exedra_analytic::End::Start),
        native.cap(exedra_analytic::End::End),
    ];
    body.faces()
        .iter()
        .enumerate()
        .map(|(id, face)| {
            let role = role(&face.role);
            let surface = match face.surface {
                Surface::Plane(_) => "plane",
                Surface::Cylinder(_) => "cylinder",
                Surface::Cone(_) => "cone",
                Surface::Torus(_) => "torus",
                Surface::Sphere(_) => "sphere",
                Surface::Ruled(_) => "ruled",
            };
            let mut features: Vec<_> = native
                .face_tags(exedra_analytic::body::FaceId(
                    u32::try_from(id).expect("native face IDs fit u32"),
                ))
                .expect("iterated native face")
                .filter_map(|tag| names.get(&tag.0))
                .collect();
            features.sort();
            features.dedup();
            let mut value = json!({"id":id,"role":role,"surface":surface,"features":features});
            let face_id =
                exedra_analytic::body::FaceId(u32::try_from(id).expect("native face IDs fit u32"));
            if caps[0] == Some(face_id) {
                value["end"] = json!("start");
            } else if caps[1] == Some(face_id) {
                value["end"] = json!("end");
            }
            if features.len() == 1 {
                value["feature"] = json!(features[0]);
            }
            value
        })
        .collect()
}
fn bore_info(source: &exedra_analytic::bore::BoredPlate) -> Result<Value, Error> {
    use exedra_analytic::geometry::Curve;
    let report = source.report();
    let definition = source.bore();
    let frame = source.frame();
    let document_point = frame.transform_point(definition.point());
    let document_direction = frame.transform_vector(definition.direction());
    if document_point
        .iter()
        .chain(&document_direction)
        .any(|v| !v.is_finite())
    {
        return Err(Error::new(
            "bore.numericLimit",
            "part.inspect",
            "Placed bore-axis evidence exceeds finite f64 values",
        ));
    }
    let rims=report.rims.iter().map(|id| {
        let edge=source.body().edge(*id).ok_or_else(||Error::new("inspection.boreRim","part.inspect","Native bore rim edge is absent"))?;
        let (curve,_)=edge.regular().ok_or_else(||Error::new("inspection.boreRim","part.inspect","Native bore rim is a collapsed boundary"))?;
        let (major,minor,local_center)=match curve {
            Curve::Ellipse(e)=>(e.semi_major(),e.semi_minor(),e.frame().transform_point([0.0;3])),
            Curve::Circle(c)=>(c.radius(),c.radius(),c.frame().transform_point([0.0;3])),
            Curve::Line(_)=>return Err(Error::new("inspection.boreRim","part.inspect","Native bore rim is not a conic")),
        };
        let document_center=frame.transform_point(local_center);
        if document_center.iter().chain([&major,&minor]).any(|v|!v.is_finite()){return Err(Error::new("bore.numericLimit","part.inspect","Placed native rim evidence exceeds finite f64 values"));}
        Ok(json!({"edge":id.0,"semiMajor":major,"semiMinor":minor,"documentCenter":document_center}))
    }).collect::<Result<Vec<_>,Error>>()?;
    Ok(
        json!({"coordinateSpace":"construction","point":definition.point(),"direction":definition.direction(),"radius":definition.radius(),
        "documentPoint":document_point,"documentDirection":document_direction,
        "rims":rims,"wallFace":report.wall.0,"minimumClearance":report.min_clearance,"maxNativeGap":report.max_gap,"axisDot":report.axis_dot}),
    )
}
pub(crate) fn metadata(
    native: &NativeSource,
    names: &BTreeMap<u32, String>,
) -> Result<Value, Error> {
    let faces = match native.geometry() {
        NativeGeometryRef::Body { body, .. } => faces(body, names, native, |_| "boundary"),
        NativeGeometryRef::Extrusion(v) => faces(v.body(), names, native, |role| match role {
            exedra_analytic::extrusion::FaceRole::Cap(_) => "cap",
            exedra_analytic::extrusion::FaceRole::Wall { .. } => "wall",
            exedra_analytic::extrusion::FaceRole::Floor { .. } => "floor",
        }),
        NativeGeometryRef::Revolution(v) => faces(v.body(), names, native, |role| match role {
            exedra_analytic::revolve::FaceRole::Cap { .. } => "cap",
            exedra_analytic::revolve::FaceRole::Wall { .. } => "wall",
        }),
        NativeGeometryRef::Sweep(v) => faces(v.body(), names, native, |role| match role {
            exedra_analytic::sweep::FaceRole::Cap { .. } => "cap",
            exedra_analytic::sweep::FaceRole::Wall { .. } => "wall",
        }),
        NativeGeometryRef::Loft(v) => faces(v.body(), names, native, |role| match role {
            exedra_analytic::loft::FaceRole::Cap { .. } => "cap",
            exedra_analytic::loft::FaceRole::Wall { .. } => "wall",
        }),
        NativeGeometryRef::BoredPlate(v) => faces(v.body(), names, native, |role| match role {
            exedra_analytic::bore::FaceRole::Plate { role, .. } => match role {
                exedra_analytic::extrusion::FaceRole::Cap(_) => "cap",
                exedra_analytic::extrusion::FaceRole::Wall { .. } => "wall",
                exedra_analytic::extrusion::FaceRole::Floor { .. } => "floor",
            },
            exedra_analytic::bore::FaceRole::BoreWall { .. } => "wall",
        }),
    };
    let support = match native.geometry() {
        NativeGeometryRef::Body { body, .. } => exedra_step::check_geometry_support(body),
        NativeGeometryRef::Extrusion(v) => exedra_step::check_geometry_support(v.body()),
        NativeGeometryRef::Revolution(v) => exedra_step::check_geometry_support(v.body()),
        NativeGeometryRef::Sweep(v) => exedra_step::check_geometry_support(v.body()),
        NativeGeometryRef::Loft(v) => exedra_step::check_geometry_support(v.body()),
        NativeGeometryRef::BoredPlate(v) => exedra_step::check_geometry_support(v.body()),
    };
    let step_support = match support {
        Ok(()) => json!({"status":"supported"}),
        Err(error) => {
            json!({"status":"unsupported","diagnostic":Error::step("part.stepSupport",error)})
        }
    };
    let bore = match native.geometry() {
        NativeGeometryRef::BoredPlate(v) => bore_info(v)?,
        _ => Value::Null,
    };
    let storage = native.boundary_storage();
    let bounds = native
        .conservative_bounds()
        .map_err(|error| Error::new("bounds.numericLimit", "part.inspect", error.to_string()))?;
    Ok(
        json!({"family":family(native.family()),"stepSupport":step_support,"chartKind":native.chart_kind().map(chart_kind),"bore":bore,"faces":faces,"measurements":{
            "volume":measure(native.volume(),"volume"),"surfaceArea":measure(native.surface_area(),"surfaceArea"),
            "bounds":bounds,"boundsKind":"conservativeNativeSupport","faces":storage.faces,"edges":storage.edges,"vertices":storage.vertices
        }}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use exedra_analytic::loft::{Loft, LoftPolicy};
    use exedra_constructive::builders;
    use exedra_math::Placement3;
    use exedra_profile::ProfilePolicy;
    #[test]
    fn valid_loft_inspection_preserves_unavailable_integrals_without_fabrication() {
        let profile = builders::rect_centered(4.0, 3.0).unwrap();
        let loft = Loft::from_profiles(
            [
                (Placement3::IDENTITY, &profile),
                (Placement3::translate(0.0, 0.0, 8.0), &profile),
            ],
            Placement3::IDENTITY,
            &LoftPolicy::new(ProfilePolicy::new(1e-6)),
        )
        .unwrap();
        let value = metadata(&NativeSource::from(loft), &BTreeMap::new()).unwrap();
        assert_eq!(value["family"], "loft");
        assert_eq!(value["stepSupport"]["status"], "unsupported");
        assert_eq!(
            value["stepSupport"]["diagnostic"]["geometryFamily"],
            "ruled"
        );
        for measure in ["volume", "surfaceArea"] {
            assert_eq!(value["measurements"][measure]["status"], "unavailable");
            assert_eq!(
                value["measurements"][measure]["diagnostic"]["code"],
                "measurement.unavailable"
            );
            assert!(value["measurements"][measure].get("value").is_none());
        }
        assert_eq!(
            value["measurements"]["boundsKind"],
            "conservativeNativeSupport"
        );
    }
    #[test]
    fn bored_plate_inspection_keeps_inherited_roles_generated_tag_and_step_support() {
        let validation = ProfilePolicy::new(1e-6);
        let profile = builders::rect_centered(40.0, 30.0)
            .unwrap()
            .to_line_arc(&validation)
            .unwrap();
        let plate = exedra_analytic::extrusion::Extrusion::new(profile, Placement3::IDENTITY, 10.0)
            .unwrap();
        let bored = plate
            .with_through_bore(
                exedra_analytic::bore::Bore::new([0.0; 3], [0.2, 0.0, 1.0], 2.0)
                    .unwrap()
                    .tagged(exedra_profile::SegTag(9)),
                &exedra_analytic::bore::BorePolicy {
                    separation: 0.1,
                    geometry_gap: 1e-8,
                    min_axis_dot: 0.1,
                },
            )
            .unwrap();
        let wall = bored.report().wall.0 as usize;
        let mut names = BTreeMap::new();
        names.insert(9, String::from("bore"));
        let value = metadata(&NativeSource::from(bored), &names).unwrap();
        assert_eq!(value["family"], "boredPlate");
        assert_eq!(value["stepSupport"]["status"], "supported");
        assert_eq!(value["measurements"]["faces"], 7);
        assert_eq!(value["faces"][wall]["role"], "wall");
        assert_eq!(value["faces"][wall]["feature"], "bore");
        assert_eq!(
            value["faces"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|face| face["role"] == "cap")
                .count(),
            2
        );
        for measure in ["volume", "surfaceArea"] {
            assert_eq!(value["measurements"][measure]["status"], "available");
            assert!(
                value["measurements"][measure]["value"]
                    .as_f64()
                    .unwrap()
                    .is_finite()
            );
        }
    }
}
