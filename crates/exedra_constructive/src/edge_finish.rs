// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Stable edge intent and attribute mapping for the mesh rounder.
//!
//! Finishing runs in the child's local coordinates before outer placement.
//! This module owns recipe-facing selection and provenance; the mesh kernel
//! owns geometric planning and atomic topology changes.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;
use core::fmt;

use exedra_mesh::{FaceId, HalfEdgeId, RoundError, RoundStats, attr, round_edges};
pub use exedra_mesh::{RoundKind, RoundPolicy};

use crate::source_map::{SourceMap, StaleSourceMap};
use crate::tessellate::{Feature, TessellatedBody};

#[cfg(test)]
#[path = "edge_finish_tests.rs"]
mod tests;

/// Semantic boundaries to finish on one body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EdgeSelection {
    /// Every edge meeting [`RoundPolicy::sharpness_threshold`].
    SharpEdges,
    /// Boundaries between distinct `FACE_REGION` values.
    ///
    /// Each pair must resolve to one connected boundary with one source
    /// feature on each side. Reused region numbers across Boolean operands
    /// can be ambiguous and are refused. Pair order, duplicates and the order
    /// of the two regions do not affect the result or recipe fingerprint.
    RegionBoundaries(Vec<[u32; 2]>),
}

impl EdgeSelection {
    pub(crate) fn canonicalize(&mut self) {
        if let Self::RegionBoundaries(pairs) = self {
            for pair in pairs.iter_mut() {
                pair.sort_unstable();
            }
            pairs.sort_unstable();
            pairs.dedup();
        }
    }

    pub(crate) fn valid(&self) -> bool {
        match self {
            Self::SharpEdges => true,
            Self::RegionBoundaries(pairs) => {
                !pairs.is_empty() && pairs.iter().all(|p| p[0] != p[1])
            }
        }
    }
}

/// A refused edge finish. Input geometry is unchanged on every error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EdgeFinishError {
    /// A boundary list is empty or names the same region on both sides.
    InvalidSelection,
    /// No edge matches a requested boundary or the sharpness threshold.
    EmptySelection,
    /// Region labels identify disconnected boundaries or different source features.
    AmbiguousSelection {
        /// Region pair that could not identify one boundary.
        regions: [u32; 2],
    },
    /// Provenance does not describe the current input mesh.
    StaleSourceMap(StaleSourceMap),
    /// The selected geometry is outside the mesh rounder's supported envelope.
    Round(RoundError),
}

impl fmt::Display for EdgeFinishError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSelection => {
                f.write_str("edge boundaries must contain distinct region pairs")
            }
            Self::EmptySelection => f.write_str("edge finish selected no edges"),
            Self::AmbiguousSelection { regions } => write!(
                f,
                "regions {regions:?} identify more than one source boundary"
            ),
            Self::StaleSourceMap(error) => error.fmt(f),
            Self::Round(error) => error.fmt(f),
        }
    }
}
impl core::error::Error for EdgeFinishError {}

impl EdgeFinishError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::InvalidSelection => "eval.edge_finish.invalid_selection",
            Self::EmptySelection => "eval.edge_finish.empty_selection",
            Self::AmbiguousSelection { .. } => "eval.edge_finish.ambiguous_selection",
            Self::StaleSourceMap(_) => "eval.edge_finish.stale_source_map",
            Self::Round(error) => match error {
                RoundError::InvalidPolicy { .. } => "eval.edge_finish.invalid_policy",
                RoundError::ConcaveEdge { .. } => "eval.edge_finish.concave_edge",
                RoundError::ClearanceExceeded { .. } => "eval.edge_finish.clearance_exceeded",
                RoundError::BoundaryEdge { .. } => "eval.edge_finish.boundary_edge",
                RoundError::NonPlanarFace { .. } => "eval.edge_finish.non_planar_face",
                RoundError::DegenerateEdge { .. } => "eval.edge_finish.degenerate_edge",
                RoundError::UnsupportedJunction { .. } => "eval.edge_finish.unsupported_junction",
                RoundError::UnsupportedEnd { .. } => "eval.edge_finish.unsupported_end",
                RoundError::UnsupportedTopology { .. } => "eval.edge_finish.unsupported_topology",
                RoundError::InvalidEdge { .. } | RoundError::Internal { .. } => {
                    "eval.edge_finish.internal"
                }
            },
        }
    }
}

