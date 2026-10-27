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
            if features.len() == 1 {
                value["feature"] = json!(features[0]);
            }
            value
        })
        .collect()
}
pub(crate) fn metadata(
    native: &NativeSource,
    names: &BTreeMap<u32, String>,
) -> Result<Value, Error> {
    let faces = match native.geometry() {
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
    };
    let support = match native.geometry() {
        NativeGeometryRef::Extrusion(v) => exedra_step::check_geometry_support(v.body()),
        NativeGeometryRef::Revolution(v) => exedra_step::check_geometry_support(v.body()),
        NativeGeometryRef::Sweep(v) => exedra_step::check_geometry_support(v.body()),
        NativeGeometryRef::Loft(v) => exedra_step::check_geometry_support(v.body()),
    };
    let step_support = match support {
        Ok(()) => json!({"status":"supported"}),
        Err(error) => {
            json!({"status":"unsupported","diagnostic":Error::step("part.stepSupport",error)})
        }
    };
    let storage = native.boundary_storage();
    let bounds = native
        .conservative_bounds()
        .map_err(|error| Error::new("bounds.numericLimit", "part.inspect", error.to_string()))?;
    Ok(
        json!({"family":family(native.family()),"stepSupport":step_support,"faces":faces,"measurements":{
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
}
