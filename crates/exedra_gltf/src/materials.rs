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
/// JSON integers: `1.0` is refused. Colors must use glTF's linear factor
/// convention; see [`Texture::image`] for each texture's encoding and
/// channels.
///
/// Material `extensions` may use this allowlist, each validated against its
/// Khronos specification, with its textures resolved and checked like core
/// textures:
///
/// - `KHR_materials_diffuse_transmission`: `diffuseTransmissionFactor`,
///   `diffuseTransmissionTexture` (A), `diffuseTransmissionColorFactor`,
///   `diffuseTransmissionColorTexture` (sRGB RGB);
/// - `KHR_materials_transmission`: `transmissionFactor`,
///   `transmissionTexture` (R);
/// - `KHR_materials_volume`: `thicknessFactor`, `thicknessTexture` (G),
///   `attenuationDistance`, `attenuationColor`. It needs a transmission
///   extension on the same material, which glTF requires for it to have any
///   effect, so a lone volume is refused;
/// - `KHR_materials_ior`: `ior` (`0` or at least `1`);
/// - `KHR_materials_specular`: `specularFactor`, `specularTexture` (A),
///   `specularColorFactor`, `specularColorTexture` (sRGB RGB);
/// - `KHR_materials_clearcoat`: `clearcoatFactor`, `clearcoatTexture` (R),
///   `clearcoatRoughnessFactor`, `clearcoatRoughnessTexture` (G),
///   `clearcoatNormalTexture` (with `scale`);
/// - `KHR_materials_sheen`: `sheenColorFactor`, `sheenColorTexture` (sRGB
///   RGB), `sheenRoughnessFactor`, `sheenRoughnessTexture` (A);
/// - `KHR_materials_emissive_strength`: `emissiveStrength`.
///
/// Every texture reference, core or extension, may carry
/// `extensions.KHR_texture_transform` (`offset`, `rotation`, `scale`,
/// `texCoord`); its `texCoord` overrides the reference's own for viewers that
/// support the extension, and both sets must be exported. Exedra
/// defines no material model: projecting a richer model such as `OpenPBR` onto
/// these extensions is the caller's conversion. Other extensions, texture
/// fields, and unknown fields are rejected, so spelling mistakes cannot
/// silently lose intent. Used extensions are listed in `extensionsUsed` and
/// none are required; see
/// [`GltfExportOptions::require_texture_transform`](crate::GltfExportOptions::require_texture_transform).
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
    ///
    /// Extension textures follow the same rule: color textures
    /// (`diffuseTransmissionColorTexture`, `specularColorTexture`,
    /// `sheenColorTexture`) are sRGB, every other one is linear and reads the
    /// channel listed on [`MaterialResolver`]; `clearcoatNormalTexture` is a
    /// normal map like `normalTexture`.
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

/// Supported texture references, in the fixed order resources are resolved:
/// the core slots, then each allowlisted extension's slots in declaration
/// order.
pub(crate) const TEXTURE_SLOTS: [TextureSlot; 15] = [
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
    ext_slot(
        "/extensions/KHR_materials_diffuse_transmission/diffuseTransmissionTexture",
        "extensions.KHR_materials_diffuse_transmission.diffuseTransmissionTexture",
    ),
    ext_slot(
        "/extensions/KHR_materials_diffuse_transmission/diffuseTransmissionColorTexture",
        "extensions.KHR_materials_diffuse_transmission.diffuseTransmissionColorTexture",
    ),
    ext_slot(
        "/extensions/KHR_materials_transmission/transmissionTexture",
        "extensions.KHR_materials_transmission.transmissionTexture",
    ),
    ext_slot(
        "/extensions/KHR_materials_volume/thicknessTexture",
        "extensions.KHR_materials_volume.thicknessTexture",
    ),
    ext_slot(
        "/extensions/KHR_materials_specular/specularTexture",
        "extensions.KHR_materials_specular.specularTexture",
    ),
    ext_slot(
        "/extensions/KHR_materials_specular/specularColorTexture",
        "extensions.KHR_materials_specular.specularColorTexture",
    ),
    ext_slot(
        "/extensions/KHR_materials_clearcoat/clearcoatTexture",
        "extensions.KHR_materials_clearcoat.clearcoatTexture",
    ),
    ext_slot(
        "/extensions/KHR_materials_clearcoat/clearcoatRoughnessTexture",
        "extensions.KHR_materials_clearcoat.clearcoatRoughnessTexture",
    ),
    TextureSlot {
        pointer: "/extensions/KHR_materials_clearcoat/clearcoatNormalTexture",
        field: "extensions.KHR_materials_clearcoat.clearcoatNormalTexture",
        extra: Some(("scale", f64::is_finite)),
    },
    ext_slot(
        "/extensions/KHR_materials_sheen/sheenColorTexture",
        "extensions.KHR_materials_sheen.sheenColorTexture",
    ),
    ext_slot(
        "/extensions/KHR_materials_sheen/sheenRoughnessTexture",
        "extensions.KHR_materials_sheen.sheenRoughnessTexture",
    ),
];

