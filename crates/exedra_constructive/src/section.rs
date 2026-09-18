// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Plane sections and capped cuts of evaluated, closed meshes.
//!
//! Cuts follow robust face triangulation, including the chosen diagonals of
//! nonplanar faces. They do not reconstruct an analytic surface. Planes use the
//! body's coordinates. Contacts within tolerance are refused rather than snapped.
//! Section frames are deterministic and right-handed; local +Z is the plane normal.
//! Omitted collinear face-boundary samples are restored only when collinear in
//! 3D, preserving the triangulated surface. Both original and f32-realized
//! section loops must retain separation, winding, and nesting.
//!
//! ```
//! use exedra_constructive::{
//!     builders::rect,
//!     ir::{CapMode, Placement3, Plane3},
//!     section::{CutCap, SectionPolicy, split_body},
//!     tessellate::{EvalPolicy, tessellate_extrude},
//! };
//! let body = tessellate_extrude(&rect(2.0, 3.0)?, &Placement3::IDENTITY,
//!     4.0, CapMode::Both, &EvalPolicy::default())?;
//! let cut = split_body(&body,
//!     Plane3 { normal: [0.3, -0.2, 1.0], distance: 1.71 },
//!     &SectionPolicy::default(), CutCap { region: 100, material: None })?;
//! assert!(cut.negative.is_some() && cut.positive.is_some());
//! assert_eq!(cut.section.regions.len(), 1);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use crate::ir::{Placement3, Plane3, SlotId};
use crate::profile_section::plane::{check_frame, convert_region};
use crate::profile_section::{SectionProfile, SectionProfileError};
use crate::tessellate::{Feature, TessellatedBody};
use alloc::vec::Vec;
use exedra_mesh_ops::section as geometry;

pub use geometry::{SectionPolicy, SectionStats};

/// Authored attributes of newly generated cap faces.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct CutCap {
    /// Region assigned to every cap triangle. Choose a value distinct from input
    /// regions when region-boundary selection should identify the rim.
    pub region: u32,
    /// Optional material-slot override. `None` inherits the body's occurrence material.
    pub material: Option<SlotId>,
}

/// One oriented section boundary in section-frame XY coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct SectionLoop {
    /// Cyclic points, without a duplicate closing point; CCW outer, CW hole.
    pub points: Vec<[f64; 2]>,
    /// Source face feature for each edge from point `i` to point `(i+1) % len`.
    pub edge_features: Vec<Feature>,
}
/// One connected filled section, with its directly enclosed holes.
#[derive(Clone, Debug, PartialEq)]
pub struct SectionRegion {
    /// Counter-clockwise outer boundary.
    pub outer: SectionLoop,
    /// Clockwise hole boundaries.
    pub holes: Vec<SectionLoop>,
}
/// A section of the triangulated input surface; empty when the plane misses it.
#[derive(Clone, Debug, PartialEq)]
pub struct PlaneSection {
    /// Maps section-local XY coordinates into body coordinates.
    pub frame: Placement3,
    /// Disconnected filled regions in deterministic boundary order.
    pub regions: Vec<SectionRegion>,
    /// Work and numerical realization evidence, not a self-intersection certificate.
    pub stats: SectionStats,
}
impl PlaneSection {
    /// Converts each filled region into a reusable profile with its placement
    /// and per-segment construction features. An empty section gives no profiles.
    ///
    /// Holes, region order, cyclic starting points and all boundary samples are
    /// preserved. No mesh traversal, tessellation, curve fitting or automatic
    /// loft correspondence occurs. Segment tags index the returned source table;
    /// they are not persistent selectors for a subsequently edited body.
    ///
    /// As with [`Self::measure`], callers editing section fields must preserve
    /// simplicity, separation and nesting. Conversion performs ordinary profile
    /// construction checks, not a fresh certification of section topology.
    /// See [`profiles_from_mesh_section`](crate::profile_section::profiles_from_mesh_section)
    /// for conversion from plain mesh sections.
    ///
    /// ```
    /// # use exedra_constructive::{builders::rect, ir::{CapMode, Placement3, Plane3},
    /// # section::{section_body, SectionPolicy}, tessellate::{EvalPolicy, tessellate_extrude}};
    /// # let policy = EvalPolicy::default();
    /// # let body = tessellate_extrude(&rect(4.0, 3.0)?, &Placement3::IDENTITY,
    /// #     2.0, CapMode::Both, &policy)?;
    /// # let section = section_body(&body,
    /// #     Plane3 { normal: [0.2, 0.0, 1.0], distance: 1.0 }, &SectionPolicy::default())?;
    /// use exedra_constructive::ir::{NodeKind, RecipeBuilder};
    /// for converted in section.to_profiles()? {
    ///     let mut builder = RecipeBuilder::new();
    ///     let profile = builder.add_profile(converted.profile);
    ///     let extrusion = builder.add(NodeKind::Extrude {
    ///         profile, placement: converted.placement, height: 0.5, caps: CapMode::Both,
    ///     })?;
    ///     let recipe = builder.finish(extrusion)?;
    ///     // Evaluate or serialize this ordinary recipe independently of the source body.
    /// }
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    /// Refuses invalid frames, malformed source arrays, degenerate polygon area,
    /// exhausted segment tags, and profile construction failures.
    pub fn to_profiles(&self) -> Result<Vec<SectionProfile<Feature>>, SectionProfileError> {
        check_frame(self.frame)?;
        self.regions
            .iter()
            .enumerate()
            .map(|(index, region)| {
                convert_region(
                    self.frame,
                    index,
                    (&region.outer.points, &region.outer.edge_features),
                    region
                        .holes
                        .iter()
                        .map(|hole| (hole.points.as_slice(), hole.edge_features.as_slice())),
                )
            })
            .collect()
    }

