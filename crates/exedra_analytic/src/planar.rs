// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Planar authoring and realization over the shared f64 boundary.

use crate::{
    body::{Body, BodyParts, BoundaryLoop, Coedge, Edge, EdgeId, Face, Sense, ValidationPolicy},
    geometry::{Curve, ParameterDomain, ParameterRange, Surface},
    usize_to_u32,
};
use alloc::{collections::BTreeMap, vec::Vec};

use exedra_math::{cross, dot, sub};
use exedra_mesh::{FaceId, Mesh, MeshBuilder};

/// Region ID carried by analytic faces and written into tessellated meshes.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct RegionId(pub u32);

/// Parameters for adding one rectangular opening on an XY-aligned planar face.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct RectOpeningParams {
    /// Rectangle minimum corner in the face's XY plane.
    pub min: [f64; 2],
    /// Rectangle maximum corner in the face's XY plane.
    pub max: [f64; 2],
}

pub use crate::body::{
    FaceId as AnalyticFaceId, LoopId as AnalyticLoopId, VertexId as AnalyticVertexId,
};

/// A planar face in the shared body model; its role is its region.
pub type PlanarFace = Face<RegionId>;

#[derive(Copy, Clone, Debug, PartialEq)]
struct Plane {
    normal: [f64; 3],
    distance: f64,
}

/// An editable planar boundary backed by the shared f64 body representation.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnalyticShell {
    body: Body<RegionId>,
}

