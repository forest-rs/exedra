// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `exedra-recipe-v1`: the JSON interchange schema for external frontends.
//!
//! The schema is defined by dedicated DTO types in this module — never by
//! deriving on the internal IR — so internal refactors cannot silently
//! change the wire format. Deserialization rebuilds through
//! [`RecipeBuilder`], re-running every validation; round-trip fingerprint
//! equality is the correctness oracle.
//!
//! Stability policy (ADR-0003):
//! - The header is `{"format": "exedra-recipe", "version": 1}`; readers
//!   reject other formats/versions with a typed error.
//! - Evolution within version 1 is additive-only: new optional fields, node
//!   kinds, and explicitly distinguishable values may appear; existing values
//!   never change meaning. Operand-qualified boundary objects extend numeric
//!   boundary pairs and are rejected by older numeric-only readers.
//!   Unknown *fields* are ignored (additive tolerance); unknown node
//!   *kinds* are hard errors — a recipe is executable content, and
//!   skipping an unknown operation would silently change geometry.
//! - Floats are plain JSON numbers; serde's shortest-round-trip rendering
//!   re-parses to the identical `f64` bits, so content fingerprints
//!   survive the wire exactly.
//! - Curve segments serialize as explicit records, insulating the format
//!   from kurbo's own types and versions.

#![cfg(feature = "serde")]

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use exedra_implicit::{Expr, ProfileExpr};
use serde::{Deserialize, Serialize};

use crate::chart::{ChartError, ChartTransform, LoftStations, SurfaceChart};
use crate::edge_finish::{EdgeSelection, OperandRegion, RoundKind, RoundPolicy};
use crate::ir::{
    CapMode, CsgOp, FramePolicy, Law, LoftPolicy, LoftSection, Node, NodeId, NodeKind, Path3,
    PathClosure, PathJoin, Placement3, Plane3, PrimitiveSpec, ProfileId, Recipe, RecipeBuilder,
    RecipeError, SectionLaw, SlotId, SourceId, VertexStretchStep,
};
use crate::profile::{Loop2, Profile2, ProfileError, Seg2, SegKind, SegTag};

/// Format name in the header.
pub const FORMAT: &str = "exedra-recipe";
/// Format version this module reads and writes.
pub const VERSION: u32 = 1;

/// Top-level interchange document.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecipeDto {
    /// Format name; always [`FORMAT`].
    pub format: String,
    /// Format version; always [`VERSION`].
    pub version: u32,
    /// Interned source-reference strings, in id order.
    pub sources: Vec<String>,
    /// Interned material-slot names, in id order.
    pub slots: Vec<String>,
    /// Interned curve-policy references, in id order.
    #[serde(default)]
    pub policies: Vec<String>,
    /// Imported meshes, in id order.
    #[serde(default)]
    pub imports: Vec<MeshDto>,
    /// Profiles, in id order.
    pub profiles: Vec<ProfileDto>,
    /// Nodes, children before parents.
    pub nodes: Vec<NodeDto>,
    /// Root node index.
    pub root: u32,
}

/// One profile: outer loop plus holes.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProfileDto {
    /// Outer loop (counter-clockwise).
    pub outer: Vec<SegDto>,
    /// Hole loops (clockwise).
    #[serde(default)]
    pub holes: Vec<Vec<SegDto>>,
}

/// One profile segment as an explicit record.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SegDto {
    /// Straight chord.
    Line {
        /// Endpoint.
        to: [f64; 2],
        /// Optional provenance tag.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tag: Option<u32>,
    },
    /// Circular arc (bulge parameterization, `tan(sweep/4)`).
    Arc {
        /// Endpoint.
        to: [f64; 2],
        /// Bulge value.
        bulge: f64,
        /// Optional provenance tag.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tag: Option<u32>,
    },
    /// Cubic Bézier.
    Cubic {
        /// Endpoint.
        to: [f64; 2],
        /// First control point.
        c1: [f64; 2],
        /// Second control point.
        c2: [f64; 2],
        /// Optional provenance tag.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tag: Option<u32>,
    },
    /// Policy-defined segment: an opaque policy reference plus its
    /// concrete realization.
    Policy {
        /// Endpoint.
        to: [f64; 2],
        /// Index into `policies`.
        policy: u32,
        /// The realization.
        realized: RealizedDto,
        /// Optional provenance tag.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tag: Option<u32>,
    },
}

/// Concrete realization of a policy segment (endpoint lives on the parent).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RealizedDto {
    /// Straight chord.
    Line,
    /// Circular arc by bulge.
    Arc {
        /// Bulge value.
        bulge: f64,
    },
    /// Cubic Bézier.
    Cubic {
        /// First control point.
        c1: [f64; 2],
        /// Second control point.
        c2: [f64; 2],
    },
}

/// An imported mesh: f32 vertex positions plus polygon face loops.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MeshDto {
    /// Vertex positions.
    pub positions: Vec<[f32; 3]>,
    /// Face loops as vertex indices.
    pub faces: Vec<Vec<u32>>,
    /// Primary UVs in face-loop order, including sparse missing values.
    /// Empty means no authored UVs. A nonempty array must cover every corner.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub corner_uvs: Vec<Vec<Option<[f32; 2]>>>,
    /// Secondary UVs in face-loop order; empty means none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub corner_uvs1: Vec<Vec<Option<[f32; 2]>>>,
    /// Dense secondary-channel default, when that channel uses dense storage.
    /// Absent means sparse storage. Finite defaults are preserved on new corners.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uv1_default: Option<[f32; 2]>,
    /// Secondary channel's declared propagation rule. Presence also preserves
    /// an empty registered layer; absence with UV values means unspecified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uv1_propagation: Option<UvPropagationDto>,
    /// Authored normals in face-loop order; empty means none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub corner_normals: Vec<Vec<Option<[f32; 3]>>>,
}

impl MeshDto {
    /// Captures positions, polygon loops, both built-in UV sets and authored normals.
    /// Other mesh attributes are outside this geometry interchange payload.
    ///
    /// # Panics
    ///
    /// Panics if a live vertex has no position or a face references a missing
    /// vertex. Callers should supply a structurally valid mesh.
    #[must_use]
    pub fn from_mesh(mesh: &exedra_mesh::Mesh) -> Self {
        let vertices: Vec<_> = mesh.vertices().collect();
        let indices: alloc::collections::BTreeMap<_, _> = vertices
            .iter()
            .enumerate()
            .map(|(index, &vertex)| {
                (
                    vertex,
                    u32::try_from(index).expect("mesh vertex count fits u32"),
                )
            })
            .collect();
        let loops: Vec<_> = mesh
            .faces()
            .map(|face| crate::ir::canonical_face_loop_half_edges(mesh, face))
            .collect();
        let mut corner_uvs: Vec<Vec<_>> = loops
            .iter()
            .map(|corners| {
                corners
                    .iter()
                    .map(|&corner| {
                        mesh.attrs()
                            .sparse(exedra_mesh::attr::CORNER_UV)
                            .and_then(|layer| layer.get(corner.into()))
                            .copied()
                    })
                    .collect()
            })
            .collect();
        let mut corner_uvs1: Vec<Vec<_>> = loops
            .iter()
            .map(|corners| {
                corners
                    .iter()
                    .map(|&corner| {
                        exedra_mesh_ops::uv::corner_uv(mesh, crate::ir::UvSet::Secondary, corner)
                    })
                    .collect()
            })
            .collect();
        if corner_uvs1.iter().flatten().all(Option::is_none) {
            corner_uvs1.clear();
        }
        let mut corner_normals: Vec<Vec<_>> = loops
            .iter()
            .map(|corners| {
                corners
                    .iter()
                    .map(|&corner| {
                        mesh.attrs()
                            .sparse(exedra_mesh::attr::CORNER_NORMAL_OVERRIDE)
                            .and_then(|layer| layer.get(corner.into()))
                            .copied()
                    })
                    .collect()
            })
            .collect();
        if corner_uvs.iter().flatten().all(Option::is_none) {
            corner_uvs.clear();
        }
        if corner_normals.iter().flatten().all(Option::is_none) {
            corner_normals.clear();
        }
        Self {
            positions: vertices
                .iter()
                .map(|&v| *mesh.vertex_position(v).expect("positioned mesh vertex"))
                .collect(),
            faces: loops
                .iter()
                .map(|corners| {
                    corners
                        .iter()
                        .map(|&corner| {
                            indices[&mesh.to_vertex(corner).expect("live corner vertex")]
                        })
                        .collect()
                })
                .collect(),
            corner_uvs,
            corner_uvs1,
            uv1_default: mesh
                .attrs()
                .dense(exedra_mesh::attr::CORNER_UV1)
                .map(|layer| *layer.default()),
            uv1_propagation: mesh
                .attrs()
                .propagation(
                    exedra_mesh::attr::CORNER_UV1.domain(),
                    exedra_mesh::attr::CORNER_UV1.name(),
                )
                .map(Into::into),
            corner_normals,
        }
    }

    /// Rebuilds geometry and its optional corner channels, rejecting malformed
    /// channel lengths or nonfinite values. Corner values follow destination
    /// vertices even if the builder rotates a face's loop.
    pub fn to_mesh(&self) -> Result<exedra_mesh::Mesh, InterchangeError> {
        let loops: Vec<_> = self.faces.iter().map(Vec::as_slice).collect();
        let mut mesh = exedra_mesh::Mesh::from_polygons(&self.positions, &loops)
            .map_err(|_| InterchangeError::InvalidImport)?;
        crate::import_mesh::restore_corner_channels(
            &mut mesh,
            &self.faces,
            &self.corner_uvs,
            &self.corner_uvs1,
            &self.corner_normals,
            self.uv1_propagation.map(Into::into),
            self.uv1_default,
        )
        .map_err(|_| InterchangeError::InvalidImport)?;
        Ok(mesh)
    }
}

/// Propagation of secondary UVs through later topology edits.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UvPropagationDto {
    /// No rule has been declared.
    Unspecified,
    /// Clear generated values intentionally.
    Clear,
    /// Copy a source corner's value.
    Copy,
    /// Interpolate between source values.
    Interpolate,
}
impl From<exedra_mesh::attributes::Propagation> for UvPropagationDto {
    fn from(rule: exedra_mesh::attributes::Propagation) -> Self {
        use exedra_mesh::attributes::Propagation;
        match rule {
            Propagation::Unspecified => Self::Unspecified,
            Propagation::Clear => Self::Clear,
            Propagation::Copy => Self::Copy,
            Propagation::Interpolate => Self::Interpolate,
        }
    }
}
impl From<UvPropagationDto> for exedra_mesh::attributes::Propagation {
    fn from(rule: UvPropagationDto) -> Self {
        match rule {
            UvPropagationDto::Unspecified => Self::Unspecified,
            UvPropagationDto::Clear => Self::Clear,
            UvPropagationDto::Copy => Self::Copy,
            UvPropagationDto::Interpolate => Self::Interpolate,
        }
    }
}

