// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Source-face UV charts for the staged rounding rewrite.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use exedra_math::{add, cross, dot, narrow, promote, scale, sub};

use super::{NewFace, Tok};
use crate::{FaceId, FaceTriangulation, Mesh, VertexId, attr};

pub(super) fn transfer(mesh: &Mesh, points: &[[f64; 3]], faces: &mut [NewFace]) {
    if mesh.attrs().sparse(attr::CORNER_UV).is_none() {
        return;
    }
    // Prepare each input chart once, even for finely sampled corner patches.
    // Nothing is triangulated on the ordinary untextured path.
    let mut sources = BTreeMap::new();
    for face in faces {
        let owner = face.source.faces()[0];
        let source = sources
            .entry(owner)
            .or_insert_with(|| FaceUvs::new(mesh, owner));
        face.uvs = face
            .entries
            .iter()
            .map(|entry| match *entry {
                Tok::Old(vertex) => source.corners[&vertex],
                Tok::New(point) => source.at(promote(narrow(points[point as usize]))),
            })
            .collect();
    }
}

struct FaceUvs {
    corners: BTreeMap<VertexId, Option<[f32; 2]>>,
    triangles: Vec<UvTriangle>,
}

impl FaceUvs {
    fn new(mesh: &Mesh, face: FaceId) -> Self {
        let layer = mesh.attrs().sparse(attr::CORNER_UV).expect("UV layer");
        let corners: BTreeMap<_, _> = mesh
            .face_loop(face)
            .map(|corner| {
                (
                    mesh.to_vertex(corner).expect("source corner"),
                    layer.get(corner.as_id()).copied(),
                )
            })
            .collect();
        let mut source = Self {
            corners,
            triangles: Vec::new(),
        };
        // A partial chart has no defined interpolation. Retain its surviving
        // corners without filling missing values with zero or another face's UVs.
        if source
            .corners
            .values()
            .any(|uv| uv.is_none_or(|uv| !uv.iter().all(|v| v.is_finite())))
        {
            return source;
        }
        let (triangles, fallback) = mesh.face_triangles_counted(face, FaceTriangulation::Robust);
        if !fallback {
            source.triangles = triangles
                .into_iter()
                .filter_map(|triangle| {
                    let points = triangle.map(|corner| {
                        promote(
                            *mesh
                                .vertex_position(mesh.to_vertex(corner).expect("source corner"))
                                .expect("source position"),
                        )
                    });
                    let uvs = triangle.map(|corner| {
                        layer
                            .get(corner.as_id())
                            .expect("complete chart")
                            .map(f64::from)
                    });
                    UvTriangle::new(points, uvs)
                })
                .collect();
        }
        source
    }

    fn at(&self, point: [f64; 3]) -> Option<[f32; 2]> {
        let mut closest = None;
        let mut distance = f64::INFINITY;
        for triangle in &self.triangles {
            let weights = triangle.weights(point);
            if weights.iter().all(|&w| w >= 0.0) {
                return triangle.interpolate(weights);
            }
            // Offset joins can extend slightly beyond a source polygon. Extend
            // its closest triangle's affine chart rather than clamping the UVs
            // to the old boundary (which would collapse texture area).
            let candidate = triangle.distance_squared(point);
            if candidate < distance {
                distance = candidate;
                closest = Some((triangle, weights));
            }
        }
        closest.and_then(|(triangle, weights)| triangle.interpolate(weights))
    }
}

struct UvTriangle {
    points: [[f64; 3]; 3],
    uvs: [[f64; 2]; 3],
    normal: [f64; 3],
    normal_squared: f64,
}

impl UvTriangle {
    fn new(points: [[f64; 3]; 3], uvs: [[f64; 2]; 3]) -> Option<Self> {
        let normal = cross(sub(points[1], points[0]), sub(points[2], points[0]));
        let normal_squared = dot(normal, normal);
        (normal_squared > 0.0 && normal_squared.is_finite()).then_some(Self {
            points,
            uvs,
            normal,
            normal_squared,
        })
    }