fn targets(
    body: &TessellatedBody,
    selection: &EdgeSelection,
    policy: &RoundPolicy,
) -> Result<Vec<HalfEdgeId>, EdgeFinishError> {
    if !selection.valid() {
        return Err(EdgeFinishError::InvalidSelection);
    }
    let mesh = &body.mesh;
    let edges: BTreeSet<_> = mesh
        .faces()
        .flat_map(|f| mesh.face_loop(f))
        .filter_map(|e| mesh.canonical_edge(e))
        .collect();
    let selected: Vec<_> = match selection {
        EdgeSelection::SharpEdges => edges
            .into_iter()
            .filter(|&e| mesh.edge_sharpness(e).unwrap_or(0.0) >= policy.sharpness_threshold)
            .collect(),
        EdgeSelection::RegionBoundaries(pairs) => {
            let mut selected = BTreeSet::new();
            let region = |face: FaceId| {
                mesh.attrs()
                    .dense(attr::FACE_REGION)
                    .and_then(|layer| layer.get(face.as_id()).copied())
            };
            for pair in pairs {
                let mut pair = *pair;
                pair.sort_unstable();
                let mut features = BTreeSet::new();
                let mut adjacency = BTreeMap::<_, Vec<_>>::new();
                for &edge in &edges {
                    let faces = [
                        mesh.face(edge).unwrap(),
                        mesh.face(mesh.twin(edge).unwrap()).unwrap(),
                    ];
                    let regions = [region(faces[0]), region(faces[1])];
                    let ordered = if regions == pair.map(Some) {
                        faces
                    } else if regions == [Some(pair[1]), Some(pair[0])] {
                        [faces[1], faces[0]]
                    } else {
                        continue;
                    };
                    features.insert(
                        ordered
                            .map(|f| body.source_map.face_feature(f).expect("current source map")),
                    );
                    selected.insert(edge);
                    let (a, b) = (
                        mesh.from_vertex(edge).unwrap(),
                        mesh.to_vertex(edge).unwrap(),
                    );
                    adjacency.entry(a).or_default().push(b);
                    adjacency.entry(b).or_default().push(a);
                }
                let Some(&start) = adjacency.keys().next() else {
                    return Err(EdgeFinishError::EmptySelection);
                };
                let mut pending = alloc::vec![start];
                let mut reached = BTreeSet::new();
                while let Some(vertex) = pending.pop() {
                    if reached.insert(vertex) {
                        pending.extend(&adjacency[&vertex]);
                    }
                }
                if features.len() != 1 || reached.len() != adjacency.len() {
                    return Err(EdgeFinishError::AmbiguousSelection { regions: pair });
                }
            }
            selected.into_iter().collect()
        }
    };
    if selected.is_empty() {
        return Err(EdgeFinishError::EmptySelection);
    }
    Ok(selected)
}

/// Finishes semantic edges on one tessellated body without modifying the input.
///
/// Existing face slots and source features survive. New bands and patches
/// inherit the first source face in ascending input-ID order; `policy.region`
/// can override their geometric region. An occurrence's default material
/// remains external to this sparse face-slot map.
///
/// New and rewritten faces have no UVs. Unchanged faces retain theirs. Fillet
/// bands and corners author radial normals for `CustomOrDerived` extraction;
/// chamfers and transverse end boundaries remain hard.
///
/// # Errors
/// Refuses empty or ambiguous selection, stale provenance, and unsupported
/// mesh geometry. There is no fallback to profile rounding or a reduced radius.
pub fn finish_edges(
    body: &TessellatedBody,
    selection: &EdgeSelection,
    policy: &RoundPolicy,
) -> Result<(TessellatedBody, RoundStats), EdgeFinishError> {
    body.source_map
        .check(&body.mesh)
        .map_err(EdgeFinishError::StaleSourceMap)?;
    policy.validate().map_err(EdgeFinishError::Round)?;
    let selected = targets(body, selection, policy)?;
    let mut mesh = body.mesh.clone();
    let result = round_edges(&mut mesh, &selected, policy).map_err(EdgeFinishError::Round)?;
    let owners: BTreeMap<_, _> = result
        .face_provenance
        .iter()
        .map(|(face, source)| (*face, source.faces()[0]))
        .collect();
    let mut face_materials = BTreeMap::new();
    let mut features = BTreeMap::new();
    for face in mesh.faces() {
        let owner = owners.get(&face).copied().unwrap_or(face);
        features.insert(
            face,
            body.source_map.face_feature(owner).expect("source face"),
        );
        if let Some(&slot) = body.face_materials.get(&owner) {
            face_materials.insert(face, slot);
        }
    }
    let mut vertex_owners = BTreeMap::<_, Feature>::new();
    for face in mesh.faces() {
        for edge in mesh.face_loop(face) {
            let feature = features[&face];
            vertex_owners
                .entry(mesh.to_vertex(edge).unwrap())
                .and_modify(|f| *f = (*f).min(feature))
                .or_insert(feature);
        }
    }
    let face_features = mesh.faces().map(|f| features[&f]).collect();
    let vertex_features = mesh
        .vertices()
        .map(|v| {
            body.source_map
                .vertex_feature(v)
                .unwrap_or_else(|| vertex_owners[&v])
        })
        .collect();
    let source_map = SourceMap::new(&mesh, face_features, vertex_features);
    Ok((
        TessellatedBody {
            mesh,
            source_map,
            face_materials,
            refinement: None,
        },
        result.stats,
    ))
}