/// Orthonormal projection frame in mesh coordinates.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UvFrameDto {
    /// Projection origin.
    pub origin: [f64; 3],
    /// Unit U direction.
    pub u: [f64; 3],
    /// Unit V direction.
    pub v: [f64; 3],
}
impl From<crate::ir::UvFrame> for UvFrameDto {
    fn from(frame: crate::ir::UvFrame) -> Self {
        Self {
            origin: frame.origin,
            u: frame.u,
            v: frame.v,
        }
    }
}
impl From<UvFrameDto> for crate::ir::UvFrame {
    fn from(frame: UvFrameDto) -> Self {
        Self {
            origin: frame.origin,
            u: frame.u,
            v: frame.v,
        }
    }
}
/// Angular seam policy in the interchange format.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UvSeamPolicyDto {
    /// Keep wrapped coordinates unchanged.
    Preserve,
    /// Lift across the largest face-local gap.
    #[default]
    Continuous,
}
/// Undefined-angle recovery policy in the interchange format.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UvPolePolicyDto {
    /// Refuse undefined angles.
    Refuse,
    /// Recover from defined face neighbors.
    #[default]
    FaceAverage,
}
/// Channel selection and angular recovery, with explicit defaults.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UvMappingOptionsDto {
    /// Source set, 0 or 1, for existing-coordinate projection.
    pub source: u8,
    /// Destination set, 0 or 1.
    pub target: u8,
    /// Angular seam handling.
    pub seams: UvSeamPolicyDto,
    /// Undefined-angle handling.
    pub poles: UvPolePolicyDto,
}
impl From<crate::ir::UvMappingOptions> for UvMappingOptionsDto {
    fn from(options: crate::ir::UvMappingOptions) -> Self {
        use crate::ir::{UvPolePolicy, UvSeamPolicy, UvSet};
        Self {
            source: u8::from(options.source == UvSet::Secondary),
            target: u8::from(options.target == UvSet::Secondary),
            seams: match options.seams {
                UvSeamPolicy::Preserve => UvSeamPolicyDto::Preserve,
                UvSeamPolicy::Continuous => UvSeamPolicyDto::Continuous,
            },
            poles: match options.poles {
                UvPolePolicy::Refuse => UvPolePolicyDto::Refuse,
                UvPolePolicy::FaceAverage => UvPolePolicyDto::FaceAverage,
            },
        }
    }
}
impl TryFrom<UvMappingOptionsDto> for crate::ir::UvMappingOptions {
    type Error = InterchangeError;
    fn try_from(options: UvMappingOptionsDto) -> Result<Self, Self::Error> {
        use crate::ir::{UvPolePolicy, UvSeamPolicy, UvSet};
        let set = |value, what| match value {
            0 => Ok(UvSet::Primary),
            1 => Ok(UvSet::Secondary),
            _ => Err(InterchangeError::Recipe(RecipeError::InvalidParameter {
                what,
            })),
        };
        Ok(Self {
            source: set(options.source, "UV source set")?,
            target: set(options.target, "UV target set")?,
            seams: match options.seams {
                UvSeamPolicyDto::Preserve => UvSeamPolicy::Preserve,
                UvSeamPolicyDto::Continuous => UvSeamPolicy::Continuous,
            },
            poles: match options.poles {
                UvPolePolicyDto::Refuse => UvPolePolicy::Refuse,
                UvPolePolicyDto::FaceAverage => UvPolePolicy::FaceAverage,
            },
        })
    }
}

/// Coordinate source for primary corner UV mapping.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum UvProjectionDto {
    /// Each face's geometric normal defines its plane.
    Face,
    /// Angular U and axial-distance V.
    Cylindrical {
        /// Radial projection frame.
        frame: UvFrameDto,
    },
    /// Longitude U and latitude V.
    Spherical {
        /// Radial projection frame.
        frame: UvFrameDto,
    },
    /// Longitude around a ring and angle around its tube.
    Toroidal {
        /// Radial projection frame.
        frame: UvFrameDto,
        /// Positive radius to the tube centerline.
        major_radius: f64,
    },
    /// Camera/projector image coordinates.
    Perspective {
        /// Projector frame; forward is U cross V.
        frame: UvFrameDto,
        /// Positive UV focal scales.
        focal: [f64; 2],
        /// Nonnegative near depth.
        near: f64,
    },

    /// Require existing authored corner UVs.
    Existing,
    /// Freely oriented orthonormal projection plane.
    Planar {
        /// Origin in mesh units.
        origin: [f64; 3],
        /// Unit U axis.
        u: [f64; 3],
        /// Unit V axis.
        v: [f64; 3],
    },
    /// Signed box projection, indexed by dominant normal X, Y, Z.
    Box {
        /// Exchange U/V on each pair of faces.
        swap_axes: [bool; 3],
        /// Negate U after exchanging axes.
        flip_u: [bool; 3],
        /// Negate V after exchanging axes.
        flip_v: [bool; 3],
    },
}

impl From<crate::ir::UvProjection> for UvProjectionDto {
    fn from(projection: crate::ir::UvProjection) -> Self {
        use crate::ir::UvProjection;
        match projection {
            UvProjection::Face => Self::Face,
            UvProjection::Cylindrical(frame) => Self::Cylindrical {
                frame: frame.into(),
            },
            UvProjection::Spherical(frame) => Self::Spherical {
                frame: frame.into(),
            },
            UvProjection::Toroidal {
                frame,
                major_radius,
            } => Self::Toroidal {
                frame: frame.into(),
                major_radius,
            },
            UvProjection::Perspective(projector) => Self::Perspective {
                frame: projector.frame.into(),
                focal: projector.focal,
                near: projector.near,
            },
            UvProjection::Existing => Self::Existing,
            UvProjection::Planar(frame) => Self::Planar {
                origin: frame.origin,
                u: frame.u,
                v: frame.v,
            },
            UvProjection::Box(layout) => Self::Box {
                swap_axes: layout.swap_axes,
                flip_u: layout.flip_u,
                flip_v: layout.flip_v,
            },
        }
    }
}
impl From<UvProjectionDto> for crate::ir::UvProjection {
    fn from(projection: UvProjectionDto) -> Self {
        match projection {
            UvProjectionDto::Face => Self::Face,
            UvProjectionDto::Cylindrical { frame } => Self::Cylindrical(frame.into()),
            UvProjectionDto::Spherical { frame } => Self::Spherical(frame.into()),
            UvProjectionDto::Toroidal {
                frame,
                major_radius,
            } => Self::Toroidal {
                frame: frame.into(),
                major_radius,
            },
            UvProjectionDto::Perspective { frame, focal, near } => {
                Self::Perspective(crate::ir::PerspectiveUv {
                    frame: frame.into(),
                    focal,
                    near,
                })
            }
            UvProjectionDto::Existing => Self::Existing,
            UvProjectionDto::Planar { origin, u, v } => {
                Self::Planar(crate::ir::UvFrame { origin, u, v })
            }
            UvProjectionDto::Box {
                swap_axes,
                flip_u,
                flip_v,
            } => Self::Box(crate::ir::BoxUv {
                swap_axes,
                flip_u,
                flip_v,
            }),
        }
    }
}

/// Affine chart mapping; entries are repeats per recipe unit and texture phase.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct ChartTransformDto {
    /// Row-major linear mapping.
    pub matrix: [[f64; 2]; 2],
    /// Texture phase in repeats.
    pub offset: [f64; 2],
}
impl From<ChartTransform> for ChartTransformDto {
    fn from(t: ChartTransform) -> Self {
        Self {
            matrix: t.matrix,
            offset: t.offset,
        }
    }
}
impl From<ChartTransformDto> for ChartTransform {
    fn from(t: ChartTransformDto) -> Self {
        Self {
            matrix: t.matrix,
            offset: t.offset,
        }
    }
}

/// Authored operation-specific construction chart.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(tag = "metric", rename_all = "snake_case")]
pub enum SurfaceChartDto {
    /// Sampled profile distance / extrusion height.
    Extrude {
        /// Wall coordinate transform.
        wall: ChartTransformDto,
        /// Profile-plane cap transform.
        caps: ChartTransformDto,
    },
    /// Angular distance at a declared radius / sampled profile distance.
    Revolve {
        /// Positive angular rest radius in recipe units.
        reference_radius: f64,
        /// Wall coordinate transform.
        wall: ChartTransformDto,
        /// Profile-plane cap transform.
        caps: ChartTransformDto,
    },
    /// Explicit reference perimeter / uniformly parameterized authored rest length.
    Loft {
        /// Zero-based reference section.
        reference_section: u32,
        /// Finite positive longitudinal rest length in recipe units.
        rest_length: f64,
        /// Wall coordinate transform.
        wall: ChartTransformDto,
        /// Each cap's profile-plane transform.
        caps: ChartTransformDto,
    },
    /// Explicit reference perimeter / authored rest length divided by the
    /// chord length between section datums. A distinct metric, so older
    /// readers refuse it rather than place sections uniformly.
    LoftDistance {
        /// Zero-based reference section.
        reference_section: u32,
        /// Finite positive longitudinal rest length in recipe units.
        rest_length: f64,
        /// Wall coordinate transform.
        wall: ChartTransformDto,
        /// Each cap's profile-plane transform.
        caps: ChartTransformDto,
    },
    /// Sampled profile distance / sampled centerline distance before placement.
    Sweep {
        /// Wall coordinate transform.
        wall: ChartTransformDto,
        /// Profile-plane cap transform.
        caps: ChartTransformDto,
    },
}
impl From<SurfaceChart> for SurfaceChartDto {
    fn from(chart: SurfaceChart) -> Self {
        match chart {
            SurfaceChart::Loft {
                reference_section,
                rest_length,
                stations: LoftStations::SectionIndex,
                wall,
                caps,
            } => Self::Loft {
                reference_section,
                rest_length,
                wall: wall.into(),
                caps: caps.into(),
            },
            SurfaceChart::Loft {
                reference_section,
                rest_length,
                stations: LoftStations::DatumDistance,
                wall,
                caps,
            } => Self::LoftDistance {
                reference_section,
                rest_length,
                wall: wall.into(),
                caps: caps.into(),
            },
            SurfaceChart::Sweep { wall, caps } => Self::Sweep {
                wall: wall.into(),
                caps: caps.into(),
            },
            SurfaceChart::Extrude { wall, caps } => Self::Extrude {
                wall: wall.into(),
                caps: caps.into(),
            },
            SurfaceChart::Revolve {
                reference_radius,
                wall,
                caps,
            } => Self::Revolve {
                reference_radius,
                wall: wall.into(),
                caps: caps.into(),
            },
        }
    }
}
impl From<SurfaceChartDto> for SurfaceChart {
    fn from(chart: SurfaceChartDto) -> Self {
        match chart {
            SurfaceChartDto::Loft {
                reference_section,
                rest_length,
                wall,
                caps,
            } => Self::Loft {
                reference_section,
                rest_length,
                stations: LoftStations::SectionIndex,
                wall: wall.into(),
                caps: caps.into(),
            },
            SurfaceChartDto::LoftDistance {
                reference_section,
                rest_length,
                wall,
                caps,
            } => Self::Loft {
                reference_section,
                rest_length,
                stations: LoftStations::DatumDistance,
                wall: wall.into(),
                caps: caps.into(),
            },
            SurfaceChartDto::Sweep { wall, caps } => Self::Sweep {
                wall: wall.into(),
                caps: caps.into(),
            },
            SurfaceChartDto::Extrude { wall, caps } => Self::Extrude {
                wall: wall.into(),
                caps: caps.into(),
            },
            SurfaceChartDto::Revolve {
                reference_radius,
                wall,
                caps,
            } => Self::Revolve {
                reference_radius,
                wall: wall.into(),
                caps: caps.into(),
            },
        }
    }
}

