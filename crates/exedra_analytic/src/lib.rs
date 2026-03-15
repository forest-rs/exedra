// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Planar analytic topology spike for Exedra.
//!
//! This crate is the first "second head" in the workspace's multi-domain
//! geometry architecture. It intentionally starts narrow:
//! - planar faces,
//! - line-segment coedges,
//! - shell/loop/coedge topology,
//! - deterministic tessellation into [`exedra::Mesh`].
//!
//! It is not a general CAD kernel. Hole loops, curved edges, booleans, and
//! reverse conversion are all deferred.

#![no_std]
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;
#[cfg(not(any(feature = "std", feature = "libm")))]
compile_error!("exedra_analytic requires either the `std` or `libm` feature");

use alloc::vec::Vec;

use exedra::{FaceId, Mesh, MeshBuilder};

/// Region ID carried by analytic faces and written into tessellated meshes.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct RegionId(pub u32);

macro_rules! analytic_id {
    ($name:ident) => {
        /// Stable identifier within one analytic shell.
        #[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
        pub struct $name(u32);

        impl $name {
            /// Creates an ID from an index.
            #[must_use]
            pub const fn from_index(index: u32) -> Self {
                Self(index)
            }

            /// Returns the zero-based index.
            #[must_use]
            pub const fn index(self) -> u32 {
                self.0
            }
        }
    };
}

analytic_id!(AnalyticVertexId);
analytic_id!(AnalyticCoedgeId);
analytic_id!(AnalyticLoopId);
analytic_id!(AnalyticFaceId);

/// Oriented support plane for one planar face.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Plane {
    /// Unit-length face normal.
    pub normal: [f32; 3],
    /// Plane equation constant `dot(normal, p) = distance`.
    pub distance: f32,
}

/// Analytic vertex record.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct AnalyticVertex {
    /// Vertex position in object space.
    pub position: [f32; 3],
}

/// Directed analytic edge record.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct AnalyticCoedge {
    /// Start vertex.
    pub start: AnalyticVertexId,
    /// End vertex.
    pub end: AnalyticVertexId,
    /// Next coedge in the enclosing loop.
    pub next: AnalyticCoedgeId,
}

/// One closed analytic boundary loop.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct AnalyticLoop {
    /// First coedge in traversal order.
    pub first: AnalyticCoedgeId,
    /// Number of coedges in the loop.
    pub len: u32,
}

/// One planar analytic face.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PlanarFace {
    /// Support plane for this face.
    pub plane: Plane,
    /// Outer boundary loop.
    pub outer: AnalyticLoopId,
    /// Semantic region carried across tessellation.
    pub region: RegionId,
}

/// Immutable analytic shell.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnalyticShell {
    vertices: Vec<AnalyticVertex>,
    coedges: Vec<AnalyticCoedge>,
    loops: Vec<AnalyticLoop>,
    faces: Vec<PlanarFace>,
}

impl AnalyticShell {
    /// Creates an empty shell.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns all vertices.
    #[must_use]
    pub fn vertices(&self) -> &[AnalyticVertex] {
        &self.vertices
    }

    /// Returns all faces.
    #[must_use]
    pub fn faces(&self) -> &[PlanarFace] {
        &self.faces
    }

    /// Returns the vertex IDs for one face's outer loop in deterministic order.
    pub fn face_vertices(&self, face: AnalyticFaceId) -> Option<Vec<AnalyticVertexId>> {
        let face_record = self.faces.get(face.index() as usize)?;
        self.loop_vertices(face_record.outer).ok()
    }

