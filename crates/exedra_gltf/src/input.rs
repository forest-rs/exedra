// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! One-shot export records. These are not an authored scene or a geometry store.

use crate::GltfError;
use exedra_compile::{CompilationMismatch, CompiledParts, PartFingerprint, PartId};
use exedra_constructive::ir::Placement3;
use std::{borrow::Cow, sync::Arc};

/// JSON values for caller-owned glTF extras.
pub use serde_json::Value as JsonValue;
/// Arbitrary JSON metadata copied into an exported node.
pub type GltfExtras = serde_json::Map<String, JsonValue>;

/// Caller-prepared glTF hierarchy and placement batches.
///
/// Indices refer only to this export. Geometry stays in the supplied compilation.
/// This record has no part registry, editing, composition, scheduling or persistent
/// identity contract. Resolve caller structure and material bindings before export.
#[derive(Clone, Debug, Default)]
pub struct GltfScene {
    /// Logical nodes in deterministic output order; parents may appear later.
    pub nodes: Vec<GltfNode>,
    /// Explicit repeated placements, independent of logical node registration.
    pub placement_sets: Vec<GltfPlacementSet>,
}

/// One logical glTF node, including geometry-free frames.
#[derive(Clone, Debug)]
pub struct GltfNode {
    /// Caller-owned display name; no imposed path grammar.
    pub name: String,
    /// Index of the parent in [`GltfScene::nodes`], or a scene root.
    pub parent: Option<usize>,
    /// Local transform relative to the parent.
    pub transform: Placement3,
    /// Geometry and resolved appearance, or a geometry-free frame.
    pub geometry: Option<GltfGeometry>,
    /// Metadata preserved on this logical node.
    pub extras: GltfExtras,
}
impl GltfNode {
    /// Creates an identity frame with no parent, geometry or metadata.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            parent: None,
            transform: Placement3::IDENTITY,
            geometry: None,
            extras: GltfExtras::new(),
        }
    }
}

/// Compiled geometry reference and resolved materials for one export occurrence.
#[derive(Clone, Debug)]
pub struct GltfMesh {
    /// Index in the supplied compilation, not a persistent caller identity.
    pub part: PartId,
    /// Captured geometry content identity; mismatches are refused.
    pub fingerprint: PartFingerprint,
    /// Caller-owned geometry name, used on auxiliary body nodes.
    pub name: String,
    /// Material keys per body, then per compiled region range, in buffer order.
    /// An empty outer list, or an empty body's list, leaves those ranges unbound.
    /// Nonempty lists must match the complete body/range counts exactly.
    pub materials: Vec<Vec<Option<String>>>,
}
impl GltfMesh {
    /// References one compiled input without material bindings.
    ///
    /// # Errors
    /// Returns [`GltfError::InvalidInput`] for an absent input handle.
    pub fn new(
        part: PartId,
        name: impl Into<String>,
        compiled: &CompiledParts,
    ) -> Result<Self, GltfError> {
        let entry = compiled
            .part(part)
            .ok_or_else(|| invalid("mesh.part", "unknown compiled input"))?;
        Ok(Self {
            part,
            fingerprint: entry.fingerprint,
            name: name.into(),
            materials: Vec::new(),
        })
    }
    pub(crate) fn material(&self, body: usize, range: usize) -> Option<&str> {
        self.materials
            .get(body)
            .and_then(|ranges| ranges.get(range))
            .and_then(Option::as_deref)
    }
}

/// Geometry of a node or placement set, with an optional explicit LOD chain.
#[derive(Clone, Debug)]
pub struct GltfGeometry {
    /// Geometry emitted when LOD output is disabled.
    pub mesh: GltfMesh,
    /// Finest-to-coarsest levels; the first must match [`Self::mesh`].
    /// Empty means no chain. Thresholds decrease strictly; fade bands may not
    /// cross the next threshold or zero. Levels cannot nest.
    pub lods: Vec<GltfLod>,
}
impl From<GltfMesh> for GltfGeometry {
    fn from(mesh: GltfMesh) -> Self {
        Self {
            mesh,
            lods: Vec::new(),
        }
    }
}

