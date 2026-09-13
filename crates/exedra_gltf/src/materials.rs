// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Maps opaque assembly material keys to glTF transport data.
//!
//! Exedra defines no shared material model. Callers own their material IDs,
//! descriptions, storage, and conversion to glTF. This module checks the
//! supported glTF subset at the export boundary.

use serde_json::Value;

/// Maps an opaque assembly material key to a glTF material JSON object.
///
/// The caller may use any internal material representation. Return its glTF
/// projection here, or `None` for an unavailable key. The exporter resolves each
/// used key once, in first-use order; keep inputs fixed during an export.
/// Missing descriptions are typed errors, not preview colors. Unassigned
/// regions never invoke the resolver. An omitted material name defaults to the
/// key; a caller-supplied name and `extras` are preserved.
///
/// Core material factors and `pbrMetallicRoughness.baseColorTexture` are supported.
/// Texture indices address this resolver's resources through
/// [`Self::resolve_texture`], not a previous export's texture table. Only
/// `TEXCOORD_0` is supported. Other texture fields, extensions, and unknown fields
/// are rejected, so spelling mistakes cannot silently lose intent.
/// Colors must use glTF's linear factor convention.
///
/// Closures implement this trait:
///
/// ```
/// use exedra_gltf::MaterialResolver;
/// use serde_json::json;
/// let resolve = |key: &str| match key {
///     "paint" => Some(json!({
///         "pbrMetallicRoughness": { "metallicFactor": 0.0 }
///     })),
///     _ => None,
/// };
/// assert!(resolve.resolve("paint").is_some());
/// assert!(resolve.resolve("missing").is_none());
/// ```
pub trait MaterialResolver {
    /// Returns a glTF material object for `key`, or `None` if unavailable.
    fn resolve(&self, key: &str) -> Option<Value>;

    /// Supplies a texture referenced by a material's caller-local index.
    ///
    /// Called once per used index, in first-use order. Keep resources fixed
    /// during export. The default supports existing untextured resolvers.
    fn resolve_texture(&self, _index: u32) -> Option<Texture<'_>> {
        None
    }
}

/// Caller-owned encoded image and glTF sampling parameters.
///
/// This is export transport data, not a shared material model. Identical image
/// bytes share storage even when textures use different samplers. The exporter
/// checks the image signature, not full decodability or color-profile contents.
#[derive(Clone, Debug)]
pub struct Texture<'a> {
    /// Complete, valid encoded PNG or JPEG bytes. Base-color images use sRGB.
    pub image: &'a [u8],
    /// `image/png` or `image/jpeg`, matching the encoded image.
    pub mime_type: &'a str,
    /// Optional core glTF sampler object. Supports `magFilter`, `minFilter`,
    /// `wrapS`, `wrapT`, `name`, and `extras`; unknown fields are refused.
    /// `None` leaves sampling to glTF defaults.
    pub sampler: Option<Value>,
}

impl<F: Fn(&str) -> Option<Value>> MaterialResolver for F {
    fn resolve(&self, key: &str) -> Option<Value> {
        self(key)
    }
}

pub(crate) fn validate(mut value: Value, key: &str) -> Result<Value, crate::GltfError> {
    let invalid = |field: &str| crate::GltfError::InvalidMaterial {
        key: key.to_owned(),
        field: field.to_owned(),
    };
    let object = value.as_object_mut().ok_or_else(|| invalid("material"))?;
    let supported = [
        "name",
        "extras",
        "pbrMetallicRoughness",
        "emissiveFactor",
        "alphaMode",
        "alphaCutoff",
        "doubleSided",
    ];
    for field in object.keys() {
        if !supported.contains(&field.as_str()) {
            return Err(crate::GltfError::UnsupportedMaterialField {
                key: key.to_owned(),
                field: field.clone(),
            });
        }
    }
    if let Some(name) = object.get("name") {
        if !name.is_string() {
            return Err(invalid("name"));
        }
    } else {
        object.insert("name".into(), Value::String(key.to_owned()));
    }
    let unit = |value: &Value| value.as_f64().is_some_and(|n| (0.0..=1.0).contains(&n));
    let color = |value: &Value, count| {
        value
            .as_array()
            .is_some_and(|a| a.len() == count && a.iter().all(unit))
    };
    if let Some(pbr) = object.get("pbrMetallicRoughness") {
        let pbr = pbr
            .as_object()
            .ok_or_else(|| invalid("pbrMetallicRoughness"))?;
        for (field, value) in pbr {
            let valid = match field.as_str() {
                "baseColorFactor" => color(value, 4),
                "metallicFactor" | "roughnessFactor" => unit(value),
                "baseColorTexture" => {
                    validate_texture_info(value, key)?;
                    true
                }
                "extras" => true,
                _ => {
                    return Err(crate::GltfError::UnsupportedMaterialField {
                        key: key.to_owned(),
                        field: format!("pbrMetallicRoughness.{field}"),
                    });
                }
            };
            if !valid {
                return Err(invalid(&format!("pbrMetallicRoughness.{field}")));
            }
        }
    }
    if object.get("emissiveFactor").is_some_and(|v| !color(v, 3)) {
        return Err(invalid("emissiveFactor"));
    }
    if object.get("doubleSided").is_some_and(|v| !v.is_boolean()) {
        return Err(invalid("doubleSided"));
    }
    if object
        .get("alphaMode")
        .is_some_and(|v| !matches!(v.as_str(), Some("OPAQUE" | "MASK" | "BLEND")))
    {
        return Err(invalid("alphaMode"));
    }
    if let Some(cutoff) = object.get("alphaCutoff")
        && (!object.contains_key("alphaMode") || !cutoff.as_f64().is_some_and(|n| n >= 0.0))
    {
        return Err(invalid("alphaCutoff"));
    }
    Ok(value)
}

