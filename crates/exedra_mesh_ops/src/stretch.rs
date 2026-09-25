// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Topology-preserving mesh realization of stretch.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use exedra_math::{Placement3, Plane3};
use exedra_mesh::{FaceId, Mesh, VertexId};

use crate::layers::{CornerSample, Transfer, VertexSample};

mod vertices;
pub use vertices::{VertexStretchError, VertexStretchStep, stretch_vertices};

/// Geometric policy for edges created by a stretched band or contraction seam.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct StretchPolicy {
    /// Mark a new seam sharp when the sine between its neighboring normals
    /// exceeds this threshold. Finite and in `[0, 1]`.
    pub sharp_sin_threshold: f64,
}
impl Default for StretchPolicy {
    fn default() -> Self {
        Self {
            sharp_sin_threshold: 0.1,
        }
    }
}

/// Geometric refusal from polygonal mesh stretch.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StretchError {
    /// Nonfinite or zero length, invalid plane, placement or policy.
    InvalidInput,
    /// Source topology is structurally invalid or has nonfinite positions.
    InvalidMesh,
    /// A transported plane, displacement or emitted position is not representable.
    NumericLimit,
    /// Contraction leaves no stationary and movable section to re-stitch.
    ContractionConsumesHalf,
    /// The placement cannot transport the authored plane.
    SingularTransform,
    /// A section touches a stored vertex, edge or coplanar face.
    AmbiguousContact,
    /// One source face contributes disconnected section segments.
    DisconnectedFaceSection,
    /// A crossing shell has an open boundary.
    OpenShell,
    /// Section segments do not form disjoint closed loops.
    NonManifoldSection,
    /// The emitted mesh violates topology invariants.
    BuildFailed,
    /// The two contraction sections cannot be re-stitched by translation.
    IncompatibleContraction,
}
impl core::fmt::Display for StretchError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::InvalidInput => "invalid mesh stretch inputs or policy",
            Self::InvalidMesh => "invalid stretch source mesh",
            Self::NumericLimit => "mesh stretch exceeds numeric limits",
            Self::ContractionConsumesHalf => "contraction does not leave two halves to re-stitch",
            Self::SingularTransform => "singular stretch placement",
            Self::AmbiguousContact => "stretch plane touches stored geometry",
            Self::DisconnectedFaceSection => "one face crosses the plane in disconnected segments",
            Self::OpenShell => "an open shell crosses the stretch plane",
            Self::NonManifoldSection => "stretch section does not form closed manifold loops",
            Self::BuildFailed => "stretched faces could not be rebuilt as a valid mesh",
            Self::IncompatibleContraction => "contraction sections are not translation-compatible",
        })
    }
}
impl core::error::Error for StretchError {}

/// Work and mapping evidence from mesh realization.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct StretchStats {
    /// Face/plane crossings; contraction may count a face on each section.
    pub split_faces: u64,
    /// New faces joining expansion rims.
    pub band_faces: u64,
    /// Crossing faces whose UVs could not be extended through the displacement.
    pub uv_unmapped_faces: u64,
    /// Caller-defined attribute values whose layer has no
    /// [`Propagation`](exedra_mesh::attributes::Propagation) rule, so the
    /// rebuild could not carry them.
    pub unpropagated_attribute_values: u64,
}

/// Geometric source of an output face.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StretchFaceSource {
    /// Surviving portion of a source face, possibly translated.
    Original(FaceId),
    /// New expansion band following the section of this source face.
    Band(FaceId),
}
impl StretchFaceSource {
    /// Source face whose attributes and authored meaning produced this face.
    #[must_use]
    pub const fn face(self) -> FaceId {
        match self {
            Self::Original(face) | Self::Band(face) => face,
        }
    }
}

/// Geometric source or generated role of an output vertex.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StretchVertexSource {
    /// Surviving source vertex, possibly translated.
    Original(VertexId),
    /// Generated section vertex. This role does not claim persistent identity
    /// or a unique original edge after contraction joins coincident sections.
    Seam {
        /// Zero for the stationary rim or contraction seam, one for the moved
        /// expansion rim.
        rim: u8,
    },
}

/// A stretched mesh and snapshot-scoped geometric correspondence.
#[derive(Clone, Debug)]
pub struct StretchResult {
    /// Realized mesh; input is never mutated.
    pub mesh: Mesh,
    /// Source for every output face.
    pub face_sources: BTreeMap<FaceId, StretchFaceSource>,
    /// Source or seam role for every output vertex.
    pub vertex_sources: BTreeMap<VertexId, StretchVertexSource>,
    /// Whether topology was rebuilt. False means IDs and all attributes survived
    /// a clone or rigid translation; true transfers the documented built-ins.
    pub topology_rebuilt: bool,
    /// Work and UV-extension evidence.
    pub stats: StretchStats,
}

/// Expands a closed mesh across a plane, or contracts a translation-compatible
/// slab, returning a new mesh and geometric source correspondence.
///
/// The plane and signed length are authored in the coordinate system mapped
/// into mesh coordinates by `placement`. Plane normals use inverse-transpose
/// transport; displacement uses the forward linear map. Negative length removes
/// the slab from the plane to `plane + abs(length)` along its local unit normal.
///
/// Contacts at stored vertices/edges are refused. A crossing mesh must be closed;
/// this does not certify freedom from self-intersections. Expansion joins section
/// rims with polygon bands. Contraction requires matching polygonal section edges.
/// Rebuilt topology preserves regions, seam/sharpness, corner UVs and normal
/// overrides. Existing UVs are extended when a planar mapping can be inferred,
/// otherwise stats report it. Caller-defined layers keep their registration and
/// [`Propagation`](exedra_mesh::attributes::Propagation) rule and are carried
/// along the same correspondence: each output face from its source face, and
/// each corner and generated vertex from the source face's corners or vertices
/// at its unmoved source position, weighted barycentrically (a source vertex
/// selects its own corner; a section point interpolates its source edge).
/// Noncrossing meshes keep all attributes and IDs. All failures leave input alone.
pub fn stretch_mesh(
    source: &Mesh,
    plane: &Plane3,
    length: f64,
    placement: &Placement3,
    policy: &StretchPolicy,
) -> Result<StretchResult, StretchError> {
    if !length.is_finite()
        || length == 0.0
        || !placement.is_finite()
        || !policy.sharp_sin_threshold.is_finite()
        || !(0.0..=1.0).contains(&policy.sharp_sin_threshold)
    {
        return Err(StretchError::InvalidInput);
    }
    if !source.validate_fast().is_empty()
        || source.vertices().any(|vertex| {
            source
                .vertex_position(vertex)
                .is_none_or(|p| p.iter().any(|v| !v.is_finite()))
        })
    {
        return Err(StretchError::InvalidMesh);
    }
    let geometry = WorldStretch::new(plane, length, placement)?;
    let (mut result, stats) = if length < 0.0 {
        stretch_mesh_contraction(source, &geometry, policy)?
    } else {
        stretch_mesh_expansion(source, &geometry, policy)?
    };
    result.stats = StretchStats {
        unpropagated_attribute_values: result.stats.unpropagated_attribute_values,
        ..stats
    };
    Ok(result)
}