/// One explicit level in an occurrence's glTF LOD chain.
#[derive(Clone, Debug)]
pub struct GltfLod {
    /// Geometry and resolved materials for this level.
    pub mesh: GltfMesh,
    /// Minimum projected screen coverage; finite and nonnegative.
    pub min_coverage: f32,
    /// Fade band below the threshold; finite and nonnegative.
    pub crossfade: f32,
}

/// One export occurrence repeated at explicit local transforms.
#[derive(Clone, Debug)]
pub struct GltfPlacementSet {
    /// Caller-owned batch name; individual nodes use `name#index`.
    pub name: String,
    /// Parent logical node, or a scene root.
    pub parent: Option<usize>,
    /// Compiled geometry and fully resolved material bindings.
    pub geometry: GltfGeometry,
    /// Local transforms, in caller order. Empty sets emit no nodes.
    pub transforms: Arc<[Placement3]>,
    /// Optional per-placement seeds, with the same length as transforms.
    pub seeds: Option<Arc<[u64]>>,
    /// Optional finite linear RGBA tints, with the same length as transforms.
    pub tints: Option<Arc<[[f32; 4]]>>,
    /// Caller-owned batch metadata.
    pub extras: GltfExtras,
}

/// Adapter from caller-owned structure to one-shot glTF records.
///
/// Implementations may borrow an already prepared [`GltfScene`] or return owned
/// records. Preparation must not evaluate geometry. The exporter validates every
/// record independently, including hierarchy, fingerprints and material ranges.
pub trait GltfSource {
    /// Prepares export structure and resolves appearance against existing buffers.
    ///
    /// # Errors
    /// Reports caller/source correspondence failures before encoding output.
    fn gltf_scene<'a>(&'a self, compiled: &CompiledParts) -> Result<Cow<'a, GltfScene>, GltfError>;
}
impl GltfSource for GltfScene {
    fn gltf_scene<'a>(&'a self, _: &CompiledParts) -> Result<Cow<'a, GltfScene>, GltfError> {
        Ok(Cow::Borrowed(self))
    }
}

pub(crate) fn invalid(path: impl Into<String>, reason: &'static str) -> GltfError {
    GltfError::InvalidInput {
        path: path.into(),
        reason,
    }
}