    /// Measures all filled regions in this section's local XY frame.
    ///
    /// Holes subtract area and add perimeter; disconnected regions contribute
    /// together. An empty section has zero area/perimeter and no centroid/bounds.
    /// Work is linear in the stored boundary edges without allocation or renewed
    /// tessellation. The section policy bounds generated boundaries.
    ///
    /// Measures the original intersection coordinates stored here, before any
    /// f32 narrowing of a split body's cap. This is not an analytic measurement.
    ///
    /// Because section fields are public, callers editing them must preserve
    /// simple, nonoverlapping regions and correctly nested holes. This query
    /// checks finite arithmetic, degeneracy and winding, not crossings/nesting.
    /// Source features and the frame remain on this section.
    ///
    /// # Errors
    /// Rejects invalid oriented loops, nonpositive net area for nonempty input,
    /// and unrepresentable arithmetic.
    pub fn measure(
        &self,
    ) -> Result<crate::measure::PlanarMeasurements, crate::measure::MeasurementError> {
        crate::measure::measure(self.regions.iter().flat_map(|region| {
            core::iter::once((region.outer.points.as_slice(), false)).chain(
                region
                    .holes
                    .iter()
                    .map(|hole| (hole.points.as_slice(), true)),
            )
        }))
    }
}
/// Both closed sides of a plane cut, sharing the same realized section vertices.
#[derive(Debug)]
pub struct PlaneSplit {
    /// `dot(normal, point) < distance`, capped toward the positive side.
    /// `None` when this half contains no input geometry.
    pub negative: Option<TessellatedBody>,
    /// `dot(normal, point) > distance`, capped toward the negative side.
    /// `None` when this half contains no input geometry.
    pub positive: Option<TessellatedBody>,
    /// Section geometry and construction evidence.
    pub section: PlaneSection,
}
/// Explicit refusal of a plane operation; no partial result is returned.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SectionError {
    /// Invalid plane, tolerance, or zero work budget.
    InvalidPolicy,
    /// The source map no longer describes the mesh.
    StaleSourceMap,
    /// Input topology is empty, invalid, or open.
    InvalidMesh,
    /// A stored input vertex lies within the contact tolerance of the plane.
    AmbiguousContact,
    /// Robust face triangulation cannot represent an input face.
    Triangulation,
    /// A configured work budget would be exceeded.
    BudgetExceeded,
    /// Section boundaries branch, touch, intersect, or have inconsistent winding.
    InvalidSection,
    /// Geometry cannot retain finite coordinates, orientation, or plane accuracy.
    NumericLimit,
    /// Generated topology could not be closed and validated.
    BuildFailed,
}
impl core::fmt::Display for SectionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::InvalidPolicy => "invalid plane-section policy or plane",
            Self::StaleSourceMap => "plane-section source map is stale",
            Self::InvalidMesh => "plane section requires a valid closed nonempty mesh",
            Self::AmbiguousContact => "plane contacts an input vertex within tolerance",
            Self::Triangulation => "plane section could not triangulate an input face",
            Self::BudgetExceeded => "plane-section work budget exceeded",
            Self::InvalidSection => "plane section has unsupported boundary topology or winding",
            Self::NumericLimit => "plane section exceeds numeric realization limits",
            Self::BuildFailed => "plane cut could not construct closed valid topology",
        })
    }
}
impl core::error::Error for SectionError {}