fn preserved(mesh: Mesh) -> StretchResult {
    StretchResult {
        face_sources: mesh
            .faces()
            .map(|face| (face, StretchFaceSource::Original(face)))
            .collect(),
        vertex_sources: mesh
            .vertices()
            .map(|vertex| (vertex, StretchVertexSource::Original(vertex)))
            .collect(),
        mesh,
        topology_rebuilt: false,
        stats: StretchStats::default(),
    }
}

struct WorldStretch {
    normal: [f64; 3],
    distance: f64,
    far_distance: Option<f64>,
    displacement: [f64; 3],
}

impl WorldStretch {
    fn new(plane: &Plane3, length: f64, world: &Placement3) -> Result<Self, StretchError> {
        let (local_normal, local_distance) =
            plane.normalized().ok_or(StretchError::InvalidInput)?;
        let linear = [
            [world.rows[0][0], world.rows[0][1], world.rows[0][2]],
            [world.rows[1][0], world.rows[1][1], world.rows[1][2]],
            [world.rows[2][0], world.rows[2][1], world.rows[2][2]],
        ];
        let inverse = inverse3(linear).ok_or(StretchError::SingularTransform)?;
        let raw_normal = [
            inverse[0][0] * local_normal[0]
                + inverse[1][0] * local_normal[1]
                + inverse[2][0] * local_normal[2],
            inverse[0][1] * local_normal[0]
                + inverse[1][1] * local_normal[1]
                + inverse[2][1] * local_normal[2],
            inverse[0][2] * local_normal[0]
                + inverse[1][2] * local_normal[1]
                + inverse[2][2] * local_normal[2],
        ];
        let normal_length = exedra_math::norm(raw_normal);
        if !normal_length.is_finite() || normal_length == 0.0 {
            return Err(StretchError::SingularTransform);
        }
        let translation = [world.rows[0][3], world.rows[1][3], world.rows[2][3]];
        let raw_distance = local_distance
            + raw_normal[0] * translation[0]
            + raw_normal[1] * translation[1]
            + raw_normal[2] * translation[2];
        let normal = raw_normal.map(|component| component / normal_length);
        let distance = raw_distance / normal_length;
        let far_distance = (length < 0.0).then(|| {
            let removed = -length;
            let raw_far_distance = local_distance
                + removed
                + raw_normal[0] * translation[0]
                + raw_normal[1] * translation[1]
                + raw_normal[2] * translation[2];
            raw_far_distance / normal_length
        });
        let local_displacement = local_normal.map(|component| component * length);
        let displacement = [
            linear[0][0] * local_displacement[0]
                + linear[0][1] * local_displacement[1]
                + linear[0][2] * local_displacement[2],
            linear[1][0] * local_displacement[0]
                + linear[1][1] * local_displacement[1]
                + linear[1][2] * local_displacement[2],
            linear[2][0] * local_displacement[0]
                + linear[2][1] * local_displacement[1]
                + linear[2][2] * local_displacement[2],
        ];
        if !distance.is_finite()
            || far_distance.is_some_and(|d| !d.is_finite())
            || displacement.iter().any(|v| !v.is_finite())
        {
            return Err(StretchError::NumericLimit);
        }
        Ok(Self {
            normal,
            distance,
            far_distance,
            displacement,
        })
    }

    fn signed(&self, point: [f64; 3]) -> f64 {
        dot3(self.normal, point) - self.distance
    }

    fn far_signed(&self, point: [f64; 3]) -> f64 {
        dot3(self.normal, point)
            - self
                .far_distance
                .expect("far plane exists only for contraction")
    }
}

