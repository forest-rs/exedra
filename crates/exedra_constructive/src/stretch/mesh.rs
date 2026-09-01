// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Topology-preserving mesh realization of stretch.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use crate::ir::{Placement3, Plane3};
use crate::tessellate::{EvalPolicy, Feature, TessellatedBody};

use super::{MeshStretchStats, StretchRefusal};

/// Stretches one already-evaluated body in world space.
///
/// `world` carries the stretch node's input frame to the body's evaluation
/// frame. The plane uses inverse-transpose transport, while the displacement
/// uses the forward linear map. Keeping those distinct is essential under
/// non-uniform affine ancestors.
pub(crate) fn stretch_mesh(
    source: &TessellatedBody,
    plane: &Plane3,
    length: f64,
    world: &Placement3,
    policy: &EvalPolicy,
) -> Result<(TessellatedBody, MeshStretchStats), StretchRefusal> {
    let geometry = WorldStretch::new(plane, length, world)?;
    if length < 0.0 {
        return stretch_mesh_contraction(source, &geometry, policy);
    }
    stretch_mesh_expansion(source, &geometry, policy)
}

struct WorldStretch {
    normal: [f64; 3],
    distance: f64,
    far_distance: Option<f64>,
    displacement: [f64; 3],
}

impl WorldStretch {
    fn new(plane: &Plane3, length: f64, world: &Placement3) -> Result<Self, StretchRefusal> {
        let (local_normal, local_distance) = plane
            .normalized()
            .expect("stretch planes are validated at recipe construction");
        let linear = [
            [world.rows[0][0], world.rows[0][1], world.rows[0][2]],
            [world.rows[1][0], world.rows[1][1], world.rows[1][2]],
            [world.rows[2][0], world.rows[2][1], world.rows[2][2]],
        ];
        let inverse = inverse3(linear).ok_or(StretchRefusal::SingularTransform)?;
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
            return Err(StretchRefusal::SingularTransform);
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
    source_feature: Option<Feature>,
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
    feature: Feature,
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
    feature: Feature,
    uv: Option<[f64; 2]>,
    normal_override: Option<[f32; 3]>,
    input: Option<ClipKey>,
}

struct OutputMesh {
    builder: exedra::MeshBuilder,
    source_edges: BTreeMap<(u32, u32), (bool, f32)>,
    source_vertex_sharpness: BTreeMap<u32, f32>,
    vertices: BTreeMap<OutputKey, u32>,
    vertex_features: Vec<Feature>,
    vertex_sharpness: Vec<Option<f32>>,
    face_features: Vec<Feature>,
    face_uvs: Vec<Vec<Option<[f32; 2]>>>,
    face_normal_overrides: Vec<Vec<Option<[f32; 3]>>>,
}

impl OutputMesh {
    fn new(source: &exedra::Mesh) -> Self {
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
            builder: exedra::MeshBuilder::new(),
            source_edges,
            source_vertex_sharpness,
            vertices: BTreeMap::new(),
            vertex_features: Vec::new(),
            vertex_sharpness: Vec::new(),
            face_features: Vec::new(),
            face_uvs: Vec::new(),
            face_normal_overrides: Vec::new(),
        }
    }