impl AnalyticShell {
    /// Creates an empty planar shell.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    /// The canonical f64 boundary, including shared edges and oriented loops.
    #[must_use]
    pub fn body(&self) -> &Body<RegionId> {
        &self.body
    }
    /// Native f64 vertex positions in stable table order.
    #[must_use]
    pub fn vertices(&self) -> &[[f64; 3]] {
        self.body.vertices()
    }
    /// Planar faces in stable table order.
    #[must_use]
    pub fn faces(&self) -> &[PlanarFace] {
        self.body.faces()
    }
    /// A planar face by its local ID.
    #[must_use]
    pub fn face(&self, face: AnalyticFaceId) -> Option<&PlanarFace> {
        self.body.face(face)
    }
    /// The face's semantic region.
    #[must_use]
    pub fn face_region(&self, face: AnalyticFaceId) -> Option<RegionId> {
        Some(self.face(face)?.role)
    }
    /// Outer-loop vertex IDs in traversal order.
    pub fn face_vertices(&self, face: AnalyticFaceId) -> Option<Vec<AnalyticVertexId>> {
        self.loop_vertices(*self.face(face)?.loops.first()?).ok()
    }
    /// Opening-loop vertex IDs in stable loop order.
    pub fn face_opening_vertices(
        &self,
        face: AnalyticFaceId,
    ) -> Option<Vec<Vec<AnalyticVertexId>>> {
        self.face(face)?
            .loops
            .iter()
            .skip(1)
            .map(|&id| self.loop_vertices(id).ok())
            .collect()
    }
    /// Changes one face's region without changing its geometry or IDs.
    ///
    /// # Errors
    /// Returns [`EditError::MissingFace`] for an unknown face.
    pub fn set_face_region(
        &mut self,
        face: AnalyticFaceId,
        region: RegionId,
    ) -> Result<(), EditError> {
        self.body
            .parts
            .faces
            .get_mut(face.index())
            .ok_or(EditError::MissingFace { face })?
            .role = region;
        Ok(())
    }
    /// Adds a rectangular opening on an XY-aligned face, retaining native f64 coordinates.
    ///
    /// # Errors
    /// Returns a typed [`EditError`] before mutation for invalid placement or contact.
    pub fn add_rect_opening_xy(
        &mut self,
        face: AnalyticFaceId,
        params: &RectOpeningParams,
    ) -> Result<AnalyticLoopId, EditError> {
        if !params.min.into_iter().chain(params.max).all(f64::is_finite)
            || params.min[0] >= params.max[0]
            || params.min[1] >= params.max[1]
        {
            return Err(EditError::InvalidRectBounds);
        }
        let record = self.face(face).ok_or(EditError::MissingFace { face })?;
        let z = xy_plane_z(face, plane_of(record))?;
        let outer = self
            .face_vertices(face)
            .ok_or(EditError::InvalidFaceTopology { face })?;
        let openings = self
            .face_opening_vertices(face)
            .ok_or(EditError::InvalidFaceTopology { face })?;
        let points = rect_loop_vertices(params, z);
        validate_rect_opening_xy(self, face, &outer, &openings, &points)?;
        let ids: Vec<_> = points
            .into_iter()
            .map(|point| {
                let id = AnalyticVertexId(usize_to_u32(self.body.parts.vertices.len()));
                self.body.parts.vertices.push(point);
                id
            })
            .collect();
        let id = push_loop(&mut self.body.parts, None, &ids);
        self.body.parts.faces[face.index()].loops.push(id);
        Ok(id)
    }
    /// Removes one opening from its face; other local IDs remain unchanged.
    ///
    /// # Errors
    /// Returns [`EditError::MissingFace`] or [`EditError::OpeningNotOnFace`].
    pub fn remove_opening(
        &mut self,
        face: AnalyticFaceId,
        opening: AnalyticLoopId,
    ) -> Result<(), EditError> {
        let record = self
            .body
            .parts
            .faces
            .get_mut(face.index())
            .ok_or(EditError::MissingFace { face })?;
        let index = record
            .loops
            .iter()
            .enumerate()
            .skip(1)
            .find(|(_, id)| **id == opening)
            .map(|(index, _)| index)
            .ok_or(EditError::OpeningNotOnFace { face, opening })?;
        record.loops.remove(index);
        Ok(())
    }
    /// Realizes the planar boundary, checking arrangements after f32 narrowing.
    ///
    /// # Errors
    /// Returns a typed [`TessellateError`] for collapsed or invalid emitted boundaries.
    pub fn to_exedra_mesh(
        &self,
        params: &TessellateParams,
    ) -> Result<TessellatedShell, TessellateError> {
        let mut builder = MeshBuilder::new();
        let mut max_vertex_displacement = 0.0_f64;
        for (index, &point) in self.vertices().iter().enumerate() {
            let emitted = exedra_math::narrow(point);
            if !emitted.iter().all(|v| v.is_finite()) {
                return Err(TessellateError::NonFiniteEmission {
                    vertex: AnalyticVertexId(usize_to_u32(index)),
                });
            }
            let delta = sub(emitted.map(f64::from), point);
            max_vertex_displacement =
                max_vertex_displacement.max(libm::hypot(libm::hypot(delta[0], delta[1]), delta[2]));
            let _ = builder.push_vertex(emitted);
        }
        let mut source_faces = Vec::new();
        for (index, face) in self.faces().iter().enumerate() {
            let id = AnalyticFaceId(usize_to_u32(index));
            for &loop_id in &face.loops {
                for coedge in &self
                    .body
                    .boundary_loop(loop_id)
                    .expect("validated loop")
                    .coedges
                {
                    let edge = &self.body.edges()[coedge.edge.index()];
                    if exedra_math::narrow(self.vertices()[edge.start.index()])
                        == exedra_math::narrow(self.vertices()[edge.end.index()])
                    {
                        return Err(TessellateError::CollapsedEdge {
                            face: id,
                            edge: coedge.edge,
                        });
                    }
                }
            }
            let triangles = self.triangulate_face_with_openings(id)?;
            if face.loops.len() == 1 {
                let polygon: Vec<_> = self
                    .face_vertices(id)
                    .expect("validated face")
                    .into_iter()
                    .map(|v| v.0)
                    .collect();
                builder
                    .add_face(&polygon)
                    .map_err(TessellateError::KernelBuild)?;
                source_faces.push(id);
            } else {
                for tri in triangles {
                    builder
                        .add_face(&tri.map(|v| v.0))
                        .map_err(TessellateError::KernelBuild)?;
                    source_faces.push(id);
                }
            }
        }
        let built = builder.build().map_err(TessellateError::KernelBuild)?;
        let mut mesh = built.mesh;
        if params.write_face_regions {
            let mut edit = mesh.edit_with(exedra_mesh::ChangeSetBuilder::new());
            for (&mesh_face, &native) in built.face_ids.iter().zip(&source_faces) {
                exedra_mesh::op::set_face_region(
                    &mut edit,
                    mesh_face,
                    self.faces()[native.index()].role.0,
                )
                .map_err(TessellateError::SetFaceRegion)?;
            }
            let _ = edit.finish();
        }
        let face_provenance = source_faces.into_iter().zip(built.face_ids).collect();
        Ok(TessellatedShell {
            mesh,
            face_provenance,
            max_vertex_displacement,
        })
    }
    fn loop_vertices(&self, id: AnalyticLoopId) -> Result<Vec<AnalyticVertexId>, LoopError> {
        let loop_ = self
            .body
            .boundary_loop(id)
            .ok_or(LoopError::MissingLoop(id))?;
        Ok(loop_
            .coedges
            .iter()
            .map(|&c| self.body.coedge_vertices(c).expect("validated edge")[0])
            .collect())
    }
    fn triangulate_face_with_openings(
        &self,
        face: AnalyticFaceId,
    ) -> Result<Vec<[AnalyticVertexId; 3]>, TessellateError> {
        let record = self
            .face(face)
            .ok_or(TessellateError::MissingFace { face })?;
        let outer = self.loop_vertices(record.loops[0])?;
        let holes = record
            .loops
            .iter()
            .skip(1)
            .map(|&id| self.loop_vertices(id))
            .collect::<Result<Vec<_>, _>>()?;
        triangulate_face_with_openings(self, face, plane_of(record), &outer, &holes)
    }
}