fn inverse3(matrix: [[f64; 3]; 3]) -> Option<[[f64; 3]; 3]> {
    let determinant = exedra_math::det3(matrix);
    if !determinant.is_finite() || determinant == 0.0 {
        return None;
    }
    let a = matrix;
    let inverse = [
        [
            (a[1][1] * a[2][2] - a[1][2] * a[2][1]) / determinant,
            (a[0][2] * a[2][1] - a[0][1] * a[2][2]) / determinant,
            (a[0][1] * a[1][2] - a[0][2] * a[1][1]) / determinant,
        ],
        [
            (a[1][2] * a[2][0] - a[1][0] * a[2][2]) / determinant,
            (a[0][0] * a[2][2] - a[0][2] * a[2][0]) / determinant,
            (a[0][2] * a[1][0] - a[0][0] * a[1][2]) / determinant,
        ],
        [
            (a[1][0] * a[2][1] - a[1][1] * a[2][0]) / determinant,
            (a[0][1] * a[2][0] - a[0][0] * a[2][1]) / determinant,
            (a[0][0] * a[1][1] - a[0][1] * a[1][0]) / determinant,
        ],
    ];
    inverse
        .iter()
        .flatten()
        .all(|value| value.is_finite())
        .then_some(inverse)
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct CutKey {
    plane: u8,
    a: u32,
    b: u32,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum ClipKey {
    Original(u32),
    Cut(CutKey),
}

#[derive(Copy, Clone)]
struct ClipVertex {
    key: ClipKey,
    point: [f64; 3],
    // Capture provenance while the face walk still has the arena's live
    // VertexId. Recovering that ID later from its stable numeric index would
    // require scanning the vertex arena for every emitted corner.
    source_vertex: Option<StretchVertexSource>,
    uv: Option<[f64; 2]>,
    normal_override: Option<[f32; 3]>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum OutputKey {
    Original { vertex: u32, moved: bool },
    Cut { cut: CutKey, rim: u8 },
    Seam { position: [u32; 3] },
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct FaceSource {
    source: StretchFaceSource,
    region: u32,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct PositionEdge {
    a: [u32; 3],
    b: [u32; 3],
}

impl PositionEdge {
    fn new(a: [u32; 3], b: [u32; 3]) -> Self {
        if a <= b {
            Self { a, b }
        } else {
            Self { a: b, b: a }
        }
    }
}

#[derive(Copy, Clone)]
struct SectionOwner {
    normal: [f64; 3],
}

struct ContractionFace {
    source: FaceSource,
    negative: Vec<ClipVertex>,
    far: Vec<ClipVertex>,
    uv_delta: Option<[f64; 2]>,
}

struct SectionSegment {
    a: CutKey,
    b: CutKey,
    source: FaceSource,
    face_normal: [f64; 3],
    a_uv: Option<[f64; 2]>,
    b_uv: Option<[f64; 2]>,
    a_normal_override: Option<[f32; 3]>,
    b_normal_override: Option<[f32; 3]>,
    uv_delta: Option<[f64; 2]>,
}

#[derive(Copy, Clone)]
struct OutputVertex {
    key: OutputKey,
    point: [f64; 3],
    /// The position on the source face before any displacement.
    source_point: [f64; 3],
    source: StretchVertexSource,
    uv: Option<[f64; 2]>,
    normal_override: Option<[f32; 3]>,
    input: Option<ClipKey>,
}

struct OutputMesh {
    builder: exedra_mesh::MeshBuilder,
    source_edges: BTreeMap<(u32, u32), (bool, f32)>,
    source_vertex_sharpness: BTreeMap<u32, f32>,
    vertices: BTreeMap<OutputKey, u32>,
    vertex_sources: Vec<StretchVertexSource>,
    vertex_sharpness: Vec<Option<f32>>,
    face_sources: Vec<StretchFaceSource>,
    face_uvs: Vec<Vec<Option<[f32; 2]>>>,
    face_normal_overrides: Vec<Vec<Option<[f32; 3]>>>,
    face_source_points: Vec<Vec<[f64; 3]>>,
    vertex_samples: Vec<Option<VertexSample>>,
}

impl OutputMesh {
    fn new(source: &Mesh) -> Self {
        let mut source_edges = BTreeMap::new();
        for face in source.faces() {
            for edge in source.face_loop(face) {
                let Some(from) = source.from_vertex(edge) else {
                    continue;
                };
                let Some(to) = source.to_vertex(edge) else {
                    continue;
                };
                let key = ordered_edge(from.index(), to.index());
                source_edges.entry(key).or_insert_with(|| {
                    (
                        source.edge_seam(edge).unwrap_or(false),
                        source.edge_sharpness(edge).unwrap_or(0.0),
                    )
                });
            }
        }
        let source_vertex_sharpness = source
            .vertices()
            .filter_map(|vertex| {
                source
                    .vertex_sharpness(vertex)
                    .map(|sharpness| (vertex.index(), sharpness))
            })
            .collect();
        Self {
            builder: exedra_mesh::MeshBuilder::new(),
            source_edges,
            source_vertex_sharpness,
            vertices: BTreeMap::new(),
            vertex_sources: Vec::new(),
            vertex_sharpness: Vec::new(),
            face_sources: Vec::new(),
            face_uvs: Vec::new(),
            face_normal_overrides: Vec::new(),
            face_source_points: Vec::new(),
            vertex_samples: Vec::new(),
        }
    }

    fn vertex(
        &mut self,
        key: OutputKey,
        point: [f64; 3],
        source: StretchVertexSource,
    ) -> Result<u32, StretchError> {
        if let Some(index) = self.vertices.get(&key) {
            return Ok(*index);
        }
        #[expect(
            clippy::cast_possible_truncation,
            reason = "stretch crosses the documented f64-to-f32 mesh emission boundary"
        )]
        let narrowed = [point[0] as f32, point[1] as f32, point[2] as f32];
        if narrowed.iter().any(|component| !component.is_finite()) {
            return Err(StretchError::BuildFailed);
        }
        let index = self.builder.push_vertex(narrowed);
        self.vertices.insert(key, index);
        self.vertex_sources.push(source);
        self.vertex_samples.push(None);
        let sharpness = match key {
            OutputKey::Original { vertex, .. } => {
                self.source_vertex_sharpness.get(&vertex).copied()
            }
            OutputKey::Cut { .. } | OutputKey::Seam { .. } => None,
        };
        self.vertex_sharpness.push(sharpness);
        Ok(index)
    }

    fn add_polygon(
        &mut self,
        vertices: &[OutputVertex],
        source: FaceSource,
        sharpness: Option<&[f32]>,
    ) -> Result<(), StretchError> {
        let corners = vertices
            .iter()
            .map(|vertex| self.vertex(vertex.key, vertex.point, vertex.source))
            .collect::<Result<Vec<_>, _>>()?;
        let mut seams = Vec::with_capacity(vertices.len());
        let mut inherited_sharpness = Vec::with_capacity(vertices.len());
        for index in 0..vertices.len() {
            let next = (index + 1) % vertices.len();
            let attrs = input_edge(vertices[index].input, vertices[next].input)
                .and_then(|edge| self.source_edges.get(&edge).copied())
                .unwrap_or((false, 0.0));
            seams.push(attrs.0);
            inherited_sharpness.push(attrs.1);
        }
        if let Some(overrides) = sharpness {
            for (inherited, authored) in inherited_sharpness.iter_mut().zip(overrides) {
                *inherited = inherited.max(*authored);
            }
        }
        self.builder
            .add_face_with_attrs(
                &corners,
                &exedra_mesh::FaceBuildAttrs {
                    region: Some(source.region),
                    edge_seams: Some(&seams),
                    edge_sharpness: Some(&inherited_sharpness),
                },
            )
            .map_err(|_| StretchError::BuildFailed)?;
        self.face_sources.push(source.source);
        #[expect(
            clippy::cast_possible_truncation,
            reason = "corner UVs cross the documented f64-to-f32 mesh emission boundary"
        )]
        self.face_uvs.push(
            vertices
                .iter()
                .map(|vertex| vertex.uv.map(|uv| [uv[0] as f32, uv[1] as f32]))
                .collect(),
        );
        self.face_normal_overrides.push(
            vertices
                .iter()
                .map(|vertex| vertex.normal_override)
                .collect(),
        );
        self.face_source_points
            .push(vertices.iter().map(|vertex| vertex.source_point).collect());
        // A vertex shared by several faces takes its first face's sample.
        for (&index, vertex) in corners.iter().zip(vertices) {
            let slot = &mut self.vertex_samples[index as usize];
            if slot.is_none() {
                *slot = Some(match vertex.source {
                    StretchVertexSource::Original(original) => VertexSample::Vertex(original),
                    StretchVertexSource::Seam { .. } => VertexSample::Point {
                        face: source.source.face(),
                        point: vertex.source_point,
                    },
                });
            }
        }
        Ok(())
    }

    fn finish(self, source: &Mesh) -> Result<StretchResult, StretchError> {
        let mut built = self
            .builder
            .build()
            .map_err(|_| StretchError::BuildFailed)?;
        if self.vertex_sharpness.iter().any(Option::is_some)
            || self.face_uvs.iter().flatten().any(Option::is_some)
            || self
                .face_normal_overrides
                .iter()
                .flatten()
                .any(Option::is_some)
        {
            let mut edit = built.mesh.edit();
            for (vertex, sharpness) in built.vertex_ids.iter().zip(&self.vertex_sharpness) {
                if let Some(sharpness) = sharpness {
                    exedra_mesh::op::set_vertex_sharpness(&mut edit, *vertex, *sharpness)
                        .map_err(|_| StretchError::BuildFailed)?;
                }
            }
            for ((edges, uvs), normals) in built
                .face_edge_ids
                .iter()
                .zip(&self.face_uvs)
                .zip(&self.face_normal_overrides)
            {
                for ((edge, uv), normal) in vertex_corners(edges).zip(uvs).zip(normals) {
                    if let Some(uv) = uv {
                        exedra_mesh::op::set_corner_uv(&mut edit, *edge, *uv)
                            .map_err(|_| StretchError::BuildFailed)?;
                    }
                    if let Some(normal) = normal {
                        exedra_mesh::op::set_corner_normal_override(
                            &mut edit,
                            *edge,
                            Some(*normal),
                        )
                        .map_err(|_| StretchError::BuildFailed)?;
                    }
                }
            }
            #[expect(unused_must_use, reason = "discard sink output")]
            {
                edit.finish();
            }
        }
        let mut unpropagated_attribute_values = 0;
        if let Some(mut transfer) = Transfer::new(source) {
            for (((&face, edges), points), face_source) in built
                .face_ids
                .iter()
                .zip(&built.face_edge_ids)
                .zip(&self.face_source_points)
                .zip(&self.face_sources)
            {
                transfer.face(
                    face,
                    face_source.face(),
                    vertex_corners(edges)
                        .copied()
                        .zip(points.iter().map(|&point| CornerSample::Point(point)))
                        .collect(),
                );
            }
            for (&vertex, sample) in built.vertex_ids.iter().zip(&self.vertex_samples) {
                if let Some(sample) = sample {
                    transfer.vertex(vertex, *sample);
                }
            }
            unpropagated_attribute_values = transfer
                .apply(&mut built.mesh)
                .map_err(|_| StretchError::BuildFailed)?;
        }
        let validation = built.mesh.validate_deep();
        if !validation.is_empty() {
            return Err(StretchError::BuildFailed);
        }
        Ok(StretchResult {
            face_sources: built.face_ids.into_iter().zip(self.face_sources).collect(),
            vertex_sources: built
                .vertex_ids
                .into_iter()
                .zip(self.vertex_sources)
                .collect(),
            mesh: built.mesh,
            topology_rebuilt: true,
            stats: StretchStats {
                unpropagated_attribute_values,
                ..StretchStats::default()
            },
        })
    }
}

/// The corners of a built face in polygon vertex order. Builder half-edge `k`
/// runs from vertex `k` to vertex `k + 1`, and a corner belongs to its
/// half-edge's destination, so vertex `k`'s corner is half-edge `k - 1`.
fn vertex_corners(
    edges: &[exedra_mesh::HalfEdgeId],
) -> impl Iterator<Item = &exedra_mesh::HalfEdgeId> {
    edges
        .iter()
        .cycle()
        .skip(edges.len().saturating_sub(1))
        .take(edges.len())
}

fn ordered_edge(a: u32, b: u32) -> (u32, u32) {
    (a.min(b), a.max(b))
}

fn input_edge(a: Option<ClipKey>, b: Option<ClipKey>) -> Option<(u32, u32)> {
    match (a?, b?) {
        (ClipKey::Original(a), ClipKey::Original(b)) => Some(ordered_edge(a, b)),
        (ClipKey::Original(vertex), ClipKey::Cut(cut))
        | (ClipKey::Cut(cut), ClipKey::Original(vertex))
            if vertex == cut.a || vertex == cut.b =>
        {
            Some((cut.a, cut.b))
        }
        _ => None,
    }
}

fn stretch_mesh_expansion(
    source: &Mesh,
    stretch: &WorldStretch,
    policy: &StretchPolicy,
) -> Result<(StretchResult, StretchStats), StretchError> {
    let mesh = source;
    let mut points = BTreeMap::new();
    let mut signed = BTreeMap::new();
    let mut min_signed = f64::INFINITY;
    let mut max_signed = f64::NEG_INFINITY;
    for vertex in mesh.vertices() {
        let position = mesh
            .vertex_position(vertex)
            .expect("live vertices have required positions");
        let point = position.map(f64::from);
        let side = stretch.signed(point);
        if side == 0.0 {
            return Err(StretchError::AmbiguousContact);
        }
        points.insert(vertex.index(), point);
        signed.insert(vertex.index(), side);
        min_signed = min_signed.min(side);
        max_signed = max_signed.max(side);
    }
    if max_signed < 0.0 {
        return Ok((preserved(mesh.clone()), StretchStats::default()));
    }
    if min_signed > 0.0 {
        return Ok((
            translate_body(source, stretch.displacement)?,
            StretchStats::default(),
        ));
    }
    if has_open_boundary(mesh) {
        return Err(StretchError::OpenShell);
    }

    let regions = mesh
        .attrs()
        .dense(exedra_mesh::attr::FACE_REGION)
        .expect("FACE_REGION is a required built-in layer");
    let uv_layer = mesh.attrs().sparse(exedra_mesh::attr::CORNER_UV);
    let normal_layer = mesh
        .attrs()
        .sparse(exedra_mesh::attr::CORNER_NORMAL_OVERRIDE);
    let has_uvs = uv_layer.is_some();
    let mut output = OutputMesh::new(mesh);
    let mut sections = Vec::new();
    let mut stats = StretchStats::default();
    for face in mesh.faces() {
        let source_face = FaceSource {
            source: StretchFaceSource::Original(face),
            region: regions.get(face.into()).copied().unwrap_or(0),
        };
        let polygon = mesh
            .face_loop(face)
            .map(|corner| {
                let vertex = mesh.to_vertex(corner).expect("face corner has a vertex");
                ClipVertex {
                    key: ClipKey::Original(vertex.index()),
                    point: points[&vertex.index()],
                    source_vertex: Some(StretchVertexSource::Original(vertex)),
                    uv: uv_layer
                        .and_then(|layer| layer.get(corner.into()).copied())
                        .map(|uv| uv.map(f64::from)),
                    normal_override: normal_layer
                        .and_then(|layer| layer.get(corner.into()).copied()),
                }
            })
            .collect::<Vec<_>>();
        let sides = polygon
            .iter()
            .map(|vertex| original_index(*vertex).map(|index| signed[&index]))
            .collect::<Result<Vec<_>, StretchError>>()?;
        let has_negative = sides.iter().any(|side| *side < 0.0);
        let has_positive = sides.iter().any(|side| *side > 0.0);
        let face_normal = polygon_normal(&polygon).ok_or(StretchError::BuildFailed)?;
        let uv_delta = planar_uv_delta(&polygon, face_normal, stretch.displacement);
        if has_negative && has_positive {
            stats.split_faces += 1;
            let cuts = polygon_cut_keys(&polygon, &sides, 0)?;
            sections.push(SectionSegment {
                a: cuts[0],
                b: cuts[1],
                source: source_face,
                face_normal,
                a_uv: cut_uv(cuts[0], &polygon, &sides),
                b_uv: cut_uv(cuts[1], &polygon, &sides),
                a_normal_override: cut_normal_override(cuts[0], &polygon, &sides),
                b_normal_override: cut_normal_override(cuts[1], &polygon, &sides),
                uv_delta,
            });
            if has_uvs
                && (uv_delta.is_none()
                    || cut_uv(cuts[0], &polygon, &sides).is_none()
                    || cut_uv(cuts[1], &polygon, &sides).is_none())
            {
                stats.uv_unmapped_faces += 1;
            }
        }
        let negative = clip_polygon(&polygon, &sides, false, 0)?;
        if negative.len() >= 3 {
            let vertices = negative
                .iter()
                .map(|vertex| output_vertex(*vertex, false, 0, [0.0; 3], None))
                .collect::<Vec<_>>();
            output.add_polygon(&vertices, source_face, None)?;
        }
        let positive = clip_polygon(&polygon, &sides, true, 0)?;
        if positive.len() >= 3 {
            let vertices = positive
                .iter()
                .map(|vertex| output_vertex(*vertex, true, 1, stretch.displacement, uv_delta))
                .collect::<Vec<_>>();
            output.add_polygon(&vertices, source_face, None)?;
        }
    }
    validate_section(&sections)?;
    for section in &sections {
        let mut a = cut_point(section.a, &points, &signed);
        let mut b = cut_point(section.b, &points, &signed);
        let mut a_key = section.a;
        let mut b_key = section.b;
        let mut a_uv = section.a_uv;
        let mut b_uv = section.b_uv;
        let mut a_normal_override = section.a_normal_override;
        let mut b_normal_override = section.b_normal_override;
        let projected_normal = cross3(stretch.displacement, sub3(b, a));
        if dot3(projected_normal, section.face_normal) < 0.0 {
            core::mem::swap(&mut a, &mut b);
            core::mem::swap(&mut a_key, &mut b_key);
            core::mem::swap(&mut a_uv, &mut b_uv);
            core::mem::swap(&mut a_normal_override, &mut b_normal_override);
        }
        let moved_a = add3(a, stretch.displacement);
        let moved_b = add3(b, stretch.displacement);
        let band_normal = normalize3(cross3(stretch.displacement, sub3(b, a)))
            .ok_or(StretchError::AmbiguousContact)?;
        let sharp = if exedra_math::norm(cross3(section.face_normal, band_normal))
            > policy.sharp_sin_threshold
        {
            1.0
        } else {
            0.0
        };
        let sharpness = [0.0, sharp, 0.0, sharp];
        let a_band_normal = (sharp == 0.0).then_some(a_normal_override).flatten();
        let b_band_normal = (sharp == 0.0).then_some(b_normal_override).flatten();
        let seam0 = StretchVertexSource::Seam { rim: 0 };
        let seam1 = StretchVertexSource::Seam { rim: 1 };
        let mapped_a_uv = section.uv_delta.and_then(|delta| add_uv(a_uv, Some(delta)));
        let mapped_b_uv = section.uv_delta.and_then(|delta| add_uv(b_uv, Some(delta)));
        output.add_polygon(
            &[
                OutputVertex {
                    key: OutputKey::Cut { cut: a_key, rim: 0 },
                    point: a,
                    source_point: a,
                    source: seam0,
                    uv: a_uv,
                    normal_override: a_band_normal,
                    input: Some(ClipKey::Cut(a_key)),
                },
                OutputVertex {
                    key: OutputKey::Cut { cut: a_key, rim: 1 },
                    point: moved_a,
                    source_point: a,
                    source: seam1,
                    uv: mapped_a_uv,
                    normal_override: a_band_normal,
                    input: Some(ClipKey::Cut(a_key)),
                },
                OutputVertex {
                    key: OutputKey::Cut { cut: b_key, rim: 1 },
                    point: moved_b,
                    source_point: b,
                    source: seam1,
                    uv: mapped_b_uv,
                    normal_override: b_band_normal,
                    input: Some(ClipKey::Cut(b_key)),
                },
                OutputVertex {
                    key: OutputKey::Cut { cut: b_key, rim: 0 },
                    point: b,
                    source_point: b,
                    source: seam0,
                    uv: b_uv,
                    normal_override: b_band_normal,
                    input: Some(ClipKey::Cut(b_key)),
                },
            ],
            FaceSource {
                source: StretchFaceSource::Band(section.source.source.face()),
                ..section.source
            },
            Some(&sharpness),
        )?;
        stats.band_faces += 1;
    }
    Ok((output.finish(source)?, stats))
}

fn stretch_mesh_contraction(
    source: &Mesh,
    stretch: &WorldStretch,
    policy: &StretchPolicy,
) -> Result<(StretchResult, StretchStats), StretchError> {
    let mesh = source;
    let mut points = BTreeMap::new();
    let mut signed_near = BTreeMap::new();
    let mut signed_far = BTreeMap::new();
    let mut min_near = f64::INFINITY;
    let mut max_near = f64::NEG_INFINITY;
    let mut min_far = f64::INFINITY;
    let mut max_far = f64::NEG_INFINITY;
    for vertex in mesh.vertices() {
        let position = mesh
            .vertex_position(vertex)
            .expect("live vertices have required positions");
        let point = position.map(f64::from);
        let near = stretch.signed(point);
        let far = stretch.far_signed(point);
        if near == 0.0 || far == 0.0 {
            return Err(StretchError::AmbiguousContact);
        }
        points.insert(vertex.index(), point);
        signed_near.insert(vertex.index(), near);
        signed_far.insert(vertex.index(), far);
        min_near = min_near.min(near);
        max_near = max_near.max(near);
        min_far = min_far.min(far);
        max_far = max_far.max(far);
    }
    if max_near < 0.0 {
        return Ok((preserved(mesh.clone()), StretchStats::default()));
    }
    if min_far > 0.0 {
        return Ok((
            translate_body(source, stretch.displacement)?,
            StretchStats::default(),
        ));
    }
    if min_near >= 0.0 || max_far <= 0.0 {
        return Err(StretchError::ContractionConsumesHalf);
    }
    if has_open_boundary(mesh) {
        return Err(StretchError::OpenShell);
    }

    let regions = mesh
        .attrs()
        .dense(exedra_mesh::attr::FACE_REGION)
        .expect("FACE_REGION is a required built-in layer");
    let uv_layer = mesh.attrs().sparse(exedra_mesh::attr::CORNER_UV);
    let normal_layer = mesh
        .attrs()
        .sparse(exedra_mesh::attr::CORNER_NORMAL_OVERRIDE);
    let has_uvs = uv_layer.is_some();
    let mut faces = Vec::new();
    let mut near_sections = BTreeMap::<PositionEdge, SectionOwner>::new();
    let mut far_sections = BTreeMap::<PositionEdge, SectionOwner>::new();
    let mut stats = StretchStats::default();
    for face in mesh.faces() {
        let source_face = FaceSource {
            source: StretchFaceSource::Original(face),
            region: regions.get(face.into()).copied().unwrap_or(0),
        };
        let polygon = mesh
            .face_loop(face)
            .map(|corner| {
                let vertex = mesh.to_vertex(corner).expect("face corner has a vertex");
                ClipVertex {
                    key: ClipKey::Original(vertex.index()),
                    point: points[&vertex.index()],
                    source_vertex: Some(StretchVertexSource::Original(vertex)),
                    uv: uv_layer
                        .and_then(|layer| layer.get(corner.into()).copied())
                        .map(|uv| uv.map(f64::from)),
                    normal_override: normal_layer
                        .and_then(|layer| layer.get(corner.into()).copied()),
                }
            })
            .collect::<Vec<_>>();
        let normal = polygon_normal(&polygon).ok_or(StretchError::BuildFailed)?;
        let uv_delta = planar_uv_delta(&polygon, normal, stretch.displacement);
        let near_sides = polygon
            .iter()
            .map(|vertex| original_index(*vertex).map(|index| signed_near[&index]))
            .collect::<Result<Vec<_>, StretchError>>()?;
        let far_sides = polygon
            .iter()
            .map(|vertex| original_index(*vertex).map(|index| signed_far[&index]))
            .collect::<Result<Vec<_>, StretchError>>()?;
        let crosses_near =
            near_sides.iter().any(|side| *side < 0.0) && near_sides.iter().any(|side| *side > 0.0);
        let crosses_far =
            far_sides.iter().any(|side| *side < 0.0) && far_sides.iter().any(|side| *side > 0.0);
        if crosses_near {
            let cuts = polygon_cut_keys(&polygon, &near_sides, 0)?;
            let edge = section_position_edge(cuts, &points, &signed_near, [0.0; 3]);
            if near_sections
                .insert(edge, SectionOwner { normal })
                .is_some()
            {
                return Err(StretchError::NonManifoldSection);
            }
            stats.split_faces += 1;
            if has_uvs && uv_delta.is_none() {
                stats.uv_unmapped_faces += 1;
            }
        }
        if crosses_far {
            let cuts = polygon_cut_keys(&polygon, &far_sides, 1)?;
            let edge = section_position_edge(cuts, &points, &signed_far, stretch.displacement);
            if far_sections.insert(edge, SectionOwner { normal }).is_some() {
                return Err(StretchError::NonManifoldSection);
            }
            stats.split_faces += 1;
            if has_uvs && uv_delta.is_none() {
                stats.uv_unmapped_faces += 1;
            }
        }
        faces.push(ContractionFace {
            source: source_face,
            negative: clip_polygon(&polygon, &near_sides, false, 0)?,
            far: clip_polygon(&polygon, &far_sides, true, 1)?,
            uv_delta,
        });
    }
    if near_sections.is_empty() || near_sections.len() != far_sections.len() {
        return Err(StretchError::IncompatibleContraction);
    }
    for edge in near_sections.keys() {
        if !far_sections.contains_key(edge) {
            return Err(StretchError::IncompatibleContraction);
        }
    }

    let mut output = OutputMesh::new(mesh);
    for face in faces {
        if face.negative.len() >= 3 {
            let sharpness = contraction_sharpness(
                &face.negative,
                0,
                [0.0; 3],
                &points,
                &signed_near,
                &near_sections,
                &far_sections,
                policy.sharp_sin_threshold,
            );
            let vertices = face
                .negative
                .iter()
                .map(|vertex| contraction_output_vertex(*vertex, false, [0.0; 3], None))
                .collect::<Vec<_>>();
            output.add_polygon(&vertices, face.source, Some(&sharpness))?;
        }
        if face.far.len() >= 3 {
            let sharpness = contraction_sharpness(
                &face.far,
                1,
                stretch.displacement,
                &points,
                &signed_far,
                &near_sections,
                &far_sections,
                policy.sharp_sin_threshold,
            );
            let vertices = face
                .far
                .iter()
                .map(|vertex| {
                    contraction_output_vertex(*vertex, true, stretch.displacement, face.uv_delta)
                })
                .collect::<Vec<_>>();
            output.add_polygon(&vertices, face.source, Some(&sharpness))?;
        }
    }
    Ok((output.finish(source)?, stats))
}

fn original_index(vertex: ClipVertex) -> Result<u32, StretchError> {
    match vertex.key {
        ClipKey::Original(index) => Ok(index),
        ClipKey::Cut(_) => Err(StretchError::AmbiguousContact),
    }
}

fn polygon_cut_keys(
    polygon: &[ClipVertex],
    sides: &[f64],
    plane: u8,
) -> Result<[CutKey; 2], StretchError> {
    let mut cuts = Vec::new();
    for current in 0..polygon.len() {
        let next = (current + 1) % polygon.len();
        if sides[current].signum() != sides[next].signum() {
            cuts.push(cut_key(polygon[current], polygon[next], plane)?);
        }
    }
    if cuts.len() > 2 {
        return Err(StretchError::DisconnectedFaceSection);
    }
    if cuts.len() != 2 || cuts[0] == cuts[1] {
        return Err(StretchError::AmbiguousContact);
    }
    Ok([cuts[0], cuts[1]])
}

fn clip_polygon(
    polygon: &[ClipVertex],
    sides: &[f64],
    keep_positive: bool,
    plane: u8,
) -> Result<Vec<ClipVertex>, StretchError> {
    let inside = |side: f64| {
        if keep_positive {
            side > 0.0
        } else {
            side < 0.0
        }
    };
    let mut output = Vec::with_capacity(polygon.len() + 2);
    for current in 0..polygon.len() {
        let previous = (current + polygon.len() - 1) % polygon.len();
        let previous_inside = inside(sides[previous]);
        let current_inside = inside(sides[current]);
        if previous_inside != current_inside {
            output.push(ClipVertex {
                key: ClipKey::Cut(cut_key(polygon[previous], polygon[current], plane)?),
                point: canonical_intersection(
                    polygon[previous],
                    sides[previous],
                    polygon[current],
                    sides[current],
                )?,
                source_vertex: None,
                uv: canonical_intersection_uv(
                    polygon[previous],
                    sides[previous],
                    polygon[current],
                    sides[current],
                ),
                normal_override: canonical_intersection_normal_override(
                    polygon[previous],
                    sides[previous],
                    polygon[current],
                    sides[current],
                ),
            });
        }
        if current_inside {
            output.push(polygon[current]);
        }
    }
    Ok(output)
}

fn section_position_edge(
    cuts: [CutKey; 2],
    points: &BTreeMap<u32, [f64; 3]>,
    signed: &BTreeMap<u32, f64>,
    displacement: [f64; 3],
) -> PositionEdge {
    PositionEdge::new(
        position_bits(add3(cut_point(cuts[0], points, signed), displacement)),
        position_bits(add3(cut_point(cuts[1], points, signed), displacement)),
    )
}

fn contraction_output_vertex(
    vertex: ClipVertex,
    moved: bool,
    displacement: [f64; 3],
    uv_delta: Option<[f64; 2]>,
) -> OutputVertex {
    let point = add3(vertex.point, displacement);
    let uv = add_uv(vertex.uv, uv_delta);
    match vertex.key {
        ClipKey::Original(index) => OutputVertex {
            key: OutputKey::Original {
                vertex: index,
                moved,
            },
            point,
            source_point: vertex.point,
            source: vertex
                .source_vertex
                .expect("original clip vertices retain source provenance"),
            uv,
            normal_override: vertex.normal_override,
            input: Some(vertex.key),
        },
        ClipKey::Cut(_) => OutputVertex {
            key: OutputKey::Seam {
                position: position_bits(point),
            },
            point,
            source_point: vertex.point,
            source: StretchVertexSource::Seam { rim: 0 },
            uv,
            normal_override: vertex.normal_override,
            input: Some(vertex.key),
        },
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the seam policy needs both section maps and the active face geometry"
)]
fn contraction_sharpness(
    polygon: &[ClipVertex],
    plane: u8,
    displacement: [f64; 3],
    points: &BTreeMap<u32, [f64; 3]>,
    signed: &BTreeMap<u32, f64>,
    near_sections: &BTreeMap<PositionEdge, SectionOwner>,
    far_sections: &BTreeMap<PositionEdge, SectionOwner>,
    threshold: f64,
) -> Vec<f32> {
    let mut values = alloc::vec![0.0; polygon.len()];
    for current in 0..polygon.len() {
        let next = (current + 1) % polygon.len();
        let (ClipKey::Cut(a), ClipKey::Cut(b)) = (polygon[current].key, polygon[next].key) else {
            continue;
        };
        if a.plane != plane || b.plane != plane {
            continue;
        }
        let edge = PositionEdge::new(
            position_bits(add3(cut_point(a, points, signed), displacement)),
            position_bits(add3(cut_point(b, points, signed), displacement)),
        );
        if let (Some(near), Some(far)) = (near_sections.get(&edge), far_sections.get(&edge)) {
            let sin = exedra_math::norm(cross3(near.normal, far.normal));
            values[current] = if sin > threshold { 1.0 } else { 0.0 };
        }
    }
    values
}

fn position_bits(point: [f64; 3]) -> [u32; 3] {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "compatibility is defined at the documented f32 mesh boundary"
    )]
    let narrowed = [point[0] as f32, point[1] as f32, point[2] as f32];
    narrowed.map(f32::to_bits)
}

