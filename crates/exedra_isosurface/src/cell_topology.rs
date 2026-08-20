// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic surface-component classification inside one cube cell.

extern crate alloc;

use alloc::vec::Vec;

use crate::CellHermiteData;

pub(crate) const CUBE_EDGES: [(usize, usize); 12] = [
    (0, 1),
    (1, 3),
    (2, 3),
    (0, 2),
    (4, 5),
    (5, 7),
    (6, 7),
    (4, 6),
    (0, 4),
    (1, 5),
    (3, 7),
    (2, 6),
];

#[derive(Copy, Clone)]
struct CubeFace {
    /// Corners in a canonical cycle shared by the low and high face on an
    /// axis. Edge slot `i` joins corner slot `i` to `(i + 1) % 4`.
    corners: [usize; 4],
    edges: [u8; 4],
}

const CUBE_FACES: [CubeFace; 6] = [
    // Z low/high, in XY order.
    CubeFace {
        corners: [0, 1, 3, 2],
        edges: [0, 1, 2, 3],
    },
    CubeFace {
        corners: [4, 5, 7, 6],
        edges: [4, 5, 6, 7],
    },
    // Y low/high, in XZ order.
    CubeFace {
        corners: [0, 1, 5, 4],
        edges: [0, 9, 4, 8],
    },
    CubeFace {
        corners: [2, 3, 7, 6],
        edges: [2, 10, 6, 11],
    },
    // X low/high, in YZ order.
    CubeFace {
        corners: [0, 2, 6, 4],
        edges: [3, 11, 7, 8],
    },
    CubeFace {
        corners: [1, 3, 7, 5],
        edges: [1, 10, 5, 9],
    },
];

/// One connected set of sign-changing cube edges.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) struct CellComponent {
    edge_mask: u16,
}

/// Sign connectivity for one cube cell.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CellTopology {
    components: Vec<CellComponent>,
    edge_components: [Option<u8>; 12],
    ambiguous_face_mask: u8,
}

impl CellTopology {
    pub(crate) fn component_for_edge(&self, edge: u8) -> Option<u8> {
        self.edge_components
            .get(usize::from(edge))
            .copied()
            .flatten()
    }

    pub(crate) const fn has_ambiguous_face(&self) -> bool {
        self.ambiguous_face_mask != 0
    }

    /// Partitions Hermite intersections without changing their edge order.
    pub(crate) fn partition_hermite(&self, hermite: &CellHermiteData) -> Vec<CellHermiteData> {
        let mut groups = Vec::with_capacity(self.components.len());
        for component in &self.components {
            debug_assert_ne!(
                component.edge_mask, 0,
                "classified components must contain a crossing edge"
            );
            groups.push(CellHermiteData::new(hermite.corner_signs));
        }

        for hit in &hermite.intersections {
            let component = self.component_for_edge(hit.edge_index);
            debug_assert!(
                component.is_some(),
                "Hermite edge {} must map to a classified crossing component",
                hit.edge_index
            );
            let Some(component) = component else {
                continue;
            };
            groups[usize::from(component)].push(hit.edge_index, hit.intersection);
        }
        groups
    }
}

/// Classifies cube-edge crossings into face-connected surface components.
///
/// Checkerboard faces use the bilinear asymptotic determinant
/// `f00 * f11 - f10 * f01`. A positive determinant pairs edge slots `(0, 1)`
/// and `(2, 3)`; a negative determinant pairs `(0, 3)` and `(1, 2)`. Exact
/// ties and non-finite determinants take the latter pairing. Because opposite
/// faces use the same canonical in-plane corner cycle, adjacent cells make the
/// same decision from the same four samples.
pub(crate) fn classify_cell(corner_values: &[f32; 8]) -> CellTopology {
    let mut parents = [u8::MAX; 12];
    for (edge, &(start, end)) in CUBE_EDGES.iter().enumerate() {
        if signs_differ(corner_values[start], corner_values[end]) {
            parents[edge] = u8::try_from(edge).expect("cube edge index fits in u8");
        }
    }

    let mut ambiguous_face_mask = 0_u8;
    for (face_index, face) in CUBE_FACES.iter().enumerate() {
        let mut crossings = [u8::MAX; 4];
        let mut crossing_count = 0_usize;
        for (slot, &edge) in face.edges.iter().enumerate() {
            if parents[usize::from(edge)] != u8::MAX {
                crossings[crossing_count] = u8::try_from(slot).expect("face edge slot fits in u8");
                crossing_count += 1;
            }
        }

        match crossing_count {
            0 => {}
            2 => union(
                &mut parents,
                face.edges[usize::from(crossings[0])],
                face.edges[usize::from(crossings[1])],
            ),
            4 => {
                ambiguous_face_mask |= 1_u8 << face_index;
                for (a, b) in checkerboard_connections(face_values(corner_values, *face)) {
                    union(
                        &mut parents,
                        face.edges[usize::from(a)],
                        face.edges[usize::from(b)],
                    );
                }
            }
            // Scalar-field values are expected to be finite. If a NaN reaches
            // this internal layer, retain the extractor's existing rule that
            // no incident edge crosses; any remaining unpaired crossings stay
            // in separate components instead of inventing NaN topology.
            1 | 3 => {}
            _ => unreachable!("a cube face has at most four edge slots"),
        }
    }

    let mut root_components = [None; 12];
    let mut components = Vec::new();
    let mut edge_components = [None; 12];
    for edge in 0..12 {
        if parents[edge] == u8::MAX {
            continue;
        }
        let root = usize::from(find(&parents, u8::try_from(edge).expect("edge fits in u8")));
        let component = match root_components[root] {
            Some(component) => component,
            None => {
                let component = u8::try_from(components.len()).expect("at most 12 components");
                root_components[root] = Some(component);
                components.push(CellComponent { edge_mask: 0 });
                component
            }
        };
        components[usize::from(component)].edge_mask |= 1_u16 << edge;
        edge_components[edge] = Some(component);
    }

    CellTopology {
        components,
        edge_components,
        ambiguous_face_mask,
    }
}