/// Mutable planar authoring over the shared boundary tables.
#[derive(Clone, Debug, Default)]
pub struct AnalyticShellBuilder {
    parts: BodyParts<RegionId>,
    edges: BTreeMap<(AnalyticVertexId, AnalyticVertexId), EdgeId>,
}
impl AnalyticShellBuilder {
    /// Creates an empty planar builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    /// Appends a native f64 position. Finalization checks finiteness, including unused vertices.
    #[must_use]
    pub fn push_vertex(&mut self, point: [f64; 3]) -> AnalyticVertexId {
        let id = AnalyticVertexId(usize_to_u32(self.parts.vertices.len()));
        self.parts.vertices.push(point);
        id
    }
    /// Adds a planar face with one outer loop.
    ///
    /// # Errors
    /// Returns [`BuildError`] for invalid geometry or references.
    pub fn add_planar_face(
        &mut self,
        vertices: &[AnalyticVertexId],
        region: RegionId,
    ) -> Result<AnalyticFaceId, BuildError> {
        self.add_planar_face_with_openings(vertices, &[], region)
    }
    /// Adds a face and explicit opening loops, sharing edges by vertex identity.
    /// Failed validation leaves the builder unchanged.
    ///
    /// # Errors
    /// Returns [`BuildError`] for invalid geometry, planarity or references.
    pub fn add_planar_face_with_openings(
        &mut self,
        outer: &[AnalyticVertexId],
        holes: &[&[AnalyticVertexId]],
        region: RegionId,
    ) -> Result<AnalyticFaceId, BuildError> {
        let points = self.validate_loop_vertices(outer)?;
        let plane = Plane::from_points(&points).ok_or(BuildError::DegenerateLoop)?;
        validate_planar_loop(&points, plane)?;
        for hole in holes {
            validate_planar_loop(&self.validate_loop_vertices(hole)?, plane)?;
        }
        let origin = points[0];
        let tangent = normalize(sub(points[1], origin)).ok_or(BuildError::DegenerateLoop)?;
        let frame = exedra_math::Placement3::from_axes(
            tangent,
            cross(plane.normal, tangent),
            plane.normal,
            origin,
        );
        let surface = Surface::plane(frame).map_err(BuildError::Geometry)?;
        let mut min = [f64::INFINITY; 2];
        let mut max = [f64::NEG_INFINITY; 2];
        for &p in &points {
            let d = sub(p, origin);
            let uv = [dot(d, tangent), dot(d, cross(plane.normal, tangent))];
            for i in 0..2 {
                min[i] = min[i].min(uv[i]);
                max[i] = max[i].max(uv[i]);
            }
        }
        let domain = ParameterDomain {
            u: ParameterRange::new(min[0], max[0]).map_err(BuildError::Geometry)?,
            v: ParameterRange::new(min[1], max[1]).map_err(BuildError::Geometry)?,
        };
        let mut loops = Vec::with_capacity(1 + holes.len());
        loops.push(push_loop(&mut self.parts, Some(&mut self.edges), outer));
        for hole in holes {
            loops.push(push_loop(&mut self.parts, Some(&mut self.edges), hole));
        }
        let id = AnalyticFaceId(usize_to_u32(self.parts.faces.len()));
        self.parts.faces.push(Face {
            surface,
            domain,
            sense: Sense::Forward,
            loops,
            role: region,
        });
        Ok(id)
    }
    /// Checks the completed boundary tables and returns an editable planar shell.
    ///
    /// # Errors
    /// Returns [`BuildError::Boundary`] for invalid references or support agreement.
    pub fn build(self) -> Result<AnalyticShell, BuildError> {
        Ok(AnalyticShell {
            body: Body::try_from_parts(
                self.parts,
                &ValidationPolicy {
                    max_gap: PLANAR_EPSILON,
                },
            )
            .map_err(BuildError::Boundary)?,
        })
    }
    fn validate_loop_vertices(
        &self,
        ids: &[AnalyticVertexId],
    ) -> Result<Vec<[f64; 3]>, BuildError> {
        if ids.len() < 3 {
            return Err(BuildError::LoopTooSmall);
        }
        if has_duplicates(ids) {
            return Err(BuildError::RepeatedVertex);
        }
        let points = ids
            .iter()
            .map(|v| {
                self.parts
                    .vertices
                    .get(v.index())
                    .copied()
                    .ok_or(BuildError::MissingVertex(*v))
            })
            .collect::<Result<Vec<_>, _>>()?;
        for i in 0..points.len() {
            Curve::line(points[i], sub(points[(i + 1) % points.len()], points[i]))
                .map_err(BuildError::Geometry)?;
        }
        Ok(points)
    }
}