fn polygon_normal(polygon: &[ClipVertex]) -> Option<[f64; 3]> {
    let mut normal = [0.0; 3];
    for index in 0..polygon.len() {
        let current = polygon[index].point;
        let next = polygon[(index + 1) % polygon.len()].point;
        normal[0] += (current[1] - next[1]) * (current[2] + next[2]);
        normal[1] += (current[2] - next[2]) * (current[0] + next[0]);
        normal[2] += (current[0] - next[0]) * (current[1] + next[1]);
    }
    normalize3(normal)
}

fn cut_uv(cut: CutKey, polygon: &[ClipVertex], sides: &[f64]) -> Option<[f64; 2]> {
    let a = polygon
        .iter()
        .position(|vertex| vertex.key == ClipKey::Original(cut.a))?;
    let b = polygon
        .iter()
        .position(|vertex| vertex.key == ClipKey::Original(cut.b))?;
    canonical_intersection_uv(polygon[a], sides[a], polygon[b], sides[b])
}

fn cut_normal_override(cut: CutKey, polygon: &[ClipVertex], sides: &[f64]) -> Option<[f32; 3]> {
    let a = polygon
        .iter()
        .position(|vertex| vertex.key == ClipKey::Original(cut.a))?;
    let b = polygon
        .iter()
        .position(|vertex| vertex.key == ClipKey::Original(cut.b))?;
    canonical_intersection_normal_override(polygon[a], sides[a], polygon[b], sides[b])
}

