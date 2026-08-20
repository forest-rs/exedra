// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Fixed analytic fixtures and exact visible patch construction.

use exedra_isosurface::analytic::{BoxField, CylinderField, Difference, TaggedField, Union};
use exedra_isosurface::{DualContourParams, EdgeSearchParams};
use exedra_qef::QefParams;
use exedra_spatial::Aabb;

pub(crate) const BOX_A_REGION: u32 = 10;
pub(crate) const BOX_B_REGION: u32 = 20;
pub(crate) const CYLINDER_REGION: u32 = 30;

pub(crate) type H1Field = Union<TaggedField<BoxField, u32>, TaggedField<BoxField, u32>>;
pub(crate) type HardField = Difference<TaggedField<BoxField, u32>, TaggedField<CylinderField, u32>>;

#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct AxisBox {
    pub(crate) center: [f64; 3],
    pub(crate) half_extents: [f64; 3],
}

impl AxisBox {
    pub(crate) fn min(self) -> [f64; 3] {
        core::array::from_fn(|axis| self.center[axis] - self.half_extents[axis])
    }

    pub(crate) fn max(self) -> [f64; 3] {
        core::array::from_fn(|axis| self.center[axis] + self.half_extents[axis])
    }
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct RectPatch {
    pub(crate) axis: usize,
    pub(crate) coordinate: f64,
    pub(crate) u_axis: usize,
    pub(crate) v_axis: usize,
    pub(crate) u: [f64; 2],
    pub(crate) v: [f64; 2],
}

pub(crate) struct H1Fixture {
    pub(crate) field: H1Field,
    pub(crate) params: DualContourParams,
    pub(crate) boxes: [AxisBox; 2],
}

pub(crate) struct HardFixture {
    pub(crate) field: HardField,
    pub(crate) params: DualContourParams,
    pub(crate) box_center: [f32; 3],
    pub(crate) box_half_extents: [f32; 3],
    pub(crate) cylinder_center: [f32; 3],
    pub(crate) cylinder_axis: [f32; 3],
    pub(crate) cylinder_radius: f32,
    pub(crate) cylinder_half_height: f32,
}

pub(crate) fn h1(depth: u8) -> H1Fixture {
    let scale = 2.75_f32;
    let translation = [2.3_f32, -1.1, 0.7];
    let transform = |local: [f32; 3]| {
        [
            translation[0] + scale * local[0],
            translation[1] + scale * local[1],
            translation[2] + scale * local[2],
        ]
    };
    let first = BoxField {
        center: transform([-0.347, -0.153, 0.017]),
        half_extents: [0.893 * scale, 0.697 * scale, 0.541 * scale],
    };
    let second = BoxField {
        center: transform([0.407, 0.293, 0.123]),
        half_extents: [0.647 * scale, 0.787 * scale, 0.397 * scale],
    };
    let root_bounds = Aabb::new(transform([-1.6, -1.5, -1.3]), transform([1.6, 1.5, 1.3]))
        .expect("H1 root bounds");
    H1Fixture {
        field: Union::new(
            TaggedField {
                field: first,
                provenance: BOX_A_REGION,
            },
            TaggedField {
                field: second,
                provenance: BOX_B_REGION,
            },
        ),
        params: params(root_bounds, depth),
        boxes: [axis_box(first), axis_box(second)],
    }
}

pub(crate) fn hard(depth: u8) -> HardFixture {
    let scale = 1.7_f32;
    let translation = [-2.1_f32, 0.8, 1.3];
    let box_center = translation;
    let box_half_extents = [1.0 * scale, 0.85 * scale, 0.7 * scale];
    let cylinder_center = [
        translation[0] + 0.19 * scale,
        translation[1],
        translation[2],
    ];
    let cylinder_axis = [0.0, 0.0, 1.0];
    let cylinder_radius = 0.43 * scale;
    let cylinder_half_height = 1.1 * scale;
    let field = Difference::new(
        TaggedField {
            field: BoxField {
                center: box_center,
                half_extents: box_half_extents,
            },
            provenance: BOX_A_REGION,
        },
        TaggedField {
            field: CylinderField {
                center: cylinder_center,
                axis: cylinder_axis,
                radius: cylinder_radius,
                half_height: cylinder_half_height,
            },
            provenance: CYLINDER_REGION,
        },
    );
    let root_bounds = Aabb::new(
        [
            translation[0] - 1.35 * scale,
            translation[1] - 1.35 * scale,
            translation[2] - 1.35 * scale,
        ],
        [
            translation[0] + 1.35 * scale,
            translation[1] + 1.35 * scale,
            translation[2] + 1.35 * scale,
        ],
    )
    .expect("hard root bounds");
    HardFixture {
        field,
        params: params(root_bounds, depth),
        box_center,
        box_half_extents,
        cylinder_center,
        cylinder_axis,
        cylinder_radius,
        cylinder_half_height,
    }
}

pub(crate) fn visible_union_patches(boxes: [AxisBox; 2]) -> Vec<RectPatch> {
    let mut patches = Vec::new();
    for owner in 0..2 {
        let value = boxes[owner];
        let other = boxes[1 - owner];
        let value_min = value.min();
        let value_max = value.max();
        let other_min = other.min();
        let other_max = other.max();
        for axis in 0..3 {
            let [u_axis, v_axis] = tangent_axes(axis);
            for (coordinate, maximum_side) in [(value_min[axis], false), (value_max[axis], true)] {
                let base = RectPatch {
                    axis,
                    coordinate,
                    u_axis,
                    v_axis,
                    u: [value_min[u_axis], value_max[u_axis]],
                    v: [value_min[v_axis], value_max[v_axis]],
                };
                if coordinate < other_min[axis] || coordinate > other_max[axis] {
                    patches.push(base);
                    continue;
                }
                let overlap_u = [
                    base.u[0].max(other_min[u_axis]),
                    base.u[1].min(other_max[u_axis]),
                ];
                let overlap_v = [
                    base.v[0].max(other_min[v_axis]),
                    base.v[1].min(other_max[v_axis]),
                ];
                if overlap_u[0] >= overlap_u[1] || overlap_v[0] >= overlap_v[1] {
                    patches.push(base);
                } else if ((coordinate == other_min[axis] && !maximum_side)
                    || (coordinate == other_max[axis] && maximum_side))
                    && owner == 0
                {
                    // Equal coplanar outward faces remain on the union boundary.
                    // The lower operand owns their overlap deterministically.
                    patches.push(base);
                } else {
                    subtract_rectangle(base, overlap_u, overlap_v, &mut patches);
                }
            }
        }
    }
    patches
}

fn subtract_rectangle(
    base: RectPatch,
    overlap_u: [f64; 2],
    overlap_v: [f64; 2],
    out: &mut Vec<RectPatch>,
) {
    push_patch(base, [base.u[0], overlap_u[0]], base.v, out);
    push_patch(base, [overlap_u[1], base.u[1]], base.v, out);
    push_patch(base, overlap_u, [base.v[0], overlap_v[0]], out);
    push_patch(base, overlap_u, [overlap_v[1], base.v[1]], out);
}

fn push_patch(base: RectPatch, u: [f64; 2], v: [f64; 2], out: &mut Vec<RectPatch>) {
    if u[0] < u[1] && v[0] < v[1] {
        out.push(RectPatch { u, v, ..base });
    }
}

fn tangent_axes(axis: usize) -> [usize; 2] {
    match axis {
        0 => [1, 2],
        1 => [0, 2],
        _ => [0, 1],
    }
}

fn axis_box(value: BoxField) -> AxisBox {
    AxisBox {
        center: value.center.map(f64::from),
        half_extents: value.half_extents.map(f64::from),
    }
}

fn params(root_bounds: Aabb, max_depth: u8) -> DualContourParams {
    DualContourParams {
        root_bounds,
        max_depth,
        cell_budget: None,
        edge_search: EdgeSearchParams {
            bisection_steps: 10,
        },
        qef: QefParams::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::{AxisBox, RectPatch, tangent_axes, visible_union_patches};

    #[test]
    fn visible_patch_partition_covers_disjoint_contained_touching_and_coplanar_unions() {
        let cases = [
            (
                [
                    axis_box([-3.0, 0.0, 0.0], [1.0; 3]),
                    axis_box([3.0, 0.0, 0.0], [1.0; 3]),
                ],
                48.0,
                "disjoint",
            ),
            (
                [
                    axis_box([0.0; 3], [2.0; 3]),
                    axis_box([0.25, -0.1, 0.3], [0.5; 3]),
                ],
                96.0,
                "containment",
            ),
            (
                [
                    axis_box([-1.0, 0.0, 0.0], [1.0; 3]),
                    axis_box([1.0, 0.0, 0.0], [1.0; 3]),
                ],
                40.0,
                "face touch",
            ),
            (
                [
                    axis_box([0.0; 3], [1.0; 3]),
                    axis_box([1.0, 0.0, 0.0], [1.0; 3]),
                ],
                32.0,
                "coplanar overlap",
            ),
        ];
        for (boxes, expected_area, label) in cases {
            assert_partition(boxes, Some(expected_area), label);
        }
    }

    #[test]
    fn visible_patch_partition_covers_general_partial_overlap() {
        assert_partition(
            [
                axis_box([0.0; 3], [1.0; 3]),
                axis_box([0.75, 0.25, 0.1], [0.75, 0.5, 0.6]),
            ],
            None,
            "general partial overlap",
        );
    }

    fn axis_box(center: [f64; 3], half_extents: [f64; 3]) -> AxisBox {
        AxisBox {
            center,
            half_extents,
        }
    }

    fn assert_partition(boxes: [AxisBox; 2], expected_area: Option<f64>, label: &str) {
        let patches = visible_union_patches(boxes);
        let reference = independent_boundary_cells(boxes);

        assert!(!patches.is_empty());
        assert!(patches.iter().all(|patch| {
            patch.u[0] < patch.u[1] && patch.v[0] < patch.v[1] && patch.coordinate.is_finite()
        }));
        for (index, first) in patches.iter().enumerate() {
            for second in &patches[index + 1..] {
                if first.axis != second.axis || first.coordinate != second.coordinate {
                    continue;
                }
                let overlap_u = first.u[0].max(second.u[0]) < first.u[1].min(second.u[1]);
                let overlap_v = first.v[0].max(second.v[0]) < first.v[1].min(second.v[1]);
                assert!(!(overlap_u && overlap_v), "patch interiors overlap");
            }
        }
        for cell in &reference {
            let point = patch_midpoint(*cell);
            assert_eq!(
                patches
                    .iter()
                    .filter(|patch| patch_contains(**patch, point))
                    .count(),
                1,
                "{label}: every independently classified boundary cell has one patch owner"
            );
        }
        let patch_area = patches.iter().map(|patch| area(*patch)).sum::<f64>();
        let reference_area = reference.iter().map(|patch| area(*patch)).sum::<f64>();
        assert!(
            (patch_area - reference_area).abs() <= 1.0e-12,
            "{label}: area partition"
        );
        if let Some(expected_area) = expected_area {
            assert!(
                (patch_area - expected_area).abs() <= 1.0e-12,
                "{label}: expected area"
            );
        }
    }

    fn independent_boundary_cells(boxes: [AxisBox; 2]) -> Vec<RectPatch> {
        let coordinates = core::array::from_fn::<_, 3, _>(|axis| {
            let mut values = boxes
                .iter()
                .flat_map(|value| [value.min()[axis], value.max()[axis]])
                .collect::<Vec<_>>();
            values.sort_by(f64::total_cmp);
            values.dedup();
            values
        });
        let mut occupied = Vec::new();
        for x in 0..coordinates[0].len() - 1 {
            for y in 0..coordinates[1].len() - 1 {
                for z in 0..coordinates[2].len() - 1 {
                    let index = [x, y, z];
                    let midpoint: [f64; 3] = core::array::from_fn(|axis| {
                        0.5 * (coordinates[axis][index[axis]] + coordinates[axis][index[axis] + 1])
                    });
                    if boxes.iter().any(|value| {
                        let min = value.min();
                        let max = value.max();
                        (0..3).all(|axis| midpoint[axis] > min[axis] && midpoint[axis] < max[axis])
                    }) {
                        occupied.push(index);
                    }
                }
            }
        }

        let mut boundary = Vec::new();
        for index in &occupied {
            for axis in 0..3 {
                let [u_axis, v_axis] = tangent_axes(axis);
                for maximum_side in [false, true] {
                    let neighbor = if maximum_side {
                        index[axis].checked_add(1)
                    } else {
                        index[axis].checked_sub(1)
                    };
                    let neighbor_occupied = neighbor.is_some_and(|value| {
                        let mut candidate = *index;
                        candidate[axis] = value;
                        occupied.contains(&candidate)
                    });
                    if !neighbor_occupied {
                        boundary.push(RectPatch {
                            axis,
                            coordinate: coordinates[axis][index[axis] + usize::from(maximum_side)],
                            u_axis,
                            v_axis,
                            u: [
                                coordinates[u_axis][index[u_axis]],
                                coordinates[u_axis][index[u_axis] + 1],
                            ],
                            v: [
                                coordinates[v_axis][index[v_axis]],
                                coordinates[v_axis][index[v_axis] + 1],
                            ],
                        });
                    }
                }
            }
        }
        boundary
    }

    fn patch_midpoint(patch: RectPatch) -> [f64; 3] {
        let mut point = [0.0; 3];
        point[patch.axis] = patch.coordinate;
        point[patch.u_axis] = 0.5 * (patch.u[0] + patch.u[1]);
        point[patch.v_axis] = 0.5 * (patch.v[0] + patch.v[1]);
        point
    }

    fn patch_contains(patch: RectPatch, point: [f64; 3]) -> bool {
        point[patch.axis] == patch.coordinate
            && point[patch.u_axis] > patch.u[0]
            && point[patch.u_axis] < patch.u[1]
            && point[patch.v_axis] > patch.v[0]
            && point[patch.v_axis] < patch.v[1]
    }

    fn area(patch: RectPatch) -> f64 {
        (patch.u[1] - patch.u[0]) * (patch.v[1] - patch.v[0])
    }
}