/// One node record: kind payload plus optional source/material bindings.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeDto {
    /// The node kind payload.
    #[serde(flatten)]
    pub kind: NodeKindDto,
    /// Optional index into `sources`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<u32>,
    /// Optional index into `slots`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<u32>,
    /// Optional index into `sources`: a spec-issue citation marking the
    /// node's specification as contradictory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue: Option<u32>,
}

/// 3x4 row-major placement.
pub type PlacementDto = [[f64; 4]; 3];

/// Plane as `[nx, ny, nz, d]` (`dot(n, p) = d`).
pub type PlaneDto = [f64; 4];

/// One ordered plane-selected vertex displacement.
#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub struct VertexStretchStepDto {
    /// Selection plane in the node's input frame.
    pub plane: PlaneDto,
    /// Signed displacement along the normalized plane normal.
    pub length: f64,
}

/// An operand-qualified face region in an edge-boundary selection.
#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub struct OperandRegionDto {
    /// Index in the producing CSG node's declared operands.
    pub operand: u16,
    /// The operand face's region number.
    pub region: u32,
}

/// Explicit edge boundaries. Numeric pairs retain the original JSON format;
/// qualified endpoints are objects that older numeric-only readers reject.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EdgeBoundariesDto {
    /// Unqualified face-region pairs.
    Regions(Vec<[u32; 2]>),
    /// Operand-qualified face-region pairs.
    Operands(Vec<[OperandRegionDto; 2]>),
}

/// A section law: a constant, or piecewise-linear `[t, value]` keys over
/// normalized arc length.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LawDto {
    /// The same value everywhere.
    Constant {
        /// The value.
        value: f64,
    },
    /// Linear interpolation of keys, `t` strictly increasing from 0 to 1.
    Linear {
        /// `[t, value]` keys.
        keys: Vec<[f64; 2]>,
    },
}

/// Explicit controlled-path connectivity, independent of endpoint coordinates.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PathClosureDto {
    /// Spatial open rail.
    Open,
    /// Closed path in the plane through its first station.
    ClosedPlanar {
        /// Finite nonzero normal in path-local coordinates.
        normal: [f64; 3],
    },
}

/// Authored behavior at curve segment boundaries and the closing seam.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PathJoinDto {
    /// Require matching analytic tangent directions.
    Smooth,
    /// Join on bounded tangent-bisector planes.
    Miter {
        /// Maximum section-plane stretch, finite and at least one.
        limit: f64,
    },
}

/// Node kind payloads. Unknown kinds are deserialization errors by design.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum NodeKindDto {
    /// Primary corner UV mapping in the child's local evaluation frame.
    UvMap {
        /// Channel selection and angular recovery policies.
        #[serde(default)]
        options: UvMappingOptionsDto,
        /// Child node index.
        child: u32,
        /// Coordinate source.
        projection: UvProjectionDto,
        /// Row-major affine UV matrix.
        matrix: [[f64; 2]; 2],
        /// Translation after the matrix.
        offset: [f64; 2],
    },
    /// Charted generating operation. The distinct opcode prevents old readers
    /// from silently dropping authored coordinates. Wrappers cannot nest.
    Charted {
        /// Extrusion or revolution operation payload.
        operation: Box<Self>,
        /// Checked construction metric and UV mapping.
        chart: SurfaceChartDto,
    },
    /// Places a child on a freshly resolved semantic support workplane.
    OnWorkplane {
        /// Unique support subtree node.
        support: u32,
        /// Child in workplane-local coordinates.
        child: u32,
        /// Retained surface and authored projection/roll.
        attachment: WorkplaneAttachmentDto,
    },
    /// Retain one capped half of every closed child body.
    PlaneCut {
        /// Child node index.
        child: u32,
        /// Plane in node-local coordinates after child transforms.
        plane: PlaneDto,
        /// Retained half-space: "negative" or "positive".
        side: String,
        /// Generated cap region.
        cap_region: u32,
        /// Optional generated cap material slot.
        cap_material: Option<u32>,
    },
    /// Closed extrusion along placement +Z to a node-local plane.
    ExtrudeToPlane {
        /// Profile index.
        profile: u32,
        /// Profile placement in node-local coordinates.
        placement: PlacementDto,
        /// Target plane in node-local coordinates.
        plane: PlaneDto,
    },
    /// Convex edge finishing in child-local space.
    EdgeFinish {
        /// Child node index.
        child: u32,
        /// Region pairs, optionally qualified by operand; absent means all sharp edges.
        boundaries: Option<EdgeBoundariesDto>,
        /// "fillet" or "chamfer".
        kind: String,
        /// Fillet radius or chamfer setback.
        offset: f64,
        /// Explicit band count, or chord-tolerance sampling.
        segments: Option<u32>,
        /// Maximum arc chord deviation.
        chord_tolerance: f64,
        /// Threshold used for whole-body sharp-edge selection.
        sharpness_threshold: f32,
        /// Optional region for new bands and patches.
        region: Option<u32>,
        /// Planarity and end-containment tolerance.
        max_planar_deviation: f64,
        /// Maximum chain turn in radians.
        max_tangent_turn: f64,
    },
    /// Extrusion along local +Z.
    Extrude {
        /// Profile index.
        profile: u32,
        /// Placement.
        placement: PlacementDto,
        /// Extrusion height.
        height: f64,
        /// Cap mode: "both" | "start" | "end" | "none".
        caps: String,
    },
    /// A blind pocket cut into a closed extrusion from one of its caps.
    Pocket {
        /// The pocketed extrusion or pocket node.
        base: u32,
        /// Profile index of the region removed.
        profile: u32,
        /// The cap the pocket opens on: "start" | "end".
        end: String,
        /// Depth from that cap.
        depth: f64,
    },
    /// Revolution about local Y.
    Revolve {
        /// Profile index.
        profile: u32,
        /// Placement.
        placement: PlacementDto,
        /// Sweep angle in radians.
        sweep: f64,
        /// Cap mode.
        caps: String,
    },
    /// Loft between placed sections with authored segment correspondence.
    Loft {
        /// Sections as `(placement, profile)` pairs.
        sections: Vec<(PlacementDto, u32)>,
        /// Optional source ids parallel to sections; empty means all unlabeled.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        section_sources: Vec<Option<u32>>,
        /// Explicit interpolation policy: `ruled` or `smooth`.
        policy: String,
        /// Cap mode.
        caps: String,
    },
    /// Sweep along a polyline path.
    Sweep {
        /// Profile index.
        profile: u32,
        /// Path points.
        points: Vec<[f64; 3]>,
        /// Cap mode.
        caps: String,
    },
    /// Explicitly oriented constant-section polyline with bounded miter joins.
    MiteredSweep {
        /// Profile index.
        profile: u32,
        /// Path-local points.
        points: Vec<[f64; 3]>,
        /// Authored path-local section-X direction.
        section_x: [f64; 3],
        /// Maximum section-plane stretch ratio.
        miter_limit: f64,
        /// Cap mode.
        caps: String,
    },
    /// Controlled polyline sweep with an explicit section datum and connectivity.
    /// This distinct operation prevents older readers ignoring closure fields.
    MiteredPathSweep {
        /// Profile index.
        profile: u32,
        /// Unique path-local stations; closed paths do not repeat the endpoint.
        points: Vec<[f64; 3]>,
        /// Authored initial section-X direction.
        section_x: [f64; 3],
        /// Profile datum mapped to the path.
        section_origin: [f64; 2],
        /// Explicit endpoint connectivity.
        closure: PathClosureDto,
        /// Maximum section-plane stretch ratio.
        miter_limit: f64,
        /// Cap mode; closed paths require none.
        caps: String,
    },
    /// Sweep along tangent-continuous analytic spatial segments.
    CurvedSweep {
        /// Profile index.
        profile: u32,
        /// Path-local starting endpoint.
        start: [f64; 3],
        /// Ordered spatial segments.
        segments: Vec<PathSegmentDto>,
        /// Authored initial section-X direction.
        section_x: [f64; 3],
        /// Cap mode.
        caps: String,
    },
    /// Analytic path sweep with an explicit datum, connectivity and corner policy.
    /// A distinct operation prevents older readers ignoring these controls.
    CurvedPathSweep {
        /// Profile index.
        profile: u32,
        /// Path-local starting endpoint.
        start: [f64; 3],
        /// Ordered spatial segments, including the closed path's final segment.
        segments: Vec<PathSegmentDto>,
        /// Authored initial section-X direction.
        section_x: [f64; 3],
        /// Profile datum mapped to the path.
        section_origin: [f64; 2],
        /// Authored endpoint connectivity.
        closure: PathClosureDto,
        /// Authored join behavior.
        joins: PathJoinDto,
        /// Cap mode; closed paths require none.
        caps: String,
    },
    /// A sweep whose section scales and twists along its path. A distinct
    /// operation, so older readers refuse it rather than ignore the law.
    ShapedSweep {
        /// Section scale law.
        scale: LawDto,
        /// Section twist law, in radians.
        twist: LawDto,
        /// The wrapped constant-section sweep operation.
        sweep: Box<Self>,
    },
    /// A shaped sweep with an explicit integral section winding. A distinct
    /// operation prevents older readers from silently dropping the turns.
    TurnedSweep {
        /// Section scale law.
        scale: LawDto,
        /// Base section twist law, in radians.
        twist: LawDto,
        /// Signed complete revolutions along the path.
        turns: i32,
        /// The wrapped constant-section sweep operation.
        sweep: Box<Self>,
    },
    /// Single-sided planar face.
    PlanarFace {
        /// Profile index.
        profile: u32,
        /// Placement.
        placement: PlacementDto,
    },
    /// Axis-aligned box primitive.
    Box {
        /// Extents.
        size: [f64; 3],
        /// Placement.
        placement: PlacementDto,
    },
    /// Capped cylinder primitive.
    Cylinder {
        /// Radius.
        radius: f64,
        /// Height.
        height: f64,
        /// Side segments.
        segments: u32,
        /// Placement.
        placement: PlacementDto,
    },
    /// Boolean combination.
    Csg {
        /// Operation: "union" | "difference" | "intersection".
        csg: String,
        /// Operand node indices.
        operands: Vec<u32>,
    },
    /// Affine transform of a child.
    Transform {
        /// Child node index.
        child: u32,
        /// Placement.
        placement: PlacementDto,
    },
    /// Mirror across a plane.
    Mirror {
        /// Child node index.
        child: u32,
        /// Mirror plane.
        plane: PlaneDto,
    },
    /// Instance of an earlier node.
    Instance {
        /// Definition node index.
        of: u32,
        /// Instance placement.
        placement: PlacementDto,
    },
    /// Grouping node.
    Group {
        /// Child node indices.
        children: Vec<u32>,
    },
    /// Opaque imported mesh leaf.
    MeshImport {
        /// Index into `imports`.
        import: u32,
        /// Placement.
        placement: PlacementDto,
    },
    /// Simultaneous vertex displacement preserving triangle topology.
    StretchVertices {
        /// Child node index.
        child: u32,
        /// Nonempty steps selecting the same input positions.
        steps: Vec<VertexStretchStepDto>,
    },
    /// Plane-split stretch.
    Stretch {
        /// Child node index.
        child: u32,
        /// Cutting plane.
        plane: PlaneDto,
        /// Signed stretch length.
        length: f64,
    },
    /// Point-grid surface body (bilinear quad patches).
    GridSurface {
        /// Row-major grid points; `rows * cols` entries.
        points: Vec<[f64; 3]>,
        /// Row count.
        rows: u32,
        /// Column count.
        cols: u32,
        /// Wrap rows.
        close_u: bool,
        /// Wrap columns.
        close_w: bool,
        /// Solid thickness; absent for an open sheet.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thickness: Option<f64>,
        /// Placement.
        placement: PlacementDto,
    },
    /// A solid given by an implicit field.
    Implicit {
        /// The field, in the leaf's local frame.
        field: ImplicitDto,
        /// The region meshed as `[min, max]`; absent to take the field's
        /// claimed bounds.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bounds: Option<[[f64; 3]; 2]>,
        /// Placement.
        placement: PlacementDto,
    },
}