/// Parameters for one deterministic analytic-to-mesh conversion.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct TessellateParams {
    /// When true, analytic face regions are written into `exedra_mesh::attr::FACE_REGION`.
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
    /// Largest distance from a native vertex to its emitted f32 position.
    pub max_vertex_displacement: f64,
}

/// Parameters for the `rect_frame_xy` helper.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct RectFrameParams {
    /// Outer rectangle minimum corner.
    pub outer_min: [f64; 2],
    /// Outer rectangle maximum corner.
    pub outer_max: [f64; 2],
    /// Inner opening minimum corner.
    pub inner_min: [f64; 2],
    /// Inner opening maximum corner.
    pub inner_max: [f64; 2],
    /// Z plane for the frame.
    pub z: f64,
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

    builder.build()
}

/// Builder-time validation error.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum BuildError {
    /// Independent curve or supporting-plane geometry is invalid.
    Geometry(crate::geometry::GeometryError),
    /// The completed shared boundary is invalid.
    Boundary(crate::body::BodyError),
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
    /// A native coordinate overflows the emitted f32 representation.
    NonFiniteEmission {
        /// Offending native vertex.
        vertex: AnalyticVertexId,
    },
    /// An active line edge collapses after f32 narrowing.
    CollapsedEdge {
        /// Owning face.
        face: AnalyticFaceId,
        /// Offending shared edge.
        edge: EdgeId,
    },
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
    KernelBuild(exedra_mesh::BuildError),
    /// `FACE_REGION` write failed unexpectedly.
    SetFaceRegion(exedra_mesh::op::SetFaceRegionError),
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum LoopError {
    MissingLoop(AnalyticLoopId),
}

impl From<LoopError> for TessellateError {
    fn from(error: LoopError) -> Self {
        match error {
            LoopError::MissingLoop(loop_id) => Self::MissingLoop { loop_id },
        }
    }
}

/// Planarity/coincidence tolerance, sourced from the shared kernel policy
/// rather than a module-local constant (`exedra_mesh::NumericPolicy`).
const PLANAR_EPSILON: f64 = exedra_mesh::NumericPolicy::DEFAULT_COPLANAR_TOLERANCE as f64;

#[derive(Copy, Clone, Debug, PartialEq)]
struct ProjectedVertex {
    id: AnalyticVertexId,
    point: [f64; 2],
}