    /// Tessellates the analytic shell into an Exedra mesh.
    pub fn to_exedra_mesh(
        &self,
        params: &TessellateParams,
    ) -> Result<TessellatedShell, TessellateError> {
        let mut builder = MeshBuilder::new();
        for vertex in &self.vertices {
            let _ = builder.push_vertex(vertex.position);
        }

        for face_index in 0..self.faces.len() {
            let face_id = AnalyticFaceId::from_index(usize_to_u32(face_index));
            let loop_vertices = self
                .face_vertices(face_id)
                .ok_or(TessellateError::MissingFace { face: face_id })?;
            let polygon = loop_vertices
                .iter()
                .map(|vertex| vertex.index())
                .collect::<Vec<_>>();
            builder
                .add_face(&polygon)
                .map_err(TessellateError::KernelBuild)?;
        }

        let build = builder.build().map_err(TessellateError::KernelBuild)?;
        let mut mesh = build.mesh;
        if params.write_face_regions {
            let mut edit = mesh.edit_with(exedra::ChangeSetBuilder::new());
            for (face_index, mesh_face) in build.face_ids.iter().enumerate() {
                let region = self.faces[face_index].region.0;
                exedra::op::set_face_region(&mut edit, *mesh_face, region)
                    .map_err(TessellateError::SetFaceRegion)?;
            }
            let _ = edit.finish();
        }

        let face_provenance = build
            .face_ids
            .iter()
            .enumerate()
            .map(|(face_index, face)| (AnalyticFaceId::from_index(usize_to_u32(face_index)), *face))
            .collect();

        Ok(TessellatedShell {
            mesh,
            face_provenance,
        })
    }

    fn loop_vertices(&self, loop_id: AnalyticLoopId) -> Result<Vec<AnalyticVertexId>, LoopError> {
        let loop_record = self
            .loops
            .get(loop_id.index() as usize)
            .ok_or(LoopError::MissingLoop(loop_id))?;
        let mut vertices = Vec::with_capacity(loop_record.len as usize);
        let mut cursor = loop_record.first;
        for _ in 0..loop_record.len {
            let coedge = self
                .coedges
                .get(cursor.index() as usize)
                .ok_or(LoopError::MissingCoedge(cursor))?;
            vertices.push(coedge.start);
            cursor = coedge.next;
        }
        if cursor != loop_record.first {
            return Err(LoopError::OpenLoop(loop_id));
        }
        Ok(vertices)
    }
}

/// Mutable builder for [`AnalyticShell`].
#[derive(Clone, Debug, Default)]
pub struct AnalyticShellBuilder {
    shell: AnalyticShell,
}

impl AnalyticShellBuilder {
    /// Creates an empty analytic shell builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one vertex and returns its stable shell-local ID.
    #[must_use]
    pub fn push_vertex(&mut self, position: [f32; 3]) -> AnalyticVertexId {
        let id = AnalyticVertexId::from_index(usize_to_u32(self.shell.vertices.len()));
        self.shell.vertices.push(AnalyticVertex { position });
        id
    }

    /// Adds one planar face with a single outer loop.
    pub fn add_planar_face(
        &mut self,
        loop_vertices: &[AnalyticVertexId],
        region: RegionId,
    ) -> Result<AnalyticFaceId, BuildError> {
        if loop_vertices.len() < 3 {
            return Err(BuildError::LoopTooSmall);
        }

        let positions = loop_vertices
            .iter()
            .map(|vertex| {
                self.shell
                    .vertices
                    .get(vertex.index() as usize)
                    .map(|record| record.position)
                    .ok_or(BuildError::MissingVertex(*vertex))
            })
            .collect::<Result<Vec<_>, _>>()?;

        if has_duplicates(loop_vertices) {
            return Err(BuildError::RepeatedVertex);
        }

        let plane = Plane::from_points(&positions).ok_or(BuildError::DegenerateLoop)?;
        for position in &positions {
            if plane.distance_to(*position).abs() > PLANAR_EPSILON {
                return Err(BuildError::NonPlanarLoop);
            }
        }

        let first_coedge = AnalyticCoedgeId::from_index(usize_to_u32(self.shell.coedges.len()));
        let loop_len = usize_to_u32(loop_vertices.len());
        for index in 0..loop_vertices.len() {
            let start = loop_vertices[index];
            let end = loop_vertices[(index + 1) % loop_vertices.len()];
            let next = if index + 1 == loop_vertices.len() {
                first_coedge
            } else {
                AnalyticCoedgeId::from_index(usize_to_u32(self.shell.coedges.len()) + 1)
            };
            self.shell.coedges.push(AnalyticCoedge { start, end, next });
        }

        let loop_id = AnalyticLoopId::from_index(usize_to_u32(self.shell.loops.len()));
        self.shell.loops.push(AnalyticLoop {
            first: first_coedge,
            len: loop_len,
        });

        let face_id = AnalyticFaceId::from_index(usize_to_u32(self.shell.faces.len()));
        self.shell.faces.push(PlanarFace {
            plane,
            outer: loop_id,
            region,
        });
        Ok(face_id)
    }

