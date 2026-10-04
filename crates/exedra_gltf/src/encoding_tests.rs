// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
#[test]
fn extension_fields_are_validated() {
    let invalid = [
        (
            json!({"KHR_materials_transmission": {"transmissionFactor": 1.5}}),
            "extensions.KHR_materials_transmission.transmissionFactor",
        ),
        (
            json!({"KHR_materials_ior": {"ior": 0.5}}),
            "extensions.KHR_materials_ior.ior",
        ),
        (
            json!({"KHR_materials_volume": {"thicknessFactor": 1.0}}),
            "extensions.KHR_materials_volume",
        ),
        (
            json!({"KHR_materials_volume": {"attenuationDistance": 0.0},
                   "KHR_materials_transmission": {}}),
            "extensions.KHR_materials_volume.attenuationDistance",
        ),
        (
            json!({"KHR_materials_specular": {"specularColorFactor": [1.0, -0.1, 1.0]}}),
            "extensions.KHR_materials_specular.specularColorFactor",
        ),
        (
            json!({"KHR_materials_sheen": {"sheenColorFactor": [1.0, 1.0]}}),
            "extensions.KHR_materials_sheen.sheenColorFactor",
        ),
        (
            json!({"KHR_materials_emissive_strength": {"emissiveStrength": -1.0}}),
            "extensions.KHR_materials_emissive_strength.emissiveStrength",
        ),
        (
            json!({"KHR_materials_clearcoat": {"clearcoatNormalTexture": {"index": 0, "scale": "1"}}}),
            "extensions.KHR_materials_clearcoat.clearcoatNormalTexture.scale",
        ),
        (
            json!({"KHR_materials_sheen": {"sheenRoughnessTexture": {"index": 0,
                "extensions": {"KHR_texture_transform": {"scale": [1.0]}}}}}),
            "extensions.KHR_materials_sheen.sheenRoughnessTexture.extensions.KHR_texture_transform.scale",
        ),
        (
            json!({"KHR_materials_ior": 1.5}),
            "extensions.KHR_materials_ior",
        ),
    ];
    for (extensions, field) in invalid {
        let material = json!({ "extensions": extensions });
        assert_eq!(
            materials::validate(material, "m"),
            Err(GltfError::InvalidMaterial {
                key: "m".into(),
                field: field.into(),
            }),
            "{field}"
        );
    }
    // IOR 0 is the spec's "infinite" value; volume with diffuse transmission
    // is valid.
    for extensions in [
        json!({"KHR_materials_ior": {"ior": 0.0}}),
        json!({"KHR_materials_volume": {"thicknessFactor": 0.01, "attenuationColor": [1.0, 0.9, 0.8]},
               "KHR_materials_diffuse_transmission": {}}),
    ] {
        assert!(materials::validate(json!({ "extensions": extensions }), "m").is_ok());
    }
}

#[test]
fn unlisted_extensions_and_fields_are_refused() {
    let unsupported = [
        (
            json!({"extensions": {"KHR_materials_unlit": {}}}),
            "extensions.KHR_materials_unlit",
        ),
        (
            json!({"extensions": {"KHR_materials_transmission": {"transmissionColor": 0.5}}}),
            "extensions.KHR_materials_transmission.transmissionColor",
        ),
        (
            json!({"extensions": {"KHR_materials_transmission": {"extensions": {}}}}),
            "extensions.KHR_materials_transmission.extensions",
        ),
        (
            json!({"normalTexture": {"index": 0,
                "extensions": {"KHR_texture_transform": {"skew": 1.0}}}}),
            "normalTexture.extensions.KHR_texture_transform.skew",
        ),
        (
            json!({"normalTexture": {"index": 0, "extensions": {"EXT_texture_webp": {}}}}),
            "normalTexture.extensions.EXT_texture_webp",
        ),
    ];
    for (material, field) in unsupported {
        assert_eq!(
            materials::validate(material, "m"),
            Err(GltfError::UnsupportedMaterialField {
                key: "m".into(),
                field: field.into(),
            }),
            "{field}"
        );
    }
}

#[test]
fn texture_info_refuses_missing_indices_fractional_sets_and_unlisted_extensions() {
    for info in [
        json!({}),
        json!({"index":-1}),
        json!({"index":1.5}),
        json!({"index":4294967296_u64}),
        json!({"index":0,"texCoord":1.5}),
        json!({"index":0,"extensions":{"KHR_texture_basisu":{}}}),
    ] {
        assert!(
            materials::validate(
                json!({"pbrMetallicRoughness":{"baseColorTexture":info}}),
                "a"
            )
            .is_err()
        );
    }
}

#[test]
fn base64_reference_vectors() {
    assert_eq!(base64(b""), "");
    assert_eq!(base64(b"a"), "YQ==");
    assert_eq!(base64(b"ab"), "YWI=");
    assert_eq!(base64(b"abc"), "YWJj");
    assert_eq!(base64(b"abcdef"), "YWJjZGVm");
}

#[test]
fn matrix_layout_is_column_major() {
    let p = Placement3::translate(1.0, 2.0, 3.0);
    let m = matrix_column_major(&p.rows);
    assert_eq!(&m[12..16], &[1.0, 2.0, 3.0, 1.0]);
    assert_eq!(m[0], 1.0);
    assert_eq!(m[15], 1.0);
}