impl Plane {
    fn from_points(points: &[[f64; 3]]) -> Option<Self> {
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

    fn distance_to(self, point: [f64; 3]) -> f64 {
        dot(self.normal, point) - self.distance
    }
}

fn has_duplicates(ids: &[AnalyticVertexId]) -> bool {
    let mut sorted = ids.to_vec();
    sorted.sort_unstable();
    sorted.windows(2).any(|pair| pair[0] == pair[1])
}

fn validate_planar_loop(positions: &[[f64; 3]], plane: Plane) -> Result<(), BuildError> {
    for position in positions {
        if plane.distance_to(*position).abs() > PLANAR_EPSILON {
            return Err(BuildError::NonPlanarLoop);
        }
    }
    Ok(())
}

fn push_loop(
    parts: &mut BodyParts<RegionId>,
    mut lookup: Option<&mut BTreeMap<(AnalyticVertexId, AnalyticVertexId), EdgeId>>,
    vertices: &[AnalyticVertexId],
) -> AnalyticLoopId {
    let mut coedges = Vec::with_capacity(vertices.len());
    for i in 0..vertices.len() {
        let (start, end) = (vertices[i], vertices[(i + 1) % vertices.len()]);
        let key = if start < end {
            (start, end)
        } else {
            (end, start)
        };
        let existing = lookup.as_ref().and_then(|map| map.get(&key)).copied();
        let id = existing.unwrap_or_else(|| {
            let id = EdgeId(usize_to_u32(parts.edges.len()));
            let from = parts.vertices[start.index()];
            let to = parts.vertices[end.index()];
            parts.edges.push(Edge {
                curve: Curve::line(from, sub(to, from)).expect("validated line"),
                range: ParameterRange::UNIT,
                start,
                end,
                role: (),
            });
            if let Some(map) = lookup.as_mut() {
                map.insert(key, id);
            }
            id
        });
        coedges.push(Coedge {
            edge: id,
            reversed: parts.edges[id.index()].start != start,
        });
    }
    let id = AnalyticLoopId(usize_to_u32(parts.loops.len()));
    parts.loops.push(BoundaryLoop { coedges });
    id
}

fn plane_of(face: &PlanarFace) -> Plane {
    let frame = face.surface.frame();
    let normal = frame.rows.map(|r| r[2] * face.sense.sign());
    Plane {
        normal,
        distance: dot(normal, frame.rows.map(|r| r[3])),
    }
}

fn xy_plane_z(face: AnalyticFaceId, plane: Plane) -> Result<f64, EditError> {
    if plane.normal[0].abs() > PLANAR_EPSILON
        || plane.normal[1].abs() > PLANAR_EPSILON
        || (plane.normal[2].abs() - 1.0).abs() > PLANAR_EPSILON
    {
        return Err(EditError::FaceNotXyAligned { face });
    }
    Ok(plane.distance / plane.normal[2])
}

fn rect_loop_vertices(params: &RectOpeningParams, z: f64) -> [[f64; 3]; 4] {
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
    rect_loop: &[[f64; 3]; 4],
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
            let position = *shell.vertices().get(vertex.index())?;
            Some(ProjectedVertex {
                id: *vertex,
                point: [position[0], position[1]],
            })
        })
        .collect()
}

fn project_xy_point_loop(points: &[[f64; 3]]) -> Vec<ProjectedVertex> {
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
    // facility (exedra_triangulate): exact-sign predicates, the
    // lowest-stable-index tie-break, and typed failures. Emitted coordinates
    // are widened exactly after narrowing, so the checked arrangement is emitted.
    let outer_points: Vec<[f64; 2]> = merged.iter().map(|v| v.point).collect();
    let opening_points: Vec<Vec<[f64; 2]>> = projected_openings
        .iter()
        .map(|opening| opening.iter().map(|v| v.point).collect())
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
    let hole = match error {
        TriError::HoleOutsideOuter { hole } | TriError::UnbridgeableHole { hole } => Some(hole),
        TriError::BoundaryContact { first, second, .. } => first.hole.or(second.hole),
        _ => None,
    };
    let openings = &shell.faces()[face.index()].loops[1..];
    match hole.and_then(|hole| openings.get(hole)) {
        Some(&opening) => TessellateError::OpeningBridgeFailed { face, opening },
        None => TessellateError::EarClipFailed { face },
    }
}

fn face_basis(
    shell: &AnalyticShell,
    plane: Plane,
    outer: &[AnalyticVertexId],
) -> Option<([f64; 3], [f64; 3], [f64; 3])> {
    let origin = *shell.vertices().get(outer.first()?.index())?;
    let tangent = outer.windows(2).find_map(|edge| {
        let start = *shell.vertices().get(edge[0].index())?;
        let end = *shell.vertices().get(edge[1].index())?;
        normalize(sub(end, start))
    })?;
    let bitangent = cross(plane.normal, tangent);
    Some((origin, tangent, bitangent))
}

