// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic f64 surface sampling and nearest-triangle queries.

use exedra_mesh::{ExtractParams, Mesh};

use crate::fixture::RectPatch;
use exedra_math::{add, cross, dot, norm, scale as mul, sub};

#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct Triangle {
    pub(crate) points: [[f64; 3]; 3],
    pub(crate) original_index: usize,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct DirectedDeviation {
    pub(crate) samples: usize,
    pub(crate) maximum: f64,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct QualityReport {
    pub(crate) spacing: f64,
    pub(crate) cap: f64,
    pub(crate) mesh_to_analytic: DirectedDeviation,
    pub(crate) analytic_to_mesh: DirectedDeviation,
}

#[derive(Clone, Debug)]
pub(crate) struct Bvh {
    triangles: Vec<Triangle>,
    nodes: Vec<Node>,
    root: usize,
}

#[derive(Clone, Debug)]
struct Node {
    bounds: Bounds,
    kind: NodeKind,
}

#[derive(Clone, Debug)]
enum NodeKind {
    Leaf(Vec<usize>),
    Branch([usize; 2]),
}

#[derive(Copy, Clone, Debug)]
struct Bounds {
    min: [f64; 3],
    max: [f64; 3],
}

impl Bvh {
    pub(crate) fn new(triangles: Vec<Triangle>) -> Self {
        assert!(!triangles.is_empty(), "BVH requires triangles");
        let mut this = Self {
            triangles,
            nodes: Vec::new(),
            root: 0,
        };
        let indices = (0..this.triangles.len()).collect();
        this.root = this.build(indices);
        this
    }

    pub(crate) fn nearest(&self, point: [f64; 3]) -> (f64, usize) {
        let mut best = (f64::INFINITY, usize::MAX);
        self.nearest_node(self.root, point, &mut best);
        (best.0.sqrt(), best.1)
    }

    #[cfg(test)]
    pub(crate) fn brute_force_nearest(&self, point: [f64; 3]) -> (f64, usize) {
        let mut best = (f64::INFINITY, usize::MAX);
        for triangle in &self.triangles {
            update_best(
                &mut best,
                point_triangle_distance_squared(point, triangle.points),
                triangle.original_index,
            );
        }
        (best.0.sqrt(), best.1)
    }

    fn build(&mut self, mut indices: Vec<usize>) -> usize {
        let bounds = indices
            .iter()
            .map(|&index| triangle_bounds(self.triangles[index]))
            .reduce(union_bounds)
            .expect("nonempty BVH partition");
        if indices.len() <= 8 {
            indices.sort_unstable_by_key(|&index| self.triangles[index].original_index);
            return self.push(Node {
                bounds,
                kind: NodeKind::Leaf(indices),
            });
        }
        let centroid_bounds = indices
            .iter()
            .map(|&index| point_bounds(centroid(self.triangles[index].points)))
            .reduce(union_bounds)
            .expect("nonempty centroid partition");
        let extent = sub(centroid_bounds.max, centroid_bounds.min);
        let axis = if extent[1] > extent[0] && extent[1] >= extent[2] {
            1
        } else if extent[2] > extent[0] && extent[2] > extent[1] {
            2
        } else {
            0
        };
        indices.sort_by(|&left, &right| {
            centroid(self.triangles[left].points)[axis]
                .total_cmp(&centroid(self.triangles[right].points)[axis])
                .then_with(|| {
                    self.triangles[left]
                        .original_index
                        .cmp(&self.triangles[right].original_index)
                })
        });
        let right = indices.split_off(indices.len() / 2);
        let left_node = self.build(indices);
        let right_node = self.build(right);
        self.push(Node {
            bounds,
            kind: NodeKind::Branch([left_node, right_node]),
        })
    }

    fn push(&mut self, node: Node) -> usize {
        let index = self.nodes.len();
        self.nodes.push(node);
        index
    }

    fn nearest_node(&self, node_index: usize, point: [f64; 3], best: &mut (f64, usize)) {
        let node = &self.nodes[node_index];
        if point_bounds_distance_squared(point, node.bounds) > best.0 {
            return;
        }
        match &node.kind {
            NodeKind::Leaf(indices) => {
                for &index in indices {
                    let triangle = self.triangles[index];
                    update_best(
                        best,
                        point_triangle_distance_squared(point, triangle.points),
                        triangle.original_index,
                    );
                }
            }
            NodeKind::Branch(children) => {
                let mut ordered = children.map(|child| {
                    (
                        point_bounds_distance_squared(point, self.nodes[child].bounds),
                        child,
                    )
                });
                ordered.sort_by(|left, right| {
                    left.0
                        .total_cmp(&right.0)
                        .then_with(|| left.1.cmp(&right.1))
                });
                self.nearest_node(ordered[0].1, point, best);
                self.nearest_node(ordered[1].1, point, best);
            }
        }
    }
}

pub(crate) fn triangles(mesh: &Mesh) -> Vec<Triangle> {
    let (mesh, _) = mesh.to_trimesh(&ExtractParams::default());
    mesh.indices
        .chunks_exact(3)
        .enumerate()
        .map(|(original_index, indices)| Triangle {
            points: core::array::from_fn(|corner| {
                mesh.positions[usize::try_from(indices[corner]).expect("u32 index fits usize")]
                    .map(f64::from)
            }),
            original_index,
        })
        .collect()
}

pub(crate) fn measure(
    triangles: &[Triangle],
    patches: &[RectPatch],
    spacing: f64,
    cap: f64,
) -> QualityReport {
    assert!(spacing.is_finite() && spacing > 0.0, "sampling spacing");
    assert!(!triangles.is_empty(), "quality mesh must contain triangles");
    assert!(!patches.is_empty(), "quality surface must contain patches");
    let bvh = Bvh::new(triangles.to_vec());
    QualityReport {
        spacing,
        cap,
        mesh_to_analytic: mesh_to_analytic(triangles, patches, spacing),
        analytic_to_mesh: analytic_to_mesh(patches, &bvh, spacing),
    }
}

fn mesh_to_analytic(
    triangles: &[Triangle],
    patches: &[RectPatch],
    spacing: f64,
) -> DirectedDeviation {
    let mut maximum = 0.0_f64;
    let mut samples = 0_usize;
    for triangle in triangles {
        let edge = distance(triangle.points[0], triangle.points[1])
            .max(distance(triangle.points[1], triangle.points[2]))
            .max(distance(triangle.points[2], triangle.points[0]));
        let divisions = divisions(edge, spacing);
        for first in 0..=divisions {
            for second in 0..=divisions - first {
                let b = first as f64 / divisions as f64;
                let c = second as f64 / divisions as f64;
                let a = 1.0 - b - c;
                let point = add(
                    mul(triangle.points[0], a),
                    add(mul(triangle.points[1], b), mul(triangle.points[2], c)),
                );
                let nearest = patches
                    .iter()
                    .map(|patch| point_patch_distance(point, *patch))
                    .fold(f64::INFINITY, f64::min);
                assert!(nearest.is_finite(), "mesh sample distance must be finite");
                maximum = maximum.max(nearest);
                samples += 1;
            }
        }
    }
    DirectedDeviation { samples, maximum }
}

fn analytic_to_mesh(patches: &[RectPatch], bvh: &Bvh, spacing: f64) -> DirectedDeviation {
    let mut maximum = 0.0_f64;
    let mut samples = 0_usize;
    for point in canonical_patch_samples(patches, spacing) {
        let nearest = bvh.nearest(point).0;
        assert!(
            nearest.is_finite(),
            "analytic sample distance must be finite"
        );
        maximum = maximum.max(nearest);
        samples += 1;
    }
    DirectedDeviation { samples, maximum }
}

fn canonical_patch_samples(patches: &[RectPatch], spacing: f64) -> Vec<[f64; 3]> {
    let mut samples = Vec::new();
    for patch in patches {
        let u_divisions = divisions(patch.u[1] - patch.u[0], spacing);
        let v_divisions = divisions(patch.v[1] - patch.v[0], spacing);
        for u_index in 0..=u_divisions {
            for v_index in 0..=v_divisions {
                let mut point = [0.0; 3];
                point[patch.axis] = patch.coordinate;
                point[patch.u_axis] =
                    patch.u[0] + (patch.u[1] - patch.u[0]) * u_index as f64 / u_divisions as f64;
                point[patch.v_axis] =
                    patch.v[0] + (patch.v[1] - patch.v[0]) * v_index as f64 / v_divisions as f64;
                samples.push(point);
            }
        }
    }
    samples
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "finite benchmark extents are bounded before conversion"
)]
fn divisions(length: f64, spacing: f64) -> usize {
    let value = (length / spacing).ceil();
    assert!(value.is_finite() && value >= 0.0 && value <= usize::MAX as f64);
    (value as usize).max(1)
}