/// Returns the affine UV displacement induced by a tangent translation of a
/// planar face. This is a local extension policy, not an unwrap: it requires
/// complete corner UVs and leaves non-tangent or underdetermined faces
/// unmapped so the evaluator can report them honestly.
fn planar_uv_delta(
    polygon: &[ClipVertex],
    normal: [f64; 3],
    displacement: [f64; 3],
) -> Option<[f64; 2]> {
    let displacement_length = exedra_math::norm(displacement);
    if dot3(normal, displacement).abs() > displacement_length * 1.0e-10
        || polygon.iter().any(|vertex| vertex.uv.is_none())
    {
        return None;
    }
    for base in 0..polygon.len() {
        for first in 0..polygon.len() {
            if first == base {
                continue;
            }
            for second in first + 1..polygon.len() {
                if second == base {
                    continue;
                }
                let e1 = sub3(polygon[first].point, polygon[base].point);
                let e2 = sub3(polygon[second].point, polygon[base].point);
                let g11 = dot3(e1, e1);
                let g12 = dot3(e1, e2);
                let g22 = dot3(e2, e2);
                let determinant = g11 * g22 - g12 * g12;
                if determinant == 0.0 || !determinant.is_finite() {
                    continue;
                }
                let rhs1 = dot3(displacement, e1);
                let rhs2 = dot3(displacement, e2);
                let first_weight = (rhs1 * g22 - rhs2 * g12) / determinant;
                let second_weight = (rhs2 * g11 - rhs1 * g12) / determinant;
                let base_uv = polygon[base].uv?;
                let first_uv = polygon[first].uv?;
                let second_uv = polygon[second].uv?;
                let delta = [
                    first_weight * (first_uv[0] - base_uv[0])
                        + second_weight * (second_uv[0] - base_uv[0]),
                    first_weight * (first_uv[1] - base_uv[1])
                        + second_weight * (second_uv[1] - base_uv[1]),
                ];
                if delta.iter().all(|value| value.is_finite()) {
                    return Some(delta);
                }
            }
        }
    }
    None
}