fn face_values(corner_values: &[f32; 8], face: CubeFace) -> [f32; 4] {
    face.corners.map(|corner| corner_values[corner])
}

fn checkerboard_connections(values: [f32; 4]) -> [(u8, u8); 2] {
    let determinant = values[0] * values[2] - values[1] * values[3];
    if determinant.is_finite() && determinant > 0.0 {
        [(0, 1), (2, 3)]
    } else {
        // The exact saddle tie has no topologically preferred branch. Pairing
        // around canonical corner slots 0 and 2 makes the choice independent
        // of cell ownership and remains unchanged under sign complement.
        [(0, 3), (1, 2)]
    }
}

fn signs_differ(a: f32, b: f32) -> bool {
    (a <= 0.0 && b > 0.0) || (a > 0.0 && b <= 0.0)
}

fn find(parents: &[u8; 12], mut edge: u8) -> u8 {
    loop {
        let parent = parents[usize::from(edge)];
        if parent == edge {
            return edge;
        }
        edge = parent;
    }
}

fn union(parents: &mut [u8; 12], a: u8, b: u8) {
    let a_root = find(parents, a);
    let b_root = find(parents, b);
    if a_root == b_root {
        return;
    }
    let (lower, higher) = if a_root < b_root {
        (a_root, b_root)
    } else {
        (b_root, a_root)
    };
    parents[usize::from(higher)] = lower;
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use super::{CUBE_EDGES, CUBE_FACES, checkerboard_connections, classify_cell, face_values};
    use crate::{CellHermiteData, HermiteIntersection};

    #[test]
    fn all_corner_masks_partition_exactly_the_crossing_edges_deterministically() {
        for mask in 0_u16..=u16::from(u8::MAX) {
            let values = values_for_mask(u8::try_from(mask).expect("mask is at most u8::MAX"));
            let first = classify_cell(&values);
            let second = classify_cell(&values);
            assert_eq!(first, second, "mask {mask:#04x}");

            let mut seen = 0_u16;
            let mut previous_minimum = None;
            for (component_index, component) in first.components.iter().enumerate() {
                assert_ne!(component.edge_mask, 0, "mask {mask:#04x}");
                assert_eq!(seen & component.edge_mask, 0, "mask {mask:#04x}");
                let minimum = component.edge_mask.trailing_zeros();
                if let Some(previous) = previous_minimum {
                    assert!(
                        minimum > previous,
                        "component minima must increase for mask {mask:#04x}: {previous} then {minimum}"
                    );
                }
                previous_minimum = Some(minimum);
                seen |= component.edge_mask;
                for edge in 0_u8..12 {
                    if component.edge_mask & (1_u16 << edge) != 0 {
                        assert_eq!(
                            first.component_for_edge(edge),
                            Some(u8::try_from(component_index).expect("at most 12 components"),),
                            "mask {mask:#04x}, edge {edge}"
                        );
                    }
                }
            }

            let expected = crossing_mask(&values);
            assert_eq!(seen, expected, "mask {mask:#04x}");
            for edge in 0_u8..12 {
                assert_eq!(
                    first.component_for_edge(edge).is_some(),
                    expected & (1_u16 << edge) != 0,
                    "mask {mask:#04x}, edge {edge}"
                );
            }
        }
    }

    #[test]
    fn opposite_inside_corners_form_two_ordered_components() {
        let topology = classify_cell(&values_for_mask(0b1000_0001));
        let masks: Vec<_> = topology
            .components
            .iter()
            .map(|component| component.edge_mask)
            .collect();
        assert_eq!(masks, vec![0b0001_0000_1001, 0b0100_0110_0000]);
    }

    #[test]
    fn complement_preserves_connectivity() {
        for mask in 0_u16..=u16::from(u8::MAX) {
            let values = core::array::from_fn(|corner| {
                let magnitude = corner as f32 + 1.0;
                if mask & (1_u16 << corner) != 0 {
                    -magnitude
                } else {
                    magnitude
                }
            });
            let complemented = values.map(|value| -value);
            assert_eq!(
                classify_cell(&values),
                classify_cell(&complemented),
                "mask {mask:#04x}"
            );
        }
    }

    #[test]
    fn checkerboard_face_decision_matches_every_adjacent_cell_view() {
        for shared in [
            [2.0, -1.0, 3.0, -1.5],
            [1.0, -3.0, 1.5, -2.0],
            [1.0, -1.0, 1.0, -1.0],
        ] {
            for [low_face, high_face] in [[0, 1], [2, 3], [4, 5]] {
                let mut low_cell = [1.0; 8];
                let mut high_cell = [1.0; 8];
                for (slot, value) in shared.into_iter().enumerate() {
                    low_cell[CUBE_FACES[high_face].corners[slot]] = value;
                    high_cell[CUBE_FACES[low_face].corners[slot]] = value;
                }

                let low_view = face_values(&low_cell, CUBE_FACES[high_face]);
                let high_view = face_values(&high_cell, CUBE_FACES[low_face]);
                assert_eq!(low_view, shared);
                assert_eq!(high_view, shared);
                assert_eq!(
                    checkerboard_connections(low_view),
                    checkerboard_connections(high_view),
                    "faces {low_face}/{high_face}, values {shared:?}"
                );
            }
        }
    }

    #[test]
    fn exact_checkerboard_tie_uses_canonical_pairing() {
        assert_eq!(
            checkerboard_connections([1.0, -1.0, 1.0, -1.0]),
            [(0, 3), (1, 2)]
        );
        assert_eq!(
            checkerboard_connections([-1.0, 1.0, -1.0, 1.0]),
            [(0, 3), (1, 2)]
        );
    }

    #[test]
    fn non_finite_checkerboard_determinant_uses_canonical_pairing() {
        for values in [
            [f32::MAX, -1.0, f32::MAX, -1.0],
            [f32::MAX, -f32::MAX, f32::MAX, -f32::MAX],
        ] {
            assert_eq!(
                checkerboard_connections(values),
                [(0, 3), (1, 2)],
                "values {values:?}"
            );
        }
    }

    #[test]
    fn every_nan_corner_keeps_the_existing_no_incident_crossing_rule() {
        for nan_corner in 0..8 {
            let mut values = values_for_mask(0b0101_1010);
            values[nan_corner] = f32::NAN;
            let topology = classify_cell(&values);

            for (edge, &(start, end)) in CUBE_EDGES.iter().enumerate() {
                if start == nan_corner || end == nan_corner {
                    assert_eq!(
                        topology.component_for_edge(
                            u8::try_from(edge).expect("cube edge index fits in u8")
                        ),
                        None,
                        "edge {edge} is incident to NaN corner {nan_corner}"
                    );
                }
            }
        }
    }

    #[test]
    fn hermite_partition_keeps_each_sample_with_its_component() {
        let signs = 0b1000_0001;
        let topology = classify_cell(&values_for_mask(signs));
        let mut hermite = CellHermiteData::new(signs);
        for edge in 0_u8..12 {
            if topology.component_for_edge(edge).is_some() {
                hermite.push(
                    edge,
                    HermiteIntersection {
                        position: [f32::from(edge), 0.0, 0.0],
                        normal: [1.0, 0.0, 0.0],
                        t: 0.5,
                    },
                );
            }
        }

        let groups = topology.partition_hermite(&hermite);
        assert_eq!(groups.len(), 2);
        for (component, group) in topology.components.iter().zip(&groups) {
            let actual = group
                .intersections
                .iter()
                .fold(0_u16, |mask, hit| mask | (1_u16 << hit.edge_index));
            assert_eq!(actual, component.edge_mask);
        }
    }

    fn values_for_mask(mask: u8) -> [f32; 8] {
        core::array::from_fn(|corner| {
            if mask & (1_u8 << corner) != 0 {
                -1.0
            } else {
                1.0
            }
        })
    }

    fn crossing_mask(values: &[f32; 8]) -> u16 {
        CUBE_EDGES
            .iter()
            .enumerate()
            .fold(0_u16, |mask, (edge, &(start, end))| {
                if (values[start] <= 0.0 && values[end] > 0.0)
                    || (values[start] > 0.0 && values[end] <= 0.0)
                {
                    mask | (1_u16 << edge)
                } else {
                    mask
                }
            })
    }
}