/// An implicit field: a shape, a profile lifted into space, or a
/// combination or placement of other fields.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum ImplicitDto {
    /// A ball.
    Sphere {
        /// Center.
        center: [f64; 3],
        /// Radius; positive.
        radius: f64,
    },
    /// A box.
    Cuboid {
        /// Center.
        center: [f64; 3],
        /// Half its extent along each axis; positive.
        half_extents: [f64; 3],
    },
    /// A finite cylinder.
    Cylinder {
        /// Center.
        center: [f64; 3],
        /// Axis direction.
        axis: [f64; 3],
        /// Radius; positive.
        radius: f64,
        /// Half its length along the axis; positive.
        half_height: f64,
    },
    /// A torus.
    Torus {
        /// Center.
        center: [f64; 3],
        /// Axis direction.
        axis: [f64; 3],
        /// Distance from the axis to the tube's middle; positive.
        major_radius: f64,
        /// The tube's radius; positive.
        minor_radius: f64,
    },
    /// The solid behind a plane.
    HalfSpace {
        /// A point on the plane.
        point: [f64; 3],
        /// Outward normal.
        normal: [f64; 3],
    },
    /// Everything within a radius of a segment.
    Capsule {
        /// One end.
        a: [f64; 3],
        /// The other end.
        b: [f64; 3],
        /// Radius; positive.
        radius: f64,
    },
    /// The hull of two balls.
    RoundCone {
        /// The first ball's center.
        a: [f64; 3],
        /// The second ball's center.
        b: [f64; 3],
        /// The first ball's radius; not negative.
        radius_a: f64,
        /// The second ball's radius; not negative.
        radius_b: f64,
    },
    /// A profile extruded along `z`.
    Extrude {
        /// The profile.
        profile: ImplicitProfileDto,
        /// Half the extrusion's length; positive.
        half_height: f64,
    },
    /// A profile, in the radius-height plane, revolved about `y`.
    Revolve {
        /// The profile.
        profile: ImplicitProfileDto,
    },
    /// Everything inside either part.
    Union {
        /// One part.
        left: Box<Self>,
        /// The other part.
        right: Box<Self>,
    },
    /// Everything inside both parts.
    Intersection {
        /// One part.
        left: Box<Self>,
        /// The other part.
        right: Box<Self>,
    },
    /// Everything inside `left` and outside `right`.
    Difference {
        /// The part cut from.
        left: Box<Self>,
        /// The part cut away.
        right: Box<Self>,
    },
    /// Two parts unioned with their crease rounded over `radius`.
    SmoothUnion {
        /// One part.
        left: Box<Self>,
        /// The other part.
        right: Box<Self>,
        /// Blend radius; positive.
        radius: f64,
    },
    /// Any number of parts blended symmetrically over `radius`.
    Blend {
        /// The parts.
        parts: Vec<Self>,
        /// Blend radius; positive.
        radius: f64,
    },
    /// A part moved by `offset`.
    Translate {
        /// The part.
        field: Box<Self>,
        /// Offset.
        offset: [f64; 3],
    },
    /// A part scaled about the origin.
    Scale {
        /// The part.
        field: Box<Self>,
        /// Scale factor; positive.
        factor: f64,
    },
    /// A part turned and moved by a rigid placement.
    Placed {
        /// The part.
        field: Box<Self>,
        /// Rigid placement.
        placement: PlacementDto,
    },
    /// A part labelled with a face region.
    Tagged {
        /// The part.
        field: Box<Self>,
        /// Region; zero means none.
        region: u32,
    },
}

/// A two-dimensional profile for [`ImplicitDto::Extrude`] and
/// [`ImplicitDto::Revolve`].
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "profile", rename_all = "snake_case")]
pub enum ImplicitProfileDto {
    /// A disk.
    Circle {
        /// Center.
        center: [f64; 2],
        /// Radius; positive.
        radius: f64,
    },
    /// An axis-aligned rectangle.
    Rectangle {
        /// Center.
        center: [f64; 2],
        /// Half its extent along each axis; positive.
        half_extents: [f64; 2],
    },
    /// The half plane behind a line.
    HalfPlane {
        /// A point on the line.
        point: [f64; 2],
        /// Outward normal.
        normal: [f64; 2],
    },
}

/// Typed interchange failure.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum InterchangeError {
    /// Wrong format name or unsupported version.
    UnsupportedFormat,
    /// An enum-like string field held an unknown value.
    UnknownValue {
        /// The field, as a stable name.
        field: &'static str,
    },
    /// Profile validation failed on rebuild.
    Profile(ProfileError),
    /// Recipe validation failed on rebuild.
    Recipe(RecipeError),
    /// An imported mesh failed to rebuild.
    InvalidImport,
    /// An implicit field's parameters were refused on rebuild.
    InvalidImplicit(exedra_implicit::FieldError),
}

impl core::fmt::Display for InterchangeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnsupportedFormat => write!(f, "unsupported format or version"),
            Self::UnknownValue { field } => write!(f, "unknown value in field {field}"),
            Self::Profile(e) => write!(f, "profile validation failed: {e}"),
            Self::Recipe(e) => write!(f, "recipe validation failed: {e}"),
            Self::InvalidImport => write!(f, "imported mesh failed to rebuild"),
            Self::InvalidImplicit(e) => write!(f, "implicit field failed to rebuild: {e}"),
        }
    }
}

impl core::error::Error for InterchangeError {}

fn caps_name(caps: CapMode) -> String {
    String::from(match caps {
        CapMode::Both => "both",
        CapMode::Start => "start",
        CapMode::End => "end",
        CapMode::None => "none",
    })
}

fn caps_value(name: &str) -> Result<CapMode, InterchangeError> {
    match name {
        "both" => Ok(CapMode::Both),
        "start" => Ok(CapMode::Start),
        "end" => Ok(CapMode::End),
        "none" => Ok(CapMode::None),
        _ => Err(InterchangeError::UnknownValue { field: "caps" }),
    }
}

fn placement_dto(p: &Placement3) -> PlacementDto {
    p.rows
}

fn placement_value(rows: PlacementDto) -> Placement3 {
    Placement3 { rows }
}

fn plane_dto(p: &Plane3) -> PlaneDto {
    [p.normal[0], p.normal[1], p.normal[2], p.distance]
}

fn plane_value(v: PlaneDto) -> Plane3 {
    Plane3 {
        normal: [v[0], v[1], v[2]],
        distance: v[3],
    }
}

fn loop_dto(source: &Loop2) -> Vec<SegDto> {
    source
        .segs()
        .iter()
        .map(|seg| {
            let to = [seg.to.x, seg.to.y];
            let tag = seg.tag.map(|SegTag(t)| t);
            match &seg.kind {
                SegKind::Line => SegDto::Line { to, tag },
                SegKind::Arc { bulge } => SegDto::Arc {
                    to,
                    bulge: *bulge,
                    tag,
                },
                SegKind::Cubic { c1, c2 } => SegDto::Cubic {
                    to,
                    c1: [c1.x, c1.y],
                    c2: [c2.x, c2.y],
                    tag,
                },
                SegKind::PolicyTo { policy, realized } => SegDto::Policy {
                    to,
                    policy: policy.0,
                    realized: match realized.as_ref() {
                        SegKind::Line => RealizedDto::Line,
                        SegKind::Arc { bulge } => RealizedDto::Arc { bulge: *bulge },
                        SegKind::Cubic { c1, c2 } => RealizedDto::Cubic {
                            c1: [c1.x, c1.y],
                            c2: [c2.x, c2.y],
                        },
                        SegKind::PolicyTo { .. } => {
                            unreachable!("nested policies are rejected at validation")
                        }
                    },
                    tag,
                },
            }
        })
        .collect()
}

fn loop_value(segs: &[SegDto]) -> Result<Loop2, InterchangeError> {
    let segs = segs
        .iter()
        .map(|dto| {
            let (seg, tag) = match dto {
                SegDto::Line { to, tag } => (Seg2::line((to[0], to[1])), tag),
                SegDto::Arc { to, bulge, tag } => (Seg2::arc((to[0], to[1]), *bulge), tag),
                SegDto::Cubic { to, c1, c2, tag } => (
                    Seg2::cubic((to[0], to[1]), (c1[0], c1[1]), (c2[0], c2[1])),
                    tag,
                ),
                SegDto::Policy {
                    to,
                    policy,
                    realized,
                    tag,
                } => {
                    let inner = match realized {
                        RealizedDto::Line => SegKind::Line,
                        RealizedDto::Arc { bulge } => SegKind::Arc { bulge: *bulge },
                        RealizedDto::Cubic { c1, c2 } => SegKind::Cubic {
                            c1: kurbo::Point::new(c1[0], c1[1]),
                            c2: kurbo::Point::new(c2[0], c2[1]),
                        },
                    };
                    (
                        Seg2::policy((to[0], to[1]), crate::ir::PolicyId(*policy), inner),
                        tag,
                    )
                }
            };
            match tag {
                Some(t) => seg.tagged(SegTag(*t)),
                None => seg,
            }
        })
        .collect();
    Loop2::new(segs).map_err(InterchangeError::Profile)
}