    /// Finalizes and returns the built shell.
    #[must_use]
    pub fn build(self) -> AnalyticShell {
        self.shell
    }
}

/// Parameters for one deterministic analytic-to-mesh conversion.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct TessellateParams {
    /// When true, analytic face regions are written into `exedra::attr::FACE_REGION`.
    pub write_face_regions: bool,
}

impl Default for TessellateParams {
    fn default() -> Self {
        Self {
            write_face_regions: true,
        }
    }
}

/// Tessellated polygon output plus deterministic provenance.
#[derive(Clone, Debug)]
pub struct TessellatedShell {
    /// Tessellated Exedra mesh.
    pub mesh: Mesh,
    /// One analytic face maps to one polygon face in this MVP.
    pub face_provenance: Vec<(AnalyticFaceId, FaceId)>,
}

/// Parameters for the `rect_frame_xy` helper.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct RectFrameParams {
    /// Outer rectangle minimum corner.
    pub outer_min: [f32; 2],
    /// Outer rectangle maximum corner.
    pub outer_max: [f32; 2],
    /// Inner opening minimum corner.
    pub inner_min: [f32; 2],
    /// Inner opening maximum corner.
    pub inner_max: [f32; 2],
    /// Z plane for the frame.
    pub z: f32,
    /// Region assigned to each frame face.
    pub region: RegionId,
}

impl Default for RectFrameParams {
    fn default() -> Self {
        Self {
            outer_min: [0.0, 0.0],
            outer_max: [4.0, 3.0],
            inner_min: [1.25, 1.0],
            inner_max: [2.75, 2.0],
            z: 0.0,
            region: RegionId(1),
        }
    }
}

/// Builds a rectangular planar frame as four analytic faces around one opening.
pub fn rect_frame_xy(params: &RectFrameParams) -> Result<AnalyticShell, BuildError> {
    if params.outer_min[0] >= params.outer_max[0] || params.outer_min[1] >= params.outer_max[1] {
        return Err(BuildError::InvalidRectFrame);
    }
    if params.inner_min[0] <= params.outer_min[0]
        || params.inner_min[1] <= params.outer_min[1]
        || params.inner_max[0] >= params.outer_max[0]
        || params.inner_max[1] >= params.outer_max[1]
        || params.inner_min[0] >= params.inner_max[0]
        || params.inner_min[1] >= params.inner_max[1]
    {
        return Err(BuildError::InvalidRectFrame);
    }

    let mut builder = AnalyticShellBuilder::new();
    let [ox0, oy0] = params.outer_min;
    let [ox1, oy1] = params.outer_max;
    let [ix0, inner_y0] = params.inner_min;
    let [ix1, inner_y1] = params.inner_max;
    let z = params.z;

    let outer_bottom_left = builder.push_vertex([ox0, oy0, z]);
    let outer_bottom_right = builder.push_vertex([ox1, oy0, z]);
    let outer_top_right = builder.push_vertex([ox1, oy1, z]);
    let outer_top_left = builder.push_vertex([ox0, oy1, z]);
    let inner_bottom_left = builder.push_vertex([ix0, inner_y0, z]);
    let inner_bottom_right = builder.push_vertex([ix1, inner_y0, z]);
    let inner_top_right = builder.push_vertex([ix1, inner_y1, z]);
    let inner_top_left = builder.push_vertex([ix0, inner_y1, z]);

    builder.add_planar_face(
        &[
            outer_bottom_left,
            inner_bottom_left,
            inner_bottom_right,
            outer_bottom_right,
        ],
        params.region,
    )?;
    builder.add_planar_face(
        &[
            inner_bottom_right,
            inner_top_right,
            outer_top_right,
            outer_bottom_right,
        ],
        params.region,
    )?;
    builder.add_planar_face(
        &[
            inner_top_left,
            outer_top_left,
            outer_top_right,
            inner_top_right,
        ],
        params.region,
    )?;
    builder.add_planar_face(
        &[
            outer_bottom_left,
            outer_top_left,
            inner_top_left,
            inner_bottom_left,
        ],
        params.region,
    )?;

    Ok(builder.build())
}

