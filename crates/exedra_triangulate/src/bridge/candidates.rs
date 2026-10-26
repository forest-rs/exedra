// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::vec::Vec;

/// Reused candidate-order scratch for one hole-splicing run.
pub(super) struct Candidates {
    positions: Vec<u32>,
    offsets: Vec<usize>,
}

impl Candidates {
    pub(super) fn new(point_count: usize, capacity: usize) -> Self {
        Self {
            positions: Vec::with_capacity(capacity),
            offsets: alloc::vec![0; point_count],
        }
    }

    pub(super) fn ordered<'a>(
        &'a mut self,
        points: &[[f64; 2]],
        ring: &[u32],
        anchor: [f64; 2],
        nearest: bool,
    ) -> &'a [u32] {
        self.positions.resize(ring.len(), 0);
        if nearest {
            for (position, value) in self.positions.iter_mut().enumerate() {
                *value = u32::try_from(position).expect("composite index budget checked");
            }
            self.positions.sort_unstable_by(|&a, &b| {
                let pa = points[ring[a as usize] as usize];
                let pb = points[ring[b as usize] as usize];
                let da = (pa[0] - anchor[0]) * (pa[0] - anchor[0])
                    + (pa[1] - anchor[1]) * (pa[1] - anchor[1]);
                let db = (pb[0] - anchor[0]) * (pb[0] - anchor[0])
                    + (pb[1] - anchor[1]) * (pb[1] - anchor[1]);
                da.partial_cmp(&db)
                    .expect("finite validated coordinate range")
                    .then(ring[a as usize].cmp(&ring[b as usize]))
                    .then(a.cmp(&b))
            });
        } else {
            // Input labels are dense. A stable counting pass gives precisely
            // (point label, ring position) order without comparison sorting.
            self.offsets.fill(0);
            for &point in ring {
                self.offsets[point as usize] += 1;
            }
            let mut offset = 0;
            for count in &mut self.offsets {
                let length = *count;
                *count = offset;
                offset += length;
            }
            for (position, &point) in ring.iter().enumerate() {
                let slot = &mut self.offsets[point as usize];
                self.positions[*slot] =
                    u32::try_from(position).expect("composite index budget checked");
                *slot += 1;
            }
        }
        &self.positions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reused_counting_order_matches_stable_labels_and_distance_order() {
        let points = [[0.0, 0.0], [2.0, 1.0], [-1.0, 2.0], [4.0, 0.0]];
        let mut scratch = Candidates::new(points.len(), 12);
        let positions_capacity = scratch.positions.capacity();
        let offsets_capacity = scratch.offsets.capacity();
        for ring in [&[3, 1, 0, 1, 3, 2][..], &[2, 0, 2, 1, 2, 3, 0][..]] {
            for nearest in [false, true] {
                let mut expected: Vec<_> = (0..u32::try_from(ring.len()).unwrap()).collect();
                if nearest {
                    expected.sort_unstable_by(|&a, &b| {
                        let pa = points[ring[a as usize] as usize];
                        let pb = points[ring[b as usize] as usize];
                        let da: f64 = pa[0] * pa[0] + pa[1] * pa[1];
                        let db: f64 = pb[0] * pb[0] + pb[1] * pb[1];
                        da.total_cmp(&db)
                            .then(ring[a as usize].cmp(&ring[b as usize]))
                            .then(a.cmp(&b))
                    });
                } else {
                    expected.sort_unstable_by_key(|&pos| (ring[pos as usize], pos));
                }
                assert_eq!(scratch.ordered(&points, ring, [0.0; 2], nearest), expected);
                assert_eq!(scratch.positions.capacity(), positions_capacity);
                assert_eq!(scratch.offsets.capacity(), offsets_capacity);
            }
        }
    }
}
