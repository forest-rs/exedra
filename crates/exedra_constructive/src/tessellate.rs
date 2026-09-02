// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Body tessellation: constructive bodies into Exedra meshes.
//!
//! Tessellation is deterministic (identical inputs and policy produce
//! bit-identical meshes on every platform) and provenance-carrying: every
//! produced face records the feature that generated it, down to profile
//! segment granularity.
//!
//! The f64 construction domain narrows to f32 exactly once, here, at vertex
//! emission (`as f32`, round-to-nearest-even).

use alloc::vec::Vec;

use exedra::{FaceBuildAttrs, MeshBuilder};
use exedra_triangulate::{PolygonInput, TriParams, triangulate};

use crate::discretize::{DiscretizePolicy, DiscretizedProfile, discretize_profile};
use crate::ir::{CapMode, Placement3, PrimitiveSpec};
use crate::len_u32;
use crate::profile::Profile2;
use exedra_math::{add, cross, dot, narrow, norm, scale, sub};

/// Evaluation policy shared by body tessellation.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct EvalPolicy {
    /// Curve discretization policy.
    pub discretize: DiscretizePolicy,
    /// Threshold on `|sin(turn angle)|` above which a profile corner
    /// authors a sharp lateral edge. Tangent-continuous junctions (arcs
    /// meeting lines smoothly) fall below any sensible threshold and stay
    /// smooth; square corners exceed it and crease.
    pub sharp_sin_threshold: f64,
}

impl Default for EvalPolicy {
    fn default() -> Self {
        Self {
            discretize: DiscretizePolicy::default(),
            sharp_sin_threshold: 0.1,
        }
    }
}

/// The feature of a body that produced a mesh element.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Feature {
    /// The surface of a single-sided planar-face body.
    PlanarFace,
    /// The start cap (profile plane, facing local -Z for extrusions).
    CapStart,
    /// The end cap.
    CapEnd,
    /// A profile boundary segment or a side-wall face generated from it.
    /// `loop_index` 0 is the outer loop, `1 + i` is hole `i`; `seg` is the
    /// source segment index within that loop.
    Wall {
        /// Which profile loop: 0 = outer, `1 + i` = hole `i`.
        loop_index: u16,
        /// Source segment index within the loop.
        seg: u32,
    },
    /// A loft wall face between sections `band` and `band + 1`.
    LoftWall {
        /// Index of the band (the gap after section `band`).
        band: u16,
        /// Which profile loop: 0 = outer, `1 + i` = hole `i`.
        loop_index: u16,
        /// Source segment index within the loop (of the first section).
        seg: u32,
    },
    /// A face of an opaque imported mesh.
    Imported,
    /// A face or vertex of a declared primitive. `region` is the stable
    /// primitive-local region emitted by `exedra_primitives` (for example a
    /// cylinder side or cap); different primitive kinds have independent
    /// region namespaces.
    PrimitiveRegion {
        /// Primitive-local semantic region.
        region: u32,
    },
    /// A face of a boolean result, attributed to the operand that
    /// produced it. Finer attribution rides `FACE_REGION`, which the
    /// pipeline carries through from the operand faces.
    BooleanFace {
        /// Operand index within the CSG node.
        operand: u16,
    },
    /// A sweep wall face between path points `band` and `band + 1`.
    SweepWall {
        /// Index of the path segment.
        band: u16,
        /// Which profile loop: 0 = outer, `1 + i` is hole `i`.
        loop_index: u16,
        /// Source segment index within the loop.
        seg: u32,
    },
    /// A face of a grid-surface body, attributed to its bilinear patch
    /// (side-wall faces attribute to the boundary patch they extend).
    GridPatch {
        /// Patch row index.
        row: u16,
        /// Patch column index.
        col: u16,
    },
    /// A vertex on a stretch section rim. Face ownership remains the source
    /// feature of the surface being extended; this additive vertex feature
    /// makes the deformation boundary addressable without replacing it.
    StretchSeam {
        /// `0` is the stationary rim and `1` is its translated counterpart.
        rim: u8,
    },
}

/// A tessellated body: the mesh plus its element provenance.
#[derive(Debug)]
pub struct TessellatedBody {
    /// The tessellated mesh.
    pub mesh: exedra::Mesh,
    /// Element provenance, pinned to the mesh's revision.
    pub source_map: crate::source_map::SourceMap,
}

/// Region values written into [`exedra::attr::FACE_REGION`].
///
/// Stable, documented mapping: `0` = start cap, `1` = end cap, `2 + k` =
/// side wall of global segment `k` (outer loop segments first, then each
/// hole's segments in order).
pub const REGION_CAP_START: u32 = 0;
/// Region value for the surface of a single-sided planar-face body.
pub const REGION_PLANAR_FACE: u32 = 0;
/// End-cap region value.
pub const REGION_CAP_END: u32 = 1;
/// First side-wall region value; segment `k` maps to `REGION_WALL_BASE + k`.
pub const REGION_WALL_BASE: u32 = 2;

/// Original wall identity retained by an internal profile rewrite.
#[derive(Copy, Clone)]
pub(crate) struct ExtrudeWallSource {
    pub(crate) region: u32,
    pub(crate) segment: u32,
}

/// Region values for grid-surface bodies (their own documented namespace,
/// like caps/walls for extrusions): the front surface (the given points).
pub const REGION_GRID_FRONT: u32 = 0;
/// The offset back surface of a thickened grid.
pub const REGION_GRID_BACK: u32 = 1;
/// First grid side-wall region; sides number `base + k` with `k` = 0 (row
/// 0 edge), 1 (last-row edge), 2 (column 0 edge), 3 (last-column edge).
pub const REGION_GRID_SIDE_BASE: u32 = 2;

/// Typed tessellation failure.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum TessellateError {
    /// Discretization failed (invalid policy).
    Discretize(crate::discretize::DiscretizeError),
    /// Profile-area triangulation failed; the profile was not simple after
    /// discretization.
    Triangulate(exedra_triangulate::TriError),
    /// Mesh construction failed (an internal invariant violation).
    Build(exedra::BuildError),
    /// A revolved profile touches or crosses the revolution axis; axis
    /// contact is out of scope for v1 (use primitives for solid cylinders
    /// and spheres).
    AxisContact {
        /// The smallest radius found in the discretized profile.
        min_radius: f64,
    },
    /// Loft sections are incompatible: correspondence requires the same
    /// hole count and identical per-loop point counts after
    /// discretization.
    SectionMismatch {
        /// Index of the offending section.
        section: usize,
    },
    /// All loft sections are coplanar, so the loft has no volumetric span.
    DegenerateLoft,
    /// A sweep path reverses onto itself at this point (anti-parallel
    /// adjacent segments give no miter tangent).
    PathCusp {
        /// Index of the offending path point.
        point: usize,
    },
    /// Geometry cannot be represented at the f32 mesh boundary (for example
    /// an overflow to infinity or a positive primitive extent narrowing to
    /// zero).
    NonFiniteGeometry,
    /// A declared cylinder requests more angular edges than the evaluation
    /// policy permits for one curved segment.
    PrimitiveSegmentLimit {
        /// Explicit segment count in the primitive specification.
        requested: u32,
        /// Maximum allowed by [`DiscretizePolicy::max_segment_edges`].
        maximum: u32,
    },
    /// A grid vertex has no usable normal (all adjacent patches are
    /// degenerate), so a thickness offset is undefined.
    DegenerateGrid {
        /// Grid row of the offending vertex.
        row: u32,
        /// Grid column of the offending vertex.
        col: u32,
    },
}

impl core::fmt::Display for TessellateError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Discretize(e) => write!(f, "discretization failed: {e}"),
            Self::Triangulate(e) => write!(f, "profile triangulation failed: {e}"),
            Self::Build(e) => write!(f, "mesh construction failed: {e:?}"),
            Self::AxisContact { min_radius } => write!(
                f,
                "revolved profile touches the axis (min radius {min_radius})"
            ),
            Self::SectionMismatch { section } => {
                write!(f, "loft section {section} does not correspond to section 0")
            }
            Self::DegenerateLoft => write!(f, "loft sections have no volumetric span"),
            Self::PathCusp { point } => {
                write!(f, "sweep path reverses onto itself at point {point}")
            }
            Self::NonFiniteGeometry => {
                write!(f, "geometry is not representable at the f32 mesh boundary")
            }
            Self::PrimitiveSegmentLimit { requested, maximum } => write!(
                f,
                "primitive requests {requested} segments, exceeding the policy limit of {maximum}"
            ),
            Self::DegenerateGrid { row, col } => {
                write!(f, "grid vertex ({row}, {col}) has no usable normal")
            }
        }
    }
}

impl core::error::Error for TessellateError {}

impl From<crate::discretize::DiscretizeError> for TessellateError {
    fn from(e: crate::discretize::DiscretizeError) -> Self {
        Self::Discretize(e)
    }
}

impl From<exedra_triangulate::TriError> for TessellateError {
    fn from(e: exedra_triangulate::TriError) -> Self {
        Self::Triangulate(e)
    }
}

impl From<exedra::BuildError> for TessellateError {
    fn from(e: exedra::BuildError) -> Self {
        Self::Build(e)
    }
}

fn apply_placement(p: &Placement3, v: [f64; 3]) -> [f64; 3] {
    let r = &p.rows;
    [
        r[0][0] * v[0] + r[0][1] * v[1] + r[0][2] * v[2] + r[0][3],
        r[1][0] * v[0] + r[1][1] * v[1] + r[1][2] * v[2] + r[1][3],
        r[2][0] * v[0] + r[2][1] * v[1] + r[2][2] * v[2] + r[2][3],
    ]
}

/// Determinant of the placement's linear part. Negative means the
/// placement reflects, and emitted face loops must reverse to keep
/// outward orientation.
pub(crate) fn det3(p: &Placement3) -> f64 {
    let r = &p.rows;
    exedra_math::det3([
        [r[0][0], r[0][1], r[0][2]],
        [r[1][0], r[1][1], r[1][2]],
        [r[2][0], r[2][1], r[2][2]],
    ])
}

/// Reorders per-edge attributes for a reversed face loop: reversed edge
/// `i` covers original edge `n-2-i` (and the last reversed edge covers the
/// original closing edge).
fn reversed_edge_attrs<T: Copy>(values: &[T]) -> Vec<T> {
    let n = values.len();
    (0..n)
        .map(|i| {
            if i + 1 < n {
                values[n - 2 - i]
            } else {
                values[n - 1]
            }
        })
        .collect()
}

/// A [`MeshBuilder`] that reverses face loops (and their per-edge
/// attributes) when the body's placement reflects, preserving outward
/// orientation under mirrors.
struct OrientedBuilder {
    inner: MeshBuilder,
    flip: bool,
    non_finite: bool,
}

impl OrientedBuilder {
    fn new(flip: bool) -> Self {
        Self {
            inner: MeshBuilder::new(),
            flip,
            non_finite: false,
        }
    }

    fn push_vertex(&mut self, position: [f32; 3]) -> u32 {
        // Extreme-but-finite f64 parameters can overflow the f32 narrowing;
        // track it so tessellation fails typed instead of emitting infinite
        // geometry.
        self.non_finite |= position.iter().any(|c| !c.is_finite());
        self.inner.push_vertex(position)
    }

    fn add_face_with_attrs(
        &mut self,
        corners: &[u32],
        attrs: &FaceBuildAttrs<'_>,
    ) -> Result<(), exedra::BuildError> {
        if !self.flip {
            return self.inner.add_face_with_attrs(corners, attrs);
        }
        let reversed: Vec<u32> = corners.iter().rev().copied().collect();
        let seams = attrs.edge_seams.map(reversed_edge_attrs);
        let sharpness = attrs.edge_sharpness.map(reversed_edge_attrs);
        self.inner.add_face_with_attrs(
            &reversed,
            &FaceBuildAttrs {
                region: attrs.region,
                edge_seams: seams.as_deref(),
                edge_sharpness: sharpness.as_deref(),
            },
        )
    }

    fn build(&self) -> Result<exedra::MeshBuildResult, TessellateError> {
        if self.non_finite {
            return Err(TessellateError::NonFiniteGeometry);
        }
        self.inner.build().map_err(TessellateError::from)
    }
}