/// Pointers of the slots that hold tangent-space normal maps.
pub(crate) const NORMAL_SLOTS: [&str; 2] = [
    "/normalTexture",
    "/extensions/KHR_materials_clearcoat/clearcoatNormalTexture",
];

const fn ext_slot(pointer: &'static str, field: &'static str) -> TextureSlot {
    TextureSlot {
        pointer,
        field,
        extra: None,
    }
}

/// How a numeric extension field is validated.
#[derive(Copy, Clone, Debug)]
enum Factor {
    /// A number in `[0, 1]`.
    Unit,
    /// A finite number `>= 0`.
    NonNegative,
    /// A finite number `> 0`.
    Positive,
    /// `0` or a finite number `>= 1` (`KHR_materials_ior`).
    Ior,
    /// Three numbers in `[0, 1]`.
    UnitColor,
    /// Three finite numbers `>= 0`.
    NonNegativeColor,
}

impl Factor {
    fn accepts(self, value: &Value) -> bool {
        let number = |value: &Value, valid: fn(f64) -> bool| value.as_f64().is_some_and(valid);
        let unit = |v: f64| (0.0..=1.0).contains(&v);
        let non_negative = |v: f64| v.is_finite() && v >= 0.0;
        let color = |value: &Value, valid: fn(f64) -> bool| {
            value
                .as_array()
                .is_some_and(|a| a.len() == 3 && a.iter().all(|c| number(c, valid)))
        };
        match self {
            Self::Unit => number(value, unit),
            Self::NonNegative => number(value, non_negative),
            Self::Positive => number(value, |v| v.is_finite() && v > 0.0),
            Self::Ior => number(value, |v| v == 0.0 || (v.is_finite() && v >= 1.0)),
            Self::UnitColor => color(value, unit),
            Self::NonNegativeColor => color(value, non_negative),
        }
    }
}

/// An allowlisted material extension: its numeric fields and texture fields.
struct ExtensionSpec {
    name: &'static str,
    factors: &'static [(&'static str, Factor)],
    textures: &'static [&'static str],
}

/// Material extensions the exporter accepts, validated field by field.
const MATERIAL_EXTENSIONS: [ExtensionSpec; 8] = [
    ExtensionSpec {
        name: "KHR_materials_diffuse_transmission",
        factors: &[
            ("diffuseTransmissionFactor", Factor::Unit),
            ("diffuseTransmissionColorFactor", Factor::UnitColor),
        ],
        textures: &[
            "diffuseTransmissionTexture",
            "diffuseTransmissionColorTexture",
        ],
    },
    ExtensionSpec {
        name: "KHR_materials_transmission",
        factors: &[("transmissionFactor", Factor::Unit)],
        textures: &["transmissionTexture"],
    },
    ExtensionSpec {
        name: "KHR_materials_volume",
        factors: &[
            ("thicknessFactor", Factor::NonNegative),
            ("attenuationDistance", Factor::Positive),
            ("attenuationColor", Factor::UnitColor),
        ],
        textures: &["thicknessTexture"],
    },
    ExtensionSpec {
        name: "KHR_materials_ior",
        factors: &[("ior", Factor::Ior)],
        textures: &[],
    },
    ExtensionSpec {
        name: "KHR_materials_specular",
        factors: &[
            ("specularFactor", Factor::Unit),
            ("specularColorFactor", Factor::NonNegativeColor),
        ],
        textures: &["specularTexture", "specularColorTexture"],
    },
    ExtensionSpec {
        name: "KHR_materials_clearcoat",
        factors: &[
            ("clearcoatFactor", Factor::Unit),
            ("clearcoatRoughnessFactor", Factor::Unit),
        ],
        textures: &[
            "clearcoatTexture",
            "clearcoatRoughnessTexture",
            "clearcoatNormalTexture",
        ],
    },
    ExtensionSpec {
        name: "KHR_materials_sheen",
        factors: &[
            ("sheenColorFactor", Factor::UnitColor),
            ("sheenRoughnessFactor", Factor::Unit),
        ],
        textures: &["sheenColorTexture", "sheenRoughnessTexture"],
    },
    ExtensionSpec {
        name: "KHR_materials_emissive_strength",
        factors: &[("emissiveStrength", Factor::NonNegative)],
        textures: &[],
    },
];

/// The texture-coordinate transform extension allowed on texture references.
pub(crate) const TEXTURE_TRANSFORM: &str = "KHR_texture_transform";

/// Returns the UV sets a validated texture info can sample: the reference's
/// own `texCoord` (default 0) and, when present and different, a
/// `KHR_texture_transform` override.
///
/// Both must be exported. A viewer that supports the transform samples the
/// override; one that does not falls back to the reference's own set, and
/// `KHR_texture_transform` is only required on request.
pub(crate) fn tex_coords(info: &Value) -> impl Iterator<Item = u32> {
    let set = |value: Option<&Value>| {
        value
            .and_then(Value::as_u64)
            .map(|set| u32::try_from(set).expect("validated texCoord"))
    };
    let base = set(info.get("texCoord")).unwrap_or(0);
    let transform =
        set(info.pointer("/extensions/KHR_texture_transform/texCoord")).filter(|&set| set != base);
    core::iter::once(base).chain(transform)
}

