// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! MikkTSpace-compatible tangent generation over extracted triangles.
//!
//! This is an in-crate port of the MikkTSpace algorithm (Morten S. Mikkelsen,
//! reference implementation `mikktspace.c`) restricted to triangle input:
//!
//! 1. Corners are welded by equal position, normal and texture coordinate.
//! 2. Each triangle gets a tangent and bitangent from its UV gradients, an
//!    orientation from the signed UV area, and magnitudes. Triangles whose
//!    UV area or magnitudes vanish "group with any" and contribute no
//!    direction.
//! 3. Around each welded vertex, edge-connected triangles of one orientation
//!    form a group. Only triangles with a usable gradient seed groups;
//!    "group with any" triangles join the first group that reaches them and
//!    adopt its orientation. Every corner's tangent is the angle-weighted
//!    average of its group's tangents, each projected into the corner
//!    normal's plane.
//! 4. Triangles with two welded vertices in common copy tangents from a
//!    valid triangle sharing the vertex.
//!
//! Deviations from the reference, recorded in ADR-0006:
//! - input is the extracted triangle list, so an ngon's tangents follow
//!   Exedra's triangulation rather than the reference's quad split;
//! - floating-point evaluation order is not guaranteed to match the C
//!   implementation bit for bit;
//! - a corner the reference leaves ungrouped (a "group with any" triangle no
//!   group reaches, for which it keeps its default `(1, 0, 0)` with `w = -1`)
//!   or whose averaged tangent vanishes or is non-finite receives a
//!   deterministic unit tangent perpendicular to its normal, with `w = 1`,
//!   and is counted.
//!
//! Cost matches the reference: grouping around one vertex is quadratic in
//! the triangles meeting there, so vertices with tens of thousands of
//! incident triangles are slow.

use alloc::vec;
use alloc::vec::Vec;
use hashbrown::HashMap;

use exedra_math::{dot, norm, sub};

use crate::math::FloatExt;

/// Per-corner tangents and fallback flags for one triangle list.
pub(super) struct CornerTangents {
    /// `xyz` tangent and handedness sign `w`, one per index.
    pub(super) tangents: Vec<[f32; 4]>,
    /// True where the corner received the fallback tangent.
    pub(super) fallback: Vec<bool>,
}

#[derive(Copy, Clone, Debug, Default)]
struct TriInfo {
    tangent: [f32; 3],
    bitangent: [f32; 3],
    orient: bool,
    group_with_any: bool,
    degenerate: bool,
    /// Neighbor across edge `i -> i + 1`.
    neighbors: [Option<u32>; 3],
    groups: [Option<u32>; 3],
}

#[derive(Clone, Debug)]
struct Group {
    vertex: u32,
    orient: bool,
    faces: Vec<u32>,
}

#[derive(Copy, Clone, Debug, Default)]
struct TSpace {
    tangent: [f32; 3],
}

fn triangle_id(f: usize) -> u32 {
    u32::try_from(f).expect("triangle count fits u32")
}

fn not_zero(value: f32) -> bool {
    value.abs() > f32::MIN_POSITIVE
}