/// Tessellates a declared primitive through the workspace's canonical
/// primitive backend.
///
/// `PrimitiveSpec` deliberately exposes only the primitive parameters that
/// belong in constructive IR. Backend conveniences such as centering and cap
/// styles are fixed here to uphold that IR contract: boxes start at their
/// minimum corner, and cylinders are capped, start at their base centre, and
/// grow along local +Z. The primitive backend's cylinder grows along +Y, so
/// its vertices are rotated `(x, y, z) -> (x, -z, y)` before placement. This
/// is a proper rotation (not a reflection), which preserves winding.
///
/// Primitive-local face regions are copied into Exedra's `FACE_REGION`
/// attribute. This is essential rather than decorative metadata: mesh CSG
/// carries that attribute into its result for downstream attribution. Named
/// backend selections do not cross this boundary because [`TessellatedBody`]
/// represents provenance through its source map and mesh regions.
///
/// # Errors
///
/// Returns a typed error when an f64 parameter cannot cross the backend's f32
/// mesh boundary, a placed vertex becomes non-finite, the explicit cylinder
/// tessellation exceeds policy, or rebuilt topology fails validation.
pub fn tessellate_primitive(
    spec: PrimitiveSpec,
    placement: &Placement3,
    policy: &EvalPolicy,
) -> Result<TessellatedBody, TessellateError> {
    let (primitive, coordinates) = match spec {
        PrimitiveSpec::Box { size } => {
            let backend_size = [
                narrow_positive_parameter(size[0])?,
                narrow_positive_parameter(size[1])?,
                narrow_positive_parameter(size[2])?,
            ];
            (
                exedra_primitives::box_primitive(&exedra_primitives::BoxParams {
                    size: backend_size,
                    centered: false,
                    segments: [1, 1, 1],
                }),
                PrimitiveCoordinates::Box { size },
            )
        }
        PrimitiveSpec::Cylinder {
            radius,
            height,
            segments,
        } => {
            if segments > policy.discretize.max_segment_edges {
                return Err(TessellateError::PrimitiveSegmentLimit {
                    requested: segments,
                    maximum: policy.discretize.max_segment_edges,
                });
            }
            (
                exedra_primitives::cylinder(&exedra_primitives::CylinderParams {
                    radius: narrow_positive_parameter(radius)?,
                    height: narrow_positive_parameter(height)?,
                    segments,
                    cap_fill: exedra_primitives::CapFill::Ngon,
                    centered: false,
                }),
                PrimitiveCoordinates::Cylinder {
                    radius,
                    height,
                    segments,
                },
            )
        }
    };

    rebuild_placed_primitive(primitive, coordinates, placement)
}

fn narrow_positive_parameter(value: f64) -> Result<f32, TessellateError> {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "declared primitive parameters cross the documented f32 mesh boundary here"
    )]
    let narrowed = value as f32;
    if narrowed.is_finite() && narrowed > 0.0 {
        Ok(narrowed)
    } else {
        Err(TessellateError::NonFiniteGeometry)
    }
}

#[derive(Copy, Clone)]
enum PrimitiveCoordinates {
    Box {
        size: [f64; 3],
    },
    Cylinder {
        radius: f64,
        height: f64,
        segments: u32,
    },
}

fn rebuild_placed_primitive(
    primitive: exedra_primitives::Primitive,
    coordinates: PrimitiveCoordinates,
    placement: &Placement3,
) -> Result<TessellatedBody, TessellateError> {
    let source = &primitive.mesh;
    let mut builder = OrientedBuilder::new(det3(placement) < 0.0);
    let mut vertex_remap: Vec<Option<u32>> = Vec::new();
    let mut local_vertex_features = Vec::new();

    for (ordinal, vertex) in source.vertices().enumerate() {
        let local = primitive_local_position(source, vertex, ordinal, coordinates);
        let local_index = builder.push_vertex(narrow(apply_placement(placement, local)));
        let index = vertex.index() as usize;
        if vertex_remap.len() <= index {
            vertex_remap.resize(index + 1, None);
        }
        vertex_remap[index] = Some(local_index);
        local_vertex_features.push(None);
    }

    let mut local_face_features = Vec::new();
    for face in source.faces() {
        let region = primitive.face_region.get(face).0;
        let feature = Feature::PrimitiveRegion { region };
        let loop_edges: Vec<_> = source.face_loop(face).collect();
        let corners: Vec<u32> = loop_edges
            .iter()
            .map(|&half_edge| {
                let vertex = source
                    .to_vertex(half_edge)
                    .expect("primitive face loops end at live vertices");
                let local_index = vertex_remap[vertex.index() as usize]
                    .expect("every primitive vertex was remapped");
                local_vertex_features[local_index as usize].get_or_insert(feature);
                local_index
            })
            .collect();
        let seams: Vec<bool> = loop_edges
            .iter()
            .map(|&half_edge| source.edge_seam(half_edge).unwrap_or(false))
            .collect();
        let sharpness: Vec<f32> = loop_edges
            .iter()
            .map(|&half_edge| source.edge_sharpness(half_edge).unwrap_or(0.0))
            .collect();
        builder.add_face_with_attrs(
            &corners,
            &FaceBuildAttrs {
                region: Some(region),
                edge_seams: Some(&seams),
                edge_sharpness: Some(&sharpness),
            },
        )?;
        local_face_features.push(feature);
    }

    let build = builder.build()?;
    let default_feature = Feature::PrimitiveRegion {
        region: primitive.face_region.default.0,
    };
    let mut face_features = alloc::vec![default_feature; build.mesh.faces().count()];
    for (local_index, &face) in build.face_ids.iter().enumerate() {
        face_features[face.index() as usize] = local_face_features[local_index];
    }
    let mut vertex_features = alloc::vec![default_feature; build.mesh.vertices().count()];
    for (local_index, &vertex) in build.vertex_ids.iter().enumerate() {
        vertex_features[vertex.index() as usize] =
            local_vertex_features[local_index].unwrap_or(default_feature);
    }
    let source_map = crate::source_map::SourceMap::new(&build.mesh, face_features, vertex_features);
    Ok(TessellatedBody {
        mesh: build.mesh,
        source_map,
    })
}

fn primitive_local_position(
    source: &exedra::Mesh,
    vertex: exedra::VertexId,
    ordinal: usize,
    coordinates: PrimitiveCoordinates,
) -> [f64; 3] {
    match coordinates {
        PrimitiveCoordinates::Box { size } => {
            let backend = source
                .vertex_position(vertex)
                .expect("live primitive vertices have positions");
            // The uncentered, one-segment backend box has only zero/max
            // coordinates. Recover that corner classification but use the
            // declared f64 extents, not their f32 backend copies, so placement
            // still happens before the one final narrowing.
            core::array::from_fn(|axis| {
                if backend[axis] == 0.0 {
                    0.0
                } else {
                    size[axis]
                }
            })
        }
        PrimitiveCoordinates::Cylinder {
            radius,
            height,
            segments,
        } => {
            // Capped-ngon cylinders contain exactly two rings, emitted in
            // increasing angular order. Re-realize that documented order in
            // f64/libm: the backend owns topology and semantic metadata, while
            // constructive owns its f64-until-emission numeric contract.
            let segments_usize = segments as usize;
            debug_assert!(
                ordinal < segments_usize * 2,
                "capped-ngon cylinder vertices are exactly two rings"
            );
            let ring_index = ordinal % segments_usize;
            #[expect(
                clippy::cast_precision_loss,
                reason = "u32 segment indices are intentionally sampled in the f64 construction domain"
            )]
            let theta = (ring_index as f64) * core::f64::consts::TAU / f64::from(segments);
            let native_x = radius * libm::cos(theta);
            let native_y = if ordinal < segments_usize {
                0.0
            } else {
                height
            };
            let native_z = radius * libm::sin(theta);
            // Rotate +90 degrees around X: backend +Y becomes constructive
            // +Z without a handedness change.
            [native_x, -native_z, native_y]
        }
    }
}

/// Tessellates a profile as one single-sided open surface in local XY,
/// facing local +Z and placed by `placement`.
///
/// The outer loop and holes are always triangulated, so the resulting mesh
/// represents the profile interior without adding a back face or thickness.
/// Every triangle carries [`REGION_PLANAR_FACE`] and [`Feature::PlanarFace`].
/// Boundary vertices retain the loop and source-segment identity of the
/// discretized edge they generate through [`Feature::Wall`].
///
/// # Errors
///
/// Returns a typed [`TessellateError`] when curve discretization, polygon
/// triangulation, coordinate narrowing, or mesh construction fails.
pub fn tessellate_planar_face(
    profile: &Profile2,
    placement: &Placement3,
    policy: &EvalPolicy,
) -> Result<TessellatedBody, TessellateError> {
    let discretized = discretize_profile(profile, &policy.discretize)?;
    let holes: Vec<&[[f64; 2]]> = discretized
        .holes
        .iter()
        .map(|hole| hole.points.as_slice())
        .collect();
    let triangles = triangulate(
        &PolygonInput {
            outer: &discretized.outer.points,
            holes: &holes,
        },
        &TriParams::default(),
    )?;

    // Triangulation indices address outer ++ holes in exactly the same order
    // used here and by profile_vertex_features. A reflecting placement flips
    // complete triangle loops after transforming points, keeping the visible
    // surface oriented toward the transformed local +Z direction.
    let mut builder = OrientedBuilder::new(det3(placement) < 0.0);
    for ring in discretized.rings() {
        for point in &ring.points {
            builder.push_vertex(narrow(apply_placement(
                placement,
                [point[0], point[1], 0.0],
            )));
        }
    }
    let mut face_features = Vec::with_capacity(triangles.triangles.len());
    for triangle in triangles.triangles {
        builder.add_face_with_attrs(
            &triangle,
            &FaceBuildAttrs {
                region: Some(REGION_PLANAR_FACE),
                ..FaceBuildAttrs::default()
            },
        )?;
        face_features.push(Feature::PlanarFace);
    }

    let result = builder.build()?;
    let vertex_features = profile_vertex_features(&discretized, 1);
    let source_map =
        crate::source_map::SourceMap::new(&result.mesh, face_features, vertex_features);
    Ok(TessellatedBody {
        mesh: result.mesh,
        source_map,
    })
}

/// Tessellates an extrusion: the profile's local XY plane extruded along
/// local +Z by `height`, placed by `placement`.
///
/// Caps are pre-triangulated for concave or holed profiles (single ngons
/// for convex hole-free ones); side walls are one quad per discretized
/// edge, with `FACE_REGION` naming the source segment. Lateral edges at
/// profile corners sharper than the policy threshold are creased, as are
/// cap rims; arc interiors stay smooth.
///
/// # Errors
///
/// Returns a typed [`TessellateError`]; never panics.
pub fn tessellate_extrude(
    profile: &Profile2,
    placement: &Placement3,
    height: f64,
    caps: CapMode,
    policy: &EvalPolicy,
) -> Result<TessellatedBody, TessellateError> {
    tessellate_extrude_with_wall_sources(profile, placement, height, caps, policy, None)
}