/// Converts a recipe into its interchange document.
#[must_use]
pub fn to_dto(recipe: &Recipe) -> RecipeDto {
    let profiles = recipe
        .profiles()
        .iter()
        .map(|profile| ProfileDto {
            outer: loop_dto(profile.outer()),
            holes: profile.holes().iter().map(loop_dto).collect(),
        })
        .collect();
    let nodes = recipe
        .nodes()
        .iter()
        .map(|node| NodeDto {
            kind: match node.surface_chart {
                Some(chart) => NodeKindDto::Charted {
                    operation: Box::new(kind_dto(&node.kind)),
                    chart: chart.into(),
                },
                None => kind_dto(&node.kind),
            },
            source: node.source.map(|SourceId(s)| s),
            material: node.material.map(|SlotId(m)| m),
            issue: node.issue.map(|SourceId(s)| s),
        })
        .collect();
    RecipeDto {
        format: String::from(FORMAT),
        version: VERSION,
        sources: recipe.sources().to_vec(),
        slots: recipe.slots().to_vec(),
        policies: recipe.policies().to_vec(),
        imports: recipe.imports().iter().map(MeshDto::from_mesh).collect(),
        profiles,
        nodes,
        root: recipe.root().0,
    }
}

fn kind_dto(kind: &NodeKind) -> NodeKindDto {
    match kind {
        NodeKind::UvMap {
            child,
            mapping,
            options,
        } => NodeKindDto::UvMap {
            options: (*options).into(),
            child: child.0,
            projection: mapping.projection.into(),
            matrix: mapping.transform.matrix,
            offset: mapping.transform.offset,
        },
        NodeKind::OnWorkplane {
            support,
            child,
            attachment,
        } => NodeKindDto::OnWorkplane {
            support: support.0,
            child: child.0,
            attachment: attachment.clone().into(),
        },
        NodeKind::PlaneCut {
            child,
            plane,
            side,
            cap,
        } => NodeKindDto::PlaneCut {
            child: child.0,
            plane: plane_dto(plane),
            side: match side {
                crate::ir::PlaneSide::Negative => "negative",
                crate::ir::PlaneSide::Positive => "positive",
            }
            .into(),
            cap_region: cap.region,
            cap_material: cap.material.map(|slot| slot.0),
        },
        NodeKind::ExtrudeToPlane {
            profile,
            placement,
            plane,
        } => NodeKindDto::ExtrudeToPlane {
            profile: profile.0,
            placement: placement.rows,
            plane: plane_dto(plane),
        },
        NodeKind::Extrude {
            profile,
            placement,
            height,
            caps,
        } => NodeKindDto::Extrude {
            profile: profile.0,
            placement: placement_dto(placement),
            height: *height,
            caps: caps_name(*caps),
        },
        NodeKind::Pocket {
            base,
            profile,
            end,
            depth,
        } => NodeKindDto::Pocket {
            base: base.0,
            profile: profile.0,
            end: match end {
                crate::ir::ExtrusionEnd::Start => "start",
                crate::ir::ExtrusionEnd::End => "end",
            }
            .into(),
            depth: *depth,
        },
        NodeKind::Revolve {
            profile,
            placement,
            sweep,
            caps,
        } => NodeKindDto::Revolve {
            profile: profile.0,
            placement: placement_dto(placement),
            sweep: *sweep,
            caps: caps_name(*caps),
        },
        NodeKind::Loft {
            sections,
            policy,
            caps,
        } => NodeKindDto::Loft {
            policy: String::from(match policy {
                LoftPolicy::Ruled => "ruled",
                LoftPolicy::Smooth => "smooth",
            }),
            sections: sections
                .iter()
                .map(|section| (placement_dto(&section.placement), section.profile.0))
                .collect(),
            section_sources: if sections.iter().any(|s| s.source.is_some()) {
                sections.iter().map(|s| s.source.map(|id| id.0)).collect()
            } else {
                Vec::new()
            },
            caps: caps_name(*caps),
        },
        NodeKind::Sweep {
            profile,
            path,
            section,
            caps,
        } if section.turns != 0 => NodeKindDto::TurnedSweep {
            scale: law_dto(&section.scale),
            twist: law_dto(&section.twist),
            turns: section.turns,
            sweep: Box::new(kind_dto(&NodeKind::Sweep {
                profile: *profile,
                path: path.clone(),
                section: SectionLaw::IDENTITY,
                caps: *caps,
            })),
        },
        NodeKind::Sweep {
            profile,
            path,
            section,
            caps,
        } if !section.is_identity() => NodeKindDto::ShapedSweep {
            scale: law_dto(&section.scale),
            twist: law_dto(&section.twist),
            sweep: Box::new(kind_dto(&NodeKind::Sweep {
                profile: *profile,
                path: path.clone(),
                section: SectionLaw::IDENTITY,
                caps: *caps,
            })),
        },
        NodeKind::Sweep {
            profile,
            path,
            section: _,
            caps,
        } => match path {
            Path3::Polyline { points, frame } => {
                let FramePolicy::RotationMinimizing = frame;
                NodeKindDto::Sweep {
                    profile: profile.0,
                    points: points.clone(),
                    caps: caps_name(*caps),
                }
            }
            Path3::MiteredPolyline {
                points,
                section_x,
                section_origin,
                closure,
                miter_limit,
            } => NodeKindDto::MiteredPathSweep {
                profile: profile.0,
                points: points.clone(),
                section_x: *section_x,
                section_origin: *section_origin,
                closure: match closure {
                    PathClosure::Open => PathClosureDto::Open,
                    PathClosure::ClosedPlanar { normal } => {
                        PathClosureDto::ClosedPlanar { normal: *normal }
                    }
                },
                miter_limit: *miter_limit,
                caps: caps_name(*caps),
            },
            Path3::Curves {
                start,
                segments,
                section_x,
                section_origin,
                closure,
                joins,
            } => NodeKindDto::CurvedPathSweep {
                profile: profile.0,
                start: *start,
                segments: segments.iter().map(path_segment_dto).collect(),
                section_x: *section_x,
                section_origin: *section_origin,
                closure: match closure {
                    PathClosure::Open => PathClosureDto::Open,
                    PathClosure::ClosedPlanar { normal } => {
                        PathClosureDto::ClosedPlanar { normal: *normal }
                    }
                },
                joins: match joins {
                    PathJoin::Smooth => PathJoinDto::Smooth,
                    PathJoin::Miter { limit } => PathJoinDto::Miter { limit: *limit },
                },
                caps: caps_name(*caps),
            },
        },
        NodeKind::PlanarFace { profile, placement } => NodeKindDto::PlanarFace {
            profile: profile.0,
            placement: placement_dto(placement),
        },
        NodeKind::Primitive { spec, placement } => match spec {
            PrimitiveSpec::Box { size } => NodeKindDto::Box {
                size: *size,
                placement: placement_dto(placement),
            },
            PrimitiveSpec::Cylinder {
                radius,
                height,
                segments,
            } => NodeKindDto::Cylinder {
                radius: *radius,
                height: *height,
                segments: *segments,
                placement: placement_dto(placement),
            },
        },
        NodeKind::Csg { op, operands } => NodeKindDto::Csg {
            csg: String::from(match op {
                CsgOp::Union => "union",
                CsgOp::Difference => "difference",
                CsgOp::Intersection => "intersection",
            }),
            operands: operands.iter().map(|n| n.0).collect(),
        },
        NodeKind::Transform { child, xf } => NodeKindDto::Transform {
            child: child.0,
            placement: placement_dto(xf),
        },
        NodeKind::Mirror { child, plane } => NodeKindDto::Mirror {
            child: child.0,
            plane: plane_dto(plane),
        },
        NodeKind::Instance { of, placement } => NodeKindDto::Instance {
            of: of.0,
            placement: placement_dto(placement),
        },
        NodeKind::Group { children } => NodeKindDto::Group {
            children: children.iter().map(|n| n.0).collect(),
        },
        NodeKind::MeshImport { import, placement } => NodeKindDto::MeshImport {
            import: import.0,
            placement: placement_dto(placement),
        },
        NodeKind::EdgeFinish {
            child,
            selection,
            policy,
        } => {
            let (kind, offset) = match policy.kind {
                RoundKind::Fillet { radius } => ("fillet", radius),
                RoundKind::Chamfer { setback } => ("chamfer", setback),
            };
            NodeKindDto::EdgeFinish {
                child: child.0,
                boundaries: match selection {
                    EdgeSelection::SharpEdges => None,
                    EdgeSelection::RegionBoundaries(pairs) => {
                        Some(EdgeBoundariesDto::Regions(pairs.clone()))
                    }
                    EdgeSelection::OperandBoundaries(pairs) => Some(EdgeBoundariesDto::Operands(
                        pairs
                            .iter()
                            .map(|pair| {
                                pair.map(|source| OperandRegionDto {
                                    operand: source.operand,
                                    region: source.region,
                                })
                            })
                            .collect(),
                    )),
                },
                kind: String::from(kind),
                offset,
                segments: policy.segments,
                chord_tolerance: policy.chord_tolerance,
                sharpness_threshold: policy.sharpness_threshold,
                region: policy.region,
                max_planar_deviation: policy.max_planar_deviation,
                max_tangent_turn: policy.max_tangent_turn,
            }
        }
        NodeKind::StretchVertices { child, steps } => NodeKindDto::StretchVertices {
            child: child.0,
            steps: steps
                .iter()
                .map(|step| VertexStretchStepDto {
                    plane: plane_dto(&step.plane),
                    length: step.length,
                })
                .collect(),
        },
        NodeKind::Stretch {
            child,
            plane,
            length,
        } => NodeKindDto::Stretch {
            child: child.0,
            plane: plane_dto(plane),
            length: *length,
        },
        NodeKind::GridSurface {
            points,
            rows,
            cols,
            close_u,
            close_w,
            thickness,
            placement,
        } => NodeKindDto::GridSurface {
            points: points.clone(),
            rows: *rows,
            cols: *cols,
            close_u: *close_u,
            close_w: *close_w,
            thickness: *thickness,
            placement: placement_dto(placement),
        },
        NodeKind::Implicit {
            field,
            bounds,
            placement,
        } => NodeKindDto::Implicit {
            field: implicit_dto(field),
            bounds: bounds.map(|b| [b.min, b.max]),
            placement: placement_dto(placement),
        },
    }
}