fn scale(v: [f32; 3], s: f32) -> [f32; 3] {
    [v[0] * s, v[1] * s, v[2] * s]
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// Normalizes when the length is representable, as the reference does.
fn normalize_if(v: [f32; 3]) -> [f32; 3] {
    let length = norm(v);
    if not_zero(length) {
        scale(v, 1.0 / length)
    } else {
        v
    }
}

/// Projects `v` into the plane of unit normal `n` and normalizes it.
fn project(n: [f32; 3], v: [f32; 3]) -> [f32; 3] {
    normalize_if(sub(v, scale(n, dot(n, v))))
}

fn weld_bits(value: f32) -> u32 {
    // The reference welds on float equality, so both zeros weld together.
    if value == 0.0 { 0 } else { value.to_bits() }
}

/// Computes MikkTSpace tangents for every corner of `indices`.
pub(super) fn corner_tangents(
    indices: &[u32],
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    uvs: &[[f32; 2]],
) -> CornerTangents {
    let corners = indices.len();
    let triangles = corners / 3;
    let position = |c: usize| positions[indices[c] as usize];
    let normal = |c: usize| normals[indices[c] as usize];
    let uv = |c: usize| uvs[indices[c] as usize];

    // 1. Weld corners by position, normal and texture coordinate.
    let mut weld: HashMap<[u32; 8], u32> = HashMap::new();
    let welded: Vec<u32> = (0..corners)
        .map(|c| {
            let (p, n, t) = (position(c), normal(c), uv(c));
            let key = [
                weld_bits(p[0]),
                weld_bits(p[1]),
                weld_bits(p[2]),
                weld_bits(n[0]),
                weld_bits(n[1]),
                weld_bits(n[2]),
                weld_bits(t[0]),
                weld_bits(t[1]),
            ];
            let next = u32::try_from(weld.len()).expect("corner count fits u32");
            *weld.entry(key).or_insert(next)
        })
        .collect();

    // 2. Per-triangle gradients, orientation and degeneracy.
    let mut tris = vec![TriInfo::default(); triangles];
    for (f, info) in tris.iter_mut().enumerate() {
        let [i0, i1, i2] = [welded[3 * f], welded[3 * f + 1], welded[3 * f + 2]];
        info.degenerate = i0 == i1 || i0 == i2 || i1 == i2;
        info.group_with_any = true;
        let (v1, v2, v3) = (position(3 * f), position(3 * f + 1), position(3 * f + 2));
        let (t1, t2, t3) = (uv(3 * f), uv(3 * f + 1), uv(3 * f + 2));
        let t21 = [t2[0] - t1[0], t2[1] - t1[1]];
        let t31 = [t3[0] - t1[0], t3[1] - t1[1]];
        let d1 = sub(v2, v1);
        let d2 = sub(v3, v1);
        let signed_area = t21[0] * t31[1] - t21[1] * t31[0];
        let mut tangent = sub(scale(d1, t31[1]), scale(d2, t21[1]));
        let mut bitangent = add(scale(d1, -t31[0]), scale(d2, t21[0]));
        info.orient = signed_area > 0.0;
        if not_zero(signed_area) {
            let area = signed_area.abs();
            let len_s = norm(tangent);
            let len_t = norm(bitangent);
            let sign = if info.orient { 1.0 } else { -1.0 };
            if not_zero(len_s) {
                tangent = scale(tangent, sign / len_s);
            }
            if not_zero(len_t) {
                bitangent = scale(bitangent, sign / len_t);
            }
            if not_zero(len_s / area) && not_zero(len_t / area) {
                info.group_with_any = false;
            }
        }
        info.tangent = tangent;
        info.bitangent = bitangent;
    }

    // 3a. Neighbors across shared welded edges among valid triangles.
    let mut edges: HashMap<(u32, u32), Vec<(u32, usize)>> = HashMap::new();
    for (f, info) in tris.iter().enumerate() {
        if info.degenerate {
            continue;
        }
        for i in 0..3 {
            let a = welded[3 * f + i];
            let b = welded[3 * f + (i + 1) % 3];
            edges.entry((a, b)).or_default().push((triangle_id(f), i));
        }
    }
    for f in 0..triangles {
        if tris[f].degenerate {
            continue;
        }
        for i in 0..3 {
            if tris[f].neighbors[i].is_some() {
                continue;
            }
            let a = welded[3 * f + i];
            let b = welded[3 * f + (i + 1) % 3];
            let Some(candidates) = edges.get(&(b, a)) else {
                continue;
            };
            let found = candidates
                .iter()
                .copied()
                .find(|&(g, j)| g as usize != f && tris[g as usize].neighbors[j].is_none());
            if let Some((g, j)) = found {
                tris[f].neighbors[i] = Some(g);
                tris[g as usize].neighbors[j] = Some(triangle_id(f));
            }
        }
    }

    // 3b. Groups around each welded vertex. Like the reference's
    // `Build4RuleGroups`, only triangles with a usable gradient seed groups;
    // "group with any" triangles join through `assign`, so seeding does not
    // depend on face order. Which group adopts a "group with any" triangle
    // between opposite orientations still follows face order, as in the
    // reference.
    let mut groups: Vec<Group> = Vec::new();
    let mut pending: Vec<u32> = Vec::new();
    for f in 0..triangles {
        if tris[f].degenerate || tris[f].group_with_any {
            continue;
        }
        for i in 0..3 {
            if tris[f].groups[i].is_some() {
                continue;
            }
            let group = u32::try_from(groups.len()).expect("group count fits u32");
            groups.push(Group {
                vertex: welded[3 * f + i],
                orient: tris[f].orient,
                faces: vec![triangle_id(f)],
            });
            tris[f].groups[i] = Some(group);
            push_neighbors(&tris[f], i, &mut pending);
            assign(&welded, &mut tris, &mut groups, &mut pending, group);
        }
    }

    // 3c. Subgroup tangent spaces, evaluated per corner.
    let mut tangents = vec![[0.0; 4]; corners];
    let mut fallback = vec![false; corners];
    let mut resolved = vec![false; corners];
    for group in &groups {
        let mut subgroups: Vec<(Vec<u32>, TSpace)> = Vec::new();
        let corner_of = |f: u32| {
            let base = 3 * f as usize;
            (0..3)
                .map(|k| base + k)
                .find(|&c| welded[c] == group.vertex)
                .expect("group face contains its vertex")
        };
        let projected = |f: u32| {
            let c = corner_of(f);
            let n = normal(c);
            let info = &tris[f as usize];
            (project(n, info.tangent), project(n, info.bitangent))
        };
        for &f in &group.faces {
            let (tangent, bitangent) = projected(f);
            let mut members: Vec<u32> = group
                .faces
                .iter()
                .copied()
                .filter(|&t| {
                    let any = tris[f as usize].group_with_any || tris[t as usize].group_with_any;
                    let (tangent2, bitangent2) = projected(t);
                    // The default angular threshold is 180 degrees.
                    any || (dot(tangent, tangent2) > -1.0 && dot(bitangent, bitangent2) > -1.0)
                })
                .collect();
            members.sort_unstable();
            let space = match subgroups.iter().find(|(existing, _)| *existing == members) {
                Some((_, space)) => *space,
                None => {
                    let space = eval_tspace(&members, &corner_of, &tris, &position, &normal);
                    subgroups.push((members, space));
                    space
                }
            };
            let c = corner_of(f);
            let sign = if group.orient { 1.0 } else { -1.0 };
            tangents[c] = [space.tangent[0], space.tangent[1], space.tangent[2], sign];
            resolved[c] = true;
        }
    }

    // 4. Degenerate triangles copy from a valid corner on the same vertex.
    let mut first_valid: HashMap<u32, usize> = HashMap::new();
    for f in 0..triangles {
        if tris[f].degenerate {
            continue;
        }
        for k in 0..3 {
            first_valid.entry(welded[3 * f + k]).or_insert(3 * f + k);
        }
    }
    for (f, info) in tris.iter().enumerate() {
        if !info.degenerate {
            continue;
        }
        for k in 0..3 {
            let c = 3 * f + k;
            if let Some(&source) = first_valid.get(&welded[c]) {
                tangents[c] = tangents[source];
                resolved[c] = true;
            }
        }
    }

    for c in 0..corners {
        let t = tangents[c];
        let xyz = [t[0], t[1], t[2]];
        let usable = resolved[c] && xyz.iter().all(|v| v.is_finite()) && not_zero(norm(xyz));
        if !usable {
            let perpendicular = fallback_tangent(normal(c));
            tangents[c] = [perpendicular[0], perpendicular[1], perpendicular[2], 1.0];
            fallback[c] = true;
        }
    }
    CornerTangents { tangents, fallback }
}

/// Queues the neighbors of corner `i` of `info` so that the left neighbor
/// is visited first, as the reference's recursion does.
fn push_neighbors(info: &TriInfo, i: usize, pending: &mut Vec<u32>) {
    let left = info.neighbors[i];
    let right = info.neighbors[if i > 0 { i - 1 } else { 2 }];
    pending.extend(right);
    pending.extend(left);
}

/// Grows `group` around its vertex from the triangles in `pending`.
///
/// An explicit depth-first worklist visiting triangles in the same pre-order
/// as the reference's `AssignRecur` (left subtree before right), so a
/// high-valence vertex cannot overflow the stack.
fn assign(
    welded: &[u32],
    tris: &mut [TriInfo],
    groups: &mut [Group],
    pending: &mut Vec<u32>,
    group: u32,
) {
    let vertex = groups[group as usize].vertex;
    let orient = groups[group as usize].orient;
    while let Some(f) = pending.pop() {
        let fi = f as usize;
        let Some(i) = (0..3).find(|&k| welded[3 * fi + k] == vertex) else {
            continue;
        };
        if tris[fi].groups[i].is_some() {
            continue;
        }
        if tris[fi].group_with_any && tris[fi].groups.iter().all(Option::is_none) {
            // The first group to reach an orientation-free triangle sets it.
            tris[fi].orient = orient;
        }
        if tris[fi].orient != orient {
            continue;
        }
        groups[group as usize].faces.push(f);
        tris[fi].groups[i] = Some(group);
        push_neighbors(&tris[fi], i, pending);
    }
}

fn eval_tspace(
    members: &[u32],
    corner_of: &impl Fn(u32) -> usize,
    tris: &[TriInfo],
    position: &impl Fn(usize) -> [f32; 3],
    normal: &impl Fn(usize) -> [f32; 3],
) -> TSpace {
    // The reference also averages gradient magnitudes over the angle sum;
    // only the direction reaches glTF-style output.
    let mut tangent = [0.0; 3];
    for &f in members {
        let info = &tris[f as usize];
        if info.group_with_any {
            continue;
        }
        let c = corner_of(f);
        let base = c - c % 3;
        let k = c % 3;
        let prev = base + if k > 0 { k - 1 } else { 2 };
        let next = base + if k < 2 { k + 1 } else { 0 };
        let n = normal(c);
        let p1 = position(c);
        let v1 = project(n, sub(position(prev), p1));
        let v2 = project(n, sub(position(next), p1));
        let cos = dot(v1, v2).clamp(-1.0, 1.0);
        let angle = cos.acos_ext();
        tangent = add(tangent, scale(project(n, info.tangent), angle));
    }
    TSpace {
        tangent: normalize_if(tangent),
    }
}

/// A unit vector perpendicular to `normal`, chosen deterministically.
fn fallback_tangent(normal: [f32; 3]) -> [f32; 3] {
    let Some(n) = exedra_math::normalize(normal) else {
        return [1.0, 0.0, 0.0];
    };
    let axis = if n[0].abs() < 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    exedra_math::normalize(sub(axis, scale(n, dot(n, axis)))).unwrap_or([1.0, 0.0, 0.0])
}

/// UV set whose gradients define tangent directions.
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub enum TangentUv {
    /// The primary UV set, [`TriMesh::uvs`](crate::TriMesh::uvs), as resolved
    /// by [`ExtractParams::uvs`](crate::ExtractParams::uvs).
    Primary,
    /// The carried `[f32; 2]` stream of this layer, for example
    /// `attr::CORNER_UV1`.
    ///
    /// The same key must also be listed in
    /// [`ExtractParams::attributes`](crate::ExtractParams::attributes):
    /// tangents read the extracted stream, not the mesh layer. Otherwise
    /// every tangent is a fallback and
    /// [`ExtractStats::missing_tangent_uv_sets`](crate::ExtractStats::missing_tangent_uv_sets)
    /// is set.
    Attribute(crate::attributes::AttrKey<[f32; 2]>),
}

/// Adds MikkTSpace tangents to `mesh`, splitting render vertices whose
/// corners disagree. Returns the output mesh and updates `stats`.
pub(super) fn apply(
    mesh: crate::TriMesh,
    uv_set: TangentUv,
    stats: &mut crate::ExtractStats,
) -> crate::TriMesh {
    let uvs: Option<&[[f32; 2]]> = match uv_set {
        TangentUv::Primary => Some(&mesh.uvs),
        TangentUv::Attribute(key) => match mesh.attribute(key) {
            Some(crate::AttributeBuffer::Vec2(values)) => Some(values),
            _ => None,
        },
    };
    let zero_uvs;
    let uvs = match uvs {
        Some(uvs) => uvs,
        None => {
            stats.missing_tangent_uv_sets = 1;
            zero_uvs = vec![[0.0, 0.0]; mesh.positions.len()];
            &zero_uvs
        }
    };
    let corner = corner_tangents(&mesh.indices, &mesh.positions, &mesh.normals, uvs);

    // Re-key render vertices by (source vertex, tangent bits) in index
    // order. First-encounter numbering reproduces the original order when
    // nothing splits, because extraction numbered vertices the same way.
    let mut key_to_index: HashMap<(u32, [u32; 4]), u32> = HashMap::new();
    let mut sources: Vec<u32> = Vec::with_capacity(mesh.positions.len());
    let mut tangents: Vec<[f32; 4]> = Vec::with_capacity(mesh.positions.len());
    let mut fallbacks = 0_u64;
    let mut indices = Vec::with_capacity(mesh.indices.len());
    let mut seen = vec![false; mesh.positions.len()];
    for (c, &source) in mesh.indices.iter().enumerate() {
        let tangent = corner.tangents[c];
        let key = (source, tangent.map(f32::to_bits));
        let index = *key_to_index.entry(key).or_insert_with(|| {
            let index = u32::try_from(sources.len()).expect("render vertex count fits u32");
            sources.push(source);
            tangents.push(tangent);
            if corner.fallback[c] {
                fallbacks += 1;
            }
            if seen[source as usize] {
                stats.tangent_split_count = stats.tangent_split_count.saturating_add(1);
                stats.split_count = stats.split_count.saturating_add(1);
            }
            seen[source as usize] = true;
            index
        });
        indices.push(index);
    }
    stats.tangent_fallback_count = fallbacks;

    // Without splits the re-keying is the identity; attach the tangents.
    let identity = sources.len() == mesh.positions.len()
        && sources.iter().enumerate().all(|(i, &s)| s as usize == i);
    if identity {
        debug_assert_eq!(
            indices, mesh.indices,
            "identity re-keying keeps the index buffer"
        );
        return crate::TriMesh { tangents, ..mesh };
    }

    let gather = |values: &[[f32; 3]]| sources.iter().map(|&s| values[s as usize]).collect();
    crate::TriMesh {
        indices,
        positions: gather(&mesh.positions),
        uvs: sources.iter().map(|&s| mesh.uvs[s as usize]).collect(),
        normals: gather(&mesh.normals),
        tangents,
        attributes: mesh
            .attributes
            .iter()
            .map(|stream| crate::AttributeStream {
                domain: stream.domain,
                name: stream.name,
                values: stream.values.gather(&sources),
            })
            .collect(),
    }
}