/// Extracts oriented section regions from an evaluated body's triangulated mesh.
///
/// # Errors
/// Refuses invalid/open input, contacts within tolerance, unsupported section
/// topology, exhausted budgets, and unrepresentable geometry. The source map must
/// still match the mesh. Distant self-intersection is not checked.
pub fn section_body(
    source: &TessellatedBody,
    plane: Plane3,
    policy: &SectionPolicy,
) -> Result<PlaneSection, SectionError> {
    source
        .source_map
        .check(&source.mesh)
        .map_err(|_| SectionError::StaleSourceMap)?;
    Ok(bind_section(
        source,
        geometry::section_mesh(&source.mesh, plane, policy).map_err(SectionError::from)?,
    ))
}

/// Splits an evaluated body into two closed, capped halves.
///
/// Surviving surface triangles retain source features, regions, material slots,
/// corner UVs/normals, and original edge seams/sharpness. Caps use `cap` and
/// section-frame XY UVs; cap rims are sharp. New faces/vertices use
/// [`Feature::PlaneCutCap`]/[`Feature::PlaneCutSeam`]. Derived bodies clear source
/// sampling and realization evidence. No solid-validity certificate is implied.
///
/// # Errors
/// Returns [`SectionError`] for the same failures as [`section_body`], cap
/// triangulation failure, or collapsed/open output. Returns no partial halves.
pub fn split_body(
    source: &TessellatedBody,
    plane: Plane3,
    policy: &SectionPolicy,
    cap: CutCap,
) -> Result<PlaneSplit, SectionError> {
    source
        .source_map
        .check(&source.mesh)
        .map_err(|_| SectionError::StaleSourceMap)?;
    let split = geometry::split_mesh(&source.mesh, plane, policy, cap.region)
        .map_err(SectionError::from)?;
    Ok(PlaneSplit {
        negative: split.negative.map(|half| bind_half(source, half, cap)),
        positive: split.positive.map(|half| bind_half(source, half, cap)),
        section: bind_section(source, split.section),
    })
}

fn bind_section(source: &TessellatedBody, section: geometry::PlaneSection) -> PlaneSection {
    let bind_loop = |boundary: geometry::SectionLoop| SectionLoop {
        points: boundary.points,
        edge_features: boundary
            .edge_faces
            .into_iter()
            .map(|face| {
                source
                    .source_map
                    .face_feature(face)
                    .unwrap_or(Feature::Imported)
            })
            .collect(),
    };
    PlaneSection {
        frame: section.frame,
        stats: section.stats,
        regions: section
            .regions
            .into_iter()
            .map(|region| SectionRegion {
                outer: bind_loop(region.outer),
                holes: region.holes.into_iter().map(bind_loop).collect(),
            })
            .collect(),
    }
}

fn bind_half(source: &TessellatedBody, half: geometry::CutMesh, cap: CutCap) -> TessellatedBody {
    let features = half
        .mesh
        .faces()
        .map(|face| match half.face_sources[&face] {
            geometry::CutFaceSource::Original(face) => source
                .source_map
                .face_feature(face)
                .unwrap_or(Feature::Imported),
            geometry::CutFaceSource::Cap => Feature::PlaneCutCap,
        })
        .collect();
    let vertices = half
        .mesh
        .vertices()
        .map(|vertex| match half.vertex_sources[&vertex] {
            geometry::CutVertexSource::Original(vertex) => source
                .source_map
                .vertex_feature(vertex)
                .unwrap_or(Feature::Imported),
            geometry::CutVertexSource::Intersection { .. } => Feature::PlaneCutSeam,
        })
        .collect();
    let face_materials = half
        .face_sources
        .iter()
        .filter_map(|(&output, provenance)| {
            let material = match provenance {
                geometry::CutFaceSource::Original(face) => source.face_materials.get(face).copied(),
                geometry::CutFaceSource::Cap => cap.material,
            };
            material.map(|slot| (output, slot))
        })
        .collect();
    let source_map = crate::source_map::SourceMap::new(&half.mesh, features, vertices);
    TessellatedBody {
        mesh: half.mesh,
        source_map,
        face_materials,
        sweep_checks: None,
        path_sampling: None,
        loft_sampling: None,
        refinement: None,
    }
}

impl From<geometry::SectionError> for SectionError {
    fn from(error: geometry::SectionError) -> Self {
        match error {
            geometry::SectionError::InvalidPolicy => Self::InvalidPolicy,
            geometry::SectionError::InvalidMesh => Self::InvalidMesh,
            geometry::SectionError::AmbiguousContact => Self::AmbiguousContact,
            geometry::SectionError::Triangulation => Self::Triangulation,
            geometry::SectionError::BudgetExceeded => Self::BudgetExceeded,
            geometry::SectionError::InvalidSection => Self::InvalidSection,
            geometry::SectionError::NumericLimit => Self::NumericLimit,
            geometry::SectionError::BuildFailed => Self::BuildFailed,
        }
    }
}

#[cfg(test)]
mod tests;