/// Tessellates an extrusion while retaining wall identities through an
/// internal profile rewrite. Public callers use the profile's segment order;
/// constructive operators can provide the source region and segment for each
/// rewritten segment instead.
pub(crate) fn tessellate_extrude_with_wall_sources(
    profile: &Profile2,
    placement: &Placement3,
    height: f64,
    caps: CapMode,
    policy: &EvalPolicy,
    wall_sources: Option<&[Vec<ExtrudeWallSource>]>,
) -> Result<TessellatedBody, TessellateError> {
    let d = discretize_profile(profile, &policy.discretize)?;
    let flip = det3(placement) < 0.0;

    // Ring layout: outer points first, then each hole's points, matching
    // the triangulator's index convention.
    let ring_starts = ring_starts(&d);
    let total: usize = d.points_len();
    let mut builder = OrientedBuilder::new(flip);

    // Bottom ring vertices (z = 0), then top ring vertices (z = height).
    for ring in d.rings() {
        for p in &ring.points {
            builder.push_vertex(narrow(apply_placement(placement, [p[0], p[1], 0.0])));
        }
    }
    for ring in d.rings() {
        for p in &ring.points {
            builder.push_vertex(narrow(apply_placement(placement, [p[0], p[1], height])));
        }
    }
    let top_offset = len_u32(total);

    let mut face_origins: Vec<Feature> = Vec::new();

    // Global segment index offsets per loop for region numbering.
    let seg_offsets = seg_offsets(profile);

    // Corner sharpness per loop, from exact source tangents.
    let corner_sharp: Vec<Vec<bool>> = core::iter::once(profile.outer())
        .chain(profile.holes().iter())
        .map(|source| loop_corner_sharpness(source, policy))
        .collect();

    // Side walls: one quad per discretized edge, every ring.
    let bottom_cap = matches!(caps, CapMode::Both | CapMode::Start);
    let top_cap = matches!(caps, CapMode::Both | CapMode::End);
    for (ring_index, ring) in d.rings().enumerate() {
        let base = ring_starts[ring_index];
        let n = len_u32(ring.points.len());
        let loop_index = u16::try_from(ring_index).unwrap_or(u16::MAX);
        for i in 0..n {
            let j = (i + 1) % n;
            let b_i = base + i;
            let b_j = base + j;
            let t_i = top_offset + base + i;
            let t_j = top_offset + base + j;
            let seg = ring.edge_seg[i as usize];
            // The exact stretch path can split one source segment into
            // several analytic segments. All pieces must keep the source
            // wall's material region and feature identity; ordinary
            // tessellation has the documented positional mapping.
            let wall_source = wall_sources
                .map(|loops| loops[ring_index][seg as usize])
                .unwrap_or(ExtrudeWallSource {
                    region: REGION_WALL_BASE + seg_offsets[ring_index] + seg,
                    segment: seg,
                });

            // Lateral sharpness at ring points i and j: only original
            // endpoints (segment boundaries) are candidates — a point where
            // edge ownership changes is the start endpoint of its segment —
            // and the verdict comes from exact source tangents, so arc
            // interiors and tangent junctions stay smooth at any
            // discretization density.
            let sharp_at = |point: u32| {
                ring.is_endpoint(point)
                    && corner_sharp[ring_index][ring.edge_seg[point as usize] as usize]
            };
            let sharp_i = sharp_at(i);
            let sharp_j = sharp_at(j);

            // Quad loop [b_i, b_j, t_j, t_i]: edges are bottom rim,
            // lateral j, top rim, lateral i.
            let sharp = [
                if bottom_cap { 1.0 } else { 0.0 },
                if sharp_j { 1.0 } else { 0.0 },
                if top_cap { 1.0 } else { 0.0 },
                if sharp_i { 1.0 } else { 0.0 },
            ];
            builder.add_face_with_attrs(
                &[b_i, b_j, t_j, t_i],
                &FaceBuildAttrs {
                    region: Some(wall_source.region),
                    edge_seams: None,
                    edge_sharpness: Some(&sharp),
                },
            )?;
            face_origins.push(Feature::Wall {
                loop_index,
                seg: wall_source.segment,
            });
        }
    }

    // Caps.
    let convex_simple = d.holes.is_empty() && is_convex_ring(&d.outer);
    if bottom_cap || top_cap {
        if convex_simple {
            let n = len_u32(d.outer.points.len());
            if bottom_cap {
                // Bottom cap faces -Z: reverse the CCW ring.
                let ring: Vec<u32> = (0..n).rev().collect();
                builder.add_face_with_attrs(
                    &ring,
                    &FaceBuildAttrs {
                        region: Some(REGION_CAP_START),
                        ..FaceBuildAttrs::default()
                    },
                )?;
                face_origins.push(Feature::CapStart);
            }
            if top_cap {
                let ring: Vec<u32> = (0..n).map(|i| top_offset + i).collect();
                builder.add_face_with_attrs(
                    &ring,
                    &FaceBuildAttrs {
                        region: Some(REGION_CAP_END),
                        ..FaceBuildAttrs::default()
                    },
                )?;
                face_origins.push(Feature::CapEnd);
            }
        } else {
            let holes: Vec<&[[f64; 2]]> = d.holes.iter().map(|h| h.points.as_slice()).collect();
            let input = PolygonInput {
                outer: &d.outer.points,
                holes: &holes,
            };
            let tri = triangulate(&input, &TriParams::default())?;
            for t in &tri.triangles {
                if bottom_cap {
                    builder.add_face_with_attrs(
                        &[t[2], t[1], t[0]],
                        &FaceBuildAttrs {
                            region: Some(REGION_CAP_START),
                            ..FaceBuildAttrs::default()
                        },
                    )?;
                    face_origins.push(Feature::CapStart);
                }
            }
            for t in &tri.triangles {
                if top_cap {
                    builder.add_face_with_attrs(
                        &[top_offset + t[0], top_offset + t[1], top_offset + t[2]],
                        &FaceBuildAttrs {
                            region: Some(REGION_CAP_END),
                            ..FaceBuildAttrs::default()
                        },
                    )?;
                    face_origins.push(Feature::CapEnd);
                }
            }
        }
    }

    let result = builder.build()?;
    let vertex_features = profile_vertex_features(&d, 2);
    let source_map = crate::source_map::SourceMap::new(&result.mesh, face_origins, vertex_features);
    Ok(TessellatedBody {
        mesh: result.mesh,
        source_map,
    })
}

/// Vertex features: each ring point's wall feature, repeated for each of
/// the body's vertex rings (2 for extrusions, one per angular step for
/// revolutions).
fn profile_vertex_features(d: &DiscretizedProfile, vertex_rings: u32) -> Vec<Feature> {
    let mut per_ring: Vec<Feature> = Vec::with_capacity(d.points_len());
    for (ring_index, ring) in d.rings().enumerate() {
        let loop_index = u16::try_from(ring_index).unwrap_or(u16::MAX);
        for &seg in &ring.edge_seg {
            per_ring.push(Feature::Wall { loop_index, seg });
        }
    }
    let mut out = Vec::with_capacity(per_ring.len() * vertex_rings as usize);
    for _ in 0..vertex_rings {
        out.extend_from_slice(&per_ring);
    }
    out
}

impl DiscretizedProfile {
    fn rings(&self) -> impl Iterator<Item = &crate::discretize::DiscretizedLoop> {
        core::iter::once(&self.outer).chain(self.holes.iter())
    }

    fn points_len(&self) -> usize {
        self.outer.points.len() + self.holes.iter().map(|h| h.points.len()).sum::<usize>()
    }
}

impl crate::discretize::DiscretizedLoop {
    /// True when ring point `i` is an exact source endpoint (edge ownership
    /// changes there).
    fn is_endpoint(&self, i: u32) -> bool {
        let n = self.edge_seg.len();
        let prev = self.edge_seg[(i as usize + n - 1) % n];
        self.edge_seg[i as usize] != prev
    }
}

fn ring_starts(d: &DiscretizedProfile) -> Vec<u32> {
    let mut starts = Vec::with_capacity(1 + d.holes.len());
    let mut acc = 0_u32;
    starts.push(acc);
    acc += len_u32(d.outer.points.len());
    for hole in &d.holes {
        starts.push(acc);
        acc += len_u32(hole.points.len());
    }
    starts
}

fn seg_offsets(profile: &Profile2) -> Vec<u32> {
    let mut offsets = Vec::with_capacity(1 + profile.holes().len());
    let mut acc = 0_u32;
    offsets.push(acc);
    acc += len_u32(profile.outer().segs().len());
    for hole in profile.holes() {
        offsets.push(acc);
        acc += len_u32(hole.segs().len());
    }
    offsets
}

/// Start and end tangent directions of a segment (unnormalized).
///
/// Arc tangents come from the tangent-chord angle: the tangent deviates
/// from the chord by half the sweep, and `cos(sweep/2)`, `sin(sweep/2)`
/// derive from the bulge by half-angle identities — pure arithmetic, no
/// trig, bit-deterministic.
fn seg_tangents(start: kurbo::Point, seg: &crate::profile::Seg2) -> ([f64; 2], [f64; 2]) {
    kind_tangents(start, seg.to, &seg.kind)
}

fn kind_tangents(
    start: kurbo::Point,
    to: kurbo::Point,
    kind: &crate::profile::SegKind,
) -> ([f64; 2], [f64; 2]) {
    use crate::profile::SegKind;
    let chord = [to.x - start.x, to.y - start.y];
    match kind {
        SegKind::Line => (chord, chord),
        SegKind::Arc { bulge } => {
            let bulge = *bulge;
            // cos(sweep/2) = (1 - b^2) / (1 + b^2); sin = 2b / (1 + b^2).
            let denom = 1.0 + bulge * bulge;
            let c = (1.0 - bulge * bulge) / denom;
            let s = 2.0 * bulge / denom;
            // Start tangent: chord rotated by -sweep/2; end: by +sweep/2.
            let start_t = [chord[0] * c + chord[1] * s, -chord[0] * s + chord[1] * c];
            let end_t = [chord[0] * c - chord[1] * s, chord[0] * s + chord[1] * c];
            (start_t, end_t)
        }
        SegKind::Cubic { c1, c2 } => {
            let s = [c1.x - start.x, c1.y - start.y];
            let e = [to.x - c2.x, to.y - c2.y];
            let s = if s == [0.0, 0.0] { chord } else { s };
            let e = if e == [0.0, 0.0] { chord } else { e };
            (s, e)
        }
        SegKind::PolicyTo {
            policy: _,
            realized,
        } => kind_tangents(start, to, realized),
    }
}

/// Per-segment corner sharpness for one source loop: entry `i` is true when
/// the original endpoint *starting* segment `i` (the junction between
/// segment `i - 1` and segment `i`) turns more than the policy threshold,
/// measured on exact source-curve tangents.
fn loop_corner_sharpness(source: &crate::profile::Loop2, policy: &EvalPolicy) -> Vec<bool> {
    let tangents: Vec<([f64; 2], [f64; 2])> = source
        .iter_with_starts()
        .map(|(start, seg)| seg_tangents(start, seg))
        .collect();
    let n = tangents.len();
    (0..n)
        .map(|i| {
            let incoming = tangents[(i + n - 1) % n].1;
            let outgoing = tangents[i].0;
            let cross = incoming[0] * outgoing[1] - incoming[1] * outgoing[0];
            let la = libm::sqrt(incoming[0] * incoming[0] + incoming[1] * incoming[1]);
            let lb = libm::sqrt(outgoing[0] * outgoing[0] + outgoing[1] * outgoing[1]);
            libm::fabs(cross) > policy.sharp_sin_threshold * la * lb
        })
        .collect()
}

/// True when every turn of the (CCW) ring is non-reflex.
fn is_convex_ring(ring: &crate::discretize::DiscretizedLoop) -> bool {
    let n = ring.points.len();
    for i in 0..n {
        let a = ring.points[i];
        let b = ring.points[(i + 1) % n];
        let c = ring.points[(i + 2) % n];
        let cross = (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0]);
        if cross < 0.0 {
            return false;
        }
    }
    true
}