/// Builder-time validation error.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum BuildError {
    /// Face loop must contain at least three vertices.
    LoopTooSmall,
    /// Face loop contains duplicate vertices.
    RepeatedVertex,
    /// Loop points are collinear or otherwise degenerate.
    DegenerateLoop,
    /// Loop vertices are not coplanar within tolerance.
    NonPlanarLoop,
    /// Referenced vertex does not exist.
    MissingVertex(AnalyticVertexId),
    /// Rectangle frame parameters are invalid.
    InvalidRectFrame,
}

/// Tessellation-time error.
#[derive(Clone, Debug, PartialEq)]
pub enum TessellateError {
    /// The analytic shell references a missing face.
    MissingFace {
        /// Missing analytic face ID.
        face: AnalyticFaceId,
    },
    /// An outer loop record is missing.
    MissingLoop {
        /// Missing loop ID.
        loop_id: AnalyticLoopId,
    },
    /// A coedge record is missing.
    MissingCoedge {
        /// Missing coedge ID.
        coedge_id: AnalyticCoedgeId,
    },
    /// A loop does not close back to its starting coedge.
    OpenLoop {
        /// Open loop ID.
        loop_id: AnalyticLoopId,
    },
    /// The downstream Exedra mesh builder rejected the polygon set.
    KernelBuild(exedra::BuildError),
    /// `FACE_REGION` write failed unexpectedly.
    SetFaceRegion(exedra::op::SetFaceRegionError),
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum LoopError {
    MissingLoop(AnalyticLoopId),
    MissingCoedge(AnalyticCoedgeId),
    OpenLoop(AnalyticLoopId),
}

impl From<LoopError> for TessellateError {
    fn from(error: LoopError) -> Self {
        match error {
            LoopError::MissingLoop(loop_id) => Self::MissingLoop { loop_id },
            LoopError::MissingCoedge(coedge_id) => Self::MissingCoedge { coedge_id },
            LoopError::OpenLoop(loop_id) => Self::OpenLoop { loop_id },
        }
    }
}

const PLANAR_EPSILON: f32 = 1.0e-5;

impl Plane {
    fn from_points(points: &[[f32; 3]]) -> Option<Self> {
        let origin = *points.first()?;
        let normal = (1..points.len().saturating_sub(1)).find_map(|index| {
            let a = sub(points[index], origin);
            let b = sub(points[index + 1], origin);
            normalize(cross(a, b))
        })?;
        Some(Self {
            normal,
            distance: dot(normal, origin),
        })
    }