fn project_loop(
    shell: &AnalyticShell,
    loop_vertices: &[AnalyticVertexId],
    basis: ([f64; 3], [f64; 3], [f64; 3]),
) -> Vec<ProjectedVertex> {
    let (origin, tangent, bitangent) = basis;
    loop_vertices
        .iter()
        .map(|vertex| {
            let position = exedra_math::narrow(shell.vertices()[vertex.index()]).map(f64::from);
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

fn signed_area(loop_vertices: &[ProjectedVertex]) -> f64 {
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

fn polygon_edges(loop_vertices: &[ProjectedVertex]) -> Vec<([f64; 2], [f64; 2])> {
    (0..loop_vertices.len())
        .map(|index| {
            (
                loop_vertices[index].point,
                loop_vertices[(index + 1) % loop_vertices.len()].point,
            )
        })
        .collect()
}

fn shares_endpoint(a0: [f64; 2], a1: [f64; 2], b0: [f64; 2], b1: [f64; 2]) -> bool {
    same_point(a0, b0) || same_point(a0, b1) || same_point(a1, b0) || same_point(a1, b1)
}

fn same_point(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() <= PLANAR_EPSILON && (a[1] - b[1]).abs() <= PLANAR_EPSILON
}

fn segments_intersect(a0: [f64; 2], a1: [f64; 2], b0: [f64; 2], b1: [f64; 2]) -> bool {
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

fn on_segment(a: [f64; 2], b: [f64; 2], point: [f64; 2]) -> bool {
    orient2d(a, b, point).abs() <= PLANAR_EPSILON
        && point[0] >= a[0].min(b[0]) - PLANAR_EPSILON
        && point[0] <= a[0].max(b[0]) + PLANAR_EPSILON
        && point[1] >= a[1].min(b[1]) - PLANAR_EPSILON
        && point[1] <= a[1].max(b[1]) + PLANAR_EPSILON
}

fn orient2d(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn point_in_polygon(point: [f64; 2], polygon: &[ProjectedVertex]) -> bool {
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

fn point_on_polygon_edge(point: [f64; 2], polygon: &[ProjectedVertex]) -> bool {
    polygon_edges(polygon)
        .iter()
        .any(|edge| on_segment(edge.0, edge.1, point))
}

fn normalize(vector: [f64; 3]) -> Option<[f64; 3]> {
    let length = libm::hypot(libm::hypot(vector[0], vector[1]), vector[2]);
    (length.is_finite() && length > 0.0).then(|| vector.map(|v| v / length))
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use exedra_mesh::ExtractParams;

    use super::{
        AnalyticFaceId, AnalyticShellBuilder, BuildError, EditError, RectFrameParams,
        RectOpeningParams, RegionId, TessellateError, TessellateParams, rect_frame_xy,
    };

    #[test]
    fn six_authored_faces_share_one_closed_boundary() {
        let mut builder = AnalyticShellBuilder::new();
        let vertices = [
            [0.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [2.0, 3.0, 0.0],
            [0.0, 3.0, 0.0],
            [0.0, 0.0, 4.0],
            [2.0, 0.0, 4.0],
            [2.0, 3.0, 4.0],
            [0.0, 3.0, 4.0],
        ]
        .map(|p| builder.push_vertex(p));
        for ids in [
            [3, 2, 1, 0],
            [4, 5, 6, 7],
            [0, 1, 5, 4],
            [1, 2, 6, 5],
            [2, 3, 7, 6],
            [3, 0, 4, 7],
        ] {
            builder
                .add_planar_face(&ids.map(|i| vertices[i]), RegionId(7))
                .unwrap();
        }
        let shell = builder.build().unwrap();
        assert_eq!(shell.body().edges().len(), 12);
        assert!(shell.body().is_closed());
        assert_eq!(
            shell
                .body()
                .validate(&crate::body::ValidationPolicy { max_gap: 1e-12 })
                .unwrap()
                .coedges,
            24
        );
        let emitted = shell.to_exedra_mesh(&TessellateParams::default()).unwrap();
        assert!(emitted.mesh.boundary_loops().unwrap().is_empty());
        assert_eq!(emitted.face_provenance.len(), 6);
        assert_eq!(emitted.max_vertex_displacement, 0.0);
    }

    #[test]
    fn native_f64_edges_survive_authoring_and_refuse_collapsed_mesh_emission() {
        let mut builder = AnalyticShellBuilder::new();
        let vertices = [
            [1e6, 0.0, 0.0],
            [1e6 + 0.001, 0.0, 0.0],
            [1e6 + 0.001, 1.0, 0.0],
            [1e6, 1.0, 0.0],
        ]
        .map(|p| builder.push_vertex(p));
        builder.add_planar_face(&vertices, RegionId(0)).unwrap();
        let shell = builder.build().unwrap();
        assert!(shell.vertices()[1][0] > shell.vertices()[0][0]);
        assert!(matches!(
            shell.to_exedra_mesh(&TessellateParams::default()),
            Err(TessellateError::CollapsedEdge { .. })
        ));
    }

    #[test]
    fn planar_realization_reports_narrowing_without_changing_native_geometry() {
        let mut builder = AnalyticShellBuilder::new();
        let z = 1_000_000.03;
        let vertices = [[0.0, 0.0, z], [2.0, 0.0, z], [2.0, 1.0, z], [0.0, 1.0, z]]
            .map(|p| builder.push_vertex(p));
        builder.add_planar_face(&vertices, RegionId(0)).unwrap();
        let shell = builder.build().unwrap();
        let emitted = shell.to_exedra_mesh(&TessellateParams::default()).unwrap();
        assert_eq!(shell.vertices()[0][2], z);
        assert!((emitted.max_vertex_displacement - 0.03).abs() < 1e-9);
    }

    #[test]
    fn failed_face_addition_preserves_all_boundary_tables() {
        let mut builder = AnalyticShellBuilder::new();
        let vertices = [
            [0.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [2.0, 2.0, 0.0],
            [0.0, 2.0, 0.0],
        ]
        .map(|p| builder.push_vertex(p));
        let before = builder.parts.clone();
        let bad = super::AnalyticVertexId::from_index(99);
        assert_eq!(
            builder.add_planar_face_with_openings(
                &vertices,
                &[&[vertices[0], vertices[1], bad]],
                RegionId(0)
            ),
            Err(BuildError::MissingVertex(bad))
        );
        assert_eq!(builder.parts, before);
        assert!(builder.edges.is_empty());
    }

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
        let shell = builder.build().expect("valid shared boundary");

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
            .dense(exedra_mesh::attr::FACE_REGION)
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
        let mut shell = builder.build().expect("valid shared boundary");

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
        let mut shell = builder.build().expect("valid shared boundary");

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
                .loops
                .get(1)
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
        let mut shell = builder.build().expect("valid shared boundary");

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
        let mut shell = builder.build().expect("valid shared boundary");

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
        let mut shell = builder.build().expect("valid shared boundary");

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
        let mut shell = builder.build().expect("valid shared boundary");

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
        let mut shell = builder.build().expect("valid shared boundary");

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

        let openings = shell.face(face).expect("face should still exist").loops[1..].to_vec();
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
    fn touching_opening_failure_preserves_its_identity() {
        let mut builder = AnalyticShellBuilder::new();
        let outer = [[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]]
            .map(|[x, y]| builder.push_vertex([x, y, 0.0]));
        let hole = [[2.0, 0.0], [2.0, 2.0], [3.0, 2.0], [3.0, 0.0]]
            .map(|[x, y]| builder.push_vertex([x, y, 0.0]));
        let face = builder
            .add_planar_face_with_openings(&outer, &[&hole], RegionId(0))
            .unwrap();
        let shell = builder.build().expect("valid shared boundary");
        let opening = shell.faces()[face.index()].loops[1];
        assert!(matches!(
            shell.to_exedra_mesh(&TessellateParams::default()),
            Err(TessellateError::OpeningBridgeFailed { face: f, opening: o })
                if f == face && o == opening
        ));
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
        let shell = builder.build().expect("valid shared boundary");

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
                .dense(exedra_mesh::attr::FACE_REGION)
                .and_then(|layer| layer.get(mesh_face.as_id()).copied());
            assert_eq!(region, Some(19));
        }
    }
}