/// Tessellates a revolution: the profile's local `(x, y)` plane revolved
/// about the local Y axis, `x` as radius and `y` as height, swept through
/// `sweep` radians (counter-clockwise viewed from +Y), placed by
/// `placement`.
///
/// A full sweep (`sweep == tau`, compared exactly) closes on itself with a
/// seam meridian tagged via edge seams; partial sweeps close their
/// boundary planes according to `caps` (start cap at angle zero). The
/// profile must lie at strictly positive radius — bodies touching the axis
/// are rejected rather than emitted with degenerate walls.
///
/// # Errors
///
/// Returns a typed [`TessellateError`]; never panics.
pub fn tessellate_revolve(
    profile: &Profile2,
    placement: &Placement3,
    sweep: f64,
    caps: CapMode,
    policy: &EvalPolicy,
) -> Result<TessellatedBody, TessellateError> {
    let d = discretize_profile(profile, &policy.discretize)?;
    let flip = det3(placement) < 0.0;
    let full = sweep == core::f64::consts::TAU;

    // Strictly positive radius: no axis contact (documented v1 scope).
    let mut min_x = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    for ring in d.rings() {
        for p in &ring.points {
            min_x = min_x.min(p[0]);
            max_x = max_x.max(p[0]);
        }
    }
    if min_x <= 0.0 || min_x.is_nan() {
        return Err(TessellateError::AxisContact { min_radius: min_x });
    }

    // Angular step count from the chord tolerance at the outermost radius.
    let steps = {
        let tol = policy.discretize.chord_tolerance;
        let per_edge = if tol >= 2.0 * max_x {
            sweep
        } else {
            2.0 * libm::acos(1.0 - tol / max_x)
        };
        let needed = libm::ceil(sweep / per_edge);
        let capped = if needed >= f64::from(policy.discretize.max_segment_edges) {
            policy.discretize.max_segment_edges
        } else {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "needed is a ceil of a finite positive value below the u32 cap"
            )]
            {
                needed as u32
            }
        };
        capped.clamp(
            policy.discretize.min_arc_edges.max(3),
            policy.discretize.max_segment_edges,
        )
    };
    // Vertex rings: `steps` for a full sweep (wrapping), `steps + 1`
    // otherwise.
    let vertex_rings = if full { steps } else { steps + 1 };

    let ring_starts = ring_starts(&d);
    let total = len_u32(d.points_len());
    let mut builder = OrientedBuilder::new(flip);

    // Vertices: angular-step major, profile point minor. Angles evaluated
    // independently per step (no accumulation drift), libm trig only.
    let step_angle = sweep / f64::from(steps);
    for k in 0..vertex_rings {
        let angle = step_angle * f64::from(k);
        let (s, c) = (libm::sin(angle), libm::cos(angle));
        for ring in d.rings() {
            for p in &ring.points {
                let v = [p[0] * c, p[1], p[0] * s];
                builder.push_vertex(narrow(apply_placement(placement, v)));
            }
        }
    }
    let vertex_at = |k: u32, flat: u32| (k % vertex_rings) * total + flat;

    let mut face_origins: Vec<Feature> = Vec::new();
    let seg_offsets = seg_offsets(profile);
    let corner_sharp: Vec<Vec<bool>> = core::iter::once(profile.outer())
        .chain(profile.holes().iter())
        .map(|source| loop_corner_sharpness(source, policy))
        .collect();

    let start_cap = !full && matches!(caps, CapMode::Both | CapMode::Start);
    let end_cap = !full && matches!(caps, CapMode::Both | CapMode::End);

    // Walls: one quad per profile edge per angular step.
    for (ring_index, ring) in d.rings().enumerate() {
        let base = ring_starts[ring_index];
        let n = len_u32(ring.points.len());
        let loop_index = u16::try_from(ring_index).unwrap_or(u16::MAX);
        for i in 0..n {
            let j = (i + 1) % n;
            let seg = ring.edge_seg[i as usize];
            let sharp_at = |point: u32| {
                ring.is_endpoint(point)
                    && corner_sharp[ring_index][ring.edge_seg[point as usize] as usize]
            };
            let ring_sharp_i = sharp_at(i);
            let ring_sharp_j = sharp_at(j);
            for k in 0..steps {
                let a = vertex_at(k, base + i);
                let d_v = vertex_at(k, base + j);
                let c_v = vertex_at(k + 1, base + j);
                let b = vertex_at(k + 1, base + i);
                // Quad loop [a, d, c, b]: edges are the theta_k meridian
                // (a->d), the ring at profile point j (d->c), the
                // theta_{k+1} meridian (c->b), and the ring at point i
                // (b->a).
                let meridian_start = k == 0 && (start_cap || full);
                let meridian_end = (k + 1 == steps) && end_cap;
                let sharp = [
                    if k == 0 && start_cap { 1.0 } else { 0.0 },
                    if ring_sharp_j { 1.0 } else { 0.0 },
                    if meridian_end { 1.0 } else { 0.0 },
                    if ring_sharp_i { 1.0 } else { 0.0 },
                ];
                // Seams only on the seam-bearing face: writing `false`
                // entries from the wrapping neighbor would clobber the
                // shared canonical edge's `true`.
                let seams = [true, false, false, false];
                let edge_seams = (meridian_start && full).then_some(&seams[..]);
                builder.add_face_with_attrs(
                    &[a, d_v, c_v, b],
                    &FaceBuildAttrs {
                        region: Some(REGION_WALL_BASE + seg_offsets[ring_index] + seg),
                        edge_seams,
                        edge_sharpness: Some(&sharp),
                    },
                )?;
                face_origins.push(Feature::Wall { loop_index, seg });
            }
        }
    }

    // Caps for partial sweeps: the profile placed at the boundary planes.
    if start_cap || end_cap {
        let convex_simple = d.holes.is_empty() && is_convex_ring(&d.outer);
        let end_ring = vertex_rings - 1;
        if convex_simple {
            let n = len_u32(d.outer.points.len());
            if start_cap {
                // Start cap faces the negative tangential direction:
                // reverse the CCW profile ring at angle zero.
                let ring: Vec<u32> = (0..n).rev().map(|i| vertex_at(0, i)).collect();
                builder.add_face_with_attrs(
                    &ring,
                    &FaceBuildAttrs {
                        region: Some(REGION_CAP_START),
                        ..FaceBuildAttrs::default()
                    },
                )?;
                face_origins.push(Feature::CapStart);
            }
            if end_cap {
                let ring: Vec<u32> = (0..n).map(|i| vertex_at(end_ring, i)).collect();
                builder.add_face_with_attrs(
                    &ring,
                    &FaceBuildAttrs {
                        region: Some(REGION_CAP_END),
                        ..FaceBuildAttrs::default()
                    },
                )?;
                face_origins.push(Feature::CapEnd);
            }
        } else {
            let holes: Vec<&[[f64; 2]]> = d.holes.iter().map(|h| h.points.as_slice()).collect();
            let input = PolygonInput {
                outer: &d.outer.points,
                holes: &holes,
            };
            let tri = triangulate(&input, &TriParams::default())?;
            for t in &tri.triangles {
                if start_cap {
                    builder.add_face_with_attrs(
                        &[vertex_at(0, t[2]), vertex_at(0, t[1]), vertex_at(0, t[0])],
                        &FaceBuildAttrs {
                            region: Some(REGION_CAP_START),
                            ..FaceBuildAttrs::default()
                        },
                    )?;
                    face_origins.push(Feature::CapStart);
                }
            }
            for t in &tri.triangles {
                if end_cap {
                    builder.add_face_with_attrs(
                        &[
                            vertex_at(end_ring, t[0]),
                            vertex_at(end_ring, t[1]),
                            vertex_at(end_ring, t[2]),
                        ],
                        &FaceBuildAttrs {
                            region: Some(REGION_CAP_END),
                            ..FaceBuildAttrs::default()
                        },
                    )?;
                    face_origins.push(Feature::CapEnd);
                }
            }
        }
    }

    let result = builder.build()?;
    let vertex_features = profile_vertex_features(&d, vertex_rings);
    let source_map = crate::source_map::SourceMap::new(&result.mesh, face_origins, vertex_features);
    Ok(TessellatedBody {
        mesh: result.mesh,
        source_map,
    })
}

/// Tessellates a ruled loft between placed sections.
///
/// Each section is a profile with its own placement; corresponding
/// discretized ring points connect with quads band by band. Sections must
/// correspond: the same hole count and identical per-loop point counts
/// after discretization (a typed [`TessellateError::SectionMismatch`]
/// otherwise — frontends control correspondence through segment structure).
/// The start cap closes section 0 (reversed), the end cap the last section;
/// sections must be ordered so counter-clockwise outer loops yield
/// outward-facing walls (the extrude convention generalized).
///
/// Intermediate section rings crease (a ruled loft is only C0 across
/// sections); lateral edges crease at sharp corners of section 0's source
/// tangents, and cap rims crease when capped.
///
/// # Errors
///
/// Returns a typed [`TessellateError`]; never panics.
pub fn tessellate_loft(
    sections: &[(Placement3, &Profile2)],
    caps: CapMode,
    policy: &EvalPolicy,
) -> Result<TessellatedBody, TessellateError> {
    debug_assert!(sections.len() >= 2, "IR validation requires >= 2 sections");
    let flip = det3(&sections[0].0) < 0.0;
    let discretized: Vec<DiscretizedProfile> = sections
        .iter()
        .map(|(_, profile)| discretize_profile(profile, &policy.discretize))
        .collect::<Result<_, _>>()?;

    // Correspondence: identical loop structure across sections.
    let reference = &discretized[0];
    for (index, d) in discretized.iter().enumerate().skip(1) {
        let matches = d.holes.len() == reference.holes.len()
            && d.outer.points.len() == reference.outer.points.len()
            && d.holes
                .iter()
                .zip(&reference.holes)
                .all(|(a, b)| a.points.len() == b.points.len());
        if !matches {
            return Err(TessellateError::SectionMismatch { section: index });
        }
    }
    if !loft_has_volumetric_span(sections, &discretized) {
        return Err(TessellateError::DegenerateLoft);
    }

    let ring_starts = ring_starts(reference);
    let total = len_u32(reference.points_len());
    let mut builder = OrientedBuilder::new(flip);
    for ((placement, _), d) in sections.iter().zip(&discretized) {
        for ring in d.rings() {
            for p in &ring.points {
                builder.push_vertex(narrow(apply_placement(placement, [p[0], p[1], 0.0])));
            }
        }
    }
    let section_offset = |k: usize| len_u32(k) * total;

    let mut face_origins: Vec<Feature> = Vec::new();
    let seg_offsets = seg_offsets(sections[0].1);
    let corner_sharp: Vec<Vec<bool>> = core::iter::once(sections[0].1.outer())
        .chain(sections[0].1.holes().iter())
        .map(|source| loop_corner_sharpness(source, policy))
        .collect();

    let start_cap = matches!(caps, CapMode::Both | CapMode::Start);
    let end_cap = matches!(caps, CapMode::Both | CapMode::End);
    let bands = sections.len() - 1;

    for band in 0..bands {
        let below = section_offset(band);
        let above = section_offset(band + 1);
        let band_u16 = u16::try_from(band).unwrap_or(u16::MAX);
        for (ring_index, ring) in reference.rings().enumerate() {
            let base = ring_starts[ring_index];
            let n = len_u32(ring.points.len());
            let loop_index = u16::try_from(ring_index).unwrap_or(u16::MAX);
            for i in 0..n {
                let j = (i + 1) % n;
                let seg = ring.edge_seg[i as usize];
                let sharp_at = |point: u32| {
                    ring.is_endpoint(point)
                        && corner_sharp[ring_index][ring.edge_seg[point as usize] as usize]
                };
                // Ring creases: caps at the outer boundaries, and always at
                // intermediate sections (ruled bands are only C0 there).
                let bottom_crease = if band == 0 { start_cap } else { true };
                let top_crease = if band + 1 == bands { end_cap } else { true };
                let sharp = [
                    if bottom_crease { 1.0 } else { 0.0 },
                    if sharp_at(j) { 1.0 } else { 0.0 },
                    if top_crease { 1.0 } else { 0.0 },
                    if sharp_at(i) { 1.0 } else { 0.0 },
                ];
                builder.add_face_with_attrs(
                    &[
                        below + base + i,
                        below + base + j,
                        above + base + j,
                        above + base + i,
                    ],
                    &FaceBuildAttrs {
                        region: Some(REGION_WALL_BASE + seg_offsets[ring_index] + seg),
                        edge_seams: None,
                        edge_sharpness: Some(&sharp),
                    },
                )?;
                face_origins.push(Feature::LoftWall {
                    band: band_u16,
                    loop_index,
                    seg,
                });
            }
        }
    }

    // Caps: each boundary section triangulated from its own discretization.
    if start_cap || end_cap {
        let mut emit_cap = |d: &DiscretizedProfile,
                            offset: u32,
                            reverse: bool,
                            feature: Feature,
                            region: u32|
         -> Result<(), TessellateError> {
            let convex_simple = d.holes.is_empty() && is_convex_ring(&d.outer);
            if convex_simple {
                let n = len_u32(d.outer.points.len());
                let ring: Vec<u32> = if reverse {
                    (0..n).rev().map(|i| offset + i).collect()
                } else {
                    (0..n).map(|i| offset + i).collect()
                };
                builder.add_face_with_attrs(
                    &ring,
                    &FaceBuildAttrs {
                        region: Some(region),
                        ..FaceBuildAttrs::default()
                    },
                )?;
                face_origins.push(feature);
            } else {
                let holes: Vec<&[[f64; 2]]> = d.holes.iter().map(|h| h.points.as_slice()).collect();
                let input = PolygonInput {
                    outer: &d.outer.points,
                    holes: &holes,
                };
                let tri = triangulate(&input, &TriParams::default())?;
                for t in &tri.triangles {
                    let corners = if reverse {
                        [offset + t[2], offset + t[1], offset + t[0]]
                    } else {
                        [offset + t[0], offset + t[1], offset + t[2]]
                    };
                    builder.add_face_with_attrs(
                        &corners,
                        &FaceBuildAttrs {
                            region: Some(region),
                            ..FaceBuildAttrs::default()
                        },
                    )?;
                    face_origins.push(feature);
                }
            }
            Ok(())
        };
        if start_cap {
            emit_cap(
                &discretized[0],
                section_offset(0),
                true,
                Feature::CapStart,
                REGION_CAP_START,
            )?;
        }
        if end_cap {
            let last = sections.len() - 1;
            emit_cap(
                &discretized[last],
                section_offset(last),
                false,
                Feature::CapEnd,
                REGION_CAP_END,
            )?;
        }
    }

    let result = builder.build()?;
    let mut vertex_features = Vec::with_capacity(reference.points_len() * sections.len());
    for d in &discretized {
        let mut per_ring: Vec<Feature> = Vec::with_capacity(d.points_len());
        for (ring_index, ring) in d.rings().enumerate() {
            let loop_index = u16::try_from(ring_index).unwrap_or(u16::MAX);
            for &seg in &ring.edge_seg {
                per_ring.push(Feature::Wall { loop_index, seg });
            }
        }
        vertex_features.extend_from_slice(&per_ring);
    }
    let source_map = crate::source_map::SourceMap::new(&result.mesh, face_origins, vertex_features);
    Ok(TessellatedBody {
        mesh: result.mesh,
        source_map,
    })
}

fn loft_has_volumetric_span(
    sections: &[(Placement3, &Profile2)],
    discretized: &[DiscretizedProfile],
) -> bool {
    let first_points: Vec<[f64; 3]> = discretized[0]
        .rings()
        .flat_map(|ring| &ring.points)
        .map(|point| apply_placement(&sections[0].0, [point[0], point[1], 0.0]))
        .collect();
    let origin = first_points[0];
    let plane_scale = first_points
        .iter()
        .map(|&point| norm(sub(point, origin)))
        .fold(0.0_f64, f64::max);
    let area_tolerance = plane_scale * plane_scale * 64.0 * f64::EPSILON;
    let Some(normal) = first_points.iter().find_map(|&a| {
        first_points.iter().find_map(|&b| {
            let normal = cross(sub(a, origin), sub(b, origin));
            (norm(normal) > area_tolerance).then_some(normal)
        })
    }) else {
        return false;
    };
    let normal_length = norm(normal);
    let span_tolerance = plane_scale * 64.0 * f64::EPSILON;

    sections
        .iter()
        .zip(discretized)
        .flat_map(|((placement, _), profile)| {
            profile.rings().flat_map(move |ring| {
                ring.points
                    .iter()
                    .map(move |point| apply_placement(placement, [point[0], point[1], 0.0]))
            })
        })
        .any(|point| dot(normal, sub(point, origin)).abs() / normal_length > span_tolerance)
}