fn has_open_boundary(mesh: &Mesh) -> bool {
    mesh.faces().any(|face| {
        mesh.face_loop(face).any(|edge| {
            mesh.twin(edge)
                .and_then(|twin| mesh.face(twin))
                .is_some_and(|face| face == FaceId::OUTSIDE)
        })
    })
}

fn translate_body(source: &Mesh, displacement: [f64; 3]) -> Result<StretchResult, StretchError> {
    let mut mesh = source.clone();
    let vertices = mesh.vertices().collect::<Vec<_>>();
    {
        let mut edit = mesh.edit();
        for vertex in vertices {
            let position = edit
                .mesh()
                .vertex_position(vertex)
                .expect("live vertex has a position")
                .map(f64::from);
            let moved = add3(position, displacement);
            #[expect(
                clippy::cast_possible_truncation,
                reason = "stretch crosses the documented f64-to-f32 mesh emission boundary"
            )]
            let narrowed = [moved[0] as f32, moved[1] as f32, moved[2] as f32];
            if narrowed.iter().any(|v| !v.is_finite()) {
                return Err(StretchError::NumericLimit);
            }
            exedra_mesh::op::set_vertex_position(&mut edit, vertex, narrowed)
                .map_err(|_| StretchError::BuildFailed)?;
        }
        #[expect(unused_must_use, reason = "discard sink output")]
        {
            edit.finish();
        }
    }
    Ok(preserved(mesh))
}