fn implicit_dto(field: &Expr) -> ImplicitDto {
    let boxed = |field: &Expr| Box::new(implicit_dto(field));
    match field {
        Expr::Sphere(s) => ImplicitDto::Sphere {
            center: s.center(),
            radius: s.radius(),
        },
        Expr::Cuboid(c) => ImplicitDto::Cuboid {
            center: c.center(),
            half_extents: c.half_extents(),
        },
        Expr::Cylinder(c) => ImplicitDto::Cylinder {
            center: c.center(),
            axis: c.axis(),
            radius: c.radius(),
            half_height: c.half_height(),
        },
        Expr::Torus(t) => ImplicitDto::Torus {
            center: t.center(),
            axis: t.axis(),
            major_radius: t.major_radius(),
            minor_radius: t.minor_radius(),
        },
        Expr::HalfSpace(h) => ImplicitDto::HalfSpace {
            point: h.point(),
            normal: h.normal(),
        },
        Expr::Capsule(c) => ImplicitDto::Capsule {
            a: c.a(),
            b: c.b(),
            radius: c.radius(),
        },
        Expr::RoundCone(c) => ImplicitDto::RoundCone {
            a: c.a(),
            b: c.b(),
            radius_a: c.radius_a(),
            radius_b: c.radius_b(),
        },
        Expr::Extrude(e) => ImplicitDto::Extrude {
            profile: implicit_profile_dto(e.profile()),
            half_height: e.half_height(),
        },
        Expr::Revolve(r) => ImplicitDto::Revolve {
            profile: implicit_profile_dto(r.profile()),
        },
        Expr::Union(u) => ImplicitDto::Union {
            left: boxed(&u.left),
            right: boxed(&u.right),
        },
        Expr::Intersection(i) => ImplicitDto::Intersection {
            left: boxed(&i.left),
            right: boxed(&i.right),
        },
        Expr::Difference(d) => ImplicitDto::Difference {
            left: boxed(&d.left),
            right: boxed(&d.right),
        },
        Expr::SmoothUnion(s) => ImplicitDto::SmoothUnion {
            left: boxed(s.left()),
            right: boxed(s.right()),
            radius: s.radius(),
        },
        Expr::Blend(b) => ImplicitDto::Blend {
            parts: b.parts().iter().map(implicit_dto).collect(),
            radius: b.radius(),
        },
        Expr::Translate(t) => ImplicitDto::Translate {
            field: boxed(t.field()),
            offset: t.offset(),
        },
        Expr::Scale(s) => ImplicitDto::Scale {
            field: boxed(s.field()),
            factor: s.factor(),
        },
        Expr::Placed(p) => ImplicitDto::Placed {
            field: boxed(p.field()),
            placement: placement_dto(&p.placement()),
        },
        Expr::Tagged(t) => ImplicitDto::Tagged {
            field: boxed(&t.field),
            region: t.region,
        },
    }
}

fn implicit_profile_dto(profile: &ProfileExpr) -> ImplicitProfileDto {
    match profile {
        ProfileExpr::Circle(c) => ImplicitProfileDto::Circle {
            center: c.center(),
            radius: c.radius(),
        },
        ProfileExpr::Rectangle(r) => ImplicitProfileDto::Rectangle {
            center: r.center(),
            half_extents: r.half_extents(),
        },
        ProfileExpr::HalfPlane(h) => ImplicitProfileDto::HalfPlane {
            point: h.point(),
            normal: h.normal(),
        },
    }
}

fn implicit_value(dto: &ImplicitDto) -> Result<Expr, InterchangeError> {
    use exedra_implicit as im;
    let field = |dto: &ImplicitDto| implicit_value(dto);
    let refused = InterchangeError::InvalidImplicit;
    Ok(match dto {
        ImplicitDto::Sphere { center, radius } => {
            Expr::from(im::Sphere::new(*center, *radius).map_err(refused)?)
        }
        ImplicitDto::Cuboid {
            center,
            half_extents,
        } => Expr::from(im::Cuboid::new(*center, *half_extents).map_err(refused)?),
        ImplicitDto::Cylinder {
            center,
            axis,
            radius,
            half_height,
        } => Expr::from(im::Cylinder::new(*center, *axis, *radius, *half_height).map_err(refused)?),
        ImplicitDto::Torus {
            center,
            axis,
            major_radius,
            minor_radius,
        } => Expr::from(
            im::Torus::new(*center, *axis, *major_radius, *minor_radius).map_err(refused)?,
        ),
        ImplicitDto::HalfSpace { point, normal } => {
            Expr::from(im::HalfSpace::new(*point, *normal).map_err(refused)?)
        }
        ImplicitDto::Capsule { a, b, radius } => {
            Expr::from(im::Capsule::new(*a, *b, *radius).map_err(refused)?)
        }
        ImplicitDto::RoundCone {
            a,
            b,
            radius_a,
            radius_b,
        } => Expr::from(im::RoundCone::new(*a, *b, *radius_a, *radius_b).map_err(refused)?),
        ImplicitDto::Extrude {
            profile,
            half_height,
        } => Expr::from(
            im::Extrude::new(implicit_profile_value(profile)?, *half_height).map_err(refused)?,
        ),
        ImplicitDto::Revolve { profile } => {
            Expr::from(im::Revolve::new(implicit_profile_value(profile)?))
        }
        ImplicitDto::Union { left, right } => {
            Expr::from(im::Union::new(field(left)?, field(right)?))
        }
        ImplicitDto::Intersection { left, right } => {
            Expr::from(im::Intersection::new(field(left)?, field(right)?))
        }
        ImplicitDto::Difference { left, right } => {
            Expr::from(im::Difference::new(field(left)?, field(right)?))
        }
        ImplicitDto::SmoothUnion {
            left,
            right,
            radius,
        } => {
            Expr::from(im::SmoothUnion::new(field(left)?, field(right)?, *radius).map_err(refused)?)
        }
        ImplicitDto::Blend { parts, radius } => {
            let parts = parts.iter().map(field).collect::<Result<Vec<_>, _>>()?;
            Expr::from(im::SmoothUnionN::new(parts, *radius).map_err(refused)?)
        }
        ImplicitDto::Translate {
            field: part,
            offset,
        } => Expr::from(im::Translate::new(field(part)?, *offset).map_err(refused)?),
        ImplicitDto::Scale {
            field: part,
            factor,
        } => Expr::from(im::Scale::new(field(part)?, *factor).map_err(refused)?),
        ImplicitDto::Placed {
            field: part,
            placement,
        } => {
            Expr::from(im::Placed::new(field(part)?, placement_value(*placement)).map_err(refused)?)
        }
        ImplicitDto::Tagged {
            field: part,
            region,
        } => Expr::from(im::Tagged::new(field(part)?, *region)),
    })
}

fn implicit_profile_value(dto: &ImplicitProfileDto) -> Result<ProfileExpr, InterchangeError> {
    use exedra_implicit as im;
    let refused = InterchangeError::InvalidImplicit;
    Ok(match dto {
        ImplicitProfileDto::Circle { center, radius } => {
            ProfileExpr::from(im::Circle::new(*center, *radius).map_err(refused)?)
        }
        ImplicitProfileDto::Rectangle {
            center,
            half_extents,
        } => ProfileExpr::from(im::Rectangle::new(*center, *half_extents).map_err(refused)?),
        ImplicitProfileDto::HalfPlane { point, normal } => {
            ProfileExpr::from(im::HalfPlane::new(*point, *normal).map_err(refused)?)
        }
    })
}

/// Rebuilds a validated recipe from its interchange document.
///
/// # Errors
///
/// Returns a typed [`InterchangeError`]; the rebuild runs full builder
/// validation, so malformed geometry fails exactly like direct
/// construction.
pub fn from_dto(dto: &RecipeDto) -> Result<Recipe, InterchangeError> {
    if dto.format != FORMAT || dto.version != VERSION {
        return Err(InterchangeError::UnsupportedFormat);
    }
    let mut builder = RecipeBuilder::new();
    for source in &dto.sources {
        builder.source_ref(source);
    }
    for slot in &dto.slots {
        builder.material_slot(slot);
    }
    for policy in &dto.policies {
        builder.curve_policy(policy);
    }
    for import in &dto.imports {
        builder
            .add_import(import.to_mesh()?)
            .map_err(InterchangeError::Recipe)?;
    }
    for profile in &dto.profiles {
        let outer = loop_value(&profile.outer)?;
        let holes = profile
            .holes
            .iter()
            .map(|hole| loop_value(hole))
            .collect::<Result<Vec<_>, _>>()?;
        builder.add_profile(Profile2::new(outer, holes).map_err(InterchangeError::Profile)?);
    }
    for node in &dto.nodes {
        let (operation, surface_chart) = match &node.kind {
            NodeKindDto::Charted { operation, chart } => {
                (operation.as_ref(), Some((*chart).into()))
            }
            kind => (kind, None),
        };
        builder
            .add_node(Node {
                surface_chart,
                kind: kind_value(operation)?,
                source: node.source.map(SourceId),
                material: node.material.map(SlotId),
                issue: node.issue.map(SourceId),
            })
            .map_err(InterchangeError::Recipe)?;
    }
    builder
        .finish(NodeId(dto.root))
        .map_err(InterchangeError::Recipe)
}