// --- Sweep -------------------------------------------------------------------

/// One sweep frame: `(origin, u, v, t)` with `u x v = t` (right-handed).
type SweepFrame = ([f64; 3], [f64; 3], [f64; 3], [f64; 3]);

/// Per-ring frames along a polyline: miter tangents plus a
/// rotation-minimizing normal transported by the double-reflection method
/// (pure arithmetic and square roots — deterministic).
///
/// Right-handedness means a straight +Z path reproduces the extrude
/// orientation exactly.
fn sweep_frames(
    points: &[[f64; 3]],
    policy: &EvalPolicy,
) -> Result<Vec<SweepFrame>, TessellateError> {
    let n = points.len();
    let mut dirs: Vec<[f64; 3]> = Vec::with_capacity(n - 1);
    for w in points.windows(2) {
        let d = sub(w[1], w[0]);
        dirs.push(scale(d, 1.0 / norm(d)));
    }
    // Miter tangents: endpoints use their segment, interior points the
    // bisector; anti-parallel segments have no bisector (typed cusp).
    let mut tangents: Vec<[f64; 3]> = Vec::with_capacity(n);
    tangents.push(dirs[0]);
    for i in 1..n - 1 {
        let sum = add(dirs[i - 1], dirs[i]);
        let len = norm(sum);
        if len <= 1e-12 {
            return Err(TessellateError::PathCusp { point: i });
        }
        tangents.push(scale(sum, 1.0 / len));
    }
    tangents.push(dirs[n - 2]);

    // Seed normal: Gram-Schmidt the world axis least aligned with t0
    // (ties resolve x before y before z) — deterministic, trig-free.
    let t0 = tangents[0];
    let abs = [libm::fabs(t0[0]), libm::fabs(t0[1]), libm::fabs(t0[2])];
    let axis_index = if abs[0] <= abs[1] && abs[0] <= abs[2] {
        0
    } else if abs[1] <= abs[2] {
        1
    } else {
        2
    };
    let mut axis = [0.0; 3];
    axis[axis_index] = 1.0;
    let mut u = sub(axis, scale(t0, dot(axis, t0)));
    u = scale(u, 1.0 / norm(u));

    let mut frames = Vec::with_capacity(n);
    let v0 = cross(tangents[0], u);
    frames.push((points[0], u, v0, tangents[0]));
    for i in 1..n {
        // Double reflection (Wang et al.): transport (u, t) from point
        // i-1 to point i without spin.
        let (p_prev, u_prev, _, t_prev) = frames[i - 1];
        let v1 = sub(points[i], p_prev);
        let c1 = dot(v1, v1);
        let u_l = sub(u_prev, scale(v1, 2.0 / c1 * dot(v1, u_prev)));
        let t_l = sub(t_prev, scale(v1, 2.0 / c1 * dot(v1, t_prev)));
        let v2 = sub(tangents[i], t_l);
        let c2 = dot(v2, v2);
        let u_i = if c2 <= 1e-24 {
            u_l
        } else {
            sub(u_l, scale(v2, 2.0 / c2 * dot(v2, u_l)))
        };
        let v_i = cross(tangents[i], u_i);
        frames.push((points[i], u_i, v_i, tangents[i]));
    }
    let _ = policy;
    Ok(frames)
}

/// Tessellates a sweep: the profile carried along a polyline path under a
/// rotation-minimizing frame, placed by `placement`.
///
/// Rings sit at every path point (miter joints); consecutive rings connect
/// with quads attributed as [`Feature::SweepWall`] per path segment. The
/// start cap closes the first ring (reversed), the end cap the last. Ring
/// edges at path corners sharper than the policy threshold crease, as do
/// profile-corner laterals (section-0 tangent rule) and cap rims. Tight
/// joints can self-intersect the miter ring — the sweep does not detect
/// that (v1 scope; keep joint angles moderate relative to profile size).
///
/// # Errors
///
/// Returns a typed [`TessellateError`]; never panics.
pub fn tessellate_sweep(
    profile: &Profile2,
    placement: &Placement3,
    path: &[[f64; 3]],
    caps: CapMode,
    policy: &EvalPolicy,
) -> Result<TessellatedBody, TessellateError> {
    debug_assert!(path.len() >= 2, "IR validation requires >= 2 path points");
    let flip = det3(placement) < 0.0;
    let d = discretize_profile(profile, &policy.discretize)?;
    let frames = sweep_frames(path, policy)?;

    // Ring creases at path corners: turn angle between adjacent segments.
    let corner_ring_sharp: Vec<bool> = {
        let mut flags = alloc::vec![false; frames.len()];
        for i in 1..path.len() - 1 {
            let a = sub(path[i], path[i - 1]);
            let b = sub(path[i + 1], path[i]);
            let cross = norm(cross(a, b));
            flags[i] = cross > policy.sharp_sin_threshold * norm(a) * norm(b);
        }
        flags
    };

    let ring_starts = ring_starts(&d);
    let total = len_u32(d.points_len());
    let mut builder = OrientedBuilder::new(flip);
    for (origin, u, v, _) in &frames {
        for ring in d.rings() {
            for p in &ring.points {
                let local = add(add(*origin, scale(*u, p[0])), scale(*v, p[1]));
                builder.push_vertex(narrow(apply_placement(placement, local)));
            }
        }
    }
    let ring_offset = |k: usize| len_u32(k) * total;

    let mut face_origins: Vec<Feature> = Vec::new();
    let seg_offsets = seg_offsets(profile);
    let corner_sharp: Vec<Vec<bool>> = core::iter::once(profile.outer())
        .chain(profile.holes().iter())
        .map(|source| loop_corner_sharpness(source, policy))
        .collect();

    let start_cap = matches!(caps, CapMode::Both | CapMode::Start);
    let end_cap = matches!(caps, CapMode::Both | CapMode::End);
    let bands = frames.len() - 1;

    for band in 0..bands {
        let below = ring_offset(band);
        let above = ring_offset(band + 1);
        let band_u16 = u16::try_from(band).unwrap_or(u16::MAX);
        for (ring_index, ring) in d.rings().enumerate() {
            let base = ring_starts[ring_index];
            let n = len_u32(ring.points.len());
            let loop_index = u16::try_from(ring_index).unwrap_or(u16::MAX);
            for i in 0..n {
                let j = (i + 1) % n;
                let seg = ring.edge_seg[i as usize];
                let sharp_at = |point: u32| {
                    ring.is_endpoint(point)
                        && corner_sharp[ring_index][ring.edge_seg[point as usize] as usize]
                };
                let bottom_crease = if band == 0 {
                    start_cap
                } else {
                    corner_ring_sharp[band]
                };
                let top_crease = if band + 1 == bands {
                    end_cap
                } else {
                    corner_ring_sharp[band + 1]
                };
                let sharp = [
                    if bottom_crease { 1.0 } else { 0.0 },
                    if sharp_at(j) { 1.0 } else { 0.0 },
                    if top_crease { 1.0 } else { 0.0 },
                    if sharp_at(i) { 1.0 } else { 0.0 },
                ];
                builder.add_face_with_attrs(
                    &[
                        below + base + i,
                        below + base + j,
                        above + base + j,
                        above + base + i,
                    ],
                    &FaceBuildAttrs {
                        region: Some(REGION_WALL_BASE + seg_offsets[ring_index] + seg),
                        edge_seams: None,
                        edge_sharpness: Some(&sharp),
                    },
                )?;
                face_origins.push(Feature::SweepWall {
                    band: band_u16,
                    loop_index,
                    seg,
                });
            }
        }
    }

    if start_cap || end_cap {
        let convex_simple = d.holes.is_empty() && is_convex_ring(&d.outer);
        let emit = |builder: &mut OrientedBuilder,
                    face_origins: &mut Vec<Feature>,
                    offset: u32,
                    reverse: bool,
                    feature: Feature,
                    region: u32|
         -> Result<(), TessellateError> {
            if convex_simple {
                let n = len_u32(d.outer.points.len());
                let ring: Vec<u32> = if reverse {
                    (0..n).rev().map(|i| offset + i).collect()
                } else {
                    (0..n).map(|i| offset + i).collect()
                };
                builder.add_face_with_attrs(
                    &ring,
                    &FaceBuildAttrs {
                        region: Some(region),
                        ..FaceBuildAttrs::default()
                    },
                )?;
                face_origins.push(feature);
            } else {
                let holes: Vec<&[[f64; 2]]> = d.holes.iter().map(|h| h.points.as_slice()).collect();
                let input = PolygonInput {
                    outer: &d.outer.points,
                    holes: &holes,
                };
                let tri = triangulate(&input, &TriParams::default())?;
                for t in &tri.triangles {
                    let corners = if reverse {
                        [offset + t[2], offset + t[1], offset + t[0]]
                    } else {
                        [offset + t[0], offset + t[1], offset + t[2]]
                    };
                    builder.add_face_with_attrs(
                        &corners,
                        &FaceBuildAttrs {
                            region: Some(region),
                            ..FaceBuildAttrs::default()
                        },
                    )?;
                    face_origins.push(feature);
                }
            }
            Ok(())
        };
        if start_cap {
            emit(
                &mut builder,
                &mut face_origins,
                ring_offset(0),
                true,
                Feature::CapStart,
                REGION_CAP_START,
            )?;
        }
        if end_cap {
            emit(
                &mut builder,
                &mut face_origins,
                ring_offset(frames.len() - 1),
                false,
                Feature::CapEnd,
                REGION_CAP_END,
            )?;
        }
    }

    let result = builder.build()?;
    let vertex_features = profile_vertex_features(&d, len_u32(frames.len()));
    let source_map = crate::source_map::SourceMap::new(&result.mesh, face_origins, vertex_features);
    Ok(TessellatedBody {
        mesh: result.mesh,
        source_map,
    })
}