fn cut_key(a: ClipVertex, b: ClipVertex, plane: u8) -> Result<CutKey, StretchError> {
    let (ClipKey::Original(a), ClipKey::Original(b)) = (a.key, b.key) else {
        return Err(StretchError::AmbiguousContact);
    };
    Ok(CutKey {
        plane,
        a: a.min(b),
        b: a.max(b),
    })
}

fn canonical_intersection(
    a: ClipVertex,
    side_a: f64,
    b: ClipVertex,
    side_b: f64,
) -> Result<[f64; 3], StretchError> {
    let (ClipKey::Original(a_index), ClipKey::Original(b_index)) = (a.key, b.key) else {
        return Err(StretchError::AmbiguousContact);
    };
    exedra_math::intersect_plane_edge((a_index, a.point, side_a), (b_index, b.point, side_b))
        .map(|(point, _)| point)
        .ok_or(StretchError::AmbiguousContact)
}

fn cut_point(
    key: CutKey,
    points: &BTreeMap<u32, [f64; 3]>,
    signed: &BTreeMap<u32, f64>,
) -> [f64; 3] {
    canonical_intersection(
        ClipVertex {
            key: ClipKey::Original(key.a),
            point: points[&key.a],
            source_vertex: None,
            uv: None,
            normal_override: None,
        },
        signed[&key.a],
        ClipVertex {
            key: ClipKey::Original(key.b),
            point: points[&key.b],
            source_vertex: None,
            uv: None,
            normal_override: None,
        },
        signed[&key.b],
    )
    .expect("section keys always straddle their plane")
}