/// Extension names a validated material uses, including
/// `KHR_texture_transform` on any texture reference, in a fixed order.
pub(crate) fn used_extensions(material: &Value) -> impl Iterator<Item = &'static str> + '_ {
    let transform = TEXTURE_SLOTS.iter().any(|slot| {
        material
            .pointer(slot.pointer)
            .and_then(|info| info.pointer("/extensions/KHR_texture_transform"))
            .is_some()
    });
    MATERIAL_EXTENSIONS
        .iter()
        .map(|spec| spec.name)
        .filter(|name| {
            material
                .get("extensions")
                .and_then(|e| e.get(*name))
                .is_some()
        })
        .chain(transform.then_some(TEXTURE_TRANSFORM))
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
        "extensions",
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
    if let Some(extensions) = object.get("extensions") {
        validate_extensions(extensions, key)?;
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

fn validate_extensions(extensions: &Value, key: &str) -> Result<(), crate::GltfError> {
    let invalid = |field: String| crate::GltfError::InvalidMaterial {
        key: key.to_owned(),
        field,
    };
    let unsupported = |field: String| crate::GltfError::UnsupportedMaterialField {
        key: key.to_owned(),
        field,
    };
    let extensions = extensions
        .as_object()
        .ok_or_else(|| invalid("extensions".into()))?;
    for (name, body) in extensions {
        let spec = MATERIAL_EXTENSIONS
            .iter()
            .find(|spec| spec.name == name)
            .ok_or_else(|| unsupported(format!("extensions.{name}")))?;
        let body = body
            .as_object()
            .ok_or_else(|| invalid(format!("extensions.{name}")))?;
        for (field, value) in body {
            if field == "extras" || spec.textures.contains(&field.as_str()) {
                // Texture references are validated with the texture slots.
                continue;
            }
            let (_, factor) = spec
                .factors
                .iter()
                .find(|(factor, _)| factor == field)
                .ok_or_else(|| unsupported(format!("extensions.{name}.{field}")))?;
            if !factor.accepts(value) {
                return Err(invalid(format!("extensions.{name}.{field}")));
            }
        }
    }
    // Volume only describes the medium behind a transmitting surface; alone
    // it has no effect, so it is refused rather than silently ignored.
    if extensions.contains_key("KHR_materials_volume")
        && !extensions.contains_key("KHR_materials_transmission")
        && !extensions.contains_key("KHR_materials_diffuse_transmission")
    {
        return Err(invalid("extensions.KHR_materials_volume".into()));
    }
    Ok(())
}

fn validate_texture_transform(
    value: &Value,
    field: &str,
    key: &str,
) -> Result<(), crate::GltfError> {
    let invalid = |part: &str| crate::GltfError::InvalidMaterial {
        key: key.to_owned(),
        field: format!("{field}.extensions.{TEXTURE_TRANSFORM}{part}"),
    };
    let object = value.as_object().ok_or_else(|| invalid(""))?;
    let finite = |value: &Value| value.as_f64().is_some_and(f64::is_finite);
    let pair = |value: &Value| {
        value
            .as_array()
            .is_some_and(|a| a.len() == 2 && a.iter().all(finite))
    };
    for (name, value) in object {
        let valid = match name.as_str() {
            "offset" | "scale" => pair(value),
            "rotation" => finite(value),
            "texCoord" => value.as_u64().and_then(|n| u32::try_from(n).ok()).is_some(),
            "extras" => true,
            _ => {
                return Err(crate::GltfError::UnsupportedMaterialField {
                    key: key.to_owned(),
                    field: format!("{field}.extensions.{TEXTURE_TRANSFORM}.{name}"),
                });
            }
        };
        if !valid {
            return Err(invalid(&format!(".{name}")));
        }
    }
    Ok(())
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
            "extensions" => {
                let extensions = value.as_object().ok_or_else(|| invalid(".extensions"))?;
                for (name, transform) in extensions {
                    if name != TEXTURE_TRANSFORM {
                        return Err(crate::GltfError::UnsupportedMaterialField {
                            key: key.to_owned(),
                            field: format!("{}.extensions.{name}", slot.field),
                        });
                    }
                    validate_texture_transform(transform, slot.field, key)?;
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
            json!({"pbrMetallicRoughness": {"metallicRoughnessTexture": {"index": 0, "extensions": {"EXT_texture_webp": {}}}}}),
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
        let sets = |info: &Value| tex_coords(info).collect::<Vec<_>>();
        assert_eq!(
            sets(&validated["pbrMetallicRoughness"]["baseColorTexture"]),
            [1]
        );
        assert_eq!(sets(&validated["occlusionTexture"]), [0]);
        let transformed = json!({"index": 0, "texCoord": 1,
            "extensions": {"KHR_texture_transform": {"texCoord": 0}}});
        assert_eq!(sets(&transformed), [1, 0]);
        let same = json!({"index": 0, "texCoord": 1,
            "extensions": {"KHR_texture_transform": {"texCoord": 1}}});
        assert_eq!(sets(&same), [1]);
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