    fn vertex(
        &mut self,
        key: OutputKey,
        point: [f64; 3],
        feature: Feature,
    ) -> Result<u32, StretchRefusal> {
        if let Some(index) = self.vertices.get(&key) {
            return Ok(*index);
        }
        #[expect(
            clippy::cast_possible_truncation,
            reason = "stretch crosses the documented f64-to-f32 mesh emission boundary"
        )]
        let narrowed = [point[0] as f32, point[1] as f32, point[2] as f32];
        if narrowed.iter().any(|component| !component.is_finite()) {
            return Err(StretchRefusal::BuildFailed);
        }
        let index = self.builder.push_vertex(narrowed);
        self.vertices.insert(key, index);
        self.vertex_features.push(feature);
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
    ) -> Result<(), StretchRefusal> {
        let corners = vertices
            .iter()
            .map(|vertex| self.vertex(vertex.key, vertex.point, vertex.feature))
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
                &exedra::FaceBuildAttrs {
                    region: Some(source.region),
                    edge_seams: Some(&seams),
                    edge_sharpness: Some(&inherited_sharpness),
                },
            )
            .map_err(|_| StretchRefusal::BuildFailed)?;
        self.face_features.push(source.feature);
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
        Ok(())
    }

    fn finish(self) -> Result<TessellatedBody, StretchRefusal> {
        let mut built = self
            .builder
            .build()
            .map_err(|_| StretchRefusal::BuildFailed)?;
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
                    exedra::op::set_vertex_sharpness(&mut edit, *vertex, *sharpness)
                        .map_err(|_| StretchRefusal::BuildFailed)?;
                }
            }
            for ((edges, uvs), normals) in built
                .face_edge_ids
                .iter()
                .zip(&self.face_uvs)
                .zip(&self.face_normal_overrides)
            {
                for ((edge, uv), normal) in edges.iter().zip(uvs).zip(normals) {
                    if let Some(uv) = uv {
                        exedra::op::set_corner_uv(&mut edit, *edge, *uv)
                            .map_err(|_| StretchRefusal::BuildFailed)?;
                    }
                    if let Some(normal) = normal {
                        exedra::op::set_corner_normal_override(&mut edit, *edge, Some(*normal))
                            .map_err(|_| StretchRefusal::BuildFailed)?;
                    }
                }
            }
            #[expect(unused_must_use, reason = "discard sink output")]
            {
                edit.finish();
            }
        }
        let validation = built.mesh.validate_deep();
        if !validation.is_empty() {
            return Err(StretchRefusal::BuildFailed);
        }
        let source_map = crate::source_map::SourceMap::new(
            &built.mesh,
            self.face_features,
            self.vertex_features,
        );
        Ok(TessellatedBody {
            mesh: built.mesh,
            source_map,
        })
    }
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
    source: &TessellatedBody,
    stretch: &WorldStretch,
    policy: &EvalPolicy,
) -> Result<(TessellatedBody, MeshStretchStats), StretchRefusal> {
    let mesh = &source.mesh;
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
            return Err(StretchRefusal::AmbiguousContact);
        }
        points.insert(vertex.index(), point);
        signed.insert(vertex.index(), side);
        min_signed = min_signed.min(side);
        max_signed = max_signed.max(side);
    }
    if max_signed < 0.0 {
        let mesh = mesh.clone();
        let source_map = source.source_map.repinned(&mesh);
        return Ok((
            TessellatedBody { mesh, source_map },
            MeshStretchStats::default(),
        ));
    }
    if min_signed > 0.0 {
        return Ok((
            translate_body(source, stretch.displacement),
            MeshStretchStats::default(),
        ));
    }
    if has_open_boundary(mesh) {
        return Err(StretchRefusal::OpenShell);
    }

    let regions = mesh
        .attrs()
        .dense(exedra::attr::FACE_REGION)
        .expect("FACE_REGION is a required built-in layer");
    let uv_layer = mesh.attrs().sparse(exedra::attr::CORNER_UV);
    let normal_layer = mesh.attrs().sparse(exedra::attr::CORNER_NORMAL_OVERRIDE);
    let has_uvs = uv_layer.is_some();
    let mut output = OutputMesh::new(mesh);
    let mut sections = Vec::new();
    let mut stats = MeshStretchStats::default();
    for face in mesh.faces() {
        let source_face = FaceSource {
            feature: source
                .source_map
                .face_feature(face)
                .unwrap_or(Feature::Imported),
            region: regions.get(face.into()).copied().unwrap_or(0),
        };
        let polygon = mesh
            .face_loop(face)
            .map(|corner| {
                let vertex = mesh.to_vertex(corner).expect("face corner has a vertex");
                ClipVertex {
                    key: ClipKey::Original(vertex.index()),
                    point: points[&vertex.index()],
                    source_feature: Some(
                        source
                            .source_map
                            .vertex_feature(vertex)
                            .unwrap_or(Feature::Imported),
                    ),
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
            .collect::<Result<Vec<_>, StretchRefusal>>()?;
        let has_negative = sides.iter().any(|side| *side < 0.0);
        let has_positive = sides.iter().any(|side| *side > 0.0);
        let face_normal = polygon_normal(&polygon).ok_or(StretchRefusal::BuildFailed)?;
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
            .ok_or(StretchRefusal::AmbiguousContact)?;
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
        let seam0 = Feature::StretchSeam { rim: 0 };
        let seam1 = Feature::StretchSeam { rim: 1 };
        let mapped_a_uv = section.uv_delta.and_then(|delta| add_uv(a_uv, Some(delta)));
        let mapped_b_uv = section.uv_delta.and_then(|delta| add_uv(b_uv, Some(delta)));
        output.add_polygon(
            &[
                OutputVertex {
                    key: OutputKey::Cut { cut: a_key, rim: 0 },
                    point: a,
                    feature: seam0,
                    uv: a_uv,
                    normal_override: a_band_normal,
                    input: Some(ClipKey::Cut(a_key)),
                },
                OutputVertex {
                    key: OutputKey::Cut { cut: a_key, rim: 1 },
                    point: moved_a,
                    feature: seam1,
                    uv: mapped_a_uv,
                    normal_override: a_band_normal,
                    input: Some(ClipKey::Cut(a_key)),
                },
                OutputVertex {
                    key: OutputKey::Cut { cut: b_key, rim: 1 },
                    point: moved_b,
                    feature: seam1,
                    uv: mapped_b_uv,
                    normal_override: b_band_normal,
                    input: Some(ClipKey::Cut(b_key)),
                },
                OutputVertex {
                    key: OutputKey::Cut { cut: b_key, rim: 0 },
                    point: b,
                    feature: seam0,
                    uv: b_uv,
                    normal_override: b_band_normal,
                    input: Some(ClipKey::Cut(b_key)),
                },
            ],
            section.source,
            Some(&sharpness),
        )?;
        stats.band_faces += 1;
    }
    Ok((output.finish()?, stats))
}

fn stretch_mesh_contraction(
    source: &TessellatedBody,
    stretch: &WorldStretch,
    policy: &EvalPolicy,
) -> Result<(TessellatedBody, MeshStretchStats), StretchRefusal> {
    let mesh = &source.mesh;
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
            return Err(StretchRefusal::AmbiguousContact);
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
        let mesh = mesh.clone();
        let source_map = source.source_map.repinned(&mesh);
        return Ok((
            TessellatedBody { mesh, source_map },
            MeshStretchStats::default(),
        ));
    }
    if min_far > 0.0 {
        return Ok((
            translate_body(source, stretch.displacement),
            MeshStretchStats::default(),
        ));
    }
    if min_near >= 0.0 || max_far <= 0.0 {
        return Err(StretchRefusal::ContractionConsumesHalf);
    }
    if has_open_boundary(mesh) {
        return Err(StretchRefusal::OpenShell);
    }

    let regions = mesh
        .attrs()
        .dense(exedra::attr::FACE_REGION)
        .expect("FACE_REGION is a required built-in layer");
    let uv_layer = mesh.attrs().sparse(exedra::attr::CORNER_UV);
    let normal_layer = mesh.attrs().sparse(exedra::attr::CORNER_NORMAL_OVERRIDE);
    let has_uvs = uv_layer.is_some();
    let mut faces = Vec::new();
    let mut near_sections = BTreeMap::<PositionEdge, SectionOwner>::new();
    let mut far_sections = BTreeMap::<PositionEdge, SectionOwner>::new();
    let mut stats = MeshStretchStats::default();
    for face in mesh.faces() {
        let source_face = FaceSource {
            feature: source
                .source_map
                .face_feature(face)
                .unwrap_or(Feature::Imported),
            region: regions.get(face.into()).copied().unwrap_or(0),
        };
        let polygon = mesh
            .face_loop(face)
            .map(|corner| {
                let vertex = mesh.to_vertex(corner).expect("face corner has a vertex");
                ClipVertex {
                    key: ClipKey::Original(vertex.index()),
                    point: points[&vertex.index()],
                    source_feature: Some(
                        source
                            .source_map
                            .vertex_feature(vertex)
                            .unwrap_or(Feature::Imported),
                    ),
                    uv: uv_layer
                        .and_then(|layer| layer.get(corner.into()).copied())
                        .map(|uv| uv.map(f64::from)),
                    normal_override: normal_layer
                        .and_then(|layer| layer.get(corner.into()).copied()),
                }
            })
            .collect::<Vec<_>>();
        let normal = polygon_normal(&polygon).ok_or(StretchRefusal::BuildFailed)?;
        let uv_delta = planar_uv_delta(&polygon, normal, stretch.displacement);
        let near_sides = polygon
            .iter()
            .map(|vertex| original_index(*vertex).map(|index| signed_near[&index]))
            .collect::<Result<Vec<_>, StretchRefusal>>()?;
        let far_sides = polygon
            .iter()
            .map(|vertex| original_index(*vertex).map(|index| signed_far[&index]))
            .collect::<Result<Vec<_>, StretchRefusal>>()?;
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
                return Err(StretchRefusal::NonManifoldSection);
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
                return Err(StretchRefusal::NonManifoldSection);
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
        return Err(StretchRefusal::IncompatibleContraction);
    }
    for edge in near_sections.keys() {
        if !far_sections.contains_key(edge) {
            return Err(StretchRefusal::IncompatibleContraction);
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
    Ok((output.finish()?, stats))
}

fn original_index(vertex: ClipVertex) -> Result<u32, StretchRefusal> {
    match vertex.key {
        ClipKey::Original(index) => Ok(index),
        ClipKey::Cut(_) => Err(StretchRefusal::AmbiguousContact),
    }
}

fn polygon_cut_keys(
    polygon: &[ClipVertex],
    sides: &[f64],
    plane: u8,
) -> Result<[CutKey; 2], StretchRefusal> {
    let mut cuts = Vec::new();
    for current in 0..polygon.len() {
        let next = (current + 1) % polygon.len();
        if sides[current].signum() != sides[next].signum() {
            cuts.push(cut_key(polygon[current], polygon[next], plane)?);
        }
    }
    if cuts.len() > 2 {
        return Err(StretchRefusal::DisconnectedFaceSection);
    }
    if cuts.len() != 2 || cuts[0] == cuts[1] {
        return Err(StretchRefusal::AmbiguousContact);
    }
    Ok([cuts[0], cuts[1]])
}

fn clip_polygon(
    polygon: &[ClipVertex],
    sides: &[f64],
    keep_positive: bool,
    plane: u8,
) -> Result<Vec<ClipVertex>, StretchRefusal> {
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
                source_feature: None,
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
            feature: vertex
                .source_feature
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
            feature: Feature::StretchSeam { rim: 0 },
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

fn has_open_boundary(mesh: &exedra::Mesh) -> bool {
    mesh.faces().any(|face| {
        mesh.face_loop(face).any(|edge| {
            mesh.twin(edge)
                .and_then(|twin| mesh.face(twin))
                .is_some_and(|face| face == exedra::FaceId::OUTSIDE)
        })
    })
}

fn translate_body(source: &TessellatedBody, displacement: [f64; 3]) -> TessellatedBody {
    let mut mesh = source.mesh.clone();
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
            let _ = exedra::op::set_vertex_position(&mut edit, vertex, narrowed);
        }
        #[expect(unused_must_use, reason = "discard sink output")]
        {
            edit.finish();
        }
    }
    let source_map = source.source_map.repinned(&mesh);
    TessellatedBody { mesh, source_map }
}

fn cut_key(a: ClipVertex, b: ClipVertex, plane: u8) -> Result<CutKey, StretchRefusal> {
    let (ClipKey::Original(a), ClipKey::Original(b)) = (a.key, b.key) else {
        return Err(StretchRefusal::AmbiguousContact);
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
) -> Result<[f64; 3], StretchRefusal> {
    let (ClipKey::Original(a_index), ClipKey::Original(b_index)) = (a.key, b.key) else {
        return Err(StretchRefusal::AmbiguousContact);
    };
    let (low, low_side, high, high_side) = if a_index < b_index {
        (a.point, side_a, b.point, side_b)
    } else {
        (b.point, side_b, a.point, side_a)
    };
    let denominator = low_side - high_side;
    if denominator == 0.0 || !denominator.is_finite() {
        return Err(StretchRefusal::AmbiguousContact);
    }
    let t = low_side / denominator;
    Ok([
        low[0] + t * (high[0] - low[0]),
        low[1] + t * (high[1] - low[1]),
        low[2] + t * (high[2] - low[2]),
    ])
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
            source_feature: None,
            uv: None,
            normal_override: None,
        },
        signed[&key.a],
        ClipVertex {
            key: ClipKey::Original(key.b),
            point: points[&key.b],
            source_feature: None,
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
            feature: vertex
                .source_feature
                .expect("original clip vertices retain source provenance"),
            uv,
            normal_override: vertex.normal_override,
            input: Some(vertex.key),
        },
        ClipKey::Cut(cut) => OutputVertex {
            key: OutputKey::Cut { cut, rim },
            point,
            feature: Feature::StretchSeam { rim },
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

fn validate_section(sections: &[SectionSegment]) -> Result<(), StretchRefusal> {
    let mut degree = BTreeMap::<CutKey, u32>::new();
    for section in sections {
        *degree.entry(section.a).or_default() += 1;
        *degree.entry(section.b).or_default() += 1;
    }
    if sections.is_empty() || degree.values().any(|count| *count != 2) {
        return Err(StretchRefusal::NonManifoldSection);
    }
    Ok(())
}

pub(super) fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn add3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub(super) fn cross3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
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