fn output_vertex(
    vertex: ClipVertex,
    moved: bool,
    rim: u8,
    displacement: [f64; 3],
    uv_delta: Option<[f64; 2]>,
) -> OutputVertex {
    let point = if moved {
        add3(vertex.point, displacement)
    } else {
        vertex.point
    };
    let uv = add_uv(vertex.uv, uv_delta);
    match vertex.key {
        ClipKey::Original(index) => OutputVertex {
            key: OutputKey::Original {
                vertex: index,
                moved,
            },
            point,
            source_point: vertex.point,
            source: vertex
                .source_vertex
                .expect("original clip vertices retain source provenance"),
            uv,
            normal_override: vertex.normal_override,
            input: Some(vertex.key),
        },
        ClipKey::Cut(cut) => OutputVertex {
            key: OutputKey::Cut { cut, rim },
            point,
            source_point: vertex.point,
            source: StretchVertexSource::Seam { rim },
            uv,
            normal_override: vertex.normal_override,
            input: Some(vertex.key),
        },
    }
}

fn add_uv(uv: Option<[f64; 2]>, delta: Option<[f64; 2]>) -> Option<[f64; 2]> {
    match (uv, delta) {
        (Some(uv), Some(delta)) => Some([uv[0] + delta[0], uv[1] + delta[1]]),
        (uv, None) => uv,
        (None, Some(_)) => None,
    }
}

fn canonical_intersection_uv(
    a: ClipVertex,
    side_a: f64,
    b: ClipVertex,
    side_b: f64,
) -> Option<[f64; 2]> {
    let (a_uv, b_uv) = (a.uv?, b.uv?);
    let (ClipKey::Original(a_index), ClipKey::Original(b_index)) = (a.key, b.key) else {
        return None;
    };
    let (low_uv, low_side, high_uv, high_side) = if a_index < b_index {
        (a_uv, side_a, b_uv, side_b)
    } else {
        (b_uv, side_b, a_uv, side_a)
    };
    let denominator = low_side - high_side;
    if denominator == 0.0 || !denominator.is_finite() {
        return None;
    }
    let parameter = low_side / denominator;
    Some([
        low_uv[0] + parameter * (high_uv[0] - low_uv[0]),
        low_uv[1] + parameter * (high_uv[1] - low_uv[1]),
    ])
}

fn canonical_intersection_normal_override(
    a: ClipVertex,
    side_a: f64,
    b: ClipVertex,
    side_b: f64,
) -> Option<[f32; 3]> {
    let (a_normal, b_normal) = (a.normal_override?, b.normal_override?);
    let (ClipKey::Original(a_index), ClipKey::Original(b_index)) = (a.key, b.key) else {
        return None;
    };
    let (low, low_side, high, high_side) = if a_index < b_index {
        (a_normal, side_a, b_normal, side_b)
    } else {
        (b_normal, side_b, a_normal, side_a)
    };
    let denominator = low_side - high_side;
    if denominator == 0.0 || !denominator.is_finite() {
        return None;
    }
    let parameter = low_side / denominator;
    let interpolated = core::array::from_fn(|axis| {
        f64::from(low[axis]) + parameter * f64::from(high[axis] - low[axis])
    });
    let normalized = normalize3(interpolated)?;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "corner normal overrides use the mesh scalar boundary"
    )]
    Some(normalized.map(|component| component as f32))
}

fn validate_section(sections: &[SectionSegment]) -> Result<(), StretchError> {
    let mut degree = BTreeMap::<CutKey, u32>::new();
    for section in sections {
        *degree.entry(section.a).or_default() += 1;
        *degree.entry(section.b).or_default() += 1;
    }
    if sections.is_empty() || degree.values().any(|count| *count != 2) {
        return Err(StretchError::NonManifoldSection);
    }
    Ok(())
}

fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn add3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize3(vector: [f64; 3]) -> Option<[f64; 3]> {
    let length = exedra_math::norm(vector);
    (length.is_finite() && length != 0.0).then(|| vector.map(|component| component / length))
}

#[cfg(test)]
mod tests;
