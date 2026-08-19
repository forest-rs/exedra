// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Planar analytic topology spike for Exedra.
//!
//! This crate is the first "second head" in the workspace's multi-domain
//! geometry architecture. It intentionally starts narrow:
//! - planar faces,
//! - line-segment coedges,
//! - shell/loop/coedge topology,
//! - explicit planar opening loops,
//! - face-level mutation for regions and XY rectangular openings,
//! - deterministic tessellation into [`exedra::Mesh`].
//!
//! It is not a general CAD kernel. Curved edges, booleans, and reverse
//! conversion are all deferred.

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

/// Parameters for adding one rectangular opening on an XY-aligned planar face.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct RectOpeningParams {
    /// Rectangle minimum corner in the face's XY plane.
    pub min: [f32; 2],
    /// Rectangle maximum corner in the face's XY plane.
    pub max: [f32; 2],
}

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
#[derive(Clone, Debug, PartialEq)]
pub struct PlanarFace {
    /// Support plane for this face.
    pub plane: Plane,
    /// Outer boundary loop.
    pub outer: AnalyticLoopId,
    /// Opening loops contained within the outer boundary.
    pub openings: Vec<AnalyticLoopId>,
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

    /// Returns one face by stable ID.
    #[must_use]
    pub fn face(&self, face: AnalyticFaceId) -> Option<&PlanarFace> {
        self.faces.get(face.index() as usize)
    }

    /// Returns the current region carried by one face.
    #[must_use]
    pub fn face_region(&self, face: AnalyticFaceId) -> Option<RegionId> {
        self.face(face).map(|record| record.region)
    }

    /// Returns the vertex IDs for one face's outer loop in deterministic order.
    pub fn face_vertices(&self, face: AnalyticFaceId) -> Option<Vec<AnalyticVertexId>> {
        let face_record = self.faces.get(face.index() as usize)?;
        self.loop_vertices(face_record.outer).ok()
    }

    /// Returns the opening-loop vertex IDs for one face in deterministic order.
    pub fn face_opening_vertices(
        &self,
        face: AnalyticFaceId,
    ) -> Option<Vec<Vec<AnalyticVertexId>>> {
        let face_record = self.faces.get(face.index() as usize)?;
        face_record
            .openings
            .iter()
            .map(|opening| self.loop_vertices(*opening).ok())
            .collect()
    }

    /// Sets the semantic region for one existing face.
    pub fn set_face_region(
        &mut self,
        face: AnalyticFaceId,
        region: RegionId,
    ) -> Result<(), EditError> {
        let record = self
            .faces
            .get_mut(face.index() as usize)
            .ok_or(EditError::MissingFace { face })?;
        record.region = region;
        Ok(())
    }

    /// Adds one rectangular opening to an existing XY-aligned planar face.
    pub fn add_rect_opening_xy(
        &mut self,
        face: AnalyticFaceId,
        params: &RectOpeningParams,
    ) -> Result<AnalyticLoopId, EditError> {
        if params.min[0] >= params.max[0] || params.min[1] >= params.max[1] {
            return Err(EditError::InvalidRectBounds);
        }

        let face_record = self.face(face).ok_or(EditError::MissingFace { face })?;
        let z = xy_plane_z(face, face_record.plane)?;
        let outer = self
            .face_vertices(face)
            .ok_or(EditError::InvalidFaceTopology { face })?;
        let openings = self
            .face_opening_vertices(face)
            .ok_or(EditError::InvalidFaceTopology { face })?;

        let rect_loop = rect_loop_vertices(params, z);
        validate_rect_opening_xy(self, face, &outer, &openings, &rect_loop)?;

        let new_vertices = rect_loop
            .iter()
            .map(|position| {
                let id = AnalyticVertexId::from_index(usize_to_u32(self.vertices.len()));
                self.vertices.push(AnalyticVertex {
                    position: [position[0], position[1], z],
                });
                id
            })
            .collect::<Vec<_>>();
        let opening = push_loop(&mut self.coedges, &mut self.loops, &new_vertices);
        let record = self
            .faces
            .get_mut(face.index() as usize)
            .ok_or(EditError::MissingFace { face })?;
        record.openings.push(opening);
        Ok(opening)
    }

    /// Removes one existing opening from a planar face.
    pub fn remove_opening(
        &mut self,
        face: AnalyticFaceId,
        opening: AnalyticLoopId,
    ) -> Result<(), EditError> {
        let record = self
            .faces
            .get_mut(face.index() as usize)
            .ok_or(EditError::MissingFace { face })?;
        let opening_index = record
            .openings
            .iter()
            .position(|loop_id| *loop_id == opening)
            .ok_or(EditError::OpeningNotOnFace { face, opening })?;
        record.openings.remove(opening_index);
        Ok(())
    }