    fn weights(&self, point: [f64; 3]) -> [f64; 3] {
        let [a, b, c] = self.points;
        let offset = sub(point, a);
        let v = dot(cross(offset, sub(c, a)), self.normal) / self.normal_squared;
        let w = dot(cross(sub(b, a), offset), self.normal) / self.normal_squared;
        [1.0 - v - w, v, w]
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "UVs narrow once; non-finite results remain unset"
    )]
    fn interpolate(&self, weights: [f64; 3]) -> Option<[f32; 2]> {
        let uv = core::array::from_fn(|axis| {
            // The difference form preserves constant charts exactly.
            (self.uvs[0][axis]
                + weights[1] * (self.uvs[1][axis] - self.uvs[0][axis])
                + weights[2] * (self.uvs[2][axis] - self.uvs[0][axis])) as f32
        });
        uv.iter().all(|v| v.is_finite()).then_some(uv)
    }

    fn distance_squared(&self, point: [f64; 3]) -> f64 {
        let mut distance = f64::INFINITY;
        for i in 0..3 {
            let a = self.points[i];
            let edge = sub(self.points[(i + 1) % 3], a);
            let t = (dot(sub(point, a), edge) / dot(edge, edge)).clamp(0.0, 1.0);
            let offset = sub(point, add(a, scale(edge, t)));
            distance = distance.min(dot(offset, offset));
        }
        distance
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MeshBuilder, op};

    #[test]
    fn concave_chart_interpolates_and_extends_its_nearest_triangle() {
        // An L-shaped face with non-affine UVs. The upper-arm probe lies in
        // triangle (0,0), (1,1), (1,3) with weights (1/4, 1/8, 5/8).
        // One affine mapping over the whole face cannot reproduce this chart.
        let mut builder = MeshBuilder::new();
        for p in [
            [0.0, 0.0, 0.0],
            [3.0, 0.0, 0.0],
            [3.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
            [1.0, 3.0, 0.0],
            [0.0, 3.0, 0.0],
        ] {
            builder.push_vertex(p);
        }
        builder.add_face(&[0, 1, 2, 3, 4, 5]).unwrap();
        let mut mesh = builder.build().unwrap().mesh;
        let face = mesh.faces().next().unwrap();
        let values: Vec<_> = mesh
            .face_loop(face)
            .map(|corner| {
                let p = mesh
                    .vertex_position(mesh.to_vertex(corner).unwrap())
                    .unwrap();
                (
                    corner,
                    [p[0], if p[1] > 1.0 { 2.0 * p[1] - 1.0 } else { p[1] }],
                )
            })
            .collect();
        let mut edit = mesh.edit();
        for (corner, uv) in values {
            op::set_corner_uv(&mut edit, corner, uv).unwrap();
        }
        let _: () = edit.finish();
        let chart = FaceUvs::new(&mesh, face);
        for (p, expected) in [
            ([2.0, 0.25, 0.0], [2.0, 0.25]),
            ([0.75, 2.0, 0.0], [0.75, 3.25]),
            ([0.75, 2.0, 4.0], [0.75, 3.25]),
            ([3.25, 0.25, 0.0], [3.25, 0.25]),
        ] {
            let actual = chart.at(p).unwrap();
            for axis in 0..2 {
                assert!(
                    (actual[axis] - expected[axis]).abs() < 1e-6,
                    "{p:?}: {actual:?}"
                );
            }
        }
    }

    #[test]
    fn interpolation_does_not_emit_overflowing_uvs() {
        let triangle = UvTriangle::new(
            [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            [[0.0, 0.0], [f64::from(f32::MAX), 0.0], [0.0, 1.0]],
        )
        .unwrap();
        assert_eq!(
            triangle.interpolate(triangle.weights([2.0, 0.0, 0.0])),
            None
        );
    }
}