fn validate_texture_info(value: &Value, key: &str) -> Result<(), crate::GltfError> {
    let invalid = |field: &str| crate::GltfError::InvalidMaterial {
        key: key.to_owned(),
        field: texture_field(field),
    };
    let object = value
        .as_object()
        .ok_or_else(|| invalid("baseColorTexture"))?;
    if object
        .get("index")
        .and_then(Value::as_u64)
        .and_then(|index| u32::try_from(index).ok())
        .is_none()
    {
        return Err(invalid("baseColorTexture.index"));
    }
    for (field, value) in object {
        match field.as_str() {
            "index" | "extras" => {}
            "texCoord" if value.as_u64() == Some(0) => {}
            _ => {
                return Err(crate::GltfError::UnsupportedMaterialField {
                    key: key.to_owned(),
                    field: texture_field(&format!("baseColorTexture.{field}")),
                });
            }
        }
    }
    Ok(())
}

fn texture_field(field: &str) -> String {
    format!("pbrMetallicRoughness.{field}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn validates_gltf_factors_and_rejects_null_nonfinite_encodings() {
        for value in [
            json!(null),
            json!("0.5"),
            json!(-0.01),
            json!(1.01),
            json!(f64::NAN),
            json!(f64::INFINITY),
        ] {
            for field in ["metallicFactor", "roughnessFactor"] {
                let material = json!({"pbrMetallicRoughness": {field: value}});
                assert!(matches!(
                    validate(material, "finish"),
                    Err(crate::GltfError::InvalidMaterial { .. })
                ));
            }
            assert!(
                validate(
                    json!({"pbrMetallicRoughness": {"baseColorFactor": [1, 1, 1, value]}}),
                    "finish"
                )
                .is_err()
            );
            assert!(validate(json!({"emissiveFactor": [0, value, 0]}), "finish").is_err());
        }
        for material in [
            json!(null),
            json!([]),
            json!({"name": 3}),
            json!({"doubleSided": 1}),
            json!({"alphaMode": "GLASS"}),
            json!({"alphaCutoff": 0.5}),
            json!({"pbrMetallicRoughness": []}),
            json!({"emissiveFactor": [1, 1]}),
            json!({"alphaMode": "MASK", "alphaCutoff": -1}),
        ] {
            assert!(matches!(
                validate(material, "finish"),
                Err(crate::GltfError::InvalidMaterial { .. })
            ));
        }
    }

    #[test]
    fn preserves_names_metadata_defaults_and_mask_semantics() {
        for cutoff in [0.0, 0.5, 1.0, 2.0] {
            let material = json!({"name": "Caller name", "extras": {"source": "finish.42"}, "alphaMode": "MASK", "alphaCutoff": cutoff});
            assert_eq!(validate(material.clone(), "id").unwrap(), material);
        }
        assert_eq!(validate(json!({}), "id").unwrap(), json!({"name": "id"}));
    }

    #[test]
    fn unsupported_resources_extensions_and_typos_are_not_silently_dropped() {
        for material in [
            json!({"normalTexture": {"index": 0}}),
            json!({"extensions": {"KHR_materials_unlit": {}}}),
            json!({"pbrMetallicRoughness": {"metallicRoughnessTexture": {"index": 0}}}),
            json!({"roughnes": 0.5}),
        ] {
            assert!(matches!(
                validate(material, "id"),
                Err(crate::GltfError::UnsupportedMaterialField { .. })
            ));
        }
    }
}