    fn distance_to(self, point: [f32; 3]) -> f32 {
        dot(self.normal, point) - self.distance
    }
}

fn has_duplicates(ids: &[AnalyticVertexId]) -> bool {
    let mut sorted = ids.to_vec();
    sorted.sort_unstable();
    sorted.windows(2).any(|pair| pair[0] == pair[1])
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn normalize(vector: [f32; 3]) -> Option<[f32; 3]> {
    let length_sq = dot(vector, vector);
    if length_sq <= PLANAR_EPSILON * PLANAR_EPSILON {
        return None;
    }
    let inv_length = 1.0 / sqrt(length_sq);
    Some([
        vector[0] * inv_length,
        vector[1] * inv_length,
        vector[2] * inv_length,
    ])
}

fn usize_to_u32(value: usize) -> u32 {
    u32::try_from(value).expect("analytic index overflowed u32")
}

fn sqrt(value: f32) -> f32 {
    #[cfg(feature = "std")]
    {
        value.sqrt()
    }
    #[cfg(all(not(feature = "std"), feature = "libm"))]
    {
        libm::sqrtf(value)
    }
}

#[cfg(test)]
mod tests {
    use exedra::ExtractParams;

    use super::{
        AnalyticShellBuilder, BuildError, RectFrameParams, RegionId, TessellateParams,
        rect_frame_xy,
    };

    #[test]
    fn planar_face_tessellates_into_valid_mesh() {
        let mut builder = AnalyticShellBuilder::new();
        let v0 = builder.push_vertex([0.0, 0.0, 0.0]);
        let v1 = builder.push_vertex([2.0, 0.0, 0.0]);
        let v2 = builder.push_vertex([2.0, 1.0, 0.0]);
        let v3 = builder.push_vertex([0.0, 1.0, 0.0]);
        let face = builder
            .add_planar_face(&[v0, v1, v2, v3], RegionId(7))
            .expect("quad should be planar");
        let shell = builder.build();

        let vertices = shell.face_vertices(face).expect("face loop should exist");
        assert_eq!(vertices.len(), 4);

        let tessellated = shell
            .to_exedra_mesh(&TessellateParams::default())
            .expect("tessellation should succeed");
        assert!(tessellated.mesh.validate_fast().is_empty());
        assert!(tessellated.mesh.validate_deep().is_empty());
        assert_eq!(tessellated.mesh.faces().count(), 1);
        let mesh_face = tessellated.face_provenance[0].1;
        let face_region = tessellated
            .mesh
            .attrs()
            .dense(exedra::attr::FACE_REGION)
            .expect("region layer exists")
            .get(mesh_face.as_id())
            .copied();
        assert_eq!(face_region, Some(7));
    }

    #[test]
    fn rect_frame_builds_four_face_shell_and_tessellates_deterministically() {
        let shell_a = rect_frame_xy(&RectFrameParams::default()).expect("frame builds");
        let shell_b = rect_frame_xy(&RectFrameParams::default()).expect("frame builds");

        let mesh_a = shell_a
            .to_exedra_mesh(&TessellateParams::default())
            .expect("tessellation should succeed");
        let mesh_b = shell_b
            .to_exedra_mesh(&TessellateParams::default())
            .expect("tessellation should succeed");

        assert_eq!(mesh_a.face_provenance.len(), 4);
        assert_eq!(mesh_b.face_provenance.len(), 4);
        assert!(mesh_a.mesh.validate_fast().is_empty());
        assert!(mesh_a.mesh.validate_deep().is_empty());

        let (tri_a, stats_a) = mesh_a.mesh.to_trimesh(&ExtractParams::default());
        let (tri_b, stats_b) = mesh_b.mesh.to_trimesh(&ExtractParams::default());
        assert_eq!(stats_a, stats_b);
        assert_eq!(tri_a.indices, tri_b.indices);
        assert_eq!(tri_a.positions, tri_b.positions);
    }

    #[test]
    fn builder_rejects_non_planar_loop() {
        let mut builder = AnalyticShellBuilder::new();
        let v0 = builder.push_vertex([0.0, 0.0, 0.0]);
        let v1 = builder.push_vertex([1.0, 0.0, 0.0]);
        let v2 = builder.push_vertex([1.0, 1.0, 0.25]);
        let v3 = builder.push_vertex([0.0, 1.0, 0.0]);

        let error = builder
            .add_planar_face(&[v0, v1, v2, v3], RegionId(1))
            .expect_err("non-planar loop should be rejected");
        assert_eq!(error, BuildError::NonPlanarLoop);
    }

    #[test]
    fn rect_frame_rejects_invalid_opening_bounds() {
        let error = rect_frame_xy(&RectFrameParams {
            inner_min: [0.0, 0.0],
            ..RectFrameParams::default()
        })
        .expect_err("opening must stay inside outer frame");
        assert_eq!(error, BuildError::InvalidRectFrame);
    }
}