impl GltfScene {
    pub(crate) fn validate(&self, compiled: &CompiledParts) -> Result<(), GltfError> {
        // Iterative coloring detects cycles in linear work, including arbitrary
        // parent order, without recursion or a caller-controlled stack depth.
        let mut state = vec![0_u8; self.nodes.len()];
        let mut checked = vec![false; compiled.parts().len()];
        for (index, node) in self.nodes.iter().enumerate() {
            self.validate_parent(node.parent, &format!("nodes[{index}].parent"))?;
            validate_transform(&node.transform, &format!("nodes[{index}].transform"))?;
            if let Some(geometry) = &node.geometry {
                validate_geometry(geometry, compiled, &mut checked)?;
            }
        }
        for start in 0..self.nodes.len() {
            let mut path = Vec::new();
            let mut current = Some(start);
            while let Some(index) = current {
                match state[index] {
                    2 => break,
                    1 => {
                        return Err(invalid(
                            format!("nodes[{index}].parent"),
                            "cyclic hierarchy",
                        ));
                    }
                    _ => {
                        state[index] = 1;
                        path.push(index);
                        current = self.nodes[index].parent;
                    }
                }
            }
            for index in path {
                state[index] = 2;
            }
        }
        for (index, set) in self.placement_sets.iter().enumerate() {
            let path = format!("placement_sets[{index}]");
            self.validate_parent(set.parent, &format!("{path}.parent"))?;
            validate_geometry(&set.geometry, compiled, &mut checked)?;
            for transform in &*set.transforms {
                validate_transform(transform, &format!("{path}.transforms"))?;
            }
            if set
                .seeds
                .as_ref()
                .is_some_and(|v| v.len() != set.transforms.len())
            {
                return Err(invalid(format!("{path}.seeds"), "placement count mismatch"));
            }
            if let Some(tints) = &set.tints {
                if tints.len() != set.transforms.len() {
                    return Err(invalid(format!("{path}.tints"), "placement count mismatch"));
                }
                if tints.iter().flatten().any(|v| !v.is_finite()) {
                    return Err(invalid(format!("{path}.tints"), "nonfinite tint"));
                }
            }
        }
        Ok(())
    }
    fn validate_parent(&self, parent: Option<usize>, path: &str) -> Result<(), GltfError> {
        if parent.is_some_and(|p| p >= self.nodes.len()) {
            return Err(invalid(path, "unknown parent node"));
        }
        Ok(())
    }
}
fn validate_transform(transform: &Placement3, path: &str) -> Result<(), GltfError> {
    if transform.rows.iter().flatten().any(|v| !v.is_finite()) {
        return Err(invalid(path, "nonfinite transform"));
    }
    Ok(())
}
fn validate_mesh(
    mesh: &GltfMesh,
    compiled: &CompiledParts,
    checked: &mut [bool],
) -> Result<(), GltfError> {
    let path = format!("geometry[{}]", mesh.part.0);
    let entry = compiled
        .part(mesh.part)
        .ok_or_else(|| invalid(&path, "unknown compiled input"))?;
    if entry.fingerprint != mesh.fingerprint {
        return Err(GltfError::CompilationMismatch(
            CompilationMismatch::PartContent {
                part: mesh.part,
                expected: mesh.fingerprint,
                actual: entry.fingerprint,
            },
        ));
    }
    if !mesh.materials.is_empty() && mesh.materials.len() != entry.bodies.len() {
        return Err(invalid(format!("{path}.materials"), "body count mismatch"));
    }
    for (index, body) in entry.bodies.iter().enumerate() {
        if let Some(ranges) = mesh.materials.get(index)
            && !ranges.is_empty()
            && ranges.len() != body.regions.len()
        {
            return Err(invalid(
                format!("{path}.materials[{index}]"),
                "region count mismatch",
            ));
        }
        if !checked[mesh.part.0 as usize]
            && body
                .tri
                .positions
                .iter()
                .flatten()
                .chain(body.tri.normals.iter().flatten())
                .chain(body.tri.uvs.iter().flatten())
                .any(|v| !v.is_finite())
        {
            return Err(invalid(&path, "nonfinite geometry stream"));
        }
    }
    checked[mesh.part.0 as usize] = true;
    Ok(())
}
fn validate_geometry(
    geometry: &GltfGeometry,
    compiled: &CompiledParts,
    checked: &mut [bool],
) -> Result<(), GltfError> {
    validate_mesh(&geometry.mesh, compiled, checked)?;
    let chain = &geometry.lods;
    if let Some(first) = chain.first()
        && (first.mesh.part != geometry.mesh.part
            || first.mesh.fingerprint != geometry.mesh.fingerprint
            || first.mesh.materials != geometry.mesh.materials)
    {
        return Err(invalid(
            "geometry.lods[0]",
            "first level must match base geometry and materials",
        ));
    }
    for (index, level) in chain.iter().enumerate() {
        validate_mesh(&level.mesh, compiled, checked)?;
        let floor = chain.get(index + 1).map_or(0.0, |next| next.min_coverage);
        if !level.min_coverage.is_finite()
            || level.min_coverage < 0.0
            || !level.crossfade.is_finite()
            || level.crossfade < 0.0
            || f64::from(level.min_coverage) - f64::from(level.crossfade) < f64::from(floor)
            || chain
                .get(index + 1)
                .is_some_and(|next| next.min_coverage >= level.min_coverage)
        {
            return Err(invalid(
                format!("geometry.lods[{index}]"),
                "invalid coverage or crossfade",
            ));
        }
        if chain[..index]
            .iter()
            .any(|previous| previous.mesh.part == level.mesh.part)
        {
            return Err(invalid(
                format!("geometry.lods[{index}]"),
                "repeated level geometry",
            ));
        }
    }
    Ok(())
}