fn kind_value(dto: &NodeKindDto) -> Result<NodeKind, InterchangeError> {
    Ok(match dto {
        NodeKindDto::Charted { .. } => {
            return Err(InterchangeError::Recipe(RecipeError::SurfaceChart(
                ChartError::WrongOperation,
            )));
        }
        NodeKindDto::OnWorkplane {
            support,
            child,
            attachment,
        } => NodeKind::OnWorkplane {
            support: NodeId(*support),
            child: NodeId(*child),
            attachment: attachment.clone().into(),
        },
        NodeKindDto::PlaneCut {
            child,
            plane,
            side,
            cap_region,
            cap_material,
        } => NodeKind::PlaneCut {
            child: NodeId(*child),
            plane: plane_value(*plane),
            side: match side.as_str() {
                "negative" => crate::ir::PlaneSide::Negative,
                "positive" => crate::ir::PlaneSide::Positive,
                _ => return Err(InterchangeError::UnknownValue { field: "side" }),
            },
            cap: crate::section::CutCap {
                region: *cap_region,
                material: cap_material.map(SlotId),
            },
        },
        NodeKindDto::ExtrudeToPlane {
            profile,
            placement,
            plane,
        } => NodeKind::ExtrudeToPlane {
            profile: ProfileId(*profile),
            placement: placement_value(*placement),
            plane: plane_value(*plane),
        },
        NodeKindDto::Extrude {
            profile,
            placement,
            height,
            caps,
        } => NodeKind::Extrude {
            profile: ProfileId(*profile),
            placement: placement_value(*placement),
            height: *height,
            caps: caps_value(caps)?,
        },
        NodeKindDto::Pocket {
            base,
            profile,
            end,
            depth,
        } => NodeKind::Pocket {
            base: NodeId(*base),
            profile: ProfileId(*profile),
            end: match end.as_str() {
                "start" => crate::ir::ExtrusionEnd::Start,
                "end" => crate::ir::ExtrusionEnd::End,
                _ => return Err(InterchangeError::UnknownValue { field: "end" }),
            },
            depth: *depth,
        },
        NodeKindDto::Revolve {
            profile,
            placement,
            sweep,
            caps,
        } => NodeKind::Revolve {
            profile: ProfileId(*profile),
            placement: placement_value(*placement),
            sweep: *sweep,
            caps: caps_value(caps)?,
        },
        NodeKindDto::Loft {
            sections,
            section_sources,
            policy,
            caps,
        } => NodeKind::Loft {
            sections: {
                if !section_sources.is_empty() && section_sources.len() != sections.len() {
                    return Err(InterchangeError::Recipe(RecipeError::InvalidParameter {
                        what: "loft section sources",
                    }));
                }
                sections
                    .iter()
                    .enumerate()
                    .map(|(i, (placement, profile))| LoftSection {
                        placement: placement_value(*placement),
                        profile: ProfileId(*profile),
                        source: section_sources.get(i).copied().flatten().map(SourceId),
                    })
                    .collect()
            },
            policy: match policy.as_str() {
                "ruled" => LoftPolicy::Ruled,
                "smooth" => LoftPolicy::Smooth,
                _ => {
                    return Err(InterchangeError::UnknownValue {
                        field: "loft policy",
                    });
                }
            },
            caps: caps_value(caps)?,
        },
        NodeKindDto::Sweep {
            profile,
            points,
            caps,
        } => NodeKind::Sweep {
            profile: ProfileId(*profile),
            section: SectionLaw::IDENTITY,
            path: Path3::Polyline {
                points: points.clone(),
                frame: FramePolicy::RotationMinimizing,
            },
            caps: caps_value(caps)?,
        },
        NodeKindDto::MiteredSweep {
            profile,
            points,
            section_x,
            miter_limit,
            caps,
        } => NodeKind::Sweep {
            profile: ProfileId(*profile),
            section: SectionLaw::IDENTITY,
            path: Path3::MiteredPolyline {
                points: points.clone(),
                section_x: *section_x,
                section_origin: [0.0; 2],
                closure: PathClosure::Open,
                miter_limit: *miter_limit,
            },
            caps: caps_value(caps)?,
        },
        NodeKindDto::MiteredPathSweep {
            profile,
            points,
            section_x,
            section_origin,
            closure,
            miter_limit,
            caps,
        } => NodeKind::Sweep {
            profile: ProfileId(*profile),
            section: SectionLaw::IDENTITY,
            path: Path3::MiteredPolyline {
                points: points.clone(),
                section_x: *section_x,
                section_origin: *section_origin,
                closure: match closure {
                    PathClosureDto::Open => PathClosure::Open,
                    PathClosureDto::ClosedPlanar { normal } => {
                        PathClosure::ClosedPlanar { normal: *normal }
                    }
                },
                miter_limit: *miter_limit,
            },
            caps: caps_value(caps)?,
        },
        NodeKindDto::CurvedSweep {
            profile,
            start,
            segments,
            section_x,
            caps,
        } => NodeKind::Sweep {
            profile: ProfileId(*profile),
            section: SectionLaw::IDENTITY,
            path: Path3::Curves {
                start: *start,
                segments: segments.iter().map(path_segment_value).collect(),
                section_x: *section_x,
                section_origin: [0.0; 2],
                closure: PathClosure::Open,
                joins: PathJoin::Smooth,
            },
            caps: caps_value(caps)?,
        },
        NodeKindDto::CurvedPathSweep {
            profile,
            start,
            segments,
            section_x,
            section_origin,
            closure,
            joins,
            caps,
        } => NodeKind::Sweep {
            profile: ProfileId(*profile),
            section: SectionLaw::IDENTITY,
            path: Path3::Curves {
                start: *start,
                segments: segments.iter().map(path_segment_value).collect(),
                section_x: *section_x,
                section_origin: *section_origin,
                closure: match closure {
                    PathClosureDto::Open => PathClosure::Open,
                    PathClosureDto::ClosedPlanar { normal } => {
                        PathClosure::ClosedPlanar { normal: *normal }
                    }
                },
                joins: match joins {
                    PathJoinDto::Smooth => PathJoin::Smooth,
                    PathJoinDto::Miter { limit } => PathJoin::Miter { limit: *limit },
                },
            },
            caps: caps_value(caps)?,
        },
        NodeKindDto::ShapedSweep {
            scale,
            twist,
            sweep,
        }
        | NodeKindDto::TurnedSweep {
            scale,
            twist,
            sweep,
            turns: _,
        } => {
            let field = if matches!(dto, NodeKindDto::TurnedSweep { .. }) {
                "turned_sweep.sweep"
            } else {
                "shaped_sweep.sweep"
            };
            if matches!(dto, NodeKindDto::TurnedSweep { turns: 0, .. }) {
                return Err(InterchangeError::UnknownValue {
                    field: "turned_sweep.turns",
                });
            }
            if matches!(
                **sweep,
                NodeKindDto::ShapedSweep { .. } | NodeKindDto::TurnedSweep { .. }
            ) {
                return Err(InterchangeError::UnknownValue { field });
            }
            let NodeKind::Sweep {
                profile,
                path,
                caps,
                ..
            } = kind_value(sweep)?
            else {
                return Err(InterchangeError::UnknownValue { field });
            };
            NodeKind::Sweep {
                profile,
                path,
                section: SectionLaw {
                    scale: law_value(scale),
                    twist: law_value(twist),
                    turns: match dto {
                        NodeKindDto::TurnedSweep { turns, .. } => *turns,
                        _ => 0,
                    },
                },
                caps,
            }
        }
        NodeKindDto::PlanarFace { profile, placement } => NodeKind::PlanarFace {
            profile: ProfileId(*profile),
            placement: placement_value(*placement),
        },
        NodeKindDto::Box { size, placement } => NodeKind::Primitive {
            spec: PrimitiveSpec::Box { size: *size },
            placement: placement_value(*placement),
        },
        NodeKindDto::Cylinder {
            radius,
            height,
            segments,
            placement,
        } => NodeKind::Primitive {
            spec: PrimitiveSpec::Cylinder {
                radius: *radius,
                height: *height,
                segments: *segments,
            },
            placement: placement_value(*placement),
        },
        NodeKindDto::Csg { csg, operands } => NodeKind::Csg {
            op: match csg.as_str() {
                "union" => CsgOp::Union,
                "difference" => CsgOp::Difference,
                "intersection" => CsgOp::Intersection,
                _ => return Err(InterchangeError::UnknownValue { field: "csg" }),
            },
            operands: operands.iter().map(|n| NodeId(*n)).collect(),
        },
        NodeKindDto::UvMap {
            options,
            child,
            projection,
            matrix,
            offset,
        } => NodeKind::UvMap {
            options: (*options).try_into()?,
            child: NodeId(*child),
            mapping: crate::ir::UvMapping {
                projection: (*projection).into(),
                transform: crate::ir::UvTransform {
                    matrix: *matrix,
                    offset: *offset,
                },
            },
        },
        NodeKindDto::Transform { child, placement } => NodeKind::Transform {
            child: NodeId(*child),
            xf: placement_value(*placement),
        },
        NodeKindDto::Mirror { child, plane } => NodeKind::Mirror {
            child: NodeId(*child),
            plane: plane_value(*plane),
        },
        NodeKindDto::Instance { of, placement } => NodeKind::Instance {
            of: NodeId(*of),
            placement: placement_value(*placement),
        },
        NodeKindDto::Group { children } => NodeKind::Group {
            children: children.iter().map(|n| NodeId(*n)).collect(),
        },
        NodeKindDto::MeshImport { import, placement } => NodeKind::MeshImport {
            import: crate::ir::ImportId(*import),
            placement: placement_value(*placement),
        },
        NodeKindDto::EdgeFinish {
            child,
            boundaries,
            kind,
            offset,
            segments,
            chord_tolerance,
            sharpness_threshold,
            region,
            max_planar_deviation,
            max_tangent_turn,
        } => NodeKind::EdgeFinish {
            child: NodeId(*child),
            selection: match boundaries {
                None => EdgeSelection::SharpEdges,
                Some(EdgeBoundariesDto::Regions(pairs)) => {
                    EdgeSelection::RegionBoundaries(pairs.clone())
                }
                Some(EdgeBoundariesDto::Operands(pairs)) => EdgeSelection::OperandBoundaries(
                    pairs
                        .iter()
                        .map(|pair| {
                            pair.map(|source| OperandRegion {
                                operand: source.operand,
                                region: source.region,
                            })
                        })
                        .collect(),
                ),
            },
            policy: RoundPolicy {
                kind: match kind.as_str() {
                    "fillet" => RoundKind::Fillet { radius: *offset },
                    "chamfer" => RoundKind::Chamfer { setback: *offset },
                    _ => {
                        return Err(InterchangeError::UnknownValue {
                            field: "edge finish kind",
                        });
                    }
                },
                segments: *segments,
                chord_tolerance: *chord_tolerance,
                sharpness_threshold: *sharpness_threshold,
                region: *region,
                max_planar_deviation: *max_planar_deviation,
                max_tangent_turn: *max_tangent_turn,
            },
        },
        NodeKindDto::StretchVertices { child, steps } => NodeKind::StretchVertices {
            child: NodeId(*child),
            steps: steps
                .iter()
                .map(|step| VertexStretchStep {
                    plane: plane_value(step.plane),
                    length: step.length,
                })
                .collect(),
        },
        NodeKindDto::Stretch {
            child,
            plane,
            length,
        } => NodeKind::Stretch {
            child: NodeId(*child),
            plane: plane_value(*plane),
            length: *length,
        },
        NodeKindDto::GridSurface {
            points,
            rows,
            cols,
            close_u,
            close_w,
            thickness,
            placement,
        } => NodeKind::GridSurface {
            points: points.clone(),
            rows: *rows,
            cols: *cols,
            close_u: *close_u,
            close_w: *close_w,
            thickness: *thickness,
            placement: placement_value(*placement),
        },
        NodeKindDto::Implicit {
            field,
            bounds,
            placement,
        } => NodeKind::Implicit {
            field: implicit_value(field)?,
            bounds: match bounds {
                None => None,
                Some([min, max]) => Some(
                    exedra_implicit::Aabb::new(*min, *max)
                        .ok_or(InterchangeError::UnknownValue { field: "bounds" })?,
                ),
            },
            placement: placement_value(*placement),
        },
    })
}

