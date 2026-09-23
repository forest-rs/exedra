// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Resolve caller-local texture indices and pack used resources deterministically.

use std::collections::HashMap;

use serde_json::{Value, json};

use crate::materials::TEXTURE_SLOTS;
use crate::{GltfError, GltfStats, MaterialResolver, push_view};

#[derive(Default)]
pub(crate) struct TextureTables {
    pub(crate) images: Vec<Value>,
    pub(crate) textures: Vec<Value>,
    pub(crate) samplers: Vec<Value>,
}

pub(crate) fn embed(
    materials: &mut [Value],
    resolver: &dyn MaterialResolver,
    buffer: &mut Vec<u8>,
    views: &mut Vec<Value>,
    stats: &mut GltfStats,
) -> Result<TextureTables, GltfError> {
    let mut tables = TextureTables::default();
    let mut texture_indices = HashMap::new();
    let mut image_indices = HashMap::new();
    // Resources resolve in first-use order: materials in emission order,
    // then the fixed `TEXTURE_SLOTS` order within each material.
    for material in materials {
        for slot in TEXTURE_SLOTS {
            let Some(info) = material.pointer_mut(slot.pointer) else {
                continue;
            };
            let source_index = u32::try_from(info["index"].as_u64().expect("validated index"))
                .expect("validated u32");
            let index = if let Some(&index) = texture_indices.get(&source_index) {
                index
            } else {
                let texture =
                    resolver
                        .resolve_texture(source_index)
                        .ok_or(GltfError::MissingTexture {
                            index: source_index,
                        })?;
                let invalid = |field| GltfError::InvalidTexture {
                    index: source_index,
                    field,
                };
                let signature_matches = match texture.mime_type {
                    "image/png" => texture.image.starts_with(b"\x89PNG\r\n\x1a\n"),
                    "image/jpeg" => texture.image.starts_with(b"\xff\xd8\xff"),
                    _ => return Err(invalid("mimeType")),
                };
                if !signature_matches {
                    return Err(invalid("image"));
                }
                let sampler = texture
                    .sampler
                    .map(|sampler| {
                        validate_sampler(&sampler).map_err(invalid)?;
                        let index = tables
                            .samplers
                            .iter()
                            .position(|s| *s == sampler)
                            .unwrap_or_else(|| {
                                tables.samplers.push(sampler);
                                tables.samplers.len() - 1
                            });
                        Ok::<_, GltfError>(index)
                    })
                    .transpose()?;
                let image = *image_indices
                    .entry((texture.mime_type, texture.image))
                    .or_insert_with(|| {
                        let view = push_view(buffer, views, texture.image);
                        tables
                            .images
                            .push(json!({"bufferView": view, "mimeType": texture.mime_type}));
                        stats.image_bytes += texture.image.len() as u64;
                        tables.images.len() - 1
                    });
                let mut value = json!({"source": image});
                if let Some(sampler) = sampler {
                    value["sampler"] = json!(sampler);
                }
                let index = tables
                    .textures
                    .iter()
                    .position(|t| *t == value)
                    .unwrap_or_else(|| {
                        tables.textures.push(value);
                        tables.textures.len() - 1
                    });
                texture_indices.insert(source_index, index);
                index
            };
            info["index"] = json!(index);
        }
    }
    stats.images = tables.images.len() as u64;
    stats.textures = tables.textures.len() as u64;
    Ok(tables)
}

fn validate_sampler(value: &Value) -> Result<(), &'static str> {
    let object = value.as_object().ok_or("sampler")?;
    for (field, value) in object {
        let valid = match field.as_str() {
            "magFilter" => matches!(value.as_u64(), Some(9728 | 9729)),
            "minFilter" => matches!(value.as_u64(), Some(9728 | 9729 | 9984..=9987)),
            "wrapS" | "wrapT" => matches!(value.as_u64(), Some(33071 | 33648 | 10497)),
            "name" => value.is_string(),
            "extras" => true,
            _ => return Err("sampler.unsupportedField"),
        };
        if !valid {
            return Err(match field.as_str() {
                "magFilter" => "sampler.magFilter",
                "minFilter" => "sampler.minFilter",
                "wrapS" => "sampler.wrapS",
                "wrapT" => "sampler.wrapT",
                "name" => "sampler.name",
                _ => unreachable!("handled sampler field"),
            });
        }
    }
    Ok(())
}
