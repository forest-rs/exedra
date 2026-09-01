// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exact and mesh-backed realization of constructive stretch.

mod exact;
mod mesh;

pub(crate) use exact::exact_plan;
pub(crate) use mesh::stretch_mesh;

#[cfg(test)]
include!("tests.rs");

/// Why a structurally recognized stretch cannot produce valid geometry.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum StretchRefusal {
    /// Contraction leaves no stationary and movable section to re-stitch.
    ContractionConsumesHalf,
    /// The requested contraction would collapse or invert an extent.
    ContractionCollapsesExtent,
    /// An ancestor transform cannot carry a plane into evaluated mesh space.
    SingularTransform,
    /// The section touches a stored vertex or lies on a face/edge.
    AmbiguousContact,
    /// One source face contributes more than one segment to a section.
    DisconnectedFaceSection,
    /// A crossing shell has an open boundary.
    OpenShell,
    /// Section segments do not form disjoint closed loops.
    NonManifoldSection,
    /// A generated mesh violates Exedra topology invariants.
    BuildFailed,
    /// General mesh contraction has sections that cannot be re-stitched.
    IncompatibleContraction,
}

impl StretchRefusal {
    pub(crate) const fn code(self) -> &'static str {
        match self {
            Self::ContractionConsumesHalf => "eval.stretch.contraction_half",
            Self::ContractionCollapsesExtent => "eval.stretch.contraction_extent",
            Self::SingularTransform => "eval.stretch.singular_transform",
            Self::AmbiguousContact => "eval.stretch.ambiguous_contact",
            Self::DisconnectedFaceSection => "eval.stretch.disconnected_face_section",
            Self::OpenShell => "eval.stretch.open_shell",
            Self::NonManifoldSection => "eval.stretch.non_manifold_section",
            Self::BuildFailed => "eval.stretch.build",
            Self::IncompatibleContraction => "eval.stretch.contraction_stitch",
        }
    }

    pub(crate) const fn message(self) -> &'static str {
        match self {
            Self::ContractionConsumesHalf => {
                "contraction does not leave both a rigid half and a movable half to re-stitch"
            }
            Self::ContractionCollapsesExtent => {
                "contraction would collapse or invert the stretched extent"
            }
            Self::SingularTransform => {
                "the stretch plane cannot pass through a singular ancestor transform"
            }
            Self::AmbiguousContact => {
                "the stretch plane touches a stored vertex, edge, or coplanar face"
            }
            Self::DisconnectedFaceSection => {
                "one source face crosses the stretch plane in multiple disjoint segments"
            }
            Self::OpenShell => "an open shell crosses the stretch plane",
            Self::NonManifoldSection => {
                "the stretch section does not form disjoint closed manifold loops"
            }
            Self::BuildFailed => "the stretched faces could not be rebuilt as a valid mesh",
            Self::IncompatibleContraction => {
                "the two contraction sections are not translation-compatible"
            }
        }
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct MeshStretchStats {
    pub(crate) split_faces: u64,
    pub(crate) band_faces: u64,
    pub(crate) uv_unmapped_faces: u64,
}