/// A spatial curve segment in the interchange format.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PathSegmentDto {
    /// Straight segment.
    Line {
        /// Endpoint.
        to: [f64; 3],
    },
    /// Circular arc around an axis.
    Arc {
        /// Point on the axis.
        axis_origin: [f64; 3],
        /// Axis direction.
        axis: [f64; 3],
        /// Signed right-handed angle.
        sweep: f64,
    },
    /// Spatial cubic Bezier.
    Cubic {
        /// First control point.
        control1: [f64; 3],
        /// Second control point.
        control2: [f64; 3],
        /// Endpoint.
        to: [f64; 3],
    },
}

fn law_dto(law: &Law) -> LawDto {
    match law {
        Law::Constant(value) => LawDto::Constant { value: *value },
        Law::Linear(keys) => LawDto::Linear { keys: keys.clone() },
    }
}

fn law_value(law: &LawDto) -> Law {
    match law {
        LawDto::Constant { value } => Law::Constant(*value),
        LawDto::Linear { keys } => Law::Linear(keys.clone()),
    }
}

fn path_segment_dto(segment: &crate::path::PathSegment3) -> PathSegmentDto {
    match segment {
        crate::path::PathSegment3::Line { to } => PathSegmentDto::Line { to: *to },
        crate::path::PathSegment3::Arc {
            axis_origin,
            axis,
            sweep,
        } => PathSegmentDto::Arc {
            axis_origin: *axis_origin,
            axis: *axis,
            sweep: *sweep,
        },
        crate::path::PathSegment3::Cubic {
            control1,
            control2,
            to,
        } => PathSegmentDto::Cubic {
            control1: *control1,
            control2: *control2,
            to: *to,
        },
    }
}
fn path_segment_value(segment: &PathSegmentDto) -> crate::path::PathSegment3 {
    match segment {
        PathSegmentDto::Line { to } => crate::path::PathSegment3::Line { to: *to },
        PathSegmentDto::Arc {
            axis_origin,
            axis,
            sweep,
        } => crate::path::PathSegment3::Arc {
            axis_origin: *axis_origin,
            axis: *axis,
            sweep: *sweep,
        },
        PathSegmentDto::Cubic {
            control1,
            control2,
            to,
        } => crate::path::PathSegment3::Cubic {
            control1: *control1,
            control2: *control2,
            to: *to,
        },
    }
}

/// Persistent surface intent; never a transient face index.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SurfaceSelectorDto {
    /// Authored start cap.
    StartCap,
    /// Authored terminal cap.
    EndCap,
    /// Start cap qualified by its generating-node source label.
    SourceStartCap {
        /// Opaque authored label.
        source: String,
    },
    /// Terminal cap qualified by its generating-node source label.
    SourceEndCap {
        /// Opaque authored label.
        source: String,
    },
    /// Unique geometric region.
    Region {
        /// Authored region number.
        region: u32,
    },
    /// Region qualified by its producing Boolean operand.
    OperandRegion {
        /// Operand index in that Boolean.
        operand: u16,
        /// Authored region number.
        region: u32,
    },
}

/// An attachment's persistent surface and authored body-local frame controls.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkplaneAttachmentDto {
    /// Surface to resolve afresh.
    pub surface: SurfaceSelectorDto,
    /// Point on the infinite projection line.
    pub anchor: [f64; 3],
    /// Projection line direction.
    pub projection: [f64; 3],
    /// Preferred in-plane X direction.
    pub x_direction: [f64; 3],
}
impl From<crate::workplane::WorkplaneAttachment> for WorkplaneAttachmentDto {
    fn from(value: crate::workplane::WorkplaneAttachment) -> Self {
        use crate::workplane::SurfaceSelector;
        Self {
            surface: match value.surface {
                SurfaceSelector::StartCap => SurfaceSelectorDto::StartCap,
                SurfaceSelector::SourceStartCap(source) => {
                    SurfaceSelectorDto::SourceStartCap { source }
                }
                SurfaceSelector::SourceEndCap(source) => {
                    SurfaceSelectorDto::SourceEndCap { source }
                }
                SurfaceSelector::EndCap => SurfaceSelectorDto::EndCap,
                SurfaceSelector::Region(region) => SurfaceSelectorDto::Region { region },
                SurfaceSelector::OperandRegion(value) => SurfaceSelectorDto::OperandRegion {
                    operand: value.operand,
                    region: value.region,
                },
            },
            anchor: value.anchor,
            projection: value.projection,
            x_direction: value.x_direction,
        }
    }
}
impl From<WorkplaneAttachmentDto> for crate::workplane::WorkplaneAttachment {
    fn from(value: WorkplaneAttachmentDto) -> Self {
        use crate::workplane::SurfaceSelector;
        Self {
            surface: match value.surface {
                SurfaceSelectorDto::StartCap => SurfaceSelector::StartCap,
                SurfaceSelectorDto::SourceStartCap { source } => {
                    SurfaceSelector::SourceStartCap(source)
                }
                SurfaceSelectorDto::SourceEndCap { source } => {
                    SurfaceSelector::SourceEndCap(source)
                }
                SurfaceSelectorDto::EndCap => SurfaceSelector::EndCap,
                SurfaceSelectorDto::Region { region } => SurfaceSelector::Region(region),
                SurfaceSelectorDto::OperandRegion { operand, region } => {
                    SurfaceSelector::OperandRegion(OperandRegion { operand, region })
                }
            },
            anchor: value.anchor,
            projection: value.projection,
            x_direction: value.x_direction,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;

    #[test]
    fn edge_boundary_wire_forms_are_distinct_and_preserve_legacy_values() {
        let legacy = "[[1,3],[2,4]]";
        let parsed: EdgeBoundariesDto = serde_json::from_str(legacy).unwrap();
        assert!(matches!(parsed, EdgeBoundariesDto::Regions(_)));
        assert_eq!(serde_json::to_string(&parsed).unwrap(), legacy);

        let qualified = r#"[[{"operand":0,"region":5},{"operand":1,"region":5}]]"#;
        let parsed: EdgeBoundariesDto = serde_json::from_str(qualified).unwrap();
        assert!(matches!(parsed, EdgeBoundariesDto::Operands(_)));
        assert_eq!(serde_json::to_string(&parsed).unwrap(), qualified);
        // This is the previous reader's boundaries-field type. The new form
        // must fail to parse rather than becoming an absent/all-sharp selection.
        assert!(serde_json::from_str::<Option<Vec<[u32; 2]>>>(qualified).is_err());
        for malformed in [
            r#"[[{"region":5},{"operand":1,"region":5}]]"#,
            r#"[[{"operand":65536,"region":5},{"operand":1,"region":5}]]"#,
            r#"[[5,{"operand":1,"region":5}]]"#,
        ] {
            assert!(serde_json::from_str::<EdgeBoundariesDto>(malformed).is_err());
        }
    }

    #[test]
    fn json_round_trip_preserves_fingerprints() {
        let recipe = crate::text::tests_support::full_coverage_recipe();
        let dto = to_dto(&recipe);
        let json = serde_json::to_string_pretty(&dto).expect("serializes");
        let parsed: RecipeDto = serde_json::from_str(&json).expect("parses");
        let rebuilt = from_dto(&parsed).expect("rebuilds");
        assert_eq!(
            recipe.recipe_fingerprint(),
            rebuilt.recipe_fingerprint(),
            "the wire must preserve content identity exactly"
        );
    }

    #[test]
    fn implicit_fields_as_deep_as_allowed_round_trip() {
        use exedra_implicit::{Expr, Sphere, Tagged};
        let field = (1..crate::implicit::MAX_IMPLICIT_NESTING).fold(
            Expr::from(Sphere::new([0.0; 3], 1.0).expect("ball")),
            |field, _| Expr::from(Tagged::new(field, 1)),
        );
        let mut b = RecipeBuilder::new();
        let leaf = b
            .add_node(NodeKind::Implicit {
                field,
                bounds: None,
                placement: Placement3::IDENTITY,
            })
            .expect("within the limit");
        let recipe = b.finish(leaf).expect("valid");
        let json = serde_json::to_string(&to_dto(&recipe)).expect("serializes");
        let parsed: RecipeDto = serde_json::from_str(&json).expect("parses");
        let rebuilt = from_dto(&parsed).expect("rebuilds");
        assert_eq!(recipe.recipe_fingerprint(), rebuilt.recipe_fingerprint());
    }

    #[test]
    fn frozen_schema_corpus_still_parses() {
        // The checked-in corpus pins v1 field names and shapes; if this
        // fails, the schema changed — additive evolution only, and this
        // file never regenerates silently.
        let json = include_str!("../goldens/recipe_v1.frozen.json");
        let dto: RecipeDto = serde_json::from_str(json).expect("frozen corpus parses");
        let recipe = from_dto(&dto).expect("frozen corpus rebuilds");
        assert_eq!(
            format!("{:032x}", recipe.recipe_fingerprint().0),
            include_str!("../goldens/recipe_v1.frozen.fingerprint").trim(),
            "frozen corpus fingerprint changed; the schema or encoding drifted"
        );
    }

    #[test]
    fn unknown_node_kinds_are_hard_errors() {
        let json = r#"{"op": "teleport", "warp": 9}"#;
        let parsed: Result<NodeKindDto, _> = serde_json::from_str(json);
        assert!(parsed.is_err(), "unknown ops must not be skippable");
    }

    #[test]
    fn unknown_fields_are_tolerated() {
        let json = r#"{
            "op": "extrude", "profile": 0, "height": 1.0, "caps": "both",
            "placement": [[1,0,0,0],[0,1,0,0],[0,0,1,0]],
            "future_field": {"anything": true}
        }"#;
        let parsed: Result<NodeDto, _> = serde_json::from_str(json);
        assert!(parsed.is_ok(), "additive evolution: unknown fields ignored");
    }

    #[test]
    fn wrong_version_is_rejected() {
        let recipe = crate::text::tests_support::full_coverage_recipe();
        let mut dto = to_dto(&recipe);
        dto.version = 2;
        assert!(matches!(
            from_dto(&dto),
            Err(InterchangeError::UnsupportedFormat)
        ));
    }

    /// Regenerates the frozen corpus. Run deliberately after a reviewed,
    /// additive schema change:
    /// `cargo test -p exedra_constructive --all-features -- --ignored bless_frozen_schema`
    #[test]
    #[ignore = "regenerates the frozen schema corpus; run deliberately"]
    fn bless_frozen_schema() {
        let recipe = crate::text::tests_support::full_coverage_recipe();
        let dto = to_dto(&recipe);
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("goldens");
        std::fs::write(
            dir.join("recipe_v1.frozen.json"),
            serde_json::to_string_pretty(&dto).expect("serializes"),
        )
        .expect("write corpus");
        let mut fingerprint = format!("{:032x}", recipe.recipe_fingerprint().0);
        fingerprint.push('\n');
        std::fs::write(dir.join("recipe_v1.frozen.fingerprint"), fingerprint)
            .expect("write fingerprint");
    }
}