fn point_patch_distance(point: [f64; 3], patch: RectPatch) -> f64 {
    let mut nearest = point;
    nearest[patch.axis] = patch.coordinate;
    nearest[patch.u_axis] = point[patch.u_axis].clamp(patch.u[0], patch.u[1]);
    nearest[patch.v_axis] = point[patch.v_axis].clamp(patch.v[0], patch.v[1]);
    distance(point, nearest)
}

fn point_triangle_distance_squared(point: [f64; 3], triangle: [[f64; 3]; 3]) -> f64 {
    let [a, b, c] = triangle;
    let ab = sub(b, a);
    let ac = sub(c, a);
    let normal = cross(ab, ac);
    let scale = dot(ab, ab).max(dot(ac, ac)).max(dot(sub(c, b), sub(c, b)));
    if !scale.is_finite() || scale == 0.0 || dot(normal, normal) <= f64::EPSILON * scale * scale {
        return point_segment_distance_squared(point, a, b)
            .min(point_segment_distance_squared(point, b, c))
            .min(point_segment_distance_squared(point, c, a));
    }

    let ap = sub(point, a);
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return dot(ap, ap);
    }
    let bp = sub(point, b);
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if d3 >= 0.0 && d4 <= d3 {
        return dot(bp, bp);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return dot(
            sub(point, add(a, mul(ab, v))),
            sub(point, add(a, mul(ab, v))),
        );
    }
    let cp = sub(point, c);
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
    if d6 >= 0.0 && d5 <= d6 {
        return dot(cp, cp);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        let delta = sub(point, add(a, mul(ac, w)));
        return dot(delta, delta);
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        let edge = sub(c, b);
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        let delta = sub(point, add(b, mul(edge, w)));
        return dot(delta, delta);
    }
    let denominator = (va + vb + vc).recip();
    let v = vb * denominator;
    let w = vc * denominator;
    let delta = sub(point, add(a, add(mul(ab, v), mul(ac, w))));
    dot(delta, delta)
}

