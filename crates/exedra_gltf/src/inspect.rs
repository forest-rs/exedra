// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Semantic inspection of binary glTF produced by this crate.

use serde_json::{Map, Value};

use crate::GltfError;

const HEADER_LENGTH: usize = 12;
const CHUNK_HEADER_LENGTH: usize = 8;

/// A parsed GLB 2.0 document for semantic export assertions.
///
/// Parsing validates the binary container and JSON chunk, but deliberately
/// does not attempt full glTF schema validation. Query methods skip malformed
/// or absent optional schema entries, which keeps this helper focused on the
/// stable questions exposed by this exporter.
#[derive(Clone, Debug)]
pub struct GlbDocument {
    json: Value,
    bin: Vec<u8>,
}

impl GlbDocument {
    /// Parses a GLB 2.0 container and owns its JSON and optional BIN chunks.
    ///
    /// The first chunk must be JSON. A BIN chunk, when present, must be the
    /// second chunk. Later unknown chunk types still participate in length
    /// and alignment validation but are otherwise ignored.
    ///
    /// # Errors
    ///
    /// Returns [`GltfError::InvalidGlb`] for a bad header, length, alignment,
    /// or chunk order, and [`GltfError::InvalidGlbJson`] when the JSON chunk
    /// cannot be parsed.
    pub fn parse(bytes: &[u8]) -> Result<Self, GltfError> {
        if bytes.len() < HEADER_LENGTH {
            return Err(invalid("header is truncated"));
        }
        if &bytes[..4] != b"glTF" {
            return Err(invalid("magic is not glTF"));
        }
        if read_u32(bytes, 4) != 2 {
            return Err(invalid("version is not 2"));
        }
        let declared_length = usize::try_from(read_u32(bytes, 8))
            .map_err(|_| invalid("declared length is not representable"))?;
        if declared_length != bytes.len() {
            return Err(invalid("declared length does not match the input"));
        }

        let mut offset = HEADER_LENGTH;
        let mut chunk_index = 0_usize;
        let mut json = None;
        let mut bin = None;
        while offset < bytes.len() {
            let header_end = offset
                .checked_add(CHUNK_HEADER_LENGTH)
                .ok_or_else(|| invalid("chunk header length overflows"))?;
            if header_end > bytes.len() {
                return Err(invalid("chunk header is truncated"));
            }
            let chunk_length = usize::try_from(read_u32(bytes, offset))
                .map_err(|_| invalid("chunk length is not representable"))?;
            if !chunk_length.is_multiple_of(4) {
                return Err(invalid("chunk length is not four-byte aligned"));
            }
            let chunk_end = header_end
                .checked_add(chunk_length)
                .ok_or_else(|| invalid("chunk length overflows"))?;
            if chunk_end > bytes.len() {
                return Err(invalid("chunk payload is truncated"));
            }
            let chunk_type = &bytes[offset + 4..header_end];
            let payload = &bytes[header_end..chunk_end];
            match chunk_type {
                b"JSON" => {
                    if chunk_index != 0 || json.is_some() {
                        return Err(invalid("JSON is not the single first chunk"));
                    }
                    json = Some(serde_json::from_slice(payload).map_err(|error| {
                        GltfError::InvalidGlbJson {
                            message: error.to_string(),
                        }
                    })?);
                }
                b"BIN\0" => {
                    if chunk_index != 1 || json.is_none() || bin.is_some() {
                        return Err(invalid("BIN is not the single optional second chunk"));
                    }
                    bin = Some(payload.to_vec());
                }
                _ if chunk_index == 0 => {
                    return Err(invalid("first chunk is not JSON"));
                }
                _ => {}
            }
            offset = chunk_end;
            chunk_index += 1;
        }

        let json = json.ok_or_else(|| invalid("JSON chunk is missing"))?;
        Ok(Self {
            json,
            bin: bin.unwrap_or_default(),
        })
    }

    /// Node names in document order, omitting unnamed nodes.
    #[must_use]
    pub fn node_names(&self) -> Vec<&str> {
        self.array("nodes")
            .into_iter()
            .flatten()
            .filter_map(|node| node.get("name").and_then(Value::as_str))
            .collect()
    }

    /// The `extras` object on the first node named `name`.
    #[must_use]
    pub fn node_extras(&self, name: &str) -> Option<&Map<String, Value>> {
        self.array("nodes")?
            .iter()
            .find(|node| node.get("name").and_then(Value::as_str) == Some(name))?
            .get("extras")?
            .as_object()
    }

    /// Material names in document order, omitting unnamed materials.
    #[must_use]
    pub fn material_names(&self) -> Vec<&str> {
        self.array("materials")
            .into_iter()
            .flatten()
            .filter_map(|material| material.get("name").and_then(Value::as_str))
            .collect()
    }