    /// Tessellates the analytic shell into an Exedra mesh.
    pub fn to_exedra_mesh(
        &self,
        params: &TessellateParams,
    ) -> Result<TessellatedShell, TessellateError> {
        let mut builder = MeshBuilder::new();
        let mut source_faces = Vec::new();
        for vertex in &self.vertices {
            let _ = builder.push_vertex(vertex.position);
        }

        for face_index in 0..self.faces.len() {
            let face_id = AnalyticFaceId::from_index(usize_to_u32(face_index));
            let face_record = self
                .faces
                .get(face_index)
                .ok_or(TessellateError::MissingFace { face: face_id })?;
            if face_record.openings.is_empty() {
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
                source_faces.push(face_id);
            } else {
                let triangles = self.triangulate_face_with_openings(face_id)?;
                for triangle in triangles {
                    builder
                        .add_face(&[
                            triangle[0].index(),
                            triangle[1].index(),
                            triangle[2].index(),
                        ])
                        .map_err(TessellateError::KernelBuild)?;
                    source_faces.push(face_id);
                }
            }
        }

        let build = builder.build().map_err(TessellateError::KernelBuild)?;
        let mut mesh = build.mesh;
        if params.write_face_regions {
            let mut edit = mesh.edit_with(exedra::ChangeSetBuilder::new());
            for (face_index, mesh_face) in build.face_ids.iter().enumerate() {
                let analytic_face = source_faces[face_index];
                let region = self.faces[analytic_face.index() as usize].region.0;
                exedra::op::set_face_region(&mut edit, *mesh_face, region)
                    .map_err(TessellateError::SetFaceRegion)?;
            }
            let _ = edit.finish();
        }

        let face_provenance = build
            .face_ids
            .iter()
            .enumerate()
            .map(|(face_index, face)| (source_faces[face_index], *face))
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

    fn triangulate_face_with_openings(
        &self,
        face: AnalyticFaceId,
    ) -> Result<Vec<[AnalyticVertexId; 3]>, TessellateError> {
        let face_record = self
            .faces
            .get(face.index() as usize)
            .ok_or(TessellateError::MissingFace { face })?;
        let outer = self.loop_vertices(face_record.outer)?;
        let openings = face_record
            .openings
            .iter()
            .map(|opening| self.loop_vertices(*opening))
            .collect::<Result<Vec<_>, _>>()?;
        triangulate_face_with_openings(self, face, face_record.plane, &outer, &openings)
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
        self.add_planar_face_with_openings(loop_vertices, &[], region)
    }

    /// Adds one planar face with an outer loop and explicit opening loops.
    pub fn add_planar_face_with_openings(
        &mut self,
        outer_vertices: &[AnalyticVertexId],
        opening_loops: &[&[AnalyticVertexId]],
        region: RegionId,
    ) -> Result<AnalyticFaceId, BuildError> {
        let outer_positions = self.validate_loop_vertices(outer_vertices)?;
        let plane = Plane::from_points(&outer_positions).ok_or(BuildError::DegenerateLoop)?;
        validate_planar_loop(&outer_positions, plane)?;

        let outer = push_loop(
            &mut self.shell.coedges,
            &mut self.shell.loops,
            outer_vertices,
        );
        let mut openings = Vec::with_capacity(opening_loops.len());
        for opening in opening_loops {
            let opening_positions = self.validate_loop_vertices(opening)?;
            validate_planar_loop(&opening_positions, plane)?;
            openings.push(push_loop(
                &mut self.shell.coedges,
                &mut self.shell.loops,
                opening,
            ));
        }

        let face_id = AnalyticFaceId::from_index(usize_to_u32(self.shell.faces.len()));
        self.shell.faces.push(PlanarFace {
            plane,
            outer,
            openings,
            region,
        });
        Ok(face_id)
    }

    /// Finalizes and returns the built shell.
    #[must_use]
    pub fn build(self) -> AnalyticShell {
        self.shell
    }

    fn validate_loop_vertices(
        &self,
        loop_vertices: &[AnalyticVertexId],
    ) -> Result<Vec<[f32; 3]>, BuildError> {
        if loop_vertices.len() < 3 {
            return Err(BuildError::LoopTooSmall);
        }
        if has_duplicates(loop_vertices) {
            return Err(BuildError::RepeatedVertex);
        }
        loop_vertices
            .iter()
            .map(|vertex| {
                self.shell
                    .vertices
                    .get(vertex.index() as usize)
                    .map(|record| record.position)
                    .ok_or(BuildError::MissingVertex(*vertex))
            })
            .collect()
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
    /// Deterministic analytic-face to mesh-face provenance.
    ///
    /// Faces without openings currently map to one polygon face. Faces with
    /// openings map to multiple tessellated mesh faces.
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

/// Builds a rectangular planar face with one rectangular opening.
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

    let _ = builder.add_planar_face_with_openings(
        &[
            outer_bottom_left,
            outer_bottom_right,
            outer_top_right,
            outer_top_left,
        ],
        &[&[
            inner_bottom_left,
            inner_top_left,
            inner_top_right,
            inner_bottom_right,
        ]],
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

/// Analytic face-mutation error.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum EditError {
    /// Face does not exist.
    MissingFace {
        /// Missing face ID.
        face: AnalyticFaceId,
    },
    /// Face loop or opening topology could not be read.
    InvalidFaceTopology {
        /// Face with invalid topology.
        face: AnalyticFaceId,
    },
    /// Rectangle bounds are degenerate or inverted.
    InvalidRectBounds,
    /// Face is not aligned with the XY plane.
    FaceNotXyAligned {
        /// Face that rejected the XY-only operation.
        face: AnalyticFaceId,
    },
    /// Opening lies outside the face interior or intersects its boundary.
    OpeningOutsideFace {
        /// Face that rejected the opening.
        face: AnalyticFaceId,
    },
    /// Opening overlaps or intersects an existing opening on the face.
    OpeningOverlapsExisting {
        /// Face that rejected the opening.
        face: AnalyticFaceId,
    },
    /// Opening is not currently owned by the requested face.
    OpeningNotOnFace {
        /// Face that does not own `opening`.
        face: AnalyticFaceId,
        /// Opening loop requested for removal.
        opening: AnalyticLoopId,
    },
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
    /// An opening loop could not be bridged into a simple tessellation ring.
    OpeningBridgeFailed {
        /// Face being tessellated.
        face: AnalyticFaceId,
        /// Opening loop that could not be bridged.
        opening: AnalyticLoopId,
    },
    /// Ear clipping failed to triangulate one planar face.
    EarClipFailed {
        /// Face being tessellated.
        face: AnalyticFaceId,
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

/// Planarity/coincidence tolerance, sourced from the shared kernel policy
/// rather than a module-local constant (exedra `NumericPolicy`).
const PLANAR_EPSILON: f32 = exedra::NumericPolicy::DEFAULT_COPLANAR_TOLERANCE;

#[derive(Copy, Clone, Debug, PartialEq)]
struct ProjectedVertex {
    id: AnalyticVertexId,
    point: [f32; 2],
}

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

fn validate_planar_loop(positions: &[[f32; 3]], plane: Plane) -> Result<(), BuildError> {
    for position in positions {
        if plane.distance_to(*position).abs() > PLANAR_EPSILON {
            return Err(BuildError::NonPlanarLoop);
        }
    }
    Ok(())
}

fn push_loop(
    coedges: &mut Vec<AnalyticCoedge>,
    loops: &mut Vec<AnalyticLoop>,
    loop_vertices: &[AnalyticVertexId],
) -> AnalyticLoopId {
    let first_coedge = AnalyticCoedgeId::from_index(usize_to_u32(coedges.len()));
    let loop_len = usize_to_u32(loop_vertices.len());
    for index in 0..loop_vertices.len() {
        let start = loop_vertices[index];
        let end = loop_vertices[(index + 1) % loop_vertices.len()];
        let next = if index + 1 == loop_vertices.len() {
            first_coedge
        } else {
            AnalyticCoedgeId::from_index(usize_to_u32(coedges.len()) + 1)
        };
        coedges.push(AnalyticCoedge { start, end, next });
    }

    let loop_id = AnalyticLoopId::from_index(usize_to_u32(loops.len()));
    loops.push(AnalyticLoop {
        first: first_coedge,
        len: loop_len,
    });
    loop_id
}

fn xy_plane_z(face: AnalyticFaceId, plane: Plane) -> Result<f32, EditError> {
    if plane.normal[0].abs() > PLANAR_EPSILON
        || plane.normal[1].abs() > PLANAR_EPSILON
        || (plane.normal[2].abs() - 1.0).abs() > PLANAR_EPSILON
    {
        return Err(EditError::FaceNotXyAligned { face });
    }
    Ok(plane.distance / plane.normal[2])
}

fn rect_loop_vertices(params: &RectOpeningParams, z: f32) -> [[f32; 3]; 4] {
    [
        [params.min[0], params.min[1], z],
        [params.min[0], params.max[1], z],
        [params.max[0], params.max[1], z],
        [params.max[0], params.min[1], z],
    ]
}

fn validate_rect_opening_xy(
    shell: &AnalyticShell,
    face: AnalyticFaceId,
    outer: &[AnalyticVertexId],
    openings: &[Vec<AnalyticVertexId>],
    rect_loop: &[[f32; 3]; 4],
) -> Result<(), EditError> {
    let outer_projected =
        project_xy_vertex_loop(shell, outer).ok_or(EditError::InvalidFaceTopology { face })?;
    let opening_projected = openings
        .iter()
        .map(|opening| {
            project_xy_vertex_loop(shell, opening).ok_or(EditError::InvalidFaceTopology { face })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let rect_projected = project_xy_point_loop(rect_loop);

    if rect_projected.iter().any(|corner| {
        !point_in_polygon(corner.point, &outer_projected)
            || point_on_polygon_edge(corner.point, &outer_projected)
    }) {
        return Err(EditError::OpeningOutsideFace { face });
    }
    if polygon_edges(&rect_projected).iter().any(|edge| {
        polygon_edges(&outer_projected).iter().any(|outer_edge| {
            !shares_endpoint(edge.0, edge.1, outer_edge.0, outer_edge.1)
                && segments_intersect(edge.0, edge.1, outer_edge.0, outer_edge.1)
        })
    }) {
        return Err(EditError::OpeningOutsideFace { face });
    }

    for opening in &opening_projected {
        if rect_projected.iter().any(|corner| {
            point_in_polygon(corner.point, opening) || point_on_polygon_edge(corner.point, opening)
        }) || opening.iter().any(|corner| {
            point_in_polygon(corner.point, &rect_projected)
                || point_on_polygon_edge(corner.point, &rect_projected)
        }) || polygon_edges(&rect_projected).iter().any(|edge| {
            polygon_edges(opening).iter().any(|opening_edge| {
                !shares_endpoint(edge.0, edge.1, opening_edge.0, opening_edge.1)
                    && segments_intersect(edge.0, edge.1, opening_edge.0, opening_edge.1)
            })
        }) {
            return Err(EditError::OpeningOverlapsExisting { face });
        }
    }

    Ok(())
}

fn project_xy_vertex_loop(
    shell: &AnalyticShell,
    loop_vertices: &[AnalyticVertexId],
) -> Option<Vec<ProjectedVertex>> {
    loop_vertices
        .iter()
        .map(|vertex| {
            let position = shell.vertices.get(vertex.index() as usize)?.position;
            Some(ProjectedVertex {
                id: *vertex,
                point: [position[0], position[1]],
            })
        })
        .collect()
}

fn project_xy_point_loop(points: &[[f32; 3]]) -> Vec<ProjectedVertex> {
    points
        .iter()
        .enumerate()
        .map(|(index, point)| ProjectedVertex {
            id: AnalyticVertexId::from_index(usize_to_u32(index)),
            point: [point[0], point[1]],
        })
        .collect()
}

fn triangulate_face_with_openings(
    shell: &AnalyticShell,
    face: AnalyticFaceId,
    plane: Plane,
    outer: &[AnalyticVertexId],
    openings: &[Vec<AnalyticVertexId>],
) -> Result<Vec<[AnalyticVertexId; 3]>, TessellateError> {
    let basis = face_basis(shell, plane, outer).ok_or(TessellateError::EarClipFailed { face })?;
    let mut merged = project_loop(shell, outer, basis);
    orient_ccw(&mut merged);

    let projected_openings = openings
        .iter()
        .map(|opening| {
            let mut projected = project_loop(shell, opening, basis);
            orient_cw(&mut projected);
            projected
        })
        .collect::<Vec<_>>();

    // Delegate bridging and ear clipping to the shared deterministic
    // facility (exedra_triangulate): exact-sign predicates, the brief-11
    // lowest-stable-index tie-break, and typed failures. f32 coordinates
    // promote to f64 exactly, so determinism carries through.
    let outer_points: Vec<[f64; 2]> = merged
        .iter()
        .map(|v| [f64::from(v.point[0]), f64::from(v.point[1])])
        .collect();
    let opening_points: Vec<Vec<[f64; 2]>> = projected_openings
        .iter()
        .map(|opening| {
            opening
                .iter()
                .map(|v| [f64::from(v.point[0]), f64::from(v.point[1])])
                .collect()
        })
        .collect();
    let holes: Vec<&[[f64; 2]]> = opening_points.iter().map(Vec::as_slice).collect();
    let input = exedra_triangulate::PolygonInput {
        outer: &outer_points,
        holes: &holes,
    };
    let triangulation =
        exedra_triangulate::triangulate(&input, &exedra_triangulate::TriParams::default())
            .map_err(|error| map_triangulate_error(shell, face, error))?;

    // Triangle indices address `outer ++ openings...` in order; map back to
    // analytic vertex ids through the same concatenation.
    let ids: Vec<AnalyticVertexId> = merged
        .iter()
        .chain(projected_openings.iter().flatten())
        .map(|v| v.id)
        .collect();
    Ok(triangulation
        .triangles
        .iter()
        .map(|t| [ids[t[0] as usize], ids[t[1] as usize], ids[t[2] as usize]])
        .collect())
}

/// Maps shared-triangulator failures onto the analytic error surface.
fn map_triangulate_error(
    shell: &AnalyticShell,
    face: AnalyticFaceId,
    error: exedra_triangulate::TriError,
) -> TessellateError {
    use exedra_triangulate::TriError;
    match error {
        TriError::HoleOutsideOuter { hole } | TriError::UnbridgeableHole { hole } => {
            let openings = &shell.faces[face.index() as usize].openings;
            match openings.get(hole) {
                Some(&opening) => TessellateError::OpeningBridgeFailed { face, opening },
                None => TessellateError::EarClipFailed { face },
            }
        }
        _ => TessellateError::EarClipFailed { face },
    }
}

fn face_basis(
    shell: &AnalyticShell,
    plane: Plane,
    outer: &[AnalyticVertexId],
) -> Option<([f32; 3], [f32; 3], [f32; 3])> {
    let origin = shell
        .vertices
        .get(outer.first()?.index() as usize)?
        .position;
    let tangent = outer.windows(2).find_map(|edge| {
        let start = shell.vertices.get(edge[0].index() as usize)?.position;
        let end = shell.vertices.get(edge[1].index() as usize)?.position;
        normalize(sub(end, start))
    })?;
    let bitangent = cross(plane.normal, tangent);
    Some((origin, tangent, bitangent))
}

fn project_loop(
    shell: &AnalyticShell,
    loop_vertices: &[AnalyticVertexId],
    basis: ([f32; 3], [f32; 3], [f32; 3]),
) -> Vec<ProjectedVertex> {
    let (origin, tangent, bitangent) = basis;
    loop_vertices
        .iter()
        .map(|vertex| {
            let position = shell.vertices[vertex.index() as usize].position;
            let delta = sub(position, origin);
            ProjectedVertex {
                id: *vertex,
                point: [dot(delta, tangent), dot(delta, bitangent)],
            }
        })
        .collect()
}

fn orient_ccw(loop_vertices: &mut [ProjectedVertex]) {
    if signed_area(loop_vertices) < 0.0 {
        loop_vertices.reverse();
    }
}

fn orient_cw(loop_vertices: &mut [ProjectedVertex]) {
    if signed_area(loop_vertices) > 0.0 {
        loop_vertices.reverse();
    }
}

fn signed_area(loop_vertices: &[ProjectedVertex]) -> f32 {
    if loop_vertices.len() < 3 {
        return 0.0;
    }
    let mut area = 0.0;
    for index in 0..loop_vertices.len() {
        let current = loop_vertices[index].point;
        let next = loop_vertices[(index + 1) % loop_vertices.len()].point;
        area += current[0] * next[1] - next[0] * current[1];
    }
    area * 0.5
}

fn polygon_edges(loop_vertices: &[ProjectedVertex]) -> Vec<([f32; 2], [f32; 2])> {
    (0..loop_vertices.len())
        .map(|index| {
            (
                loop_vertices[index].point,
                loop_vertices[(index + 1) % loop_vertices.len()].point,
            )
        })
        .collect()
}

fn shares_endpoint(a0: [f32; 2], a1: [f32; 2], b0: [f32; 2], b1: [f32; 2]) -> bool {
    same_point(a0, b0) || same_point(a0, b1) || same_point(a1, b0) || same_point(a1, b1)
}

fn same_point(a: [f32; 2], b: [f32; 2]) -> bool {
    (a[0] - b[0]).abs() <= PLANAR_EPSILON && (a[1] - b[1]).abs() <= PLANAR_EPSILON
}

fn segments_intersect(a0: [f32; 2], a1: [f32; 2], b0: [f32; 2], b1: [f32; 2]) -> bool {
    let o1 = orient2d(a0, a1, b0);
    let o2 = orient2d(a0, a1, b1);
    let o3 = orient2d(b0, b1, a0);
    let o4 = orient2d(b0, b1, a1);

    if on_segment(a0, a1, b0)
        || on_segment(a0, a1, b1)
        || on_segment(b0, b1, a0)
        || on_segment(b0, b1, a1)
    {
        return true;
    }

    (o1 > PLANAR_EPSILON && o2 < -PLANAR_EPSILON || o1 < -PLANAR_EPSILON && o2 > PLANAR_EPSILON)
        && (o3 > PLANAR_EPSILON && o4 < -PLANAR_EPSILON
            || o3 < -PLANAR_EPSILON && o4 > PLANAR_EPSILON)
}

fn on_segment(a: [f32; 2], b: [f32; 2], point: [f32; 2]) -> bool {
    orient2d(a, b, point).abs() <= PLANAR_EPSILON
        && point[0] >= a[0].min(b[0]) - PLANAR_EPSILON
        && point[0] <= a[0].max(b[0]) + PLANAR_EPSILON
        && point[1] >= a[1].min(b[1]) - PLANAR_EPSILON
        && point[1] <= a[1].max(b[1]) + PLANAR_EPSILON
}

fn orient2d(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> f32 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn point_in_polygon(point: [f32; 2], polygon: &[ProjectedVertex]) -> bool {
    let mut inside = false;
    for index in 0..polygon.len() {
        let a = polygon[index].point;
        let b = polygon[(index + 1) % polygon.len()].point;
        let crosses = (a[1] > point[1]) != (b[1] > point[1]);
        if crosses {
            let t = (point[1] - a[1]) / (b[1] - a[1]);
            let x = a[0] + t * (b[0] - a[0]);
            if x >= point[0] - PLANAR_EPSILON {
                inside = !inside;
            }
        }
    }
    inside
}

fn point_on_polygon_edge(point: [f32; 2], polygon: &[ProjectedVertex]) -> bool {
    polygon_edges(polygon)
        .iter()
        .any(|edge| on_segment(edge.0, edge.1, point))
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
    use alloc::vec;

    use exedra::ExtractParams;

    use super::{
        AnalyticFaceId, AnalyticShellBuilder, BuildError, EditError, RectFrameParams,
        RectOpeningParams, RegionId, TessellateParams, rect_frame_xy,
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
    fn rect_frame_builds_one_face_with_opening_and_tessellates_deterministically() {
        let shell_a = rect_frame_xy(&RectFrameParams::default()).expect("frame builds");
        let shell_b = rect_frame_xy(&RectFrameParams::default()).expect("frame builds");

        assert_eq!(shell_a.faces().len(), 1);
        assert_eq!(
            shell_a
                .face_opening_vertices(AnalyticFaceId::from_index(0))
                .expect("opening loop should exist")
                .len(),
            1
        );

        let mesh_a = shell_a
            .to_exedra_mesh(&TessellateParams::default())
            .expect("tessellation should succeed");
        let mesh_b = shell_b
            .to_exedra_mesh(&TessellateParams::default())
            .expect("tessellation should succeed");

        assert_eq!(mesh_a.mesh.faces().count(), 8);
        assert_eq!(mesh_b.mesh.faces().count(), 8);
        assert_eq!(mesh_a.face_provenance.len(), 8);
        assert_eq!(mesh_b.face_provenance.len(), 8);
        assert!(
            mesh_a
                .face_provenance
                .iter()
                .all(|(analytic_face, _)| *analytic_face == AnalyticFaceId::from_index(0))
        );
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

    #[test]
    fn set_face_region_updates_existing_face() {
        let mut builder = AnalyticShellBuilder::new();
        let v0 = builder.push_vertex([0.0, 0.0, 0.0]);
        let v1 = builder.push_vertex([2.0, 0.0, 0.0]);
        let v2 = builder.push_vertex([2.0, 1.0, 0.0]);
        let v3 = builder.push_vertex([0.0, 1.0, 0.0]);
        let face = builder
            .add_planar_face(&[v0, v1, v2, v3], RegionId(3))
            .expect("quad should build");
        let mut shell = builder.build();

        shell
            .set_face_region(face, RegionId(9))
            .expect("region update should succeed");
        assert_eq!(shell.face_region(face), Some(RegionId(9)));
    }

    #[test]
    fn add_rect_opening_xy_mutates_existing_face() {
        let mut builder = AnalyticShellBuilder::new();
        let v0 = builder.push_vertex([0.0, 0.0, 0.0]);
        let v1 = builder.push_vertex([4.0, 0.0, 0.0]);
        let v2 = builder.push_vertex([4.0, 3.0, 0.0]);
        let v3 = builder.push_vertex([0.0, 3.0, 0.0]);
        let face = builder
            .add_planar_face(&[v0, v1, v2, v3], RegionId(5))
            .expect("outer face should build");
        let mut shell = builder.build();

        let opening = shell
            .add_rect_opening_xy(
                face,
                &RectOpeningParams {
                    min: [1.0, 1.0],
                    max: [3.0, 2.0],
                },
            )
            .expect("opening should be added");

        let openings = shell
            .face_opening_vertices(face)
            .expect("opening loop should exist");
        assert_eq!(openings.len(), 1);
        assert_eq!(openings[0].len(), 4);
        assert_eq!(
            shell
                .face(face)
                .expect("face should still exist")
                .openings
                .first()
                .copied(),
            Some(opening)
        );

        let tessellated = shell
            .to_exedra_mesh(&TessellateParams::default())
            .expect("mutated shell should tessellate");
        assert!(tessellated.mesh.validate_fast().is_empty());
        assert!(tessellated.mesh.validate_deep().is_empty());
        assert_eq!(tessellated.mesh.faces().count(), 8);
    }

    #[test]
    fn add_rect_opening_xy_rejects_out_of_bounds_opening() {
        let mut builder = AnalyticShellBuilder::new();
        let v0 = builder.push_vertex([0.0, 0.0, 0.0]);
        let v1 = builder.push_vertex([4.0, 0.0, 0.0]);
        let v2 = builder.push_vertex([4.0, 3.0, 0.0]);
        let v3 = builder.push_vertex([0.0, 3.0, 0.0]);
        let face = builder
            .add_planar_face(&[v0, v1, v2, v3], RegionId(5))
            .expect("outer face should build");
        let mut shell = builder.build();

        let error = shell
            .add_rect_opening_xy(
                face,
                &RectOpeningParams {
                    min: [3.5, 1.0],
                    max: [4.5, 2.0],
                },
            )
            .expect_err("opening outside outer face should fail");
        assert_eq!(error, EditError::OpeningOutsideFace { face });
    }

    #[test]
    fn add_rect_opening_xy_accepts_clockwise_xy_face() {
        let mut builder = AnalyticShellBuilder::new();
        let v0 = builder.push_vertex([0.0, 0.0, 0.0]);
        let v1 = builder.push_vertex([4.0, 0.0, 0.0]);
        let v2 = builder.push_vertex([4.0, 3.0, 0.0]);
        let v3 = builder.push_vertex([0.0, 3.0, 0.0]);
        let face = builder
            .add_planar_face(&[v0, v3, v2, v1], RegionId(6))
            .expect("clockwise outer face should build");
        let mut shell = builder.build();

        shell
            .add_rect_opening_xy(
                face,
                &RectOpeningParams {
                    min: [1.0, 1.0],
                    max: [3.0, 2.0],
                },
            )
            .expect("opening should be added to clockwise XY face");

        let tessellated = shell
            .to_exedra_mesh(&TessellateParams::default())
            .expect("mutated shell should tessellate");
        assert!(tessellated.mesh.validate_fast().is_empty());
        assert!(tessellated.mesh.validate_deep().is_empty());
        assert_eq!(shell.face_region(face), Some(RegionId(6)));
        assert_eq!(tessellated.mesh.faces().count(), 8);
    }

    #[test]
    fn remove_opening_removes_existing_face_opening() {
        let mut builder = AnalyticShellBuilder::new();
        let v0 = builder.push_vertex([0.0, 0.0, 0.0]);
        let v1 = builder.push_vertex([4.0, 0.0, 0.0]);
        let v2 = builder.push_vertex([4.0, 3.0, 0.0]);
        let v3 = builder.push_vertex([0.0, 3.0, 0.0]);
        let face = builder
            .add_planar_face(&[v0, v1, v2, v3], RegionId(11))
            .expect("outer face should build");
        let mut shell = builder.build();

        let opening = shell
            .add_rect_opening_xy(
                face,
                &RectOpeningParams {
                    min: [1.0, 1.0],
                    max: [3.0, 2.0],
                },
            )
            .expect("opening should be added");
        shell
            .remove_opening(face, opening)
            .expect("existing opening should be removed");

        let openings = shell
            .face_opening_vertices(face)
            .expect("face topology should stay readable");
        assert!(openings.is_empty());

        let tessellated = shell
            .to_exedra_mesh(&TessellateParams::default())
            .expect("shell without openings should tessellate");
        assert!(tessellated.mesh.validate_fast().is_empty());
        assert!(tessellated.mesh.validate_deep().is_empty());
        assert_eq!(tessellated.mesh.faces().count(), 1);
        assert_eq!(tessellated.face_provenance.len(), 1);
        assert_eq!(tessellated.face_provenance[0].0, face);
    }

    #[test]
    fn remove_opening_rejects_non_member_opening() {
        let mut builder = AnalyticShellBuilder::new();
        let v0 = builder.push_vertex([0.0, 0.0, 0.0]);
        let v1 = builder.push_vertex([8.0, 0.0, 0.0]);
        let v2 = builder.push_vertex([8.0, 4.0, 0.0]);
        let v3 = builder.push_vertex([0.0, 4.0, 0.0]);
        let face_a = builder
            .add_planar_face(&[v0, v1, v2, v3], RegionId(13))
            .expect("first face should build");
        let v4 = builder.push_vertex([10.0, 0.0, 0.0]);
        let v5 = builder.push_vertex([14.0, 0.0, 0.0]);
        let v6 = builder.push_vertex([14.0, 3.0, 0.0]);
        let v7 = builder.push_vertex([10.0, 3.0, 0.0]);
        let face_b = builder
            .add_planar_face(&[v4, v5, v6, v7], RegionId(17))
            .expect("second face should build");
        let mut shell = builder.build();

        let opening = shell
            .add_rect_opening_xy(
                face_a,
                &RectOpeningParams {
                    min: [1.0, 1.0],
                    max: [3.0, 2.0],
                },
            )
            .expect("opening should be added");

        let error = shell
            .remove_opening(face_b, opening)
            .expect_err("opening on a different face should be rejected");
        assert_eq!(
            error,
            EditError::OpeningNotOnFace {
                face: face_b,
                opening
            }
        );

        shell
            .remove_opening(face_a, opening)
            .expect("original owner should still remove the opening");
        let error = shell
            .remove_opening(face_a, opening)
            .expect_err("removing the same opening twice should fail");
        assert_eq!(
            error,
            EditError::OpeningNotOnFace {
                face: face_a,
                opening
            }
        );
    }

    #[test]
    fn remove_opening_keeps_other_openings_and_remaining_opening_tessellates() {
        let mut builder = AnalyticShellBuilder::new();
        let v0 = builder.push_vertex([0.0, 0.0, 0.0]);
        let v1 = builder.push_vertex([8.0, 0.0, 0.0]);
        let v2 = builder.push_vertex([8.0, 4.0, 0.0]);
        let v3 = builder.push_vertex([0.0, 4.0, 0.0]);
        let face = builder
            .add_planar_face(&[v0, v1, v2, v3], RegionId(23))
            .expect("outer face should build");
        let mut shell = builder.build();

        let opening_a = shell
            .add_rect_opening_xy(
                face,
                &RectOpeningParams {
                    min: [1.0, 1.0],
                    max: [2.5, 2.5],
                },
            )
            .expect("first opening should be added");
        let opening_b = shell
            .add_rect_opening_xy(
                face,
                &RectOpeningParams {
                    min: [5.0, 1.0],
                    max: [6.5, 2.5],
                },
            )
            .expect("second opening should be added");

        shell
            .remove_opening(face, opening_a)
            .expect("first opening should be removed");

        let openings = shell
            .face(face)
            .expect("face should still exist")
            .openings
            .clone();
        assert_eq!(openings, vec![opening_b]);
        assert_eq!(
            shell
                .face_opening_vertices(face)
                .expect("remaining opening should still be readable")
                .len(),
            1
        );

        let tessellated = shell
            .to_exedra_mesh(&TessellateParams::default())
            .expect("shell with one remaining opening should tessellate");
        assert!(tessellated.mesh.validate_fast().is_empty());
        assert!(tessellated.mesh.validate_deep().is_empty());
        assert_eq!(tessellated.mesh.faces().count(), 8);
        assert!(
            tessellated
                .face_provenance
                .iter()
                .all(|(analytic_face, _)| *analytic_face == face)
        );
    }

    #[test]
    fn planar_face_with_explicit_opening_tessellates_and_preserves_region() {
        let mut builder = AnalyticShellBuilder::new();
        let outer_bottom_left = builder.push_vertex([0.0, 0.0, 0.0]);
        let outer_bottom_right = builder.push_vertex([4.0, 0.0, 0.0]);
        let outer_top_right = builder.push_vertex([4.0, 3.0, 0.0]);
        let outer_top_left = builder.push_vertex([0.0, 3.0, 0.0]);
        let inner_bottom_left = builder.push_vertex([1.0, 1.0, 0.0]);
        let inner_bottom_right = builder.push_vertex([3.0, 1.0, 0.0]);
        let inner_top_right = builder.push_vertex([3.0, 2.0, 0.0]);
        let inner_top_left = builder.push_vertex([1.0, 2.0, 0.0]);

        let face = builder
            .add_planar_face_with_openings(
                &[
                    outer_bottom_left,
                    outer_bottom_right,
                    outer_top_right,
                    outer_top_left,
                ],
                &[&[
                    inner_bottom_left,
                    inner_top_left,
                    inner_top_right,
                    inner_bottom_right,
                ]],
                RegionId(19),
            )
            .expect("face with opening should build");
        let shell = builder.build();

        let tessellated = shell
            .to_exedra_mesh(&TessellateParams::default())
            .expect("tessellation should succeed");
        assert!(tessellated.mesh.validate_fast().is_empty());
        assert!(tessellated.mesh.validate_deep().is_empty());
        assert_eq!(
            shell
                .face_opening_vertices(face)
                .expect("opening loop should exist")
                .len(),
            1
        );
        assert!(
            tessellated
                .face_provenance
                .iter()
                .all(|(analytic_face, _)| *analytic_face == face)
        );
        for (_, mesh_face) in &tessellated.face_provenance {
            let region = tessellated
                .mesh
                .attrs()
                .dense(exedra::attr::FACE_REGION)
                .and_then(|layer| layer.get(mesh_face.as_id()).copied());
            assert_eq!(region, Some(19));
        }
    }
}