fn point_segment_distance_squared(point: [f64; 3], start: [f64; 3], end: [f64; 3]) -> f64 {
    let edge = sub(end, start);
    let length_squared = dot(edge, edge);
    if length_squared == 0.0 {
        let delta = sub(point, start);
        return dot(delta, delta);
    }
    let t = (dot(sub(point, start), edge) / length_squared).clamp(0.0, 1.0);
    let delta = sub(point, add(start, mul(edge, t)));
    dot(delta, delta)
}

fn update_best(best: &mut (f64, usize), distance_squared: f64, index: usize) {
    if distance_squared < best.0 || (distance_squared == best.0 && index < best.1) {
        *best = (distance_squared, index);
    }
}

fn triangle_bounds(triangle: Triangle) -> Bounds {
    triangle
        .points
        .into_iter()
        .map(point_bounds)
        .reduce(union_bounds)
        .expect("triangle has points")
}

fn point_bounds(point: [f64; 3]) -> Bounds {
    Bounds {
        min: point,
        max: point,
    }
}

fn union_bounds(left: Bounds, right: Bounds) -> Bounds {
    Bounds {
        min: core::array::from_fn(|axis| left.min[axis].min(right.min[axis])),
        max: core::array::from_fn(|axis| left.max[axis].max(right.max[axis])),
    }
}

fn point_bounds_distance_squared(point: [f64; 3], bounds: Bounds) -> f64 {
    (0..3)
        .map(|axis| {
            let delta = if point[axis] < bounds.min[axis] {
                bounds.min[axis] - point[axis]
            } else if point[axis] > bounds.max[axis] {
                point[axis] - bounds.max[axis]
            } else {
                0.0
            };
            delta * delta
        })
        .sum()
}

fn centroid(triangle: [[f64; 3]; 3]) -> [f64; 3] {
    mul(add(triangle[0], add(triangle[1], triangle[2])), 1.0 / 3.0)
}

fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    norm(sub(a, b))
}

#[cfg(test)]
mod tests {
    use crate::fixture::RectPatch;

    use super::{Bvh, NodeKind, Triangle, canonical_patch_samples};