/// Tessellates a grid-surface body: a row-major `rows x cols` point grid
/// emitted as bilinear quad patches, placed by `placement`.
///
/// With `thickness` the given points form the front surface, a copy
/// offset by `thickness` along averaged *inward* vertex normals (the
/// negative of the front winding's normal direction) forms the back, and
/// every open boundary grows a sharp-edged side wall — a closed solid.
/// Without it the sheet is emitted open. `close_u` wraps rows, `close_w`
/// wraps columns; a closed direction has no boundary there.
///
/// `FACE_REGION` mapping: [`REGION_GRID_FRONT`], [`REGION_GRID_BACK`],
/// then [`REGION_GRID_SIDE_BASE`]` + k` per side. Provenance is
/// [`Feature::GridPatch`] per patch (sides attribute to the boundary
/// patch they extend).
///
/// # Errors
///
/// Returns a typed [`TessellateError`]; never panics. Thickened grids
/// whose vertices have no usable averaged normal fail with
/// [`TessellateError::DegenerateGrid`].
pub fn tessellate_grid(
    points: &[[f64; 3]],
    rows: u32,
    cols: u32,
    close_u: bool,
    close_w: bool,
    thickness: Option<f64>,
    placement: &Placement3,
) -> Result<TessellatedBody, TessellateError> {
    let rows = rows as usize;
    let cols = cols as usize;
    debug_assert_eq!(points.len(), rows * cols, "validated at node insertion");
    let flip = det3(placement) < 0.0;
    let placed: Vec<[f64; 3]> = points
        .iter()
        .map(|p| apply_placement(placement, *p))
        .collect();

    // Patch counts: wrapping directions close the last band back to 0.
    let patch_rows = if close_u { rows } else { rows - 1 };
    let patch_cols = if close_w { cols } else { cols - 1 };
    let vertex_index = |r: usize, c: usize| (r % rows) * cols + (c % cols);
    let sub = |a: [f64; 3], b: [f64; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let cross = |a: [f64; 3], b: [f64; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };

    // Area-weighted patch normals (diagonal cross), oriented with the
    // front winding.
    let patch_normal = |r: usize, c: usize| {
        let a = placed[vertex_index(r, c)];
        let b = placed[vertex_index(r, c + 1)];
        let d = placed[vertex_index(r + 1, c + 1)];
        let e = placed[vertex_index(r + 1, c)];
        cross(sub(d, a), sub(e, b))
    };

    let mut builder = OrientedBuilder::new(flip);
    let patch_of = |r: usize, c: usize| Feature::GridPatch {
        row: u16::try_from(r.min(patch_rows - 1)).unwrap_or(u16::MAX),
        col: u16::try_from(c.min(patch_cols - 1)).unwrap_or(u16::MAX),
    };

    // Front vertices.
    for p in &placed {
        builder.push_vertex(narrow(*p));
    }
    let mut vertex_features: Vec<Feature> = Vec::with_capacity(placed.len() * 2);
    for r in 0..rows {
        for c in 0..cols {
            vertex_features.push(patch_of(r, c));
        }
    }

    // Back vertices: offset along averaged inward normals.
    let back_offset = len_u32(placed.len());
    if let Some(t) = thickness {
        for r in 0..rows {
            for c in 0..cols {
                // Adjacent patches: rows {r-1, r} x cols {c-1, c}, wrapping
                // when the direction closes, absent at open boundaries.
                let prev_row = if r > 0 {
                    Some(r - 1)
                } else if close_u {
                    Some(patch_rows - 1)
                } else {
                    None
                };
                let cur_row = (r < patch_rows).then_some(r);
                let prev_col = if c > 0 {
                    Some(c - 1)
                } else if close_w {
                    Some(patch_cols - 1)
                } else {
                    None
                };
                let cur_col = (c < patch_cols).then_some(c);
                let mut normal = [0.0_f64; 3];
                for pr in [prev_row, cur_row].into_iter().flatten() {
                    for pc in [prev_col, cur_col].into_iter().flatten() {
                        let n = patch_normal(pr, pc);
                        normal = [normal[0] + n[0], normal[1] + n[1], normal[2] + n[2]];
                    }
                }
                let len = libm::sqrt(
                    normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2],
                );
                if !(len.is_finite() && len > 0.0) {
                    return Err(TessellateError::DegenerateGrid {
                        row: len_u32(r),
                        col: len_u32(c),
                    });
                }
                let p = placed[vertex_index(r, c)];
                let scale = t / len;
                builder.push_vertex(narrow([
                    p[0] - normal[0] * scale,
                    p[1] - normal[1] * scale,
                    p[2] - normal[2] * scale,
                ]));
            }
        }
        for r in 0..rows {
            for c in 0..cols {
                vertex_features.push(patch_of(r, c));
            }
        }
    }

    let mut face_origins: Vec<Feature> = Vec::new();
    let front = |r: usize, c: usize| len_u32(vertex_index(r, c));
    let back = |r: usize, c: usize| back_offset + len_u32(vertex_index(r, c));

    // Front patches.
    for r in 0..patch_rows {
        for c in 0..patch_cols {
            builder.add_face_with_attrs(
                &[
                    front(r, c),
                    front(r, c + 1),
                    front(r + 1, c + 1),
                    front(r + 1, c),
                ],
                &FaceBuildAttrs {
                    region: Some(REGION_GRID_FRONT),
                    ..FaceBuildAttrs::default()
                },
            )?;
            face_origins.push(patch_of(r, c));
        }
    }

    if thickness.is_some() {
        // Back patches: reversed winding, outward on the offset side.
        for r in 0..patch_rows {
            for c in 0..patch_cols {
                builder.add_face_with_attrs(
                    &[
                        back(r, c),
                        back(r + 1, c),
                        back(r + 1, c + 1),
                        back(r, c + 1),
                    ],
                    &FaceBuildAttrs {
                        region: Some(REGION_GRID_BACK),
                        ..FaceBuildAttrs::default()
                    },
                )?;
                face_origins.push(patch_of(r, c));
            }
        }
        // Side walls on open boundaries, sharp-creased.
        let sharp = [1.0_f32; 4];
        let side = |builder: &mut OrientedBuilder,
                    face_origins: &mut Vec<Feature>,
                    corners: [u32; 4],
                    side_index: u32,
                    feature: Feature|
         -> Result<(), exedra::BuildError> {
            builder.add_face_with_attrs(
                &corners,
                &FaceBuildAttrs {
                    region: Some(REGION_GRID_SIDE_BASE + side_index),
                    edge_sharpness: Some(&sharp),
                    ..FaceBuildAttrs::default()
                },
            )?;
            face_origins.push(feature);
            Ok(())
        };
        if !close_u {
            let last = rows - 1;
            for c in 0..patch_cols {
                side(
                    &mut builder,
                    &mut face_origins,
                    [front(0, c + 1), front(0, c), back(0, c), back(0, c + 1)],
                    0,
                    patch_of(0, c),
                )?;
                side(
                    &mut builder,
                    &mut face_origins,
                    [
                        front(last, c),
                        front(last, c + 1),
                        back(last, c + 1),
                        back(last, c),
                    ],
                    1,
                    patch_of(patch_rows - 1, c),
                )?;
            }
        }
        if !close_w {
            let last = cols - 1;
            for r in 0..patch_rows {
                side(
                    &mut builder,
                    &mut face_origins,
                    [front(r, 0), front(r + 1, 0), back(r + 1, 0), back(r, 0)],
                    2,
                    patch_of(r, 0),
                )?;
                side(
                    &mut builder,
                    &mut face_origins,
                    [
                        front(r + 1, last),
                        front(r, last),
                        back(r, last),
                        back(r + 1, last),
                    ],
                    3,
                    patch_of(r, patch_cols - 1),
                )?;
            }
        }
    }

    let result = builder.build()?;
    let source_map = crate::source_map::SourceMap::new(&result.mesh, face_origins, vertex_features);
    Ok(TessellatedBody {
        mesh: result.mesh,
        source_map,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builders;

    /// Signed volume via the divergence theorem, fanning each face loop.
    /// Valid for planar convex faces (all faces this tessellator emits).
    fn mesh_volume(mesh: &exedra::Mesh) -> f64 {
        let mut vol = 0.0;
        for face in mesh.faces() {
            let verts: Vec<[f64; 3]> = mesh
                .face_loop(face)
                .filter_map(|he| mesh.to_vertex(he))
                .filter_map(|v| mesh.vertex_position(v))
                .map(|p| [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])])
                .collect();
            for i in 1..verts.len().saturating_sub(1) {
                let (a, b, c) = (verts[0], verts[i], verts[i + 1]);
                vol += a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                    + a[2] * (b[0] * c[1] - b[1] * c[0]);
            }
        }
        vol / 6.0
    }

    fn assert_clean(body: &TessellatedBody) {
        let errors = body.mesh.validate_deep();
        assert!(errors.is_empty(), "validate_deep: {errors:?}");
        assert_eq!(
            body.source_map.face_count(),
            body.mesh.faces().count(),
            "one origin per face"
        );
    }

    #[test]
    fn planar_face_preserves_curved_hole_boundaries_regions_and_provenance() {
        // A holed curved profile exercises the path that a hand-built ngon
        // cannot represent: both rings must follow DiscretizePolicy, survive
        // triangulation as mesh boundaries, and retain their source segments.
        let profile = builders::ring(2.0, 1.0).expect("annular profile");
        let mut coarse_policy = EvalPolicy::default();
        coarse_policy.discretize.chord_tolerance = 0.25;
        coarse_policy.discretize.min_arc_edges = 1;
        let mut fine_policy = coarse_policy;
        fine_policy.discretize.chord_tolerance = 0.01;

        let coarse = tessellate_planar_face(&profile, &Placement3::IDENTITY, &coarse_policy)
            .expect("coarse face tessellates");
        let fine = tessellate_planar_face(&profile, &Placement3::IDENTITY, &fine_policy)
            .expect("fine face tessellates");

        assert_clean(&coarse);
        assert_clean(&fine);
        assert_eq!(
            fine.mesh
                .boundary_loops()
                .expect("boundaries enumerate")
                .len(),
            2,
            "the outer boundary and profile hole both remain open rims"
        );
        assert!(
            fine.mesh.vertices().count() > coarse.mesh.vertices().count(),
            "a tighter curve tolerance must refine the arc-bounded face"
        );

        let regions = fine
            .mesh
            .attrs()
            .dense(exedra::attr::FACE_REGION)
            .expect("planar-face region layer");
        for face in fine.mesh.faces() {
            assert_eq!(regions.get(face.as_id()).copied(), Some(REGION_PLANAR_FACE));
            assert_eq!(
                fine.source_map.face_feature(face),
                Some(Feature::PlanarFace)
            );
        }
        let vertex_features: alloc::collections::BTreeSet<_> = fine
            .mesh
            .vertices()
            .filter_map(|vertex| fine.source_map.vertex_feature(vertex))
            .collect();
        assert!(
            vertex_features
                .iter()
                .any(|feature| matches!(feature, Feature::Wall { loop_index: 0, .. })),
            "outer boundary vertices retain their generating segments"
        );
        assert!(
            vertex_features
                .iter()
                .any(|feature| matches!(feature, Feature::Wall { loop_index: 1, .. })),
            "hole boundary vertices retain their generating segments"
        );
    }

    #[test]
    fn primitive_regions_and_winding_survive_cylinder_axis_conversion_and_mirror() {
        // Primitive evaluation rebuilds the backend mesh to rotate its native
        // +Y cylinder onto constructive +Z. This pins all information that
        // rebuild could accidentally damage: outward winding under a mirror,
        // topology, sharp-edge metadata, FACE_REGION, and source-map agreement
        // with those regions. Named primitive selections intentionally do not
        // enter TessellatedBody; constructive provenance is region-based.
        let spec = PrimitiveSpec::Cylinder {
            radius: 2.0,
            height: 3.0,
            segments: 12,
        };
        let ordinary = tessellate_primitive(spec, &Placement3::IDENTITY, &EvalPolicy::default())
            .expect("ordinary cylinder");
        let reflected = tessellate_primitive(
            spec,
            &Placement3 {
                rows: [
                    [-1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                ],
            },
            &EvalPolicy::default(),
        )
        .expect("reflected cylinder");

        assert_clean(&ordinary);
        assert_clean(&reflected);
        let ordinary_volume = mesh_volume(&ordinary.mesh);
        let reflected_volume = mesh_volume(&reflected.mesh);
        assert!(ordinary_volume > 0.0, "ordinary volume {ordinary_volume}");
        assert!(
            reflected_volume > 0.0,
            "reflected volume {reflected_volume}"
        );
        assert!(
            (ordinary_volume - reflected_volume).abs() < 1.0e-5,
            "mirror changed volume: {ordinary_volume} vs {reflected_volume}"
        );

        // The second bottom-ring vertex is off both principal axes, so its
        // exact bits pin the libm coordinate path even when this test target
        // also enables exedra_primitives/std through dev-dependencies.
        let sampled = ordinary
            .mesh
            .vertices()
            .nth(1)
            .and_then(|vertex| ordinary.mesh.vertex_position(vertex))
            .expect("second cylinder vertex");
        assert_eq!(sampled.map(f32::to_bits), [0x3fdd_b3d7, 0xbf80_0000, 0]);

        let regions = reflected
            .mesh
            .attrs()
            .dense(exedra::attr::FACE_REGION)
            .expect("primitive regions were copied onto the mesh");
        let mut seen = alloc::collections::BTreeSet::new();
        for face in reflected.mesh.faces() {
            let region = regions
                .get(face.as_id())
                .copied()
                .expect("every primitive face has a region");
            seen.insert(region);
            assert_eq!(
                reflected.source_map.face_feature(face),
                Some(Feature::PrimitiveRegion { region })
            );
        }
        assert_eq!(
            seen,
            [
                exedra_primitives::REGION_SIDE.0,
                exedra_primitives::REGION_CAP_TOP.0,
                exedra_primitives::REGION_CAP_BOTTOM.0,
            ]
            .into_iter()
            .collect()
        );
        assert!(
            reflected
                .mesh
                .faces()
                .flat_map(|face| reflected.mesh.face_loop(face))
                .any(|edge| reflected.mesh.edge_sharpness(edge).is_some_and(|v| v > 0.0)),
            "cap-rim sharpness must survive rebuilding"
        );
    }

    #[test]
    fn primitive_evaluation_rejects_unrepresentable_parameters_and_segment_bombs() {
        // Recipe validation accepts finite positive f64 values and explicit
        // u32 segment counts. Evaluation must fail typed before an f32
        // overflow or an attacker-sized cylinder reaches the backend.
        assert!(matches!(
            tessellate_primitive(
                PrimitiveSpec::Box {
                    size: [f64::MAX, 1.0, 1.0],
                },
                &Placement3::IDENTITY,
                &EvalPolicy::default(),
            ),
            Err(TessellateError::NonFiniteGeometry)
        ));

        let mut policy = EvalPolicy::default();
        policy.discretize.max_segment_edges = 8;
        assert!(matches!(
            tessellate_primitive(
                PrimitiveSpec::Cylinder {
                    radius: 1.0,
                    height: 1.0,
                    segments: 9,
                },
                &Placement3::IDENTITY,
                &policy,
            ),
            Err(TessellateError::PrimitiveSegmentLimit {
                requested: 9,
                maximum: 8,
            })
        ));
    }

    #[test]
    fn rect_extrude_is_a_box() {
        let profile = builders::rect(2.0, 1.0).expect("rect");
        let body = tessellate_extrude(
            &profile,
            &Placement3::IDENTITY,
            3.0,
            CapMode::Both,
            &EvalPolicy::default(),
        )
        .expect("tessellates");
        assert_clean(&body);
        assert_eq!(body.mesh.faces().count(), 6, "4 walls + 2 ngon caps");
        assert!((mesh_volume(&body.mesh) - 6.0).abs() < 1e-4);
    }

    #[test]
    fn l_profile_extrude_has_triangulated_caps() {
        let profile = builders::l_profile(1.0, 1.0, 0.5, 0.5).expect("L");
        let body = tessellate_extrude(
            &profile,
            &Placement3::IDENTITY,
            2.0,
            CapMode::Both,
            &EvalPolicy::default(),
        )
        .expect("tessellates");
        assert_clean(&body);
        assert!((mesh_volume(&body.mesh) - 1.5).abs() < 1e-4);
        let caps = body
            .source_map
            .face_features()
            .iter()
            .filter(|f| matches!(f, Feature::CapStart | Feature::CapEnd))
            .count();
        assert_eq!(caps, 8, "4 triangles per concave cap");
    }

    #[test]
    fn holed_profile_extrude() {
        let profile = builders::ring(2.0, 1.0).expect("ring");
        let body = tessellate_extrude(
            &profile,
            &Placement3::IDENTITY,
            1.0,
            CapMode::Both,
            &EvalPolicy::default(),
        )
        .expect("tessellates");
        assert_clean(&body);
        let expected = core::f64::consts::PI * 3.0;
        let vol = mesh_volume(&body.mesh);
        // Discretized circles under-approximate the true area slightly.
        assert!(
            (vol - expected).abs() < 0.05,
            "ring volume {vol} vs {expected}"
        );
        // Both wall families exist: outer loop and hole loop.
        assert!(
            body.source_map
                .face_features()
                .iter()
                .any(|f| matches!(f, Feature::Wall { loop_index: 0, .. }))
        );
        assert!(
            body.source_map
                .face_features()
                .iter()
                .any(|f| matches!(f, Feature::Wall { loop_index: 1, .. }))
        );
    }

    #[test]
    fn rounded_profile_walls_are_smooth_at_tangent_junctions() {
        let profile = builders::rounded_rect(4.0, 2.0, 0.5).expect("rounded rect");
        let body = tessellate_extrude(
            &profile,
            &Placement3::IDENTITY,
            1.0,
            CapMode::Both,
            &EvalPolicy::default(),
        )
        .expect("tessellates");
        assert_clean(&body);
        // Tangent-continuous junctions: no lateral edge may be sharp. Cap
        // rims are sharp. Count sharp edges: expect exactly the two rims.
        let mesh = &body.mesh;
        let mut sharp_lateral = 0;
        let mut sharp_rim = 0;
        for face in mesh.faces() {
            for he in mesh.face_loop(face) {
                if let Some(edge) = mesh.canonical_edge(he)
                    && mesh.edge_sharpness(edge).unwrap_or(0.0) > 0.5
                {
                    // Classify by geometry: rim edges are horizontal
                    // (endpoints share z), laterals vertical.
                    let a = mesh.to_vertex(he).and_then(|v| mesh.vertex_position(v));
                    let b = mesh
                        .to_vertex(mesh.twin(he).expect("twin"))
                        .and_then(|v| mesh.vertex_position(v));
                    if let (Some(a), Some(b)) = (a, b) {
                        if (a[2] - b[2]).abs() < 1e-6 {
                            sharp_rim += 1;
                        } else {
                            sharp_lateral += 1;
                        }
                    }
                }
            }
        }
        assert_eq!(sharp_lateral, 0, "tangent junctions must stay smooth");
        assert!(sharp_rim > 0, "cap rims must crease");
    }

    #[test]
    fn square_corners_crease_laterals() {
        let profile = builders::rect(1.0, 1.0).expect("rect");
        let body = tessellate_extrude(
            &profile,
            &Placement3::IDENTITY,
            1.0,
            CapMode::Both,
            &EvalPolicy::default(),
        )
        .expect("tessellates");
        let mesh = &body.mesh;
        let mut sharp_lateral = 0;
        for face in mesh.faces() {
            for he in mesh.face_loop(face) {
                if let Some(edge) = mesh.canonical_edge(he)
                    && mesh.edge_sharpness(edge).unwrap_or(0.0) > 0.5
                {
                    let a = mesh.to_vertex(he).and_then(|v| mesh.vertex_position(v));
                    let b = mesh
                        .to_vertex(mesh.twin(he).expect("twin"))
                        .and_then(|v| mesh.vertex_position(v));
                    if let (Some(a), Some(b)) = (a, b)
                        && (a[2] - b[2]).abs() > 1e-6
                    {
                        sharp_lateral += 1;
                    }
                }
            }
        }
        // 4 lateral edges, each visited from two adjacent faces.
        assert_eq!(sharp_lateral, 8, "square corners crease all laterals");
    }

    #[test]
    fn open_shell_has_boundaries() {
        let profile = builders::rect(1.0, 1.0).expect("rect");
        let body = tessellate_extrude(
            &profile,
            &Placement3::IDENTITY,
            1.0,
            CapMode::None,
            &EvalPolicy::default(),
        )
        .expect("tessellates");
        assert_clean(&body);
        assert_eq!(body.mesh.faces().count(), 4, "walls only");
    }

    #[test]
    fn placement_moves_the_body() {
        let profile = builders::rect(1.0, 1.0).expect("rect");
        let placed = Placement3::translate(10.0, 0.0, 5.0);
        let body = tessellate_extrude(
            &profile,
            &placed,
            1.0,
            CapMode::Both,
            &EvalPolicy::default(),
        )
        .expect("tessellates");
        let any = body
            .mesh
            .faces()
            .next()
            .and_then(|f| body.mesh.face_loop(f).next())
            .and_then(|he| body.mesh.to_vertex(he))
            .and_then(|v| body.mesh.vertex_position(v))
            .copied()
            .expect("has a vertex");
        assert!(any[0] >= 10.0 && any[2] >= 5.0);
    }

    #[test]
    fn tessellation_is_deterministic() {
        let profile = builders::rounded_rect(4.0, 2.0, 0.5).expect("rounded rect");
        let policy = EvalPolicy::default();
        let sig = |body: &TessellatedBody| {
            let (tri, _) = body.mesh.to_trimesh(&exedra::ExtractParams::default());
            exedra_testkit::golden::trimesh_signature(&tri)
        };
        let a = tessellate_extrude(&profile, &Placement3::IDENTITY, 1.0, CapMode::Both, &policy)
            .expect("first");
        let b = tessellate_extrude(&profile, &Placement3::IDENTITY, 1.0, CapMode::Both, &policy)
            .expect("second");
        assert_eq!(sig(&a), sig(&b), "double-run signature equality");
    }

    /// An off-axis square cross-section: `a x a` at radial center `r0`.
    fn annulus_square(r0: f64, a: f64) -> Profile2 {
        use crate::profile::{Loop2, Seg2};
        let (x0, x1) = (r0 - a / 2.0, r0 + a / 2.0);
        let outer = Loop2::new(alloc::vec![
            Seg2::line((x1, 0.0)),
            Seg2::line((x1, a)),
            Seg2::line((x0, a)),
            Seg2::line((x0, 0.0)),
        ])
        .expect("valid square section");
        Profile2::simple(outer).expect("valid profile")
    }

    #[test]
    fn full_revolve_square_torus_volume_matches_pappus() {
        // Fine tolerance so the discretized ring area is close to ideal.
        let policy = EvalPolicy {
            discretize: DiscretizePolicy {
                chord_tolerance: 1e-3,
                ..Default::default()
            },
            ..Default::default()
        };
        let profile = annulus_square(3.0, 1.0);
        let body = tessellate_revolve(
            &profile,
            &Placement3::IDENTITY,
            core::f64::consts::TAU,
            CapMode::Both,
            &policy,
        )
        .expect("tessellates");
        assert_clean(&body);
        // Pappus: V = 2 pi R A. The polygonal ring slightly under-sweeps;
        // fine steps keep it within a fraction of a percent.
        let expected = core::f64::consts::TAU * 3.0 * 1.0;
        let vol = mesh_volume(&body.mesh);
        assert!(
            (vol - expected).abs() / expected < 0.005,
            "torus volume {vol} vs {expected}"
        );
        // Full sweeps have no caps.
        assert!(
            body.source_map
                .face_features()
                .iter()
                .all(|f| matches!(f, Feature::Wall { .. })),
            "full sweep emits walls only"
        );
    }

    #[test]
    fn full_revolve_tags_a_seam() {
        let profile = annulus_square(2.0, 0.5);
        let body = tessellate_revolve(
            &profile,
            &Placement3::IDENTITY,
            core::f64::consts::TAU,
            CapMode::Both,
            &EvalPolicy::default(),
        )
        .expect("tessellates");
        let mesh = &body.mesh;
        let mut seam_edges = 0;
        for face in mesh.faces() {
            for he in mesh.face_loop(face) {
                if let Some(edge) = mesh.canonical_edge(he)
                    && mesh.edge_seam(edge) == Some(true)
                {
                    seam_edges += 1;
                }
            }
        }
        assert!(seam_edges > 0, "full sweep tags its closure meridian");
    }

    #[test]
    fn quarter_revolve_with_caps() {
        let policy = EvalPolicy {
            discretize: DiscretizePolicy {
                chord_tolerance: 1e-3,
                ..Default::default()
            },
            ..Default::default()
        };
        let profile = annulus_square(3.0, 1.0);
        let body = tessellate_revolve(
            &profile,
            &Placement3::IDENTITY,
            core::f64::consts::FRAC_PI_2,
            CapMode::Both,
            &policy,
        )
        .expect("tessellates");
        assert_clean(&body);
        let expected = core::f64::consts::TAU * 3.0 / 4.0;
        let vol = mesh_volume(&body.mesh);
        assert!(
            (vol - expected).abs() / expected < 0.005,
            "quarter torus volume {vol} vs {expected}"
        );
        let caps = body
            .source_map
            .face_features()
            .iter()
            .filter(|f| matches!(f, Feature::CapStart | Feature::CapEnd))
            .count();
        assert_eq!(caps, 2, "one convex ngon cap per boundary plane");
    }

    #[test]
    fn axis_contact_is_rejected() {
        let profile = builders::rect(1.0, 1.0).expect("rect touches x = 0");
        let result = tessellate_revolve(
            &profile,
            &Placement3::IDENTITY,
            core::f64::consts::TAU,
            CapMode::Both,
            &EvalPolicy::default(),
        );
        assert!(
            matches!(result, Err(TessellateError::AxisContact { .. })),
            "axis-touching profiles are rejected, got {result:?}"
        );
    }

    #[test]
    fn revolve_is_deterministic() {
        let profile = annulus_square(2.0, 0.5);
        let policy = EvalPolicy::default();
        let sig = |body: &TessellatedBody| {
            let (tri, _) = body.mesh.to_trimesh(&exedra::ExtractParams::default());
            exedra_testkit::golden::trimesh_signature(&tri)
        };
        let a = tessellate_revolve(
            &profile,
            &Placement3::IDENTITY,
            core::f64::consts::FRAC_PI_2,
            CapMode::Both,
            &policy,
        )
        .expect("first");
        let b = tessellate_revolve(
            &profile,
            &Placement3::IDENTITY,
            core::f64::consts::FRAC_PI_2,
            CapMode::Both,
            &policy,
        )
        .expect("second");
        assert_eq!(sig(&a), sig(&b), "double-run signature equality");
    }

    #[test]
    fn two_section_rect_loft_matches_extrude() {
        // A loft between two identical rects offset along z is a prism.
        let profile = builders::rect(2.0, 1.0).expect("rect");
        let sections = [
            (Placement3::IDENTITY, &profile),
            (Placement3::translate(0.0, 0.0, 3.0), &profile),
        ];
        let body =
            tessellate_loft(&sections, CapMode::Both, &EvalPolicy::default()).expect("tessellates");
        assert_clean(&body);
        assert!((mesh_volume(&body.mesh) - 6.0).abs() < 1e-4);
    }

    #[test]
    fn zero_span_loft_is_rejected() {
        let profile = builders::rect(2.0, 1.0).expect("rect");
        let sections = [
            (Placement3::IDENTITY, &profile),
            (Placement3::translate(1.0, 0.0, 0.0), &profile),
        ];
        assert!(matches!(
            tessellate_loft(&sections, CapMode::Both, &EvalPolicy::default()),
            Err(TessellateError::DegenerateLoft)
        ));
    }

    #[test]
    fn tapered_loft_volume_matches_frustum() {
        // Similar rectangles: 4x2 at z=0 to 2x1 at z=3, centered. Frustum
        // volume: h/3 (A1 + A2 + sqrt(A1 A2)) = 1 * (8 + 2 + 4) = 14.
        let big = builders::rect(4.0, 2.0).expect("rect");
        let small = builders::rect(2.0, 1.0).expect("rect");
        let sections = [
            (Placement3::IDENTITY, &big),
            (Placement3::translate(1.0, 0.5, 3.0), &small),
        ];
        let body =
            tessellate_loft(&sections, CapMode::Both, &EvalPolicy::default()).expect("tessellates");
        assert_clean(&body);
        assert!(
            (mesh_volume(&body.mesh) - 14.0).abs() < 1e-3,
            "frustum volume {}",
            mesh_volume(&body.mesh)
        );
    }

    #[test]
    fn three_section_loft_creases_intermediate_ring() {
        let profile = builders::rect(1.0, 1.0).expect("rect");
        let wide = builders::rect(1.0, 1.0).expect("rect");
        let sections = [
            (Placement3::IDENTITY, &profile),
            (Placement3::translate(0.4, 0.0, 1.0), &wide),
            (Placement3::translate(0.0, 0.0, 2.0), &profile),
        ];
        let body =
            tessellate_loft(&sections, CapMode::Both, &EvalPolicy::default()).expect("tessellates");
        assert_clean(&body);
        // Intermediate ring edges (z == 1) crease.
        let mesh = &body.mesh;
        let mut mid_creases = 0;
        for face in mesh.faces() {
            for he in mesh.face_loop(face) {
                if let Some(edge) = mesh.canonical_edge(he)
                    && mesh.edge_sharpness(edge).unwrap_or(0.0) > 0.5
                {
                    let a = mesh.to_vertex(he).and_then(|v| mesh.vertex_position(v));
                    let b = mesh
                        .to_vertex(mesh.twin(he).expect("twin"))
                        .and_then(|v| mesh.vertex_position(v));
                    if let (Some(a), Some(b)) = (a, b)
                        && (a[2] - 1.0).abs() < 1e-6
                        && (b[2] - 1.0).abs() < 1e-6
                    {
                        mid_creases += 1;
                    }
                }
            }
        }
        assert!(mid_creases > 0, "intermediate section ring must crease");
        // Bands are attributed.
        assert!(
            body.source_map
                .face_features()
                .iter()
                .any(|f| matches!(f, Feature::LoftWall { band: 1, .. }))
        );
    }

    #[test]
    fn mismatched_sections_are_rejected() {
        let rect = builders::rect(1.0, 1.0).expect("rect");
        let ring = builders::ring(1.0, 0.5).expect("ring");
        let sections = [
            (Placement3::IDENTITY, &rect),
            (Placement3::translate(0.0, 0.0, 1.0), &ring),
        ];
        let result = tessellate_loft(&sections, CapMode::Both, &EvalPolicy::default());
        assert_eq!(
            result
                .err()
                .map(|e| matches!(e, TessellateError::SectionMismatch { section: 1 })),
            Some(true)
        );
    }

    #[test]
    fn loft_is_deterministic() {
        let big = builders::rect(4.0, 2.0).expect("rect");
        let small = builders::rect(2.0, 1.0).expect("rect");
        let sections = [
            (Placement3::IDENTITY, &big),
            (Placement3::translate(1.0, 0.5, 3.0), &small),
        ];
        let sig = |body: &TessellatedBody| {
            let (tri, _) = body.mesh.to_trimesh(&exedra::ExtractParams::default());
            exedra_testkit::golden::trimesh_signature(&tri)
        };
        let a = tessellate_loft(&sections, CapMode::Both, &EvalPolicy::default()).expect("a");
        let b = tessellate_loft(&sections, CapMode::Both, &EvalPolicy::default()).expect("b");
        assert_eq!(sig(&a), sig(&b));
    }

    #[test]
    fn straight_sweep_matches_extrude_volume() {
        // A straight +Z path reproduces the extrusion exactly (the frame
        // seed keeps u x v = t right-handed).
        let profile = builders::rect(2.0, 1.0).expect("rect");
        let path = [[0.0, 0.0, 0.0], [0.0, 0.0, 3.0]];
        let body = tessellate_sweep(
            &profile,
            &Placement3::IDENTITY,
            &path,
            CapMode::Both,
            &EvalPolicy::default(),
        )
        .expect("tessellates");
        assert_clean(&body);
        assert!((mesh_volume(&body.mesh) - 6.0).abs() < 1e-4);
    }

    #[test]
    fn l_path_sweep_is_clean_and_creases_the_corner() {
        let profile = builders::rect(0.4, 0.4).expect("rect");
        let path = [[0.0, 0.0, 0.0], [0.0, 0.0, 2.0], [2.0, 0.0, 2.0]];
        let body = tessellate_sweep(
            &profile,
            &Placement3::IDENTITY,
            &path,
            CapMode::Both,
            &EvalPolicy::default(),
        )
        .expect("tessellates");
        assert_clean(&body);
        // Both bands attributed; corner ring creased.
        assert!(
            body.source_map
                .face_features()
                .iter()
                .any(|f| matches!(f, Feature::SweepWall { band: 0, .. }))
        );
        assert!(
            body.source_map
                .face_features()
                .iter()
                .any(|f| matches!(f, Feature::SweepWall { band: 1, .. }))
        );
        let mesh = &body.mesh;
        let creased = mesh
            .faces()
            .flat_map(|face| mesh.face_loop(face))
            .filter(|&he| {
                mesh.canonical_edge(he)
                    .map(|e| mesh.edge_sharpness(e).unwrap_or(0.0) > 0.5)
                    .unwrap_or(false)
            })
            .count();
        assert!(creased > 0, "corner ring and rims crease");
    }

    #[test]
    fn cusp_paths_are_rejected() {
        let profile = builders::rect(0.4, 0.4).expect("rect");
        let path = [[0.0, 0.0, 0.0], [0.0, 0.0, 2.0], [0.0, 0.0, 0.0]];
        let result = tessellate_sweep(
            &profile,
            &Placement3::IDENTITY,
            &path,
            CapMode::Both,
            &EvalPolicy::default(),
        );
        assert!(matches!(
            result,
            Err(TessellateError::PathCusp { point: 1 })
        ));
    }

    #[test]
    fn sweep_is_deterministic() {
        let profile = builders::rounded_rect(0.6, 0.4, 0.1).expect("rounded");
        let path = [
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 2.0],
            [1.5, 0.0, 3.5],
            [3.0, 1.0, 3.5],
        ];
        let sig = |body: &TessellatedBody| {
            let (tri, _) = body.mesh.to_trimesh(&exedra::ExtractParams::default());
            exedra_testkit::golden::trimesh_signature(&tri)
        };
        let a = tessellate_sweep(
            &profile,
            &Placement3::IDENTITY,
            &path,
            CapMode::Both,
            &EvalPolicy::default(),
        )
        .expect("a");
        let b = tessellate_sweep(
            &profile,
            &Placement3::IDENTITY,
            &path,
            CapMode::Both,
            &EvalPolicy::default(),
        )
        .expect("b");
        assert_eq!(sig(&a), sig(&b));
        assert_clean(&a);
    }

    fn flat_grid(rows: usize, cols: usize) -> Vec<[f64; 3]> {
        let mut points = Vec::new();
        for r in 0..rows {
            for c in 0..cols {
                points.push([c as f64, r as f64, 0.0]);
            }
        }
        points
    }

    #[test]
    fn grid_solid_volume_and_regions() {
        let body = tessellate_grid(
            &flat_grid(4, 5),
            4,
            5,
            false,
            false,
            Some(0.5),
            &Placement3::IDENTITY,
        )
        .expect("grid tessellates");
        let mesh = &body.mesh;
        let errors = mesh.validate_deep();
        assert!(errors.is_empty(), "{errors:?}");
        // Flat 4x3 patch sheet thickened by 0.5: volume 4 * 3 * 0.5.
        let vol = mesh_volume(mesh);
        assert!((vol - 6.0).abs() < 1e-6, "volume {vol}");
        // Faces: 12 front + 12 back + 2*4 row-side + 2*3 col-side.
        assert_eq!(mesh.faces().count(), 38);
        // Regions: front, back, and all four sides present.
        let regions = mesh
            .attrs()
            .dense(exedra::attr::FACE_REGION)
            .expect("region layer");
        let mut seen = alloc::collections::BTreeSet::new();
        for face in mesh.faces() {
            seen.insert(regions.get(face.as_id()).copied().unwrap_or(u32::MAX));
        }
        let expected: alloc::collections::BTreeSet<u32> = [
            REGION_GRID_FRONT,
            REGION_GRID_BACK,
            REGION_GRID_SIDE_BASE,
            REGION_GRID_SIDE_BASE + 1,
            REGION_GRID_SIDE_BASE + 2,
            REGION_GRID_SIDE_BASE + 3,
        ]
        .into_iter()
        .collect();
        assert_eq!(seen, expected);
        // Provenance: every face attributes to a patch inside the grid.
        for face in mesh.faces() {
            match body.source_map.face_feature(face) {
                Some(Feature::GridPatch { row, col }) => {
                    assert!(row < 3 && col < 4, "patch ({row}, {col}) out of range");
                }
                other => panic!("expected GridPatch, got {other:?}"),
            }
        }
    }

    #[test]
    fn grid_open_sheet_area() {
        let body = tessellate_grid(
            &flat_grid(4, 5),
            4,
            5,
            false,
            false,
            None,
            &Placement3::IDENTITY,
        )
        .expect("sheet tessellates");
        let mesh = &body.mesh;
        assert_eq!(mesh.faces().count(), 12);
        // Total area of the flat sheet is 4 * 3.
        let mut area = 0.0_f64;
        for face in mesh.faces() {
            let verts: Vec<[f64; 3]> = mesh
                .face_loop(face)
                .filter_map(|he| mesh.to_vertex(he))
                .filter_map(|v| mesh.vertex_position(v))
                .map(|p| [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])])
                .collect();
            for i in 1..verts.len() - 1 {
                let u = [
                    verts[i][0] - verts[0][0],
                    verts[i][1] - verts[0][1],
                    verts[i][2] - verts[0][2],
                ];
                let w = [
                    verts[i + 1][0] - verts[0][0],
                    verts[i + 1][1] - verts[0][1],
                    verts[i + 1][2] - verts[0][2],
                ];
                let cx = u[1] * w[2] - u[2] * w[1];
                let cy = u[2] * w[0] - u[0] * w[2];
                let cz = u[0] * w[1] - u[1] * w[0];
                area += 0.5 * libm::sqrt(cx * cx + cy * cy + cz * cz);
            }
        }
        assert!((area - 12.0).abs() < 1e-6, "area {area}");
        // Open sheet: boundary edges exist (twin-less half-edges).
        let boundary = mesh.boundary_loops().expect("boundary enumerates");
        assert_eq!(boundary.len(), 1, "one rectangular rim");
    }

    #[test]
    fn grid_closed_w_tube_is_watertight() {
        // A square tube: 4 perimeter columns wrapped, 2 rows along +Z.
        let square = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let mut points = Vec::new();
        for z in [0.0, 1.0] {
            for p in square {
                points.push([p[0], p[1], z]);
            }
        }
        let t = 0.1;
        let body = tessellate_grid(&points, 2, 4, false, true, Some(t), &Placement3::IDENTITY)
            .expect("tube tessellates");
        let mesh = &body.mesh;
        let errors = mesh.validate_deep();
        assert!(errors.is_empty(), "{errors:?}");
        // Corner offsets run along averaged (diagonal) normals, so the
        // inner square side shrinks by sqrt(2) * t.
        let inner = 1.0 - core::f64::consts::SQRT_2 * t;
        let expected = 1.0 - inner * inner;
        let vol = mesh_volume(mesh);
        assert!((vol - expected).abs() < 1e-6, "volume {vol} vs {expected}");
    }

    #[test]
    fn grid_degenerate_normal_fails_typed() {
        // All points collinear: adjacent points distinct, but every patch
        // is degenerate, so a thickness offset has no normal.
        let mut points = Vec::new();
        for r in 0..2 {
            for c in 0..3 {
                points.push([f64::from(c + r), 0.0, 0.0]);
            }
        }
        let result = tessellate_grid(
            &points,
            2,
            3,
            false,
            false,
            Some(0.5),
            &Placement3::IDENTITY,
        );
        assert!(
            matches!(result, Err(TessellateError::DegenerateGrid { .. })),
            "{result:?}"
        );
    }

    #[test]
    fn grid_double_run_is_bit_identical() {
        let run = || {
            tessellate_grid(
                &flat_grid(3, 3),
                3,
                3,
                false,
                false,
                Some(0.25),
                &Placement3::rotate_z_then_translate(0.4, 1.0, 2.0, 3.0),
            )
            .expect("grid tessellates")
        };
        let sig = |body: &TessellatedBody| {
            let (tri, _) = body.mesh.to_trimesh(&exedra::ExtractParams::default());
            exedra_testkit::golden::trimesh_signature(&tri)
        };
        let (a, b) = (run(), run());
        assert_eq!(sig(&a), sig(&b));
        assert_eq!(a.source_map.dump(), b.source_map.dump());
    }
}