    /// Union of bounds declared by referenced `POSITION` accessors.
    ///
    /// These are the coordinates stored in mesh accessors. Node transforms
    /// and the optional scene-root coordinate conversion are intentionally
    /// not applied, matching the exporter's local-accessor convention.
    #[must_use]
    pub fn position_bounds(&self) -> Option<([f64; 3], [f64; 3])> {
        let accessors = self.array("accessors")?;
        let mut bounds: Option<([f64; 3], [f64; 3])> = None;
        for primitive in self.primitives() {
            let Some(index) = primitive
                .get("attributes")
                .and_then(|attributes| attributes.get("POSITION"))
                .and_then(Value::as_u64)
                .and_then(|index| usize::try_from(index).ok())
            else {
                continue;
            };
            let Some(accessor) = accessors.get(index) else {
                continue;
            };
            let (Some(min), Some(max)) = (
                accessor.get("min").and_then(json_vec3),
                accessor.get("max").and_then(json_vec3),
            ) else {
                continue;
            };
            if min.iter().zip(max).any(|(&min, max)| min > max) {
                continue;
            }
            match &mut bounds {
                Some((bounds_min, bounds_max)) => {
                    for axis in 0..3 {
                        bounds_min[axis] = bounds_min[axis].min(min[axis]);
                        bounds_max[axis] = bounds_max[axis].max(max[axis]);
                    }
                }
                None => bounds = Some((min, max)),
            }
        }
        bounds
    }

    /// Triangle count declared by index accessors of triangle primitives.
    ///
    /// This describes meshes physically present in the GLB. Repeated nodes
    /// that instance one mesh do not multiply the count. Non-indexed and
    /// non-triangle primitives are outside this exporter's output and are
    /// ignored. A malformed total saturates at `u64::MAX` rather than
    /// panicking in test inspection.
    #[must_use]
    pub fn triangle_count(&self) -> u64 {
        let Some(accessors) = self.array("accessors") else {
            return 0;
        };
        self.primitives().fold(0_u64, |total, primitive| {
            let triangle_mode = primitive.get("mode").and_then(Value::as_u64);
            if !matches!(triangle_mode, None | Some(4)) {
                return total;
            }
            let count = primitive
                .get("indices")
                .and_then(Value::as_u64)
                .and_then(|index| usize::try_from(index).ok())
                .and_then(|index| accessors.get(index))
                .and_then(|accessor| accessor.get("count"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            total.saturating_add(count / 3)
        })
    }

    /// Parsed JSON document for assertions beyond the semantic helpers.
    #[must_use]
    pub fn json(&self) -> &Value {
        &self.json
    }

    /// Raw BIN chunk payload, including its container padding.
    #[must_use]
    pub fn bin(&self) -> &[u8] {
        &self.bin
    }

    fn array(&self, key: &str) -> Option<&Vec<Value>> {
        self.json.get(key)?.as_array()
    }

    fn primitives(&self) -> impl Iterator<Item = &Value> {
        self.array("meshes")
            .into_iter()
            .flatten()
            .filter_map(|mesh| mesh.get("primitives").and_then(Value::as_array))
            .flatten()
    }
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("caller checked the four-byte field"),
    )
}

fn invalid(reason: &'static str) -> GltfError {
    GltfError::InvalidGlb { reason }
}

fn json_vec3(value: &Value) -> Option<[f64; 3]> {
    let values = value.as_array()?;
    let [x, y, z] = values.as_slice() else {
        return None;
    };
    Some([x.as_f64()?, y.as_f64()?, z.as_f64()?])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_glb(json: &[u8]) -> Vec<u8> {
        let mut json = json.to_vec();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        let total = HEADER_LENGTH + CHUNK_HEADER_LENGTH + json.len();
        let mut bytes = Vec::with_capacity(total);
        bytes.extend_from_slice(b"glTF");
        bytes.extend_from_slice(&2_u32.to_le_bytes());
        bytes.extend_from_slice(&u32::try_from(total).unwrap().to_le_bytes());
        bytes.extend_from_slice(&u32::try_from(json.len()).unwrap().to_le_bytes());
        bytes.extend_from_slice(b"JSON");
        bytes.extend_from_slice(&json);
        bytes
    }

    #[test]
    fn parser_rejects_broken_container_boundaries() {
        // Container checks must fail before semantic JSON queries can hide a
        // truncated payload or a header that names a different byte length.
        let valid = minimal_glb(br#"{"asset":{"version":"2.0"}}"#);
        let mut wrong_magic = valid.clone();
        wrong_magic[0] = b'G';
        let mut wrong_length = valid.clone();
        wrong_length[8..12].copy_from_slice(&12_u32.to_le_bytes());
        let truncated = &valid[..valid.len() - 1];

        for bytes in [&wrong_magic[..], &wrong_length[..], truncated] {
            assert!(matches!(
                GlbDocument::parse(bytes),
                Err(GltfError::InvalidGlb { .. })
            ));
        }
    }

    #[test]
    fn parser_distinguishes_invalid_json_from_invalid_layout() {
        // Callers can report malformed JSON separately from a corrupt GLB
        // chunk table, which makes failed export fixtures actionable.
        let bytes = minimal_glb(b"not json");
        assert!(matches!(
            GlbDocument::parse(&bytes),
            Err(GltfError::InvalidGlbJson { .. })
        ));
    }
}
