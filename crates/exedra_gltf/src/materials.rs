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
/// Core material factors and four texture references are supported:
/// `pbrMetallicRoughness.baseColorTexture`,
/// `pbrMetallicRoughness.metallicRoughnessTexture`, `normalTexture` (with
/// `scale`), and `occlusionTexture` (with `strength` in `[0, 1]`). Texture
/// indices address this resolver's resources through
/// [`Self::resolve_texture`], not a previous export's texture table. A
/// reference's `texCoord` (default 0) names the UV set it samples: set 0 needs
/// finite UVs on every textured corner, and set `n >= 1` needs an exported
/// `TEXCOORD_n` attribute mapping (see
/// [`GltfExportOptions::attributes`](crate::GltfExportOptions::attributes)).
/// Both are checked per primitive; set `n >= 1` is checked for export, not for
/// authored coverage. Integer fields such as `index` and `texCoord` must be
/// JSON integers: `1.0` is refused. Other texture fields, extensions, and
/// unknown fields are rejected, so spelling mistakes cannot silently lose
/// intent. Colors must use glTF's linear factor convention; see
/// [`Texture::image`] for each texture's encoding and channels.
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
    /// Complete, valid encoded PNG or JPEG bytes.
    ///
    /// glTF fixes each slot's encoding and channels, and the exporter cannot
    /// check them, so they are the caller's contract:
    ///
    /// - base color: sRGB color, alpha in A;
    /// - metallic-roughness: linear, roughness in G and metalness in B
    ///   (R and A are ignored);
    /// - occlusion: linear, read from R (so an ORM image packs occlusion,
    ///   roughness and metalness into R, G and B);
    /// - normal: linear tangent-space XYZ in RGB, +Y up (OpenGL convention).
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

/// A numeric texture-info field and its validity rule.
type ExtraField = (&'static str, fn(f64) -> bool);

/// A texture reference a material may carry.
#[derive(Copy, Clone, Debug)]
pub(crate) struct TextureSlot {
    /// JSON pointer to the texture info within a material.
    pub(crate) pointer: &'static str,
    /// Field path used in errors.
    pub(crate) field: &'static str,
    /// Extra numeric field the texture info may carry, with its validity rule.
    extra: Option<ExtraField>,
}

/// Supported texture references, in the fixed order resources are resolved.
pub(crate) const TEXTURE_SLOTS: [TextureSlot; 4] = [
    TextureSlot {
        pointer: "/pbrMetallicRoughness/baseColorTexture",
        field: "pbrMetallicRoughness.baseColorTexture",
        extra: None,
    },
    TextureSlot {
        pointer: "/pbrMetallicRoughness/metallicRoughnessTexture",
        field: "pbrMetallicRoughness.metallicRoughnessTexture",
        extra: None,
    },
    TextureSlot {
        pointer: "/normalTexture",
        field: "normalTexture",
        extra: Some(("scale", f64::is_finite)),
    },
    TextureSlot {
        pointer: "/occlusionTexture",
        field: "occlusionTexture",
        extra: Some(("strength", |v| (0.0..=1.0).contains(&v))),
    },
];

/// Returns the UV set a validated texture info samples.
pub(crate) fn tex_coord(info: &Value) -> u32 {
    info.get("texCoord")
        .and_then(Value::as_u64)
        .map_or(0, |set| u32::try_from(set).expect("validated texCoord"))
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
        "normalTexture",
        "occlusionTexture",
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
                "baseColorTexture" | "metallicRoughnessTexture" => true,
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
    for slot in TEXTURE_SLOTS {
        if let Some(info) = value.pointer(slot.pointer) {
            validate_texture_info(info, slot, key)?;
        }
    }
    let object = value.as_object().expect("checked object");
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

fn validate_texture_info(
    value: &Value,
    slot: TextureSlot,
    key: &str,
) -> Result<(), crate::GltfError> {
    let invalid = |field: &str| crate::GltfError::InvalidMaterial {
        key: key.to_owned(),
        field: format!("{}{field}", slot.field),
    };
    let object = value.as_object().ok_or_else(|| invalid(""))?;
    let u32_valued = |value: &Value| value.as_u64().and_then(|n| u32::try_from(n).ok());
    if object.get("index").and_then(u32_valued).is_none() {
        return Err(invalid(".index"));
    }
    for (field, value) in object {
        match field.as_str() {
            "index" | "extras" => {}
            "texCoord" => {
                if u32_valued(value).is_none() {
                    return Err(invalid(".texCoord"));
                }
            }
            name if slot.extra.is_some_and(|(extra, _)| extra == name) => {
                let (_, valid) = slot.extra.expect("matched extra field");
                if !value.as_f64().is_some_and(valid) {
                    return Err(invalid(&format!(".{name}")));
                }
            }
            _ => {
                return Err(crate::GltfError::UnsupportedMaterialField {
                    key: key.to_owned(),
                    field: format!("{}.{field}", slot.field),
                });
            }
        }
    }
    Ok(())
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
            json!({"emissiveTexture": {"index": 0}}),
            json!({"extensions": {"KHR_materials_unlit": {}}}),
            json!({"normalTexture": {"index": 0, "strength": 1.0}}),
            json!({"occlusionTexture": {"index": 0, "scale": 1.0}}),
            json!({"pbrMetallicRoughness": {"metallicRoughnessTexture": {"index": 0, "extensions": {}}}}),
            json!({"roughnes": 0.5}),
        ] {
            assert!(matches!(
                validate(material, "id"),
                Err(crate::GltfError::UnsupportedMaterialField { .. })
            ));
        }
    }

    #[test]
    fn accepts_the_supported_texture_references() {
        let material = json!({
            "pbrMetallicRoughness": {
                "baseColorTexture": {"index": 0, "texCoord": 1},
                "metallicRoughnessTexture": {"index": 1, "extras": {"packing": "orm"}}
            },
            "normalTexture": {"index": 2, "scale": -0.5, "texCoord": 0},
            "occlusionTexture": {"index": 1, "strength": 0.75}
        });
        let validated = validate(material.clone(), "bark").unwrap();
        assert_eq!(validated["normalTexture"], material["normalTexture"]);
        assert_eq!(
            tex_coord(&validated["pbrMetallicRoughness"]["baseColorTexture"]),
            1
        );
        assert_eq!(tex_coord(&validated["occlusionTexture"]), 0);
    }

    #[test]
    fn rejects_invalid_texture_parameters() {
        for (material, field) in [
            (
                json!({"normalTexture": {"index": 0, "scale": "1"}}),
                "normalTexture.scale",
            ),
            (
                json!({"occlusionTexture": {"index": 0, "strength": 1.5}}),
                "occlusionTexture.strength",
            ),
            (
                json!({"occlusionTexture": {"index": 0, "strength": -0.1}}),
                "occlusionTexture.strength",
            ),
            (
                json!({"normalTexture": {"index": 0, "texCoord": 1.5}}),
                "normalTexture.texCoord",
            ),
            (
                json!({"normalTexture": {"index": 0, "texCoord": -1}}),
                "normalTexture.texCoord",
            ),
            (json!({"occlusionTexture": {}}), "occlusionTexture.index"),
            (json!({"normalTexture": 3}), "normalTexture"),
            (
                json!({"pbrMetallicRoughness": {"metallicRoughnessTexture": {"index": 1.5}}}),
                "pbrMetallicRoughness.metallicRoughnessTexture.index",
            ),
        ] {
            assert_eq!(
                validate(material, "bark"),
                Err(crate::GltfError::InvalidMaterial {
                    key: "bark".into(),
                    field: field.into(),
                })
            );
        }
    }
}