    #[test]
    fn bvh_matches_brute_force_with_stable_centroid_ties() {
        let triangles = vec![
            Triangle {
                points: [[-1.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                original_index: 7,
            },
            Triangle {
                points: [[-1.0, 0.0, 2.0], [1.0, 0.0, 2.0], [0.0, 1.0, 2.0]],
                original_index: 3,
            },
            Triangle {
                points: [[0.0, -1.0, 1.0], [0.0, 1.0, 1.0], [0.0, 0.0, 1.0]],
                original_index: 11,
            },
        ];
        let bvh = Bvh::new(triangles);
        for point in [
            [0.0, 0.25, 0.1],
            [0.0, 0.25, 1.9],
            [0.2, 0.1, 1.0],
            [4.0, -2.0, 0.5],
        ] {
            assert_eq!(bvh.nearest(point), bvh.brute_force_nearest(point));
        }
    }

    #[test]
    fn bvh_matches_brute_force_for_point_and_segment_triangles() {
        let triangles = vec![
            Triangle {
                points: [[1.0, 2.0, 3.0]; 3],
                original_index: 0,
            },
            Triangle {
                points: [[-1.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 0.0]],
                original_index: 1,
            },
        ];
        let bvh = Bvh::new(triangles);
        for point in [[0.25, 1.0, 0.0], [1.0, 2.0, 4.0], [-2.0, 0.0, 0.0]] {
            assert_eq!(bvh.nearest(point), bvh.brute_force_nearest(point));
        }
    }

    #[test]
    fn bvh_uses_original_index_for_exact_nearest_ties_across_branches() {
        let higher_index_tie = Triangle {
            points: [[-4.0, -2.0, 1.0], [1.0, -2.0, 1.0], [0.0, 4.0, 1.0]],
            original_index: 17,
        };
        let lower_index_tie = Triangle {
            points: [[-1.0, -2.0, 1.0], [4.0, -2.0, 1.0], [0.0, 4.0, 1.0]],
            original_index: 0,
        };
        let filler = |center: f64, original_index| Triangle {
            points: [
                [center - 0.25, -0.25, 20.0],
                [center + 0.25, -0.25, 20.0],
                [center, 0.25, 20.0],
            ],
            original_index,
        };
        let mut triangles = vec![higher_index_tie, lower_index_tie];
        triangles.extend(
            (0_u32..8).map(|offset| filler(-20.0 + f64::from(offset), 100 + offset as usize)),
        );
        triangles.extend(
            (0_u32..8).map(|offset| filler(13.0 + f64::from(offset), 200 + offset as usize)),
        );
        let bvh = Bvh::new(triangles);
        let NodeKind::Branch(children) = bvh.nodes[bvh.root].kind else {
            panic!("18 triangles must create distinct root branches");
        };
        assert!(subtree_contains(&bvh, children[0], 17));
        assert!(!subtree_contains(&bvh, children[0], 0));
        assert!(subtree_contains(&bvh, children[1], 0));
        assert!(!subtree_contains(&bvh, children[1], 17));

        let query = [0.0, 0.0, 0.0];
        let child_distances = children
            .map(|child| super::point_bounds_distance_squared(query, bvh.nodes[child].bounds));
        assert_eq!(child_distances, [1.0, 1.0]);
        assert!(
            children[0] < children[1],
            "left child must win the BVH node tie"
        );

        let mut visited_best = (f64::INFINITY, usize::MAX);
        bvh.nearest_node(children[0], query, &mut visited_best);
        assert_eq!(visited_best, (1.0, 17));
        bvh.nearest_node(children[1], query, &mut visited_best);
        assert_eq!(visited_best, (1.0, 0));

        assert_eq!(bvh.nearest(query), (1.0, 0));
        assert_eq!(bvh.nearest(query), bvh.brute_force_nearest(query));
    }

    #[test]
    fn multi_node_bvh_matches_brute_force_for_every_canonical_patch_sample() {
        let triangles = (0_u32..24)
            .map(|index| {
                let z = -2.3 + 0.2 * f64::from(index);
                Triangle {
                    points: [[-1.7, -1.2, z], [1.5, -1.1, z], [-0.2, 1.8, z]],
                    original_index: usize::try_from(100 - index).expect("small index"),
                }
            })
            .collect::<Vec<_>>();
        let patches = [
            RectPatch {
                axis: 2,
                coordinate: 0.17,
                u_axis: 0,
                v_axis: 1,
                u: [-1.1, 1.3],
                v: [-0.9, 1.4],
            },
            RectPatch {
                axis: 0,
                coordinate: -0.31,
                u_axis: 1,
                v_axis: 2,
                u: [-0.7, 0.8],
                v: [-1.2, 1.1],
            },
        ];
        let samples = canonical_patch_samples(&patches, 0.23);
        assert!(
            samples.len() > 100,
            "fixture must exercise many canonical samples"
        );
        let bvh = Bvh::new(triangles);
        assert!(matches!(bvh.nodes[bvh.root].kind, NodeKind::Branch(_)));
        for point in samples {
            assert_eq!(bvh.nearest(point), bvh.brute_force_nearest(point));
        }
    }

    fn subtree_contains(bvh: &Bvh, node: usize, original_index: usize) -> bool {
        match &bvh.nodes[node].kind {
            NodeKind::Leaf(indices) => indices
                .iter()
                .any(|&index| bvh.triangles[index].original_index == original_index),
            NodeKind::Branch(children) => children
                .iter()
                .any(|&child| subtree_contains(bvh, child, original_index)),
        }
    }
}
